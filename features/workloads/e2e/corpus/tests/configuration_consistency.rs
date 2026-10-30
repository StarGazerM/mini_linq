//! Check the handwritten configuration witnesses without expanding any pipeline.
//! Names are decoded independently; the complete matcher and expansion token trees
//! must match their meaning, so missing, duplicate, or extra settings cannot hide.
use proc_macro2::{Delimiter, Group, TokenStream, TokenTree};
use std::collections::BTreeSet;

// Numeric order is the nine-axis contract in matrix.rs, not macro-arm order.
const AXES: [&[&str]; 9] = [
    &[
        "written_coarse",
        "written_fine",
        "hypergraph",
        "prefix_connected_coarse",
        "prefix_connected_fine",
        "prefix_greedy_coarse",
        "prefix_greedy_fine",
        "elimination_coarse",
        "elimination_fine",
        "le",
        "le_runs",
    ],
    &["probe", "merge"],
    &["keep", "canon"],
    &["first", "smallest"],
    &["slice", "hashmap", "btreemap"],
    &["default", "sorted"],
    &["eager", "lazy"],
    &["pull", "push"],
    &["unadvised", "positional"],
];

const PLANNERS: [&str; 11] = [
    "written, coarse",
    "written, fine",
    "hypergraph",
    "prefix connected, coarse",
    "prefix connected, fine",
    "prefix greedy, coarse",
    "prefix greedy, fine",
    "elimination, coarse",
    "elimination, fine",
    "le",
    "le_runs",
];

fn decode_name(name: &str) -> [usize; 9] {
    let mut rest = name;
    let mut row = [0; 9];
    for (axis, values) in AXES.iter().enumerate() {
        let (value, suffix) = values
            .iter()
            .enumerate()
            .filter_map(|(value, spelling)| {
                let suffix = rest.strip_prefix(spelling)?;
                if axis == 8 {
                    suffix.is_empty().then_some((value, suffix))
                } else {
                    suffix.strip_prefix('_').map(|suffix| (value, suffix))
                }
            })
            .min_by_key(|(_, suffix)| suffix.len())
            .unwrap_or_else(|| panic!("{name}: unknown or missing axis {axis} in {rest:?}"));
        row[axis] = value;
        rest = suffix;
    }
    assert!(
        row[4] == 0 || row[8] == 0,
        "{name}: positional advice is unsupported for maps"
    );
    row
}

fn group(token: Option<TokenTree>, delimiter: Delimiter, context: &str) -> Group {
    match token {
        Some(TokenTree::Group(group)) if group.delimiter() == delimiter => group,
        other => panic!("{context}: expected {delimiter:?} group, got {other:?}"),
    }
}

fn punctuation(token: Option<TokenTree>, expected: char, name: &str) {
    assert!(
        matches!(&token, Some(TokenTree::Punct(punct)) if punct.as_char() == expected),
        "{name}: expected punctuation {expected:?}, got {token:?}"
    );
}

fn assert_tokens(actual: TokenStream, expected: &str, name: &str, part: &str) {
    let expected: TokenStream = expected.parse().expect("valid expected Rust tokens");
    // Both sides are tokenized first: whitespace/comments are immaterial, but
    // every identifier, literal, punctuation token and delimiter is retained.
    assert_eq!(
        actual.to_string(),
        expected.to_string(),
        "{name}: {part} drift"
    );
}

fn expected_expansion(row: [usize; 9]) -> String {
    let planner = PLANNERS[row[0]];
    let merge = ["", "cq_rewrites::merge_joins,"][row[1]];
    let canon = ["", "cq_rewrites::canon_prefixes,"][row[2]];
    let intersection = ["", "cq_rewrites::smallest_intersections,"][row[3]];
    let host = ["[take] slice", "[take_hash] map", "[take_btree] map"][row[4]];
    let payload = ["", "cq_workload_passes::sorted_payload,"][row[5]];
    let membership = ["", "cq_rewrites::lazy_semijoins,"][row[6]];
    let printer = ["mini_linq::passes::translate_pull", "own_push::push"][row[7]];
    let print_input = ["", "print_input: structures;"][row[7]];
    let advice = ["", "positional"][row[8]];
    format!(
        "pub const CONFIG: [usize; 9] = {row:?};
         pipeline_configuration! {{ $case, golden_scenarios;
             planner: [{planner}];
             merge: [{merge}];
             canon: [{canon}];
             intersection: [{intersection}];
             host: {host} [{advice}];
             payload: [{payload}];
             membership: [{membership}];
             {print_input}
             printer: {printer};
         }}"
    )
}

