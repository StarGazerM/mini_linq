use cq_lang_sequence::structures::Capability;
use cq_lang_sequence::{LevelNode, Machine, OutputProtocol, Program, Term};
use cq_sequence_lowering::{check, compile};

struct Stack {
    physical: cq_lang_structures::physical::Program,
    structures: cq_lang_structures::Program,
}

impl syn::parse::Parse for Stack {
    fn parse(input: syn::parse::ParseStream<'_>) -> syn::Result<Self> {
        let physical = input.parse()?;
        let structures = cq_lang_structures::Program::parse(input, &physical)?;
        Ok(Self { physical, structures })
    }
}

fn accept(physical: &cq_lang_structures::physical::Program, source: &cq_lang_structures::Program) -> Program {
    let target = compile(physical, source).unwrap();
    assert!(target.graded.is_none());
    assert_eq!(target.sequence.output, Some(OutputProtocol::Count));
    cq_lang_sequence::contract::check(physical, source, &target).unwrap();
    cq_lang_sequence::check_grading(
        physical,
        source,
        &target,
        &Machine {
            gpu: None,
            layout: None,
            name: syn::parse_quote!(sequential),
            items: 1,
            key_partition_threshold: 65536,
            key_partition_cache_threshold: 6 * 1024 * 1024,
            root: LevelNode {
                shared_bytes: None,
                name: syn::parse_quote!(host),
                host_rust: true,
                collaborative: false,
                dynamic: false,
                width: None,
                children: vec![],
            },
        },
    )
    .unwrap();
    check(physical, source, &target).unwrap();
    target
}

#[test]
fn every_authored_structures_scenario() {
    for scenario in cq_testkit::SCENARIOS {
        let physical = scenario.parse_physical().unwrap();
        accept(&physical, &scenario.parse_structures(&physical).unwrap());
    }
}

#[test]
fn probe_shape_and_structural_checker_reject_mutations() {
    let Stack { physical, structures: source }: Stack = syn::parse_quote! {
        physical {
            relation L(a:i32); relation R(a:i32,b:i32);
            l = pipe rename L {a -> a};
            r = let rename R {a -> a,b -> b};
            j = pipe probe first [a] l,r;
            out = let canon project j keep {b};
            return out as answer(b);
        } structures { providers { reader = Reader { reads [exact, range, membership], build inferred }; provider = Provider { reads [exact, range, membership], build inferred }; sort = ::cq_storage::dedup::Sort { reads [distinct], build inferred, shared }; }
            l host scan = @reader as Reader []; r host scan = @reader as Reader [];
            r = @provider as Provider [a]; out rows = @sort as ::cq_storage::dedup::Sort; out canon = rows;
        }
    };
    let target = accept(&physical, &source);
    let expected: Term = syn::parse_quote!(flatten lookup r [a] l);
    assert_eq!(target.sequence.definitions[2].term, expected);
    let analysis = cq_lang_sequence::contract::check(&physical, &source, &target).unwrap();
    assert_eq!(
        analysis.capabilities[&("r".into(), vec!["a".into()])],
        Capability::Exact
    );
    let mut changed = target.clone();
    let Term::Flatten(inner) = changed.sequence.definitions[2].term.clone() else {
        panic!()
    };
    changed.sequence.definitions[2].term = *inner;
    assert!(check(&physical, &source, &changed).is_err());
    let mut changed = target.clone();
    changed.sequence.output = Some(OutputProtocol::Reserve);
    assert!(check(&physical, &source, &changed).is_err());
    let mut changed = target;
    changed.sequence.definitions.swap(0, 1);
    assert!(check(&physical, &source, &changed).is_err());
}

#[test]
fn correspondence_accepts_authored_output_names_but_not_another_result() {
    let Stack { physical, structures: source }: Stack = syn::parse_quote! {
        physical {
            relation R(a:i32); relation S(a:i32);
            r = let canon rename R {a -> a};
            s = let canon rename S {a -> a};
            u = let canon union r,s;
            return u as answer(a);
        } structures { providers { reader = Reader { reads [exact, range, membership], build inferred }; sort = ::cq_storage::dedup::Sort { reads [distinct], build inferred, shared }; }
            r host scan = @reader as Reader []; s host scan = @reader as Reader [];
            r rows = @sort as ::cq_storage::dedup::Sort; r canon = rows; s rows = @sort as ::cq_storage::dedup::Sort; s canon = rows; u rows = @sort as ::cq_storage::dedup::Sort; u canon = rows;
        }
    };
    let target: Program = syn::parse_quote! {
        sequence {
            r = canon rows r;
            s = canon rows s;
            u = canon union [r,s];
            authored_output = store u;
            output count;
        }
    };
    check(&physical, &source, &target).unwrap();
    let mut wrong = target;
    wrong.sequence.definitions.last_mut().unwrap().term = syn::parse_quote!(store s);
    assert!(check(&physical, &source, &wrong).is_err());
}

