//! Public-macro end-to-end tests: Cargo expands the entire macro chain and
//! compiles its Rust. Test code calls only the generated user API and oracles.
#[allow(dead_code)]
#[path = "../../cq-compiler/tests/stage1_reference/cases.rs"]
mod cases;
#[allow(dead_code)]
#[path = "../../cq-compiler/tests/stage1_reference/oracle.rs"]
mod oracle;

use cq_ir::{cq, relational_plan, symbol_name};
use std::collections::{BTreeMap, BTreeSet};

// Host inputs deliberately retain duplicates and iteration order. Only the
// independent set-semantics oracle receives normalized copies.
type HostDatabase = BTreeMap<String, Vec<Vec<i32>>>;

macro_rules! input_rows {
    ($database:ident, $relation:ident, $($column:ident),+) => {
        $database[stringify!($relation)].iter().map(|row| {
            let mut fields = row.iter().copied();
            $(let $column = fields.next().expect(stringify!($column));)+
            assert!(fields.next().is_none(), "input tuple arity");
            ($($column,)+)
        })
    };
}

// This test helper repeats the literal CQ into the real public entry and a
// source-parity assertion. It does not call a compiler pass or rewrite an IR.
macro_rules! shape {
    ($name:ident {
        struct $program:ident;
        $(relation $input:ident($($column:ident: $ty:ty),+);)+
        answer($($head:ident),+) :- $($body:tt)+
    }) => {
        mod $name {
            mini_linq::planned_query! {
                struct $program;
                $(relation $input($($column: $ty),+);)+
                answer($($head),+) :- $($body)+
            }

            #[test]
            fn public_macro_end_to_end() {
                let source = stringify! {
                    struct $program;
                    $(relation $input($($column: $ty),+);)+
                    answer($($head),+) :- $($body)+
                };
                super::check_case(stringify!($name), source, |database, expected, scenario| {
                    let storage = $program::build(
                        $(input_rows!(database, $input, $($column),+)),+
                    );
                    let actual: Vec<_> = storage.materialize().into_iter()
                        .map(|($($head,)+)| vec![$($head),+]).collect();
                    assert_eq!(actual, expected, "{scenario}: build/materialize");
                    // Compare vectors, preserving multiplicity. Converting the
                    // actual output to a set would hide broken DISTINCT.
                    for _ in 0..2 {
                        let mut actual: Vec<_> = storage.query()
                            .map(|($($head,)+)| vec![$($head),+]).collect();
                        actual.sort();
                        assert_eq!(actual, expected, "{scenario}: repeatable query");
                    }
                    let actual: Vec<_> = $program::run(
                        $(input_rows!(database, $input, $($column),+)),+
                    ).into_iter().map(|($($head,)+)| vec![$($head),+]).collect();
                    assert_eq!(actual, expected, "{scenario}: run");
                });
            }
        }
    };
}

macro_rules! shapes {
    ($($name:ident { $($query:tt)* })+) => {
        $(shape!($name { $($query)* });)+

        #[test]
        fn every_reference_shape_has_a_public_macro_test() {
            let names = [$(stringify!($name)),+];
            let actual: BTreeSet<_> = names.into_iter().collect();
            assert_eq!(actual.len(), names.len(), "duplicate test shape");
            let expected = cases::CASES.iter().map(|case| case.name).collect();
            assert_eq!(actual, expected);
        }
    };
}

