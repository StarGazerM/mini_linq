//! Classic workload order and teaching metadata. Matrix tests compile the
//! authoritative literal programs in `src/queries/classic` directly.

use crate::{QueryCase, Suite};

const COLORED_CQ_SCOPE: &str = "MiniLinq evaluates a set-valued colored conjunctive query (a homomorphism pattern), not induced-subgraph matching: no pairwise inequality or non-edge constraint is implied.";

const SPADE_SCOPE: &str = "`spade` is this course's alias for the triangle-with-one-edge-handle shape called lollipop in join-processing literature and paw in graph theory. It remains a colored CQ/homomorphism pattern, not induced-subgraph matching: no pairwise inequality or non-edge constraint is implied.";

macro_rules! classic_case {
    ($module:ident, $name:literal, $purpose:expr, $scope_note:expr $(,)?) => {{
        QueryCase {
            suite: Suite::Classic,
            name: $name,
            rust_path: concat!("src/queries/classic/", stringify!($module), ".rs"),
            purpose: $purpose,
            scope_note: $scope_note,
            sql: include_str!(concat!("../../sql/classic/", $name, ".sql")),
            inputs: super::schemas::$module::INPUTS,
            arity: super::schemas::$module::ARITY,
        }
    }};
}

/// Ten small, structurally distinct positive CQs used throughout the course.
///
/// The order and command-line names are public workload data. Each entry keeps
/// its complete rule in one authored Rust query module, so a geometric
/// nickname can never define semantics by itself.
pub static CLASSIC: &[QueryCase] = &[
    classic_case!(
        intersection,
        "intersection",
        "The smallest equality join: scan one unary relation, then perform full-key membership in the other. It exercises a fully bound clause with no complement columns.",
        COLORED_CQ_SCOPE,
    ),
    classic_case!(
        cartesian_product,
        "cartesian-product",
        "A disconnected CQ whose second atom shares no variables with the first. It verifies that a later zero-bound-key occurrence remains an independent row scan rather than acquiring an index.",
        COLORED_CQ_SCOPE,
    ),
    classic_case!(
        oriented_chain,
        "oriented-chain",
        "A three-edge chain over one self-joined relation. Its source order requires both Edge[0] and the non-leading Edge[1] index from one normalized input; projecting away b and c also exercises output set semantics.",
        COLORED_CQ_SCOPE,
    ),
    classic_case!(
        claw,
        "claw",
        "The variable-graph star K_1,3 (the claw): an acyclic branch that repeatedly reuses one center binding while enumerating three independent leaves.",
        COLORED_CQ_SCOPE,
    ),
    classic_case!(
        fact_star,
        "fact-star",
        "The traditional relation-join-graph star: scan one ternary fact relation, then probe three dimensions through different fact columns. It distinguishes a fact/dimension star from the variable-graph claw.",
        COLORED_CQ_SCOPE,
    ),
    classic_case!(
        triangle,
        "triangle",
        "The minimal cyclic CQ and the repository's recurring baseline: one scan, one partial lookup, and one full-key membership that closes the cycle.",
        COLORED_CQ_SCOPE,
    ),
    classic_case!(
        cycle_4,
        "cycle-4",
        "A chordless four-cycle: cyclic but not a clique. It extends the partial-lookup chain before a final full-key membership closes the cycle.",
        COLORED_CQ_SCOPE,
    ),
    classic_case!(
        spade,
        "spade",
        "A triangle with a one-edge handle: execution closes the cyclic core and then resumes a branch from the earlier x binding. `spade` is the course alias for the lollipop/paw shape.",
        SPADE_SCOPE,
    ),
    classic_case!(
        barbell,
        "barbell",
        "Two triangles connected by one edge. It exercises two separated cyclic regions, two full-key membership closures, and a deeper resumable continuation than the one-cycle cases.",
        COLORED_CQ_SCOPE,
    ),
    classic_case!(
        loomis_whitney_4,
        "loomis-whitney-4",
        "The four-dimensional Loomis-Whitney query: each ternary atom omits a different variable. It exercises a two-column partial key followed by full ternary memberships and is not a padded larger clique.",
        COLORED_CQ_SCOPE,
    ),
];
