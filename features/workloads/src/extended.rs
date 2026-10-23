//! Authored surface and extension queries, independent SQL, and tiny fixtures.
//! This catalog does not send an extension through a positive-CQ evaluator.
use std::path::{Path, PathBuf};

use cq_syntax::RelationDecl;

use crate::{
    GenerationConfig, Scale, Scenario, Suite,
    dataset::{Dataset, DatasetError, RelationData},
    schema::{self, ColumnType::I32, Relation},
};

#[derive(Clone, Copy, Debug)]
pub struct Case {
    pub name: &'static str,
    pub family: &'static str,
    pub relations: &'static [Relation],
    pub rust_path: &'static str,
    pub arity: usize,
    /// Independently authored SQL, using set-valued input CTEs.
    pub sql: &'static str,
}
impl Case {
    pub fn schema(self) -> Vec<RelationDecl> {
        schema::declarations(self.relations)
    }
    pub fn config(self, scenario: Scenario) -> GenerationConfig {
        GenerationConfig {
            scenario,
            scale: Scale::Tiny,
            seed: crate::seed_for_name(self.name),
        }
    }
    pub fn path(self, directory: &str) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join(directory)
            .join("extended")
            .join(self.name)
    }
    pub fn generate(self, scenario: Scenario) -> Result<Dataset, DatasetError> {
        let schema = self.schema();
        let mut rows = fixture(self.name, scenario);
        if scenario == Scenario::EmptyInput {
            rows[0].clear();
        }
        assert_eq!(
            rows.len(),
            schema.len(),
            "{} fixture input count",
            self.name
        );
        let relations = schema
            .iter()
            .zip(rows)
            .map(|(declaration, rows)| {
                RelationData::new(declaration.name.to_string(), declaration.arity(), rows)
            })
            .collect::<Result<Vec<_>, _>>()?;
        Dataset::new_schema(
            &schema,
            Suite::Extended,
            self.name,
            self.config(scenario),
            relations,
        )
    }
    pub fn load(self, scenario: Scenario) -> Result<Dataset, DatasetError> {
        let schema = self.schema();
        let dataset = Dataset::read_schema(
            &schema,
            self.path("data").join(scenario.as_str()).join("tiny"),
        )?;
        dataset.validate_provenance(Suite::Extended, self.name, self.config(scenario))?;
        if dataset != self.generate(scenario)? {
            return Err(DatasetError::Schema(format!(
                "stale extension fixture: {}",
                self.name
            )));
        }
        Ok(dataset)
    }
    pub fn inputs(self, scenario: Scenario) -> schema::Inputs {
        schema::Inputs::from_dataset(self.load(scenario).unwrap())
    }
    pub fn check(self, scenario: Scenario, mut actual: Vec<Vec<i32>>) {
        let expected = crate::dataset::read_result_rows(
            self.path("golden").join(scenario.as_str()).join("tiny.csv"),
            self.arity,
        )
        .unwrap();
        let count = actual.len();
        actual.sort();
        actual.dedup();
        assert_eq!(
            actual.len(),
            count,
            "{}/{} duplicate extension answers",
            self.name,
            scenario.as_str()
        );
        assert_eq!(actual, expected, "{}/{}", self.name, scenario.as_str());
    }
}