fn check_case(name: &str, text: &str, execute: impl Fn(&HostDatabase, &[Vec<i32>], &str)) {
    let case = cases::CASES.iter().find(|case| case.name == name).unwrap();
    let known: relational_plan::Module = syn::parse_str(case.plan).unwrap();
    let source: cq::Module = syn::parse_str(text).unwrap();
    assert_eq!(
        source.program, known.program,
        "macro CQ differs from fixture"
    );
    let authored: HostDatabase = case
        .inputs
        .iter()
        .map(|(name, rows)| {
            (
                (*name).to_owned(),
                rows.iter().map(|row| row.to_vec()).collect(),
            )
        })
        .collect();
    let mut scenarios = vec![("authored".to_owned(), authored.clone())];
    for (label, domain, count) in [("binary", &[0, 1][..], 64), ("signed", &[-2, 0, 3][..], 32)] {
        scenarios.extend((0..count).map(|seed| {
            (
                format!("{label}/{seed}"),
                generated_database(&source.program, domain, seed),
            )
        }));
    }
    let used: BTreeSet<_> = source
        .program
        .query
        .body
        .iter()
        .filter_map(cq::BodyItem::atom)
        .map(|atom| symbol_name(&atom.relation))
        .collect();
    for input in used {
        let mut empty = authored.clone();
        empty.get_mut(&input).unwrap().clear();
        scenarios.push((format!("empty/{input}"), empty));
    }
    for (scenario, mut database) in scenarios {
        let normalized = database
            .iter()
            .map(|(name, rows)| (name.clone(), rows.iter().cloned().collect()))
            .collect();
        let expected: Vec<_> = oracle::evaluate_cq(&source.program, &normalized)
            .into_iter()
            .collect();
        if scenario == "authored" {
            let fixed: Vec<_> = case.expected.iter().map(|row| row.to_vec()).collect();
            assert_eq!(expected, fixed, "authored answer oracle");
        }
        if scenario.starts_with("empty/") {
            assert!(
                expected.is_empty(),
                "every positive input remains an obligation"
            );
        }
        execute(&database, &expected, &scenario);
        for rows in database.values_mut() {
            rows.reverse();
            rows.extend_from_within(..);
        }
        execute(
            &database,
            &expected,
            &format!("{scenario}/reversed-duplicates"),
        );
    }
}

fn generated_database(program: &cq::Program, domain: &[i32], seed: u64) -> HostDatabase {
    let mut random = seed;
    program
        .inputs
        .iter()
        .map(|input| {
            let rows = (0..domain.len().pow(input.arity() as u32))
                .filter_map(|mut ordinal| {
                    random = random.wrapping_mul(6364136223846793005).wrapping_add(1);
                    let include = match seed {
                        0 => false,
                        1 => true,
                        _ => (random >> 61) <= seed % 8,
                    };
                    include.then(|| {
                        (0..input.arity())
                            .map(|_| {
                                let value = domain[ordinal % domain.len()];
                                ordinal /= domain.len();
                                value
                            })
                            .collect()
                    })
                })
                .collect();
            (symbol_name(&input.name), rows)
        })
        .collect()
}