#[test]
fn output_name_is_fresh_and_return_canon_is_preserved() {
    let Stack { physical, structures: source }: Stack = syn::parse_quote! {
        physical {
            relation R(a:i32);
            __sequence_output = pipe rename R {a -> a};
            return canon __sequence_output as answer(a);
        } structures { providers { reader = Reader { reads [exact, range, membership], build inferred }; sort = ::cq_storage::dedup::Sort { reads [distinct], build inferred, shared }; }
            __sequence_output host scan = @reader as Reader [];
            __sequence_output head = @sort as ::cq_storage::dedup::Sort;
        }
    };
    let target = accept(&physical, &source);
    let last = target.sequence.definitions.last().unwrap();
    assert_eq!(last.name, "__sequence_output_");
    assert!(
        matches!(&last.term, Term::Store { capacity: None, inner } if matches!(inner.as_ref(), Term::Canon(_)))
    );
}

#[test]
fn merge_uses_lookup_with_both_ordered_provider_obligations() {
    let physical = syn::parse_quote! {
        physical {
            relation L(a:i32); relation R(a:i32,b:i32);
            l = pipe rename L {a -> a};
            r = let rename R {a -> a,b -> b};
            j = let canon merge [a] l,r;
            return j as answer(a,b);
        }
    };
    let source = cq_structure_selection::compile(&physical).unwrap();
    let target = accept(&physical, &source);
    assert!(matches!(&target.sequence.definitions[2].term,
        Term::Canon(inner) if matches!(inner.as_ref(), Term::Flatten(_))));
    let analysis = cq_lang_sequence::contract::check(&physical, &source, &target).unwrap();
    for provider in ["l", "r"] {
        assert_eq!(
            analysis.capabilities[&(provider.into(), vec!["a".into()])],
            Capability::Ordered,
        );
    }
    assert_eq!(analysis.results["j"], "j");
}

#[test]
fn groups_lower_to_segmented_folds_after_complete_valuation_canon() {
    let physical = syn::parse_quote! {
        physical {
            relation R(k:i32, witness:i32, value:i64, anonymous:i32);
            raw = pipe rename R {k -> k, witness -> witness, value -> value, anonymous -> anonymous};
            valuations = let canon project raw keep {k, witness, value};
            grouped = pipe group valuations by [k] {
                n: usize = count();
                total: i64 = sum(value);
                low: Option<i64> = min(value);
                high: Option<i64> = max(value);
            };
            return grouped as answer(k,n,total,low,high);
        }
    };
    let source = cq_structure_selection::compile(&physical).unwrap();
    let target = accept(&physical, &source);
    assert_eq!(
        target.sequence.definitions[1].term,
        syn::parse_quote!(canon map valuations raw)
    );
    assert_eq!(
        target.sequence.definitions[2].term,
        syn::parse_quote!(group grouped valuations)
    );
    let mut changed = target;
    changed.sequence.definitions[2].term = syn::parse_quote!(map grouped valuations);
    assert!(check(&physical, &source, &changed).is_err());
    assert!(
        cq_lang_sequence::contract::check(&physical, &source, &changed)
            .unwrap_err()
            .to_string()
            .contains("BE-Group")
    );
}

#[test]
fn input_valuation_set_needs_no_redundant_group_input_canon() {
    let physical = syn::parse_quote! {
        physical {
            relation R(k:i32, witness:i32);
            r = pipe rename R {k -> k, witness -> witness};
            grouped = pipe group r by [k] { n: usize = count(); };
            return grouped as answer(k,n);
        }
    };
    let source = cq_structure_selection::compile(&physical).unwrap();
    let target = accept(&physical, &source);
    assert_eq!(
        target.sequence.definitions[0].term,
        syn::parse_quote!(rows r)
    );
    assert_eq!(
        target.sequence.definitions[1].term,
        syn::parse_quote!(group grouped r)
    );
}

