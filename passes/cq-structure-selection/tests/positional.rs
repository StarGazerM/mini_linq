use cq_lang_physical as physical;
use cq_lang_structures::{self as ir, Structure};
use quote::{ToTokens, quote};
fn physical() -> physical::Program {
    syn::parse_quote! { physical {
        relation L(a:i64,b:i64); relation R(c:i64,d:i64);
        l = pipe rename L@0 {a -> x,b -> y};
        r = pipe rename R@1 {c -> x,d -> z};
        s = let point select r if (z = 1);
        t = let semi eager [x] l,s;
        u = pipe semi eager [x] s,t;
        j = pipe probe first [x] u,t;
        out = pipe canon project j keep {x,y}; return out as answer(x,y);
    }}
}
fn selected() -> ir::Program {
    cq_structure_selection::compile_with(&physical(), quote!(positional(L, R))).unwrap()
}
#[test]
fn decisions_roundtrip_and_physical_is_identical() {
    let source = physical();
    let target = selected();
    assert_eq!(source, target.physical);
    assert_eq!(
        source.to_token_stream().to_string(),
        target.physical.to_token_stream().to_string()
    );
    assert_eq!(
        syn::parse2::<ir::Program>(target.to_token_stream()).unwrap(),
        target
    );
    let facts = ir::contract::check(&target).unwrap();
    assert_eq!(facts.selections.len(), 2);
    assert_eq!(facts.keysets.len(), 2);
    assert_eq!(facts.reads.len(), 3);
    assert!(facts.retained.is_empty());
    assert_eq!(
        cq_structure_selection::compile_with(&source, quote!()).unwrap(),
        cq_structure_selection::compile(&source).unwrap()
    );
}
#[test]
fn lineage_and_summary_forgery_are_rejected_independently() {
    for alteration in 0..7 {
        let mut target = selected();
        for e in &mut target.structures.entries {
            match (&mut e.structure, alteration) {
                (Structure::Selection { origin, .. }, 0) => {
                    origin.base = syn::parse_quote!(L);
                    break;
                }
                (Structure::Selection { origin, .. }, 1) => {
                    origin.columns.pop();
                    break;
                }
                (Structure::Selection { origin, .. }, 2) => {
                    origin.columns[1].column = origin.columns[0].column.clone();
                    break;
                }
                (
                    Structure::Read {
                        predicate: ir::ReadPredicate::Membership { build, .. },
                        ..
                    },
                    3,
                ) => {
                    *build = syn::parse_quote!(j);
                    break;
                }
                (Structure::Read { marker, .. }, 4) => {
                    *marker = syn::parse_quote!(::cq_load::ColumnAt<99>);
                    break;
                }
                (Structure::KeySet { ty, .. }, 5) => {
                    *ty = syn::parse_quote!(::std::collections::HashSet<_>);
                    break;
                }
                (Structure::KeySet { ty, .. }, 6) => {
                    *ty = syn::parse_quote!(::cq_storage::AdaptiveKeySet<_,::cq_storage::BorrowedI64TupleCodec,64>);
                    break;
                }
                _ => {}
            }
        }
        assert!(
            ir::contract::check(&target).is_err(),
            "forgery {alteration}"
        );
    }
}
#[test]
fn representation_rewrite_cannot_change_an_execution_choice() {
    let source = selected();
    let mut target = source.clone();
    if let physical::Operator::Semi(_, k) = &mut target.physical.program.definitions[3].operator {
        k.choice = Some(syn::parse_quote!(lazy));
    }
    assert!(ir::check_rewrite(&source, &target).is_err());
}
#[test]
fn duplicate_and_missing_summaries_and_false_payload_indices_fail() {
    let source = selected();
    let mut duplicate = source.clone();
    duplicate
        .structures
        .entries
        .push(source.structures.entries.last().unwrap().clone());
    assert!(ir::contract::check(&duplicate).is_err());
    for kind in ["read", "build"] {
        let mut target = source.clone();
        target.structures.entries.retain(|e| {
            if kind == "read" {
                !matches!(e.structure, Structure::Read { .. })
            } else {
                !matches!(e.structure, Structure::Built { .. })
            }
        });
        assert!(ir::contract::check(&target).is_err());
    }
}

#[test]
fn index_only_rhs_prunes_dependent_candidates_without_changing_the_old_representation() {
    let physical = syn::parse_quote! { physical {
        relation L(a:i64,b:i64); relation H(c:i64); relation R(d:i64,e:i64);
        l = pipe rename L@0 {a -> x,b -> y}; h = pipe rename H@1 {c -> x};
        a = let semi eager [x] l,h;
        r = pipe rename R@2 {d -> x,e -> z};
        t = let semi eager [x] r,a;
        out = pipe canon project t keep {x,z}; return out as answer(x,z);
    }};
    let original = cq_structure_selection::compile(&physical).unwrap();
    let selected =
        cq_structure_selection::compile_with(&physical, quote!(positional(L, H, R))).unwrap();
    assert_eq!(original, selected);
}