shapes! {
    scan {
        struct Scan;
        relation R(value: i32);
        answer(x) :- R(x).
    }
    projection {
        struct Projection;
        relation R(first: i32, second: i32, witness: i32);
        answer(y, x) :- R(x, y, z).
    }
    self_chain {
        struct SelfChain;
        relation Road(src: i32, dst: i32);
        answer(a, c) :- Road(a, b), Road(b, c).
    }
    return_trip {
        struct ReturnTrip;
        relation Road(src: i32, dst: i32);
        answer(a) :- Road(a, b), Road(b, a).
    }
    intersection {
        struct Intersection;
        relation R(first: i32, second: i32);
        relation S(first: i32, second: i32);
        answer(x, y) :- R(x, y), S(x, y).
    }
    unary_guard {
        struct UnaryGuard;
        relation R(first: i32, second: i32);
        relation Approved(value: i32);
        answer(x) :- R(x, y), Approved(y).
    }
    chain_projection {
        struct ChainProjection;
        relation R(first: i32, second: i32);
        relation S(first: i32, second: i32);
        relation T(first: i32, second: i32);
        answer(a, d) :- R(a, b), S(b, c), T(c, d).
    }
    claw {
        struct Claw;
        relation A(origin: i32, endpoint: i32);
        relation B(origin: i32, endpoint: i32);
        relation C(origin: i32, endpoint: i32);
        answer(a, b, c) :- A(h, a), B(h, b), C(h, c).
    }
    fact_star {
        struct FactStar;
        relation F(k1: i32, k2: i32, k3: i32);
        relation A(key: i32, value: i32);
        relation B(key: i32, value: i32);
        relation C(key: i32, value: i32);
        answer(a, b, c) :- F(x, y, z), A(x, a), B(y, b), C(z, c).
    }
    triangle {
        struct Triangle;
        relation R(first: i32, second: i32);
        relation S(first: i32, second: i32);
        relation T(first: i32, second: i32);
        answer(a, b, c) :- R(a, b), S(b, c), T(c, a).
    }
    cycle4 {
        struct CycleFour;
        relation R(first: i32, second: i32);
        relation S(first: i32, second: i32);
        relation T(first: i32, second: i32);
        relation U(first: i32, second: i32);
        answer(a, c) :- R(a, b), S(b, c), T(c, d), U(d, a).
    }
    cartesian {
        struct Cartesian;
        relation R(value: i32);
        relation S(value: i32);
        answer(y, x) :- R(x), S(y).
    }
    existential {
        struct Existential;
        relation R(value: i32);
        relation S(first: i32, second: i32);
        answer(x) :- R(x), S(y, z).
    }
    ternary_guard {
        struct TernaryGuard;
        relation A(first: i32, second: i32);
        relation B(first: i32, second: i32);
        relation C(first: i32, second: i32);
        relation Stock(depot: i32, item: i32, day: i32);
        answer(d, i, t) :- A(d, i), B(i, t), C(d, t), Stock(d, i, t).
    }
    composite {
        struct Composite;
        relation R(first: i32, second: i32, value: i32);
        relation S(first: i32, second: i32, value: i32);
        answer(x, y) :- R(a, b, x), S(b, a, y).
    }
    bushy {
        struct Bushy;
        relation R(first: i32, second: i32);
        relation S(first: i32, second: i32);
        relation T(first: i32, second: i32);
        relation U(first: i32, second: i32);
        answer(a, e) :- R(a, b), S(b, c), T(c, d), U(d, e).
    }
    ya_delivery {
        struct YaDelivery;
        relation D(driver_id: i32, depot_id: i32);
        relation S(depot_id: i32, item_id: i32);
        relation N(job_id: i32, item_id: i32);
        answer(driver, job) :- D(driver, depot), S(depot, item), N(job, item).
    }
    oriented_chain {
        struct OrientedChain;
        relation Edge(src: i32, dst: i32);
        answer(a, d) :- Edge(a, b), Edge(b, c), Edge(d, c).
    }
    spade {
        struct Spade;
        relation R(first: i32, second: i32);
        relation S(first: i32, second: i32);
        relation T(first: i32, second: i32);
        relation H(first: i32, second: i32);
        answer(x, y, z, w) :- R(x, y), S(y, z), T(x, z), H(x, w).
    }
    barbell {
        struct Barbell;
        relation R(first: i32, second: i32);
        relation S(first: i32, second: i32);
        relation T(first: i32, second: i32);
        relation Bridge(first: i32, second: i32);
        relation U(first: i32, second: i32);
        relation V(first: i32, second: i32);
        relation W(first: i32, second: i32);
        answer(x, y, z, u, v, w) :-
            R(x, y), S(y, z), T(x, z), Bridge(x, u), U(u, v), V(v, w), W(u, w).
    }
    loomis_whitney_4 {
        struct LoomisWhitneyFour;
        relation ABC(first: i32, second: i32, third: i32);
        relation BCD(first: i32, second: i32, third: i32);
        relation ACD(first: i32, second: i32, third: i32);
        relation ABD(first: i32, second: i32, third: i32);
        answer(a, b, c, d) :- ABC(a, b, c), BCD(b, c, d), ACD(a, c, d), ABD(a, b, d).
    }
    ya_deep {
        struct YaDeep;
        relation R(first: i32, second: i32);
        relation S(first: i32, second: i32);
        relation T(first: i32, second: i32);
        relation U(first: i32, second: i32);
        answer(a, e) :- R(a, b), S(b, c), T(c, d), U(d, e).
    }
    ya_composite {
        struct YaComposite;
        relation R(first: i32, second: i32, value: i32);
        relation S(first: i32, second: i32, third: i32);
        relation T(first: i32, second: i32, value: i32);
        answer(u, v) :- R(a, b, u), S(a, b, c), T(b, c, v).
    }
    ya_disconnected {
        struct YaDisconnected;
        relation R(value: i32);
        relation S(first: i32, second: i32);
        answer(x) :- R(x), S(y, z).
    }
    cartesian_first {
        struct CartesianFirst;
        relation R(first: i32, second: i32);
        relation S(first: i32, second: i32);
        relation T(first: i32, second: i32);
        answer(a, d) :- R(a, b), S(b, c), T(c, d).
    }
}
