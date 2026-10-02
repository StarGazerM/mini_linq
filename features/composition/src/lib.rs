//! A caller composes the default passes through the public pipeline grammar.
//!
//! [`query`] writes every stage explicitly in `mini_linq::compile!`: written
//! planning, IX demand derivation, Physical lowering, Structures selection,
//! and pull printing. Default lowering records probe/first/eager choices in
//! Physical; selection records concrete fallback and dedup types in Structures.
//! The caller owns the function signature, hosts, and answer collection.
//!
//! `L` is scanned. `R` is a `HashMap` keyed by its leading declared column `a`,
//! and `S` is a `BTreeMap` keyed by its leading column `b`. The public storage
//! traits probe both maps directly; the recorded fallbacks stay uninitialized
//! on these native paths. Keys and payloads are tuples in declaration order.
//! The query returns references to host fields. Only the executable copies
//! integers for deterministic display.
//!
//! The meaning is Kleisli over bottom + Set, with Leibniz equality on integers.
//! Rust steps are opaque partial functions and stay pinned relative to each
//! other and their binders. Tests use cq-testkit's independent CQ oracle and
//! written-order refinement harness. Core dependencies have version and path
//! entries so the external gate can replace them with packaged crates.
//!
//! ```
//! use mini_linq_composition_example::query;
//! use std::collections::{BTreeMap, HashMap};
//! let left = [(1,), (1,)];
//! let middle = HashMap::from([((1,), vec![(10,)])]);
//! let right = BTreeMap::from([((10,), vec![(100,)])]);
//! let answers = query(&left, &middle, &right);
//! assert_eq!(answers.into_iter().map(|(a, b, c)| (*a, *b, *c)).collect::<Vec<_>>(),
//!            vec![(1, 10, 100)]);
//! ```
#![recursion_limit = "512"]

use std::collections::{BTreeMap, HashMap, HashSet};

