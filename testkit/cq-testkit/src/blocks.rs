//! Authored core block corpus, including branch locals, overlap, nesting,
//! ordered fallback and head-only multi-rule sugar. Arbitrary Rust and panic
//! refinements are compiled in the facade and seeded fuzz lanes.

use crate::Case;

pub const CASES: &[Case] = &[
    Case {
        name: "union_overlap",
        cq: "struct Q; relation R(x:i32,a:i32); relation S(x:i32,b:i32); answer(x) :- {R(x,a)} | {S(x,b)};",
        inputs: &[
            ("R", &[&[1, 10], &[1, 11], &[2, 12]]),
            ("S", &[&[1, 20], &[3, 21]]),
        ],
        expected: &[&[1], &[2], &[3]],
    },
    Case {
        name: "fallback_per_free_row",
        cq: "struct Q; relation P(x:i32); relation R(x:i32,y:i32); relation S(x:i32,y:i32); answer(x,y) :- P(x), {R(x,y)} else {S(x,y)};",
        inputs: &[
            ("P", &[&[1], &[2], &[3]]),
            ("R", &[&[1, 10], &[1, 11]]),
            ("S", &[&[1, 99], &[2, 20]]),
        ],
        expected: &[&[1, 10], &[1, 11], &[2, 20]],
    },
    Case {
        name: "test_union",
        cq: "struct Q; relation P(x:i32); relation R(x:i32,a:i32); relation S(x:i32,b:i32); answer(x) :- P(x), {R(x,a)} | {S(x,b)};",
        inputs: &[
            ("P", &[&[1], &[2], &[3]]),
            ("R", &[&[1, 10], &[1, 11]]),
            ("S", &[&[1, 20], &[2, 21]]),
        ],
        expected: &[&[1], &[2]],
    },
    Case {
        name: "test_fallback_chain",
        cq: "struct Q; relation P(x:i32); relation R(x:i32,a:i32); relation S(x:i32,b:i32); relation T(x:i32,c:i32); answer(x) :- P(x), {R(x,a)} else {S(x,b)} else {T(x,c)};",
        inputs: &[
            ("P", &[&[1], &[2], &[3], &[4]]),
            ("R", &[&[1, 10]]),
            ("S", &[&[1, 20], &[2, 21]]),
            ("T", &[&[3, 30]]),
        ],
        expected: &[&[1], &[2], &[3]],
    },
    Case {
        name: "nested_union_fallback",
        cq: "struct Q; relation R(x:i32); relation S(x:i32); relation T(x:i32); answer(x) :- {{R(x)} else {S(x)}} | {T(x)};",
        inputs: &[("R", &[&[1]]), ("S", &[&[2]]), ("T", &[&[3]])],
        expected: &[&[1], &[3]],
    },
    Case {
        name: "multi_rule_locals",
        cq: "struct Q; relation R(x:i32,a:i32); relation S(x:i32,a:i32); answer(x) :- R(x,a); answer(y) :- S(y,a);",
        inputs: &[("R", &[&[1, 10], &[1, 11]]), ("S", &[&[2, 20]])],
        expected: &[&[1], &[2]],
    },
    Case {
        name: "prefix_constraint_domain",
        cq: "struct Q; relation P(x:i32); answer(x) :- P(x), {if x == 1} else {if x == 2};",
        inputs: &[("P", &[&[1], &[2], &[3]])],
        expected: &[&[1], &[2]],
    },
];

/// Complete authored programs at every public stage, independently of passes.
pub const SCENARIOS: &[crate::corpus::Scenario] = &[
    crate::corpus::Scenario {
        name: "block_union",
        cq: include_str!("scenarios/block_union.cq"),
        vo: include_str!("scenarios/block_union.vo"),
        ra: include_str!("scenarios/block_union.ra"),
        ix: include_str!("scenarios/block_union.ix"),
        physical: include_str!("scenarios/block_union.physical"),
        structures: include_str!("scenarios/block_union.structures"),
    },
    crate::corpus::Scenario {
        name: "block_fallback",
        cq: include_str!("scenarios/block_fallback.cq"),
        vo: include_str!("scenarios/block_fallback.vo"),
        ra: include_str!("scenarios/block_fallback.ra"),
        ix: include_str!("scenarios/block_fallback.ix"),
        physical: include_str!("scenarios/block_fallback.physical"),
        structures: include_str!("scenarios/block_fallback.structures"),
    },
];
