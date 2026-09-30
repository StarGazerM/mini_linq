//! Authored surface and extension queries, independent SQL, and tiny fixtures.
//! This catalog does not send an extension through a positive-CQ evaluator.
use std::path::{Path, PathBuf};

use cq_syntax::RelationDecl;
use proc_macro2::{TokenStream, TokenTree};

use crate::{
    GenerationConfig, Scale, Scenario, Suite,
    dataset::{Dataset, DatasetError, RelationData},
};

/// Export the authored token tree to real consumers, keeping host hygiene.
#[macro_export]
macro_rules! extended_query {
    ($d:tt; $exported:ident; $case:literal; $family:ident; $arity:literal; parameters {$($params:tt)*}; {$($program:tt)*}) => {
        #[doc(hidden)]
        #[macro_export]
        macro_rules! $exported {
            ($d($d callback:ident)::+) => {
                $d($d callback)::+! { [$case; $family; $arity; parameters {$($params)*}] $($program)* }
            };
        }
        pub use $exported as with_query;
        pub const SOURCE: &str = stringify!($($program)*);
        pub const PARAMETERS: &str = stringify!($($params)*);
        pub const FAMILY: &str = stringify!($family);
        pub const ARITY: usize = $arity;
        pub const RUST_PATH: &str = file!();
    };
}

#[derive(Clone, Copy, Debug)]
pub struct Case {
    pub name: &'static str,
    pub family: &'static str,
    pub source: &'static str,
    pub parameters: &'static str,
    pub rust_path: &'static str,
    pub arity: usize,
    /// Independently authored SQL, using set-valued input CTEs.
    pub sql: &'static str,
}
impl Case {
    pub fn schema(self) -> syn::Result<Vec<RelationDecl>> {
        schema(self.source.parse()?)
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
        let schema = self.schema().map_err(DatasetError::InvalidQuery)?;
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
        let schema = self.schema().map_err(DatasetError::InvalidQuery)?;
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

/// Parse only the CQ header. Aggregation has a fold prelude; union/recursion
/// have additional rules. The extension's own parser checks those surfaces.
pub fn schema(tokens: TokenStream) -> syn::Result<Vec<RelationDecl>> {
    let tokens = tokens
        .into_iter()
        .skip_while(|token| !matches!(token, TokenTree::Ident(id) if id == "struct"))
        .collect();
    let rules: cq_ext_support::Rules = syn::parse2(tokens)?;
    Ok(rules.first.inputs().cloned().collect())
}

macro_rules! case {
    ($leaf:ident) => {{
        use crate::queries::extended::$leaf as query;
        Case {
            name: stringify!($leaf),
            family: query::FAMILY,
            source: query::SOURCE,
            parameters: query::PARAMETERS,
            rust_path: query::RUST_PATH,
            arity: query::ARITY,
            sql: include_str!(concat!("../sql/extended/", stringify!($leaf), ".sql")),
        }
    }};
}
pub const CASES: &[Case] = &[
    case!(surface_constant),
    case!(surface_negative_constant),
    case!(surface_repeat),
    case!(surface_wildcards),
    case!(surface_compare),
    case!(surface_arithmetic_guard),
    case!(surface_boolean_guard),
    case!(surface_let_sum),
    case!(surface_let_chain),
    case!(surface_computed_head),
    case!(surface_self_join),
    case!(surface_disconnected),
    case!(union_two),
    case!(union_join),
    case!(union_project),
    case!(negation_key),
    case!(negation_pair),
    case!(negation_twice),
    case!(aggregate_stats),
    case!(aggregate_two_keys),
    case!(aggregate_join),
    case!(recursion_closure),
    case!(recursion_reach),
    case!(recursion_reverse),
    case!(correlated_parameter),
    case!(correlated_exists),
    case!(correlated_scalar),
    case!(stacked_antijoin),
    case!(stacked_mixed),
];
pub fn case(name: &str) -> Option<&'static Case> {
    CASES.iter().find(|case| case.name == name)
}

fn fixture(name: &str, scenario: Scenario) -> Vec<Vec<Vec<i32>>> {
    let no_match = scenario == Scenario::NoMatch;
    // All coverage fixtures retain duplicates and deliberately unsorted rows.
    // No-match keeps every input nonempty. Empty-input empties only input 0.
    match (name, no_match) {
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