const R_X_TAG: Relation = Relation {
    name: "R",
    columns: &[("x", I32), ("tag", I32)],
};
const R_TAG_X: Relation = Relation {
    name: "R",
    columns: &[("tag", I32), ("x", I32)],
};
const R_X_Y: Relation = Relation {
    name: "R",
    columns: &[("x", I32), ("y", I32)],
};
const R_X_Y_Z: Relation = Relation {
    name: "R",
    columns: &[("x", I32), ("y", I32), ("z", I32)],
};
const R_X_UNUSED: Relation = Relation {
    name: "R",
    columns: &[("x", I32), ("unused", I32)],
};
const S_Y: Relation = Relation {
    name: "S",
    columns: &[("y", I32)],
};
const A_X: Relation = Relation {
    name: "A",
    columns: &[("x", I32)],
};
const B_X: Relation = Relation {
    name: "B",
    columns: &[("x", I32)],
};
const C_X: Relation = Relation {
    name: "C",
    columns: &[("x", I32)],
};
const D_X: Relation = Relation {
    name: "D",
    columns: &[("x", I32)],
};
const A_X_Y: Relation = Relation {
    name: "A",
    columns: &[("x", I32), ("y", I32)],
};
const B_X_Y: Relation = Relation {
    name: "B",
    columns: &[("x", I32), ("y", I32)],
};
const A_X_PAYLOAD: Relation = Relation {
    name: "A",
    columns: &[("x", I32), ("payload", I32)],
};
const S_X_Y: Relation = Relation {
    name: "S",
    columns: &[("x", I32), ("y", I32)],
};
const C_X_Y: Relation = Relation {
    name: "C",
    columns: &[("x", I32), ("y", I32)],
};
const SALE_D_ID_A: Relation = Relation {
    name: "Sale",
    columns: &[("d", I32), ("id", I32), ("a", I32)],
};
const SALE_D_K_ID_A: Relation = Relation {
    name: "Sale",
    columns: &[("d", I32), ("k", I32), ("id", I32), ("a", I32)],
};
const ALLOWED_D: Relation = Relation {
    name: "Allowed",
    columns: &[("d", I32)],
};
const EDGE_X_Y: Relation = Relation {
    name: "Edge",
    columns: &[("x", I32), ("y", I32)],
};
const SEED_X: Relation = Relation {
    name: "Seed",
    columns: &[("x", I32)],
};
const GOAL_X: Relation = Relation {
    name: "Goal",
    columns: &[("x", I32)],
};
const S_X_Y_ID: Relation = Relation {
    name: "S",
    columns: &[("x", I32), ("y", I32), ("id", I32)],
};
const BLOCKED_X_Y: Relation = Relation {
    name: "Blocked",
    columns: &[("x", I32), ("y", I32)],
};
const HIDDEN_X_Y: Relation = Relation {
    name: "Hidden",
    columns: &[("x", I32), ("y", I32)],
};

