#[macro_use]
mod support;
use cq_lang_sequence::structures::{Program as Structures, physical::Program as Physical};
use cq_lang_sequence::{Program, bounds, contract};
use cq_sequence_plan::Advice;

fn program() -> (Physical, Structures, Program) {
    fixture! {
        physical {
            relation R(a:i32); relation S(a:i32,b:i32); relation T(b:i32,c:i32);
            r = pipe rename R {a -> a};
            s = let rename S {a -> a,b -> b};
            t = let rename T {b -> b,c -> c};
            joined = pipe probe first [a] r,s;
            last = pipe probe first [b] joined,t;
            answer = let canon project last keep {a,b,c};
            return answer as answer(a,b,c);
        }
        structures {
            providers {
                reader = Reader { reads [exact, range, membership], build inferred };
                index = Index { reads [exact, range, membership], build inferred };
                sort = ::cq_storage::dedup::Sort { reads [distinct, comparator_rows], build inferred, shared };
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
        evidence from authored("passes/cq-sequence-plan/tests/prefix_bounds.rs fixture") {
            count R = 1;
            count S = 100;
            count T = 1000;
        }
    }
}

#[test]
fn prefix_bound_checker_accepts_weaker_records_and_rejects_missing_premises() {
    let (physical, structures, source) = program();
    let analysis = contract::check(&physical, &structures, &source).unwrap();
    assert_eq!(analysis.prefix_bounds["joined"].rows, Some(100));
    assert_eq!(analysis.prefix_bounds["last"].rows, None);
    assert_eq!(analysis.prefix_bounds["s"].keys, vec![["a".to_owned(), "b".to_owned()].into_iter().collect()]);
    let mut partial_key = analysis.prefix_bounds.clone();
    partial_key.get_mut("s").unwrap().keys = vec![["a".to_owned()].into_iter().collect()];
    assert!(bounds::check(&physical, &source, &partial_key).is_err());
    let mut fabricated_bound = analysis.prefix_bounds.clone();
    fabricated_bound.get_mut("last").unwrap().rows = Some(1000);
    assert!(bounds::check(&physical, &source, &fabricated_bound).is_err());
    let mut alternative = analysis.prefix_bounds.clone();
    alternative.get_mut("joined").unwrap().rows = Some(101);
    bounds::check(&physical, &source, &alternative).unwrap();
    alternative.get_mut("joined").unwrap().rows = Some(99);
    assert!(bounds::check(&physical, &source, &alternative).is_err());
    let mut unsupported = analysis.prefix_bounds.clone();
    unsupported.get_mut("r").unwrap().keys.clear();
    assert!(bounds::check(&physical, &source, &unsupported).is_err());
}

#[test]
fn retention_checks_policy_and_chain_premises_without_replaying_advice() {
    let (physical, structures, source) = program();
    // Independent policy records are legal; neither is required to equal a
    // producer's preferred machine geometry.
    for (tile, minimum) in [(32, 50), (64, 100)] {
        Advice::Retain {
            name: syn::parse_quote!(joined),
            rows: 100,
            tile,
            minimum,
        }
        .apply(&physical, &structures, &source)
        .unwrap();
    }
    for (rows, tile, minimum) in [(99, 32, 50), (100, 1, 50), (100, 32, 101)] {
        assert!(
            Advice::Retain {
                name: syn::parse_quote!(joined),
                rows,
                tile,
                minimum
            }
            .apply(&physical, &structures, &source)
            .is_err()
        );
    }
    let mut no_follower = source.clone();
    no_follower.sequence.definitions[4].term = syn::parse_quote!(store flatten lookup t [b] joined);
    contract::check(&physical, &structures, &no_follower).unwrap();
    assert!(
        Advice::Retain {
            name: syn::parse_quote!(joined),
            rows: 100,
            tile: 32,
            minimum: 50,
        }
        .apply(&physical, &structures, &no_follower)
        .is_err()
    );
}
