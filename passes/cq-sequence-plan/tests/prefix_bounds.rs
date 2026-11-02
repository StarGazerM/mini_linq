use cq_lang_sequence::{Program, bounds, contract};
use cq_sequence_plan::Advice;

fn program() -> Program {
    syn::parse_quote! {
        physical {
            relation R(a:i32); relation S(a:i32,b:i32); relation T(b:i32,c:i32);
            r = pipe rename R@0 {a -> a};
            s = let rename S@1 {a -> a,b -> b};
            t = let rename T@2 {b -> b,c -> c};
            joined = pipe probe first [a] r,s;
            last = pipe probe first [b] joined,t;
            answer = let canon project last keep {a,b,c};
            return answer as answer(a,b,c);
        }
        structures {
            providers {
                reader = Reader { reads [exact, range, membership], build inferred };
                index = Index { reads [exact, range, membership], build inferred };
                sort = ::cq_storage::dedup::Sort { reads [distinct], build inferred, shared };
            }
            r host scan = @reader as Reader [];
            s host scan = @reader as Reader [];
            t host scan = @reader as Reader [];
            s = @index as Index [a]; t = @index as Index [b];
            answer rows = @sort as ::cq_storage::dedup::Sort; answer canon = rows;
        }
        sequence {
            r = rows r; s = store rows s; t = store rows t;
            joined = flatten lookup s [a] r;
            last = flatten lookup t [b] joined;
            result = store canon map answer last;
        }
        evidence {
            count R = 1; key R [a];
            count S = 100; key S [a,b];
            count T = 1000; key T [b,c];
        }
    }
}

#[test]
fn prefix_bound_checker_accepts_weaker_records_and_rejects_missing_premises() {
    let source = program();
    let analysis = contract::check(&source).unwrap();
    assert_eq!(analysis.prefix_bounds["joined"].rows, Some(100));
    let mut alternative = analysis.prefix_bounds.clone();
    alternative.get_mut("joined").unwrap().rows = Some(101);
    bounds::check(&source, &alternative).unwrap();
    alternative.get_mut("joined").unwrap().rows = Some(99);
    assert!(bounds::check(&source, &alternative).is_err());
    let mut unsupported = analysis.prefix_bounds.clone();
    unsupported.get_mut("r").unwrap().keys.clear();
    assert!(bounds::check(&source, &unsupported).is_err());
}

#[test]
fn retention_checks_policy_and_chain_premises_without_replaying_advice() {
    let source = program();
    // Independent policy records are legal; neither is required to equal a
    // producer's preferred machine geometry.
    for (tile, minimum) in [(32, 50), (64, 100)] {
        Advice::Retain { name: syn::parse_quote!(joined), rows: 100, tile, minimum }
            .apply(&source).unwrap();
    }
    for (rows, tile, minimum) in [(99, 32, 50), (100, 1, 50), (100, 32, 101)] {
        assert!(Advice::Retain { name: syn::parse_quote!(joined), rows, tile, minimum }
            .apply(&source).is_err());
    }
    let mut no_follower = source.clone();
    no_follower.sequence.definitions[4].term =
        syn::parse_quote!(store flatten lookup t [b] joined);
    contract::check(&no_follower).unwrap();
    assert!(Advice::Retain {
        name: syn::parse_quote!(joined), rows: 100, tile: 32, minimum: 50,
    }.apply(&no_follower).is_err());
}