pub const CASES: &[Case] = &[
    Case {
        name: "retail_h04",
        family: "retail",
        relations: &[
            Relation {
                name: "Orders",
                columns: &[("order_key", I32)],
            },
            Relation {
                name: "LateLineitem",
                columns: &[("order_key", I32)],
            },
        ],
        rust_path: "src/queries/extended/retail_h04.rs",
        arity: 1,
        sql: include_str!("../sql/extended/retail_h04.sql"),
    },
    Case {
        name: "retail_h04_not_exists",
        family: "retail",
        relations: &[
            Relation {
                name: "Orders",
                columns: &[("order_key", I32)],
            },
            Relation {
                name: "LateLineitem",
                columns: &[("order_key", I32)],
            },
        ],
        rust_path: "src/queries/extended/retail_h04_not_exists.rs",
        arity: 1,
        sql: include_str!("../sql/extended/retail_h04_not_exists.sql"),
    },
    Case {
        name: "retail_h16",
        family: "retail",
        relations: &[
            Relation {
                name: "Part",
                columns: &[("part_key", I32)],
            },
            Relation {
                name: "Partsupp",
                columns: &[("part_key", I32), ("suppkey", I32)],
            },
            Relation {
                name: "BadSupplier",
                columns: &[("suppkey", I32)],
            },
        ],
        rust_path: "src/queries/extended/retail_h16.rs",
        arity: 2,
        sql: include_str!("../sql/extended/retail_h16.sql"),
    },
    Case {
        name: "retail_h22",
        family: "retail",
        relations: &[
            Relation {
                name: "Customer",
                columns: &[("custkey", I32)],
            },
            Relation {
                name: "Orders",
                columns: &[("order_key", I32), ("custkey", I32)],
            },
        ],
        rust_path: "src/queries/extended/retail_h22.rs",
        arity: 1,
        sql: include_str!("../sql/extended/retail_h22.sql"),
    },
    Case {
        name: "surface_constant",
        family: "surface",
        relations: &[R_X_TAG],
        rust_path: "src/queries/extended/surface_constant.rs",
        arity: 1,
        sql: include_str!("../sql/extended/surface_constant.sql"),
    },
    Case {
        name: "surface_negative_constant",
        family: "surface",
        relations: &[R_TAG_X],
        rust_path: "src/queries/extended/surface_negative_constant.rs",
        arity: 1,
        sql: include_str!("../sql/extended/surface_negative_constant.sql"),
    },
    Case {
        name: "surface_repeat",
        family: "surface",
        relations: &[R_X_Y],
        rust_path: "src/queries/extended/surface_repeat.rs",
        arity: 1,
        sql: include_str!("../sql/extended/surface_repeat.sql"),
    },
    Case {
        name: "surface_wildcards",
        family: "surface",
        relations: &[R_X_Y_Z],
        rust_path: "src/queries/extended/surface_wildcards.rs",
        arity: 1,
        sql: include_str!("../sql/extended/surface_wildcards.sql"),
    },
    Case {
        name: "surface_compare",
        family: "surface",
        relations: &[R_X_Y],
        rust_path: "src/queries/extended/surface_compare.rs",
        arity: 2,
        sql: include_str!("../sql/extended/surface_compare.sql"),
    },
    Case {
        name: "surface_arithmetic_guard",
        family: "surface",
        relations: &[R_X_Y],
        rust_path: "src/queries/extended/surface_arithmetic_guard.rs",
        arity: 2,
        sql: include_str!("../sql/extended/surface_arithmetic_guard.sql"),
    },
    Case {
        name: "surface_boolean_guard",
        family: "surface",
        relations: &[R_X_Y],
        rust_path: "src/queries/extended/surface_boolean_guard.rs",
        arity: 2,
        sql: include_str!("../sql/extended/surface_boolean_guard.sql"),
    },
    Case {
        name: "surface_let_sum",
        family: "surface",
        relations: &[R_X_Y],
        rust_path: "src/queries/extended/surface_let_sum.rs",
        arity: 2,
        sql: include_str!("../sql/extended/surface_let_sum.sql"),
    },
    Case {
        name: "surface_let_chain",
        family: "surface",
        relations: &[R_X_Y],
        rust_path: "src/queries/extended/surface_let_chain.rs",
        arity: 2,
        sql: include_str!("../sql/extended/surface_let_chain.sql"),
    },
    Case {
        name: "surface_computed_head",
        family: "surface",
        relations: &[R_X_Y],
        rust_path: "src/queries/extended/surface_computed_head.rs",
        arity: 2,
        sql: include_str!("../sql/extended/surface_computed_head.sql"),
    },
    Case {
        name: "surface_self_join",
        family: "surface",
        relations: &[R_X_Y],
        rust_path: "src/queries/extended/surface_self_join.rs",
        arity: 2,
        sql: include_str!("../sql/extended/surface_self_join.sql"),
    },
    Case {
        name: "surface_disconnected",
        family: "surface",
        relations: &[R_X_UNUSED, S_Y],
        rust_path: "src/queries/extended/surface_disconnected.rs",
        arity: 1,
        sql: include_str!("../sql/extended/surface_disconnected.sql"),
    },
    Case {
        name: "union_two",
        family: "union",
        relations: &[A_X, B_X],
        rust_path: "src/queries/extended/union_two.rs",
        arity: 1,
        sql: include_str!("../sql/extended/union_two.sql"),
    },
    Case {
        name: "union_join",
        family: "union",
        relations: &[A_X, B_X, C_X, D_X],
        rust_path: "src/queries/extended/union_join.rs",
        arity: 1,
        sql: include_str!("../sql/extended/union_join.sql"),
    },
    Case {
        name: "union_project",
        family: "union",
        relations: &[A_X_Y, B_X_Y],
        rust_path: "src/queries/extended/union_project.rs",
        arity: 1,
        sql: include_str!("../sql/extended/union_project.sql"),
    },
    Case {
        name: "negation_key",
        family: "negation",
        relations: &[A_X_PAYLOAD, S_X_Y],
        rust_path: "src/queries/extended/negation_key.rs",
        arity: 1,
        sql: include_str!("../sql/extended/negation_key.sql"),
    },
    Case {
        name: "negation_pair",
        family: "negation",
        relations: &[A_X_Y, S_X_Y],
        rust_path: "src/queries/extended/negation_pair.rs",
        arity: 2,
        sql: include_str!("../sql/extended/negation_pair.sql"),
    },
    Case {
        name: "negation_twice",
        family: "negation",
        relations: &[A_X, B_X_Y, C_X_Y],
        rust_path: "src/queries/extended/negation_twice.rs",
        arity: 1,
        sql: include_str!("../sql/extended/negation_twice.sql"),
    },
    Case {
        name: "aggregate_stats",
        family: "aggregate",
        relations: &[SALE_D_ID_A],
        rust_path: "src/queries/extended/aggregate_stats.rs",
        arity: 5,
        sql: include_str!("../sql/extended/aggregate_stats.sql"),
    },
    Case {
        name: "aggregate_two_keys",
        family: "aggregate",
        relations: &[SALE_D_K_ID_A],
        rust_path: "src/queries/extended/aggregate_two_keys.rs",
        arity: 4,
        sql: include_str!("../sql/extended/aggregate_two_keys.sql"),
    },
    Case {
        name: "aggregate_join",
        family: "aggregate",
        relations: &[SALE_D_ID_A, ALLOWED_D],
        rust_path: "src/queries/extended/aggregate_join.rs",
        arity: 3,
        sql: include_str!("../sql/extended/aggregate_join.sql"),
    },
    Case {
        name: "recursion_closure",
        family: "recursion",
        relations: &[EDGE_X_Y],
        rust_path: "src/queries/extended/recursion_closure.rs",
        arity: 2,
        sql: include_str!("../sql/extended/recursion_closure.sql"),
    },
    Case {
        name: "recursion_reach",
        family: "recursion",
        relations: &[SEED_X, EDGE_X_Y],
        rust_path: "src/queries/extended/recursion_reach.rs",
        arity: 1,
        sql: include_str!("../sql/extended/recursion_reach.sql"),
    },
    Case {
        name: "recursion_reverse",
        family: "recursion",
        relations: &[GOAL_X, EDGE_X_Y],
        rust_path: "src/queries/extended/recursion_reverse.rs",
        arity: 1,
        sql: include_str!("../sql/extended/recursion_reverse.sql"),
    },
    Case {
        name: "correlated_parameter",
        family: "correlated",
        relations: &[A_X_PAYLOAD],
        rust_path: "src/queries/extended/correlated_parameter.rs",
        arity: 1,
        sql: include_str!("../sql/extended/correlated_parameter.sql"),
    },
    Case {
        name: "correlated_exists",
        family: "correlated",
        relations: &[A_X, S_X_Y],
        rust_path: "src/queries/extended/correlated_exists.rs",
        arity: 1,
        sql: include_str!("../sql/extended/correlated_exists.sql"),
    },
    Case {
        name: "correlated_scalar",
        family: "correlated",
        relations: &[A_X, S_X_Y_ID],
        rust_path: "src/queries/extended/correlated_scalar.rs",
        arity: 2,
        sql: include_str!("../sql/extended/correlated_scalar.sql"),
    },
    Case {
        name: "stacked_antijoin",
        family: "stacked",
        relations: &[A_X, B_X, BLOCKED_X_Y],
        rust_path: "src/queries/extended/stacked_antijoin.rs",
        arity: 1,
        sql: include_str!("../sql/extended/stacked_antijoin.sql"),
    },
    Case {
        name: "stacked_mixed",
        family: "stacked",
        relations: &[A_X, B_X, BLOCKED_X_Y, HIDDEN_X_Y],
        rust_path: "src/queries/extended/stacked_mixed.rs",
        arity: 1,
        sql: include_str!("../sql/extended/stacked_mixed.sql"),
    },
];