#[test]
fn unique_fallback_union_remains_unique_through_projection_into_group() {
    let physical = syn::parse_quote! { physical {
        relation R(x:i32,y:i32); relation S(x:i32,y:i32);
        r = pipe rename R {x -> x,y -> y};
        s = pipe rename S {x -> x,y -> y};
        a = pipe anti eager [x] s,r;
        u = pipe union r,a block free[x] interface[y] fallback;
        p = pipe project u keep {x,y};
        grouped = pipe group p by [] { n:usize = count(); };
        return grouped as answer(n);
    }};
    let structures = cq_structure_selection::compile(&physical).unwrap();
    let target = accept(&physical, &structures);
    assert_eq!(
        target.sequence.definitions[3].term,
        Term::Union(vec![Term::Ref(syn::parse_quote!(r)), Term::Ref(syn::parse_quote!(a))])
    );
    assert_eq!(target.sequence.definitions[4].term, syn::parse_quote!(map p u));
    assert_eq!(target.sequence.definitions[5].term, syn::parse_quote!(group grouped p));
    let mut overlapping = physical.clone();
    let cq_lang_structures::physical::Operator::Union(union) =
        &mut overlapping.program.definitions[3].operator else { unreachable!() };
    union.origin = None;
    overlapping.program.definitions[2].operator = syn::parse_quote!(semi eager [x,y] s,r);
    assert!(cq_structure_selection::compile(&overlapping).is_err());
    overlapping.program.definitions[3].canon = Some(Default::default());
    overlapping.program.definitions[3].mode = syn::parse_quote!(let);
    let structures = cq_structure_selection::compile(&overlapping).unwrap();
    let mut missing = accept(&overlapping, &structures);
    missing.sequence.definitions[3].term = target.sequence.definitions[3].term.clone();
    assert!(cq_lang_sequence::contract::check(&overlapping, &structures, &missing).is_err());
}

#[test]
fn projected_membership_probe_lowers_to_flat_compact_before_expansion() {
    let physical = syn::parse_quote! {
        physical {
            relation L(a:i32,b:i32); relation R(a:i32,unused:i32); relation T(b:i32,c:i32);
            l = pipe rename L {a -> a,b -> b};
            r = let rename R {a -> a,unused -> unused};
            t = let rename T {b -> b,c -> c};
            member = pipe probe first [a] l,r;
            expanded = pipe probe first [b] member,t;
            out = let canon project expanded keep {a,b,c};
            return out as answer(a,b,c);
        }
    };
    let structures = cq_structure_selection::compile(&physical).unwrap();
    let target = accept(&physical, &structures);
    let facts = cq_lang_sequence::contract::check(&physical, &structures, &target).unwrap();
    let structures_analysis = cq_lang_structures::contract::check(&physical, &structures).unwrap();
    assert!(
        structures_analysis
            .membership
            .contains(&("r".into(), vec!["a".into()]))
    );
    assert_eq!(
        target.sequence.definitions[3].term,
        syn::parse_quote!(compact member lookup r [a] l)
    );
    assert_eq!(
        target.sequence.definitions[4].term,
        syn::parse_quote!(flatten lookup t [b] member)
    );
    assert_eq!(facts.results["member"], "member");
    assert_eq!(
        facts.capabilities[&("r".into(), vec!["a".into()])],
        Capability::Membership
    );
}

