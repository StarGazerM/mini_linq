//! Retail-derived workload order and teaching metadata. The authoritative
//! query programs are ordinary Rust modules under [`crate::queries::retail`].

use crate::{QueryCase, Suite, queries};

macro_rules! retail_case {
    ($module:ident, $name:literal, $purpose:expr, $scope_note:expr $(,)?) => {{
        mod loaded {
            crate::queries::retail::$module::with_query!(crate::loading::capture);
        }
        QueryCase {
            suite: Suite::Retail,
            name: $name,
            rust_path: queries::retail::$module::RUST_PATH,
            purpose: $purpose,
            scope_note: $scope_note,
            sql: include_str!(concat!("../../sql/retail/", $name, ".sql")),
            module: queries::retail::$module::module,
            load: loaded::read_relations,
        }
    }};
}

// These independently named integer join cores are inspired by TPC-H/TPC-DS
// topology. They are not compliant TPC benchmark queries or results.
pub static RETAIL: &[QueryCase] = &[
    retail_case!(
        retail_h03,
        "retail_h03",
        "A customer-to-order-to-lineitem chain whose two successive foreign-key bindings become partial-key lookups; its narrow head also exercises output set semantics.",
        "Inspired only by the Customer-Orders-Lineitem join topology of TPC-H Query 3. It changes the projection and omits the market-segment and date filters, revenue expression and aggregation, ordering, and limit; it adds no outer-join or negative semantics and is not a compliant TPC query or result.",
    ),
    retail_case!(
        retail_h05,
        "retail_h05",
        "A genuine Customer-Orders-Lineitem-Supplier-Nation cycle with a Region tail; the shared nation binding closes the cycle before a final dimension membership.",
        "Inspired only by the join topology of TPC-H Query 5. It changes the projection and omits the region and order-date filters, price/discount expression and revenue aggregation, and ordering; it adds no outer-join or negative semantics and is not a compliant TPC query or result.",
    ),
    retail_case!(
        retail_h07,
        "retail_h07",
        "A shipping chain with two role-playing accesses to one Nation input; both occurrences require the same key shape and should share one physical index.",
        "Inspired only by the two-nation join topology of TPC-H Query 7. It changes the projection and omits the nation-pair disjunction and ship-date filters, price/discount expression and revenue aggregation, year extraction, and ordering; it adds no outer-join or negative semantics and is not a compliant TPC query or result.",
    ),
    retail_case!(
        retail_h08,
        "retail_h08",
        "A branched snowflake rooted at Lineitem, with independent part, supplier-nation, and order-customer-region branches and two differently bound Nation occurrences.",
        "Inspired only by the snowflake join topology of TPC-H Query 8. It changes the projection and omits the part-type, region, nation, and order-date filters, price/discount and conditional market-share aggregates, year extraction, and ordering; it adds no outer-join or negative semantics and is not a compliant TPC query or result.",
    ),
    retail_case!(
        retail_h09,
        "retail_h09",
        "A fact-centered cyclic/diamond shape whose PartSupp occurrence is a fully bound composite-key membership rather than two independent unary probes.",
        "Inspired only by the composite PartSupp join topology of TPC-H Query 9. It changes the projection and omits the part-name filter, price, discount, cost, and quantity arithmetic, profit aggregation, year extraction, and ordering; it adds no outer-join or negative semantics and is not a compliant TPC query or result.",
    ),
    retail_case!(
        retail_h21_positive,
        "retail_h21_positive",
        "A positive same-order witness with two Lineitem occurrences reached through different bound columns, forcing separate supplier-key and order-key indexes on one input.",
        "Inspired only by the positive witness joins inside TPC-H Query 21. It changes the projection and deliberately omits the order-status, receipt/commit-date, and nation filters, the different-supplier inequality, the NOT EXISTS negative witness, count aggregation, ordering, and limit; it uses no outer-join semantics and is not a compliant TPC query or result.",
    ),
    retail_case!(
        retail_d19,
        "retail_d19",
        "A StoreSales snowflake with direct date, item, and store dimensions plus a two-hop customer-to-address branch.",
        "Inspired only by the join topology of TPC-DS Query 19. It changes the projection and omits the manager, month, year, and ZIP-inequality filters, brand and manufacturer attributes, sales-price aggregation, ordering, and limit; it adds no outer-join or negative semantics and is not a compliant TPC query or result.",
    ),
    retail_case!(
        retail_d27,
        "retail_d27",
        "A conventional fact-to-four-dimensions star that contrasts with the variable-graph claw and the deeper D19 snowflake.",
        "Inspired only by the StoreSales star inside TPC-DS Query 27. It changes the projection and omits demographic, year, and state filters, quantity and price measure averages, grouped/union result construction, ordering, and limit; it adds no outer-join or negative semantics and is not a compliant TPC query or result.",
    ),
    retail_case!(
        retail_d72_inner,
        "retail_d72_inner",
        "A CatalogSales-to-Inventory fact join correlated through item and DateDim week, with three DateDim roles that require partial lookup, full membership, and index reuse.",
        "Inspired only by the inner-join spine of TPC-DS Query 72. It deliberately omits the Promotion and CatalogReturns left outer joins, all date, demographic, inventory, and shipping filters, CASE/count aggregates, grouping, ordering, and limit; it adds no negative semantics and is not a compliant TPC query or result.",
    ),
    retail_case!(
        retail_d85,
        "retail_d85",
        "A composite order/item sale-return join with two role-playing CustomerDemo memberships that should share one full-key index.",
        "Inspired only by the join topology of TPC-DS Query 85. It changes the projection and omits the date, demographic, price, profit, country, and address filters, reason-text projection, average aggregations, grouping, ordering, and limit; it adds no outer-join or negative semantics and is not a compliant TPC query or result.",
    ),
];
