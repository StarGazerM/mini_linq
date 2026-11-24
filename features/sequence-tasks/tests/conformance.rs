use cq_lang_sequence::structures;
use quote::ToTokens;

// Authored fixtures retain each language layer independently of Sequence IR.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Fixture {
    physical: structures::physical::Program,
    structures: structures::Program,
    program: cq_lang_sequence::Program,
}

impl syn::parse::Parse for Fixture {
    fn parse(input: syn::parse::ParseStream<'_>) -> syn::Result<Self> {
        let physical = input.parse()?;
        let structures = structures::Program::parse(input, &physical)?;
        let program = input.parse()?;
        Ok(Self { physical, structures, program })
    }
}

impl From<(structures::physical::Program, structures::Program, cq_lang_sequence::Program)> for Fixture {
    fn from((physical, structures, program): (structures::physical::Program, structures::Program, cq_lang_sequence::Program)) -> Self {
        Self { physical, structures, program }
    }
}

impl ToTokens for Fixture {
    fn to_tokens(&self, tokens: &mut proc_macro2::TokenStream) {
        self.physical.to_tokens(tokens);
        self.structures.to_tokens(tokens);
        self.program.to_tokens(tokens);
    }
}

use cq_sequence_conformance::{Target, assert_conformant, assert_conformant_with_inputs};

const TARGETS: &[Target] = &[Target::Sequential, Target::Tasks];

// Complete the authored provider packet before target-specific planning.
fn program(mut program: Fixture) -> Fixture {
    let authored = program.structures.structures.clone();
    program.structures = cq_structure_selection::compile(&program.physical).unwrap();
    for provider in authored.entries {
        program.structures.structures.entries.retain(|entry| {
            entry.name != provider.name
                || std::mem::discriminant(&entry.structure)
                    != std::mem::discriminant(&provider.structure)
        });
        program.structures.structures.entries.push(provider);
    }
    program
        .structures
        .structures
        .declare(&authored.providers.declarations).unwrap();
    program
}

#[test]
fn named_tile_pipeline_preserves_rows_across_join() {
    let program = program(syn::parse_quote! {
        physical {
            relation R(a:i32,witness:i32);
            r = pipe rename R {a -> a,witness -> witness};
            out = let canon project r keep {a};
            return out as answer(a);
        }
        structures { providers {  }}
        sequence {
            renamed_input = rows r;
            renamed_cover = tile 2 renamed_input;
            renamed_rows = flatten renamed_cover;
            result = store canon map out renamed_rows;
        }
    });
    assert_conformant_with_inputs(&program.physical, &program.structures, &program.program, TARGETS, &[("R", &[&[3,0], &[1,1], &[3,2], &[2,3], &[9,4]])]);
}

#[test]
fn host_hash_lookup_store_keeps_exact_capability_on_tasks_machine() {
    let program = program(syn::parse_quote! {
        physical {
            relation L(a:i32); relation R(a:i32,b:i32);
            consumer canonical;
            l = pipe rename L {a -> a};
            r = let rename R {a -> a,b -> b};
            j = pipe probe first [a] l,r;
            return j as answer(a,b);
        }
        structures { providers {  }}
        sequence {
            r = store rows r;
            input = rows l;
            joined = store flatten lookup r [a] input;
            result = store joined;
        }
    });
    let (_, _, _, analysis) = cq_sequence_conformance::plan(&program.physical, &program.structures, &program.program, Target::Tasks).unwrap();
    assert_eq!(
        analysis.capabilities[&("r".into(), vec!["a".into()])],
        cq_lang_sequence::structures::Capability::Exact,
    );
    assert_conformant(&program.physical, &program.structures, &program.program, TARGETS);
    assert_conformant_with_inputs(&program.physical, &program.structures, &program.program,
        TARGETS,
        &[
            ("L", &[&[1], &[2]]),
            ("R", &[&[1, 10], &[2, 20]]),
        ],
    );
}

#[test]
fn join() {
    let program = program(syn::parse_quote! {
        physical {
            relation L(a:i32,witness:i32); relation R(a:i32,b:i32);
            raw = pipe rename L {a -> a,witness -> witness};
            l = pipe project raw keep {a};
            r = let rename R {a -> a,b -> b};
            j = let canon probe first [a] l,r;
            return j as answer(a,b);
        } structures { providers { sorted_run = ::cq_storage::SortedRun<_, _> { reads [exact, range, membership], supports [retain, expand, keys], build typed, join_filter, run_filter, shared, statistics_build [rows, max_group], statistics [ rows: rows occurrences exact run_finalize stat_rows host required build single, keys: keys distinct_keys exact run_finalize stat_keys host required build single, max_group: max_group max_bucket_occurrences exact run_finalize stat_max_group host required build single, runs: runs runs exact run_finalize stat_runs host required build single ] }; } r = @sorted_run as ::cq_storage::SortedRun<_, _> [a]; }
        sequence {
            r = store rows r;
            input = store map l rows raw;
            result = store tile 2 canon flatten lookup r [a] input;
        }
    });
    assert_conformant(&program.physical, &program.structures, &program.program, TARGETS);
    assert_conformant_with_inputs(&program.physical, &program.structures, &program.program,
        TARGETS,
        &[
            ("L", &[&[1,0], &[2,1], &[1,2], &[9,3]]),
            ("R", &[&[1, 10], &[1, 11], &[2, 20], &[4, 40]]),
        ],
    );
}