/// Join a scanned host to two native maps, borrowing every answer column.
#[allow(non_snake_case)]
pub fn query<'a>(
    L: &'a [(i32,)],
    R: &'a HashMap<(i32,), Vec<(i32,)>>,
    S: &'a BTreeMap<(i32,), Vec<(i32,)>>,
) -> HashSet<(&'a i32, &'a i32, &'a i32)> {
    mini_linq::compile! {
        pipeline {
            plan = mini_linq::passes::plan_written;
            ix = mini_linq::passes::index_demands;
            lower = mini_linq::passes::lower;
            physical = [];
            select = mini_linq::passes::select;
            structures = [];
            print = mini_linq::passes::print_pull;
            surface = [];
        }
        struct Join;
        relation L(a: i32);
        relation R(a: i32, b: i32);
        relation S(b: i32, c: i32);
        answer(a, b, c) :- L(a), R(a, b), S(b, c);
    }
    .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use cq_testkit::{Database, Rows, cq_semantics, harness};

    fn source() -> cq_testkit::cq::Program {
        syn::parse_quote! {
            struct Join;
            relation L(a: i32);
            relation R(a: i32, b: i32);
            relation S(b: i32, c: i32);
            answer(a, b, c) :- L(a), R(a, b), S(b, c);
        }
    }

    fn database(
        left: &[(i32,)],
        middle: &HashMap<(i32,), Vec<(i32,)>>,
        right: &BTreeMap<(i32,), Vec<(i32,)>>,
    ) -> Database {
        [
            ("L".to_owned(), left.iter().map(|&(a,)| vec![a]).collect()),
            (
                "R".to_owned(),
                middle
                    .iter()
                    .flat_map(|(&(a,), values)| values.iter().map(move |&(b,)| vec![a, b]))
                    .collect(),
            ),
            (
                "S".to_owned(),
                right
                    .iter()
                    .flat_map(|(&(b,), values)| values.iter().map(move |&(c,)| vec![b, c]))
                    .collect(),
            ),
        ]
        .into()
    }

    fn rows(answer: HashSet<(&i32, &i32, &i32)>) -> Rows {
        answer
            .into_iter()
            .map(|(a, b, c)| vec![*a, *b, *c])
            .collect()
    }

    #[test]
    fn duplicate_witnesses_empty_groups_and_borrowed_host_fields() {
        let left = [(1,), (1,), (2,), (9,)];
        let middle = HashMap::from([
            ((1,), vec![(10,), (10,), (11,)]),
            ((2,), vec![(20,)]),
            ((9,), vec![]),
        ]);
        let right = BTreeMap::from([
            ((10,), vec![(100,), (100,), (101,)]),
            ((11,), vec![]),
            ((20,), vec![(200,)]),
        ]);
        let answer = query(&left, &middle, &right);
        for &(a, b, c) in &answer {
            assert!(left.iter().any(|row| std::ptr::eq(a, &row.0)));
            assert!(middle[&(*a,)].iter().any(|row| std::ptr::eq(b, &row.0)));
            assert!(right[&(*b,)].iter().any(|row| std::ptr::eq(c, &row.0)));
        }
        assert_eq!(
            rows(answer),
            cq_semantics::evaluate_cq(&source(), &database(&left, &middle, &right))
        );
    }

    #[test]
    fn all_binary_hosts_match_the_independent_cq_oracle() {
        let source = source();
        cq_testkit::cq::contract::check(&source).unwrap();
        let pairs = [(0, 0), (0, 1), (1, 0), (1, 1)];
        for left_mask in 0..4 {
            let left: Vec<_> = (0..2)
                .filter(|a| left_mask & (1 << a) != 0)
                .map(|a| (a,))
                .collect();
            for middle_mask in 0..16 {
                let mut middle = HashMap::<_, Vec<_>>::new();
                for (i, &(a, b)) in pairs.iter().enumerate() {
                    if middle_mask & (1 << i) != 0 {
                        middle.entry((a,)).or_default().push((b,));
                    }
                }
                for right_mask in 0..16 {
                    let mut right = BTreeMap::<_, Vec<_>>::new();
                    for (i, &(b, c)) in pairs.iter().enumerate() {
                        if right_mask & (1 << i) != 0 {
                            right.entry((b,)).or_default().push((c,));
                        }
                    }
                    assert_eq!(
                        rows(query(&left, &middle, &right)),
                        cq_semantics::evaluate_cq(&source, &database(&left, &middle, &right))
                    );
                }
            }
        }
    }

    #[test]
    #[allow(non_snake_case)]
    fn native_ordered_host_needs_no_hash_on_its_key() {
        #[derive(Clone, PartialEq, Eq, PartialOrd, Ord)]
        struct OrderedKey(i32);
        let L = [(1,)];
        let R = HashMap::from([((1,), vec![(OrderedKey(10),)])]);
        let S = BTreeMap::from([((OrderedKey(10),), vec![(100,)])]);
        let answer: Rows = mini_linq::compile! {
            pipeline {
                plan = mini_linq::passes::plan_written;
                ix = mini_linq::passes::index_demands;
                lower = mini_linq::passes::lower;
                select = mini_linq::passes::select;
                print = mini_linq::passes::print_pull;
            }
            struct Native;
            relation L(a: i32);
            relation R(a: i32, b: OrderedKey);
            relation S(b: OrderedKey, c: i32);
            answer(a, c) :- L(a), R(a, b), S(b, c);
        }
        .map(|(a, c)| vec![*a, *c])
        .collect();
        assert_eq!(answer, Rows::from([vec![1, 100]]));
    }

    #[test]
    #[allow(non_snake_case)]
    fn ordered_rust_steps_refine_the_written_reference() {
        fn first(b: &i32) -> i32 {
            assert_ne!(*b, 9);
            *b + 1
        }
        fn second(d: &i32) -> bool {
            assert_ne!(*d, 8);
            *d > 2
        }
        for value in [0, 1, 3, 7, 9] {
            for has_middle in [false, true] {
                for has_right in [false, true] {
                    let L = [(1,), (1,)];
                    let R =
                        HashMap::from([((1,), if has_middle { vec![(value,)] } else { vec![] })]);
                    let S =
                        BTreeMap::from([((value,), if has_right { vec![(100,)] } else { vec![] })]);
                    harness::check_refinement(
                        || {
                            let mut answer = Rows::new();
                            for &(a,) in &L {
                                for &(b,) in &R[&(a,)] {
                                    let d = first(&b);
                                    if second(&d) {
                                        for &(c,) in &S[&(b,)] {
                                            answer.insert(vec![a, b, c]);
                                        }
                                    }
                                }
                            }
                            answer
                        },
                        || {
                            mini_linq::compile! {
                                pipeline {
                                    plan = mini_linq::passes::plan_written;
                                    ix = mini_linq::passes::index_demands;
                                    lower = mini_linq::passes::lower;
                                    select = mini_linq::passes::select;
                                    print = mini_linq::passes::print_pull;
                                }
                                struct Partial;
                                relation L(a: i32);
                                relation R(a: i32, b: i32);
                                relation S(b: i32, c: i32);
                                answer(a, b, c) :- L(a), R(a, b),
                                    let d: i32 = first(b), if second(d), S(b, c);
                            }
                            .map(|(a, b, c)| vec![*a, *b, *c])
                            .collect::<Rows>()
                        },
                    )
                    .unwrap();
                }
            }
        }
    }
}