#[test]
fn raw_full_key_host_membership_lowers_to_compact_without_type_inspection() {
    for key_type in [syn::parse_quote!(i32), syn::parse_quote!(StringCode)] {
        let key_type: syn::Type = key_type;
        let physical = syn::parse_quote! {
            physical {
                relation L(a:#key_type,b:i32,c:i32);
                relation R(a:#key_type,b:i32);
                relation T(c:i32,d:i32);
                l = pipe rename L {a -> a,b -> b,c -> c};
                r = pipe rename R {a -> a,b -> b};
                t = let rename T {c -> c,d -> d};
                member = pipe probe first [a,b] l,r;
                expanded = pipe probe first [c] member,t;
                out = let canon project expanded keep {a,b,c,d};
                return out as answer(a,b,c,d);
            }
        };
        let structures = cq_structure_selection::compile(&physical).unwrap();
        let target = accept(&physical, &structures);
        let facts = cq_lang_sequence::contract::check(&physical, &structures, &target).unwrap();
        let structures_analysis = cq_lang_structures::contract::check(&physical, &structures).unwrap();
        let key = ("r".into(), vec!["a".into(), "b".into()]);
        assert!(structures_analysis.membership.contains(&key));
        assert_eq!(facts.capabilities[&key], Capability::Membership);
        assert_eq!(
            target.sequence.definitions[3].term,
            syn::parse_quote!(compact member lookup r [a,b] l)
        );
        assert_eq!(
            target.sequence.definitions[4].term,
            syn::parse_quote!(flatten lookup t [c] member)
        );
        assert!(structures.structures.entries.iter().any(|entry| {
            entry.name == "r"
                && matches!(&entry.structure,
                    cq_lang_structures::Structure::Host { keyed, .. }
                    if keyed.membership.is_some() && keyed.filtered.is_none())
        }));

        let mut expanded_membership = target.clone();
        expanded_membership.sequence.definitions[3].term =
            syn::parse_quote!(flatten lookup r [a,b] l);
        assert!(check(&physical, &structures, &expanded_membership).is_err());

        // A Rust type, even the selected membership implementation, supplies no proof.
        let mut uncertified = structures;
        for entry in &mut uncertified.structures.entries {
            if entry.name == "r"
                && let cq_lang_structures::Structure::Host { keyed, .. } = &mut entry.structure
            {
                keyed.membership = None;
            }
        }
        cq_lang_structures::contract::check(&physical, &uncertified).unwrap();
        assert!(cq_lang_sequence::contract::check(&physical, &uncertified, &target).is_err());
    }
}

fn blocked_source() -> (cq_lang_structures::physical::Program, cq_lang_structures::Program) {
    let physical = syn::parse_quote!(physical {
        relation R(a: i64 [1], b: i64 [1]);
        r = pipe rename R {a -> x, b -> y};
        p = pipe range select r if (x >= 3);
        q = pipe range select p if (y < 9);
        g = pipe guard q if (opaque(x));
        t = pipe range select g if (y >= 1);
        return canon t as answer(x,y);
    });
    cq_structure_selection::compile_with(&physical, syn::parse_quote!(
        blocks(R { block_rows 1024; summary none; columns [a: bytes u64, b: plain u64]; })
    )).unwrap()
}

#[test]
fn blocked_execution_roundtrips_and_accepts_lawful_shorter_runs() {
    let (physical, source) = blocked_source();
    let target = accept(&physical, &source);
    assert_eq!(target.representation.blocks.runs["r"], ["p", "q"]);
    assert_eq!(target.representation.blocks.runs["t"], ["t"]);
    let printed = syn::parse_quote!(#target);
    assert_eq!(syn::parse2::<Program>(printed).unwrap(), target);
    let mut separate = target.clone();
    separate.representation.blocks.runs.remove("r");
    separate.representation.blocks.runs.insert("p".into(), vec!["p".into()]);
    separate.representation.blocks.runs.insert("q".into(), vec!["q".into()]);
    check(&physical, &source, &separate).unwrap();
    let mut alternative = source;
    for predicate in alternative.encoded.blocks.predicates.values_mut() {
        predicate.test = cq_lang_structures::blocks::Test::DecodeCodes;
        predicate.prune = cq_lang_structures::blocks::Prune::None;
        predicate.strategy = cq_lang_structures::blocks::Strategy::Formula;
    }
    accept(&physical, &alternative);
}

#[test]
fn blocked_execution_rejects_forged_phases_scratch_and_opaque_crossing() {
    let (physical, source) = blocked_source();
    let target = accept(&physical, &source);
    let mut changed = target.clone();
    changed.representation.blocks.predicates.get_mut("p").unwrap().swap(0,1);
    assert!(check(&physical, &source, &changed).is_err());
    let mut changed = target.clone();
    changed.representation.blocks.scans.get_mut("r").unwrap().scratch_rows = 2048;
    assert!(check(&physical, &source, &changed).is_err());
    let mut changed = target.clone();
    changed.representation.blocks.runs.get_mut("r").unwrap().push("t".into());
    changed.representation.blocks.runs.remove("t");
    assert!(check(&physical, &source, &changed).is_err());
    let mut changed = target.clone();
    changed.representation.blocks.runs.get_mut("r").unwrap().pop();
    assert!(check(&physical, &source, &changed).is_err());
    let mut changed = target;
    changed.representation.blocks.reads.remove("g");
    assert!(check(&physical, &source, &changed).is_err());
}