fn check_configurations(source: &str, macro_name: &str, expected: usize) -> BTreeSet<usize> {
    let source = syn::parse_file(source).expect("configuration source must parse as Rust");
    let definitions: Vec<_> = source
        .items
        .into_iter()
        .filter_map(|item| match item {
            syn::Item::Macro(item) if item.ident.as_ref().is_some_and(|id| id == macro_name) => {
                Some(item)
            }
            _ => None,
        })
        .collect();
    assert_eq!(definitions.len(), 1, "expected one configuration macro");
    let definition = &definitions[0];
    assert!(definition.mac.path.is_ident("macro_rules"));
    let mut arms = definition.mac.tokens.clone().into_iter();
    let mut names = BTreeSet::new();
    let mut planners = BTreeSet::new();
    while let Some(matcher) = arms.next() {
        let matcher = group(Some(matcher), Delimiter::Parenthesis, "configuration arm");
        let name = match matcher.stream().into_iter().next() {
            Some(TokenTree::Ident(name)) => name.to_string(),
            other => panic!("configuration arm: expected a configuration name, got {other:?}"),
        };
        assert!(
            names.insert(name.clone()),
            "{name}: duplicate configuration arm"
        );
        let row = decode_name(&name);
        planners.insert(row[0]);
        assert_tokens(
            matcher.stream(),
            &format!("{name}, $case:ident"),
            &name,
            "matcher",
        );
        punctuation(arms.next(), '=', &name);
        punctuation(arms.next(), '>', &name);
        let expansion = group(arms.next(), Delimiter::Brace, &name);
        assert_tokens(
            expansion.stream(),
            &expected_expansion(row),
            &name,
            "CONFIG/pipeline",
        );
        punctuation(arms.next(), ';', &name);
    }
    assert_eq!(names.len(), expected, "configuration arm count changed");
    planners
}

#[test]
fn configuration_names_rows_and_pipeline_tokens_agree() {
    assert_eq!(
        check_configurations(
            include_str!("support/configurations.rs"),
            "configuration",
            80
        ),
        (0..9).collect(),
        "the original 80 witnesses and nine planner indices stay unchanged",
    );
    assert_eq!(
        check_configurations(
            include_str!("support/le_configurations.rs"),
            "le_configuration",
            18
        ),
        [9, 10].into_iter().collect(),
        "both LE planners must have independent configuration witnesses",
    );
}

// Expand only the registration constants, never a query or compiler macro.
// This checks the same inventory consumed by runtime and Sequence conformance.
mod eligibility {
    use cq_workloads::{catalog, pipelines::Pipeline};

    macro_rules! pipeline_configuration {
        ($($tokens:tt)*) => {};
    }
    #[macro_use]
    mod le_configurations {
        include!("support/le_configurations.rs");
    }
    #[macro_use]
    mod configurations {
        include!("support/configurations.rs");
    }

    macro_rules! cases {
        ($(($case:ident, $runner:ident)),+ $(,)?) => {
            $(mod $case { case_configurations!($case, $runner); })+

            #[test]
            fn registered_configurations_are_eligible() {
                let cases = [$( (stringify!($case), $case::CONFIGURATIONS) ),+];
                assert_eq!(cases.len(), catalog::all().count());
                let mut rejected = Vec::new();
                for case in catalog::all() {
                    let stem = case.name.replace('-', "_");
                    let rows = cases.iter().find(|(name, _)| *name == stem)
                        .expect("every catalog query has registered configurations").1;
                    let source = case.module().unwrap();
                    for row in rows {
                        let pipeline = Pipeline {
                            // LE and LE-with-sorted-runs share the physical plan.
                            plan: [0, 1, 2, 5, 6, 7, 8, 9, 10, 4, 4][row[0]],
                            merge: row[1] != 0,
                            canon: row[2] != 0,
                            smallest: row[3] != 0,
                            sorted_payload: row[5] != 0,
                            lazy: row[6] != 0,
                            push: row[7] != 0,
                            positional: row[8] != 0,
                        };
                        if let Some(reason) = pipeline.rejection(&source, row[4]).unwrap() {
                            let name = row.iter().enumerate()
                                .map(|(axis, &value)| super::AXES[axis][value])
                                .collect::<Vec<_>>().join("_");
                            rejected.push(format!("{}/{name}: {reason}", case.name));
                        }
                    }
                }
                assert!(rejected.is_empty(), "ineligible registrations:\n{}", rejected.join("\n"));
            }
        };
    }

    cases! {
        (intersection, run_intersection),
        (cartesian_product, run_cartesian_product),
        (oriented_chain, run_oriented_chain),
        (claw, run_claw),
        (fact_star, run_fact_star),
        (triangle, run_triangle),
        (cycle_4, run_cycle_4),
        (spade, run_spade),
        (barbell, run_barbell),
        (loomis_whitney_4, run_loomis_whitney_4),
        (retail_h03, run_retail_h03),
        (retail_h05, run_retail_h05),
        (retail_h07, run_retail_h07),
        (retail_h08, run_retail_h08),
        (retail_h09, run_retail_h09),
        (retail_h21_positive, run_retail_h21_positive),
        (retail_d19, run_retail_d19),
        (retail_d27, run_retail_d27),
        (retail_d72_inner, run_retail_d72_inner),
        (retail_d85, run_retail_d85),
    }
}