#[test]
fn chain() {
    let program = program(syn::parse_quote! {
        physical {
            relation L(a:i32); relation R(a:i32,b:i32); relation S(b:i32,c:i32);
            l = pipe rename L {a -> a};
            r = let rename R {a -> a,b -> b};
            s = let rename S {b -> b,c -> c};
            j = pipe probe first [a] l,r;
            k = let canon probe first [b] j,s;
            return k as answer(a,b,c);
        } structures { providers { sorted_run = ::cq_storage::SortedRun<_, _> { reads [exact, range, membership], supports [retain, expand, keys], build typed, join_filter, run_filter, shared, statistics_build [rows, max_group], statistics [ rows: rows occurrences exact run_finalize stat_rows host required build single, keys: keys distinct_keys exact run_finalize stat_keys host required build single, max_group: max_group max_bucket_occurrences exact run_finalize stat_max_group host required build single, runs: runs runs exact run_finalize stat_runs host required build single ] }; }
            r = @sorted_run as ::cq_storage::SortedRun<_, _> [a];
            s = @sorted_run as ::cq_storage::SortedRun<_, _> [b];
        }
        sequence {
            r = store rows r;
            s = store rows s;
            input = store rows l;
            result = store tile 2 canon flatten lookup s [b] flatten lookup r [a] input;
        }
    });
    assert_conformant_with_inputs(&program.physical, &program.structures, &program.program,
        TARGETS,
        &[
            ("L", &[&[1], &[2], &[3]]),
            ("R", &[&[1, 10], &[1, 11], &[2, 20]]),
            ("S", &[&[10, 100], &[11, 110], &[20, 200], &[10, 101]]),
        ],
    );
}

#[test]
fn semi() {
    let program = program(syn::parse_quote! {
        physical {
            relation L(a:i32,witness:i32); relation R(a:i32,witness:i32);
            raw_l = pipe rename L {a -> a,witness -> left_witness};
            l = pipe project raw_l keep {a};
            raw_r = pipe rename R {a -> a,witness -> right_witness};
            r = let project raw_r keep {a};
            j = let canon semi eager [a] l,r;
            return j as answer(a);
        } structures { providers {  }}
        sequence {
            r = store map r rows raw_r;
            input = store map l rows raw_l;
            result = store tile 2 canon compact j lookup r [a] input;
        }
    });
    assert_conformant_with_inputs(&program.physical, &program.structures, &program.program,
        TARGETS,
        &[("L", &[&[1,0], &[2,1], &[1,2], &[3,3]]), ("R", &[&[1,0], &[1,1], &[3,2]])],
    );
}

#[test]
fn canon_sorts_the_whole_stream_across_task_boundaries() {
    let program = program(syn::parse_quote! {
        physical {
            relation R(a:i32,witness:i32);
            raw = pipe rename R {a -> a,witness -> witness};
            r = let canon project raw keep {a};
            return r as answer(a);
        } structures { providers {  }}
        sequence {
            input = map r rows raw;
            parts = tile 64 input;
            joined = flatten parts;
            result = store canon joined;
        }
    });
    // Duplicate projected rows straddle Rayon partition boundaries.
    let rows: Vec<_> = (0..5000).map(|index| [index % 7, index]).collect();
    let borrowed: Vec<_> = rows.iter().map(|row| row.as_slice()).collect();
    assert_conformant_with_inputs(&program.physical, &program.structures, &program.program, TARGETS, &[("R", &borrowed)]);
}

#[test]
fn empty_and_partial_tiles_have_equal_answers() {
    let program = program(syn::parse_quote! {
        physical {
            relation R(a:i32);
            r = let canon rename R {a -> a};
            return r as answer(a);
        } structures { providers {  }}
        sequence { input = store rows r; result = store tile 2 canon input; }
    });
    for rows in [
        &[][..],
        &[&[4][..]][..],
        &[&[3][..], &[1], &[4], &[2], &[5]][..],
    ] {
        assert_conformant_with_inputs(&program.physical, &program.structures, &program.program, TARGETS, &[("R", rows)]);
    }
}
