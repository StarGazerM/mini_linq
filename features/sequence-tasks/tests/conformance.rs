use cq_lang_sequence::Program;
use cq_testkit::conformance::{Target, assert_conformant, assert_conformant_with_inputs};

const TARGETS: &[Target] = &[Target::Sequential, Target::Tasks];

// Complete the authored provider packet before target-specific planning.
fn program(mut program: Program) -> Program {
    let providers = program.structures.structures.entries.clone();
    program.structures = cq_structure_selection::compile(&program.structures.physical).unwrap();
    for provider in providers {
        program.structures.structures.entries.retain(|entry| {
            entry.name != provider.name
                || std::mem::discriminant(&entry.structure)
                    != std::mem::discriminant(&provider.structure)
        });
        program.structures.structures.entries.push(provider);
    }
    program
}

#[test]
fn host_hash_lookup_store_keeps_exact_capability_on_tasks_machine() {
    let program = program(syn::parse_quote! {
        physical {
            relation L(a:i32); relation R(a:i32,b:i32);
            consumer canonical;
            l = pipe rename L@0 {a -> a};
            r = let rename R@1 {a -> a,b -> b};
            j = pipe probe first [a] l,r;
            return j as answer(a,b);
        }
        structures {}
        sequence {
            r = store rows r;
            input = rows l;
            joined = store flatten lookup r [a] input;
            result = store joined;
        }
    });
    let (_, analysis) = cq_testkit::conformance::plan(&program, Target::Tasks).unwrap();
    assert_eq!(
        analysis.capabilities[&("r".into(), vec!["a".into()])],
        cq_lang_sequence::Capability::Exact,
    );
    assert_conformant(&program, TARGETS);
    assert_conformant_with_inputs(
        &program,
        TARGETS,
        &[
            ("L", &[&[1], &[1], &[2]]),
            ("R", &[&[1, 10], &[1, 10], &[2, 20]]),
        ],
    );
}

#[test]
fn join() {
    let program = program(syn::parse_quote! {
        physical {
            relation L(a:i32); relation R(a:i32,b:i32);
            l = pipe rename L@0 {a -> a};
            r = let rename R@1 {a -> a,b -> b};
            j = pipe canon probe first [a] l,r;
            return j as answer(a,b);
        } structures { r = ::cq_storage::SortedRun<_, _> [a]; }
        sequence {
            r = store rows r;
            input = store rows l;
            result = store tile 2 canon flatten lookup r [a] input;
        }
    });
    assert_conformant(&program, TARGETS);
    assert_conformant_with_inputs(
        &program,
        TARGETS,
        &[
            ("L", &[&[1], &[2], &[1], &[9]]),
            ("R", &[&[1, 10], &[1, 11], &[2, 20], &[4, 40]]),
        ],
    );
}

#[test]
fn chain() {
    let program = program(syn::parse_quote! {
        physical {
            relation L(a:i32); relation R(a:i32,b:i32); relation S(b:i32,c:i32);
            l = pipe rename L@0 {a -> a};
            r = let rename R@1 {a -> a,b -> b};
            s = let rename S@2 {b -> b,c -> c};
            j = pipe probe first [a] l,r;
            k = pipe canon probe first [b] j,s;
            return k as answer(a,b,c);
        } structures {
            r = ::cq_storage::SortedRun<_, _> [a];
            s = ::cq_storage::SortedRun<_, _> [b];
        }
        sequence {
            r = store rows r;
            s = store rows s;
            input = store rows l;
            result = store tile 2 canon flatten lookup s [b] flatten lookup r [a] input;
        }
    });
    assert_conformant_with_inputs(
        &program,
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
            relation L(a:i32); relation R(a:i32);
            l = pipe rename L@0 {a -> a};
            r = let rename R@1 {a -> a};
            j = pipe canon semi eager [a] l,r;
            return j as answer(a);
        } structures {}
        sequence {
            r = store rows r;
            input = store rows l;
            result = store tile 2 canon compact j lookup r [a] input;
        }
    });
    assert_conformant_with_inputs(
        &program,
        TARGETS,
        &[("L", &[&[1], &[2], &[1], &[3]]), ("R", &[&[1], &[1], &[3]])],
    );
}

#[test]
fn canon_prefilters_parts_then_deduplicates_globally() {
    let program = program(syn::parse_quote! {
        physical {
            relation R(a:i32);
            r = pipe canon rename R@0 {a -> a};
            return r as answer(a);
        } structures {}
        sequence { input = store rows r; result = store canon input; }
    });
    // Duplicate rows straddle Rayon partition boundaries.
    let rows: Vec<_> = (0..5000).map(|index| [index % 7]).collect();
    let borrowed: Vec<_> = rows.iter().map(|row| row.as_slice()).collect();
    assert_conformant_with_inputs(&program, TARGETS, &[("R", &borrowed)]);
}

#[test]
fn empty_and_partial_tiles_have_equal_answers() {
    let program = program(syn::parse_quote! {
        physical {
            relation R(a:i32);
            r = pipe canon rename R@0 {a -> a};
            return r as answer(a);
        } structures {}
        sequence { input = store rows r; result = store tile 2 canon input; }
    });
    for rows in [
        &[][..],
        &[&[4][..]][..],
        &[&[3][..], &[1], &[4], &[2], &[5]][..],
    ] {
        assert_conformant_with_inputs(&program, TARGETS, &[("R", rows)]);
    }
}