pub fn case(name: &str) -> Option<&'static Case> {
    CASES.iter().find(|case| case.name == name)
}

fn fixture(name: &str, scenario: Scenario) -> Vec<Vec<Vec<i32>>> {
    let no_match = scenario == Scenario::NoMatch;
    // All coverage fixtures retain duplicates and deliberately unsorted rows.
    // No-match keeps every input nonempty. Empty-input empties only input 0.
    match (name, no_match) {
        ("retail_h04", false) | ("retail_h04_not_exists", false) => vec![
            vec![vec![3], vec![1], vec![2], vec![1]],
            vec![vec![1], vec![1], vec![9]],
        ],
        ("retail_h04", true) => vec![vec![vec![1]], vec![vec![9]]],
        ("retail_h04_not_exists", true) => vec![vec![vec![1]], vec![vec![1]]],
        ("retail_h16", false) => vec![
            vec![vec![2], vec![1], vec![1]],
            vec![
                vec![1, 10],
                vec![1, 11],
                vec![2, 10],
                vec![2, 12],
                vec![1, 11],
            ],
            vec![vec![10], vec![10], vec![99]],
        ],
        ("retail_h16", true) => vec![vec![vec![1]], vec![vec![1, 10]], vec![vec![10]]],
        ("retail_h22", false) => vec![
            vec![vec![3], vec![1], vec![2], vec![3]],
            vec![vec![10, 1], vec![11, 1], vec![10, 1], vec![90, 9]],
        ],
        ("retail_h22", true) => vec![vec![vec![1]], vec![vec![10, 1]]],
        ("surface_constant", false) => vec![vec![vec![3, 7], vec![-1, 7], vec![3, 8], vec![3, 7]]],
        ("surface_constant", true) => vec![vec![vec![3, 8]]],
        ("surface_negative_constant", false) => {
            vec![vec![vec![-2, 3], vec![2, 3], vec![-2, -1], vec![-2, 3]]]
        }
        ("surface_negative_constant", true) => vec![vec![vec![2, 3]]],
        ("surface_repeat", false) => vec![vec![vec![2, 2], vec![-1, -1], vec![2, 3], vec![2, 2]]],
        ("surface_repeat", true) => vec![vec![vec![2, 3]]],
        ("surface_wildcards", false) => vec![vec![
            vec![2, 3, 4],
            vec![2, 5, 6],
            vec![-1, 7, 7],
            vec![2, 3, 4],
        ]],
        ("surface_wildcards", true) => vec![vec![vec![-1, 7, 8]]],
        ("surface_compare", false) => vec![vec![
            vec![-2, 1],
            vec![1, 2],
            vec![2, 2],
            vec![-3, 4],
            vec![1, 2],
        ]],
        ("surface_compare", true) => vec![vec![vec![2, 1]]],
        ("surface_arithmetic_guard", false) => vec![vec![
            vec![-1, 2],
            vec![3, 0],
            vec![0, 1],
            vec![5, -2],
            vec![-1, 2],
        ]],
        ("surface_arithmetic_guard", true) => vec![vec![vec![0, 1]]],
        ("surface_boolean_guard", false) => vec![vec![
            vec![-1, 0],
            vec![2, 3],
            vec![-1, -1],
            vec![3, 3],
            vec![1, 2],
            vec![-1, 0],
        ]],
        ("surface_boolean_guard", true) => vec![vec![vec![1, 2]]],
        ("surface_let_sum" | "surface_let_chain", false) => vec![vec![
            vec![2, 3],
            vec![3, 2],
            vec![-1, 2],
            vec![1, -2],
            vec![2, 3],
        ]],
        ("surface_let_sum" | "surface_let_chain", true) => vec![vec![vec![1, -2]]],
        ("surface_computed_head", false) => {
            vec![vec![vec![2, 3], vec![0, -1], vec![-1, 2], vec![2, 3]]]
        }
        ("surface_computed_head", true) => vec![vec![vec![-1, 2]]],
        ("surface_self_join", false) => vec![vec![
            vec![1, 2],
            vec![2, 3],
            vec![3, 1],
            vec![2, 4],
            vec![1, 2],
        ]],
        ("surface_self_join", true) => vec![vec![vec![2, 1]]],
        ("surface_disconnected", false) => vec![
            vec![vec![3, 8], vec![3, 9], vec![-1, 7], vec![3, 8]],
            vec![vec![1], vec![-2], vec![4], vec![1]],
        ],
        ("surface_disconnected", true) => vec![vec![vec![1, 8]], vec![vec![2]]],
        ("union_two", false) => vec![
            vec![vec![3], vec![1], vec![1], vec![-1]],
            vec![vec![2], vec![3], vec![3], vec![-2]],
        ],
        ("union_two", true) => vec![vec![vec![-1]], vec![vec![-2]]],
        ("union_join", false) => vec![
            vec![vec![1], vec![1], vec![2]],
            vec![vec![1], vec![3]],
            vec![vec![1], vec![4]],
            vec![vec![4], vec![1]],
        ],
        ("union_join", true) => vec![vec![vec![1]], vec![vec![2]], vec![vec![3]], vec![vec![4]]],
        ("union_project", false) => vec![
            vec![vec![1, 10], vec![1, 11], vec![2, -1]],
            vec![vec![1, 12], vec![3, 13], vec![3, 13]],
        ],
        ("union_project", true) => vec![vec![vec![1, -1]], vec![vec![2, -2]]],
        ("negation_key", false) => vec![
            vec![
                vec![3, 10],
                vec![3, 11],
                vec![1, 20],
                vec![2, 30],
                vec![3, 10],
            ],
            vec![vec![2, 20], vec![2, 21], vec![9, 90], vec![2, 20]],
        ],
        ("negation_key", true) => vec![vec![vec![1, 10]], vec![vec![1, 10]]],
        ("negation_pair", false) => vec![
            vec![vec![1, 10], vec![1, 11], vec![2, 20], vec![1, 11]],
            vec![vec![1, 10], vec![9, 99], vec![1, 10]],
        ],
        ("negation_pair", true) => vec![vec![vec![1, 10]], vec![vec![1, 10]]],
        ("negation_twice", false) => vec![
            vec![vec![1], vec![2], vec![3], vec![4], vec![4]],
            vec![vec![2, 20], vec![9, 90]],
            vec![vec![3, 30], vec![9, 90]],
        ],
        ("negation_twice", true) => vec![vec![vec![1]], vec![vec![1, 10]], vec![vec![2, 20]]],
        ("aggregate_stats", false) => vec![vec![
            vec![2, 5, 7],
            vec![1, 2, 10],
            vec![1, 1, 10],
            vec![1, 3, -5],
            vec![1, 1, 10],
            vec![-1, 9, 99],
        ]],
        ("aggregate_stats", true) => vec![vec![vec![-1, 1, 10]]],
        ("aggregate_two_keys", false) => vec![vec![
            vec![1, 2, 1, 8],
            vec![1, 2, 2, 8],
            vec![1, 3, 3, -4],
            vec![2, 2, 4, 5],
            vec![1, 2, 1, 8],
        ]],
        ("aggregate_two_keys", true) => vec![vec![vec![-1, 2, 1, 8]]],
        ("aggregate_join", false) => vec![
            vec![
                vec![1, 1, 10],
                vec![1, 2, -5],
                vec![2, 3, 7],
                vec![3, 4, 99],
                vec![1, 1, 10],
            ],
            vec![vec![2], vec![1], vec![1]],
        ],
        ("aggregate_join", true) => vec![vec![vec![1, 1, 10]], vec![vec![9]]],
        ("recursion_closure", false) => vec![vec![
            vec![0, 1],
            vec![1, 2],
            vec![2, 1],
            vec![2, 3],
            vec![7, 8],
            vec![0, 1],
        ]],
        ("recursion_closure", true) => vec![vec![vec![-2, -1]]],
        ("recursion_reach", false) => vec![
            vec![vec![0], vec![0]],
            vec![
                vec![0, 1],
                vec![1, 2],
                vec![2, 1],
                vec![2, 3],
                vec![7, 8],
                vec![0, 1],
            ],
        ],
        ("recursion_reach", true) => vec![vec![vec![9]], vec![vec![0, 1]]],
        ("recursion_reverse", false) => vec![
            vec![vec![3], vec![3]],
            vec![
                vec![0, 1],
                vec![1, 2],
                vec![2, 1],
                vec![2, 3],
                vec![7, 8],
                vec![0, 1],
            ],
        ],
        ("recursion_reverse", true) => vec![vec![vec![9]], vec![vec![0, 1]]],
        ("correlated_parameter", false) => vec![vec![
            vec![1, 10],
            vec![3, 20],
            vec![5, 30],
            vec![3, 21],
            vec![3, 20],
        ]],
        ("correlated_parameter", true) => vec![vec![vec![1, 10], vec![2, 20]]],
        ("correlated_exists", false) => vec![
            vec![vec![1], vec![2], vec![3], vec![1]],
            vec![vec![1, 5], vec![1, 7], vec![2, 1], vec![9, 8], vec![1, 5]],
        ],
        ("correlated_exists", true) => vec![vec![vec![1]], vec![vec![1, 2], vec![9, 9]]],
        ("correlated_scalar", false) => vec![
            vec![vec![1], vec![2], vec![3], vec![1]],
            vec![
                vec![1, 7, 10],
                vec![1, 7, 11],
                vec![2, 5, 20],
                vec![9, 8, 30],
                vec![1, 7, 10],
            ],
        ],
        ("correlated_scalar", true) => vec![
            vec![vec![1], vec![2]],
            vec![vec![1, 2, 10], vec![1, 2, 11], vec![9, 9, 20]],
        ],
        ("stacked_antijoin", false) => vec![
            vec![vec![1], vec![2], vec![3], vec![1]],
            vec![vec![3], vec![4], vec![4]],
            vec![vec![2, 20], vec![9, 90]],
        ],
        ("stacked_antijoin", true) => vec![vec![vec![1]], vec![vec![1]], vec![vec![1, 10]]],
        ("stacked_mixed", false) => vec![
            vec![vec![1], vec![2], vec![3], vec![1]],
            vec![vec![3], vec![4], vec![4]],
            vec![vec![2, 20]],
            vec![vec![3, 30]],
        ],
        ("stacked_mixed", true) => vec![
            vec![vec![1]],
            vec![vec![1]],
            vec![vec![1, 10]],
            vec![vec![1, 10]],
        ],
        _ => panic!("missing fixture {name}"),
    }
}
