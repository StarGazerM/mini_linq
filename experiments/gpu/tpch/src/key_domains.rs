//! Static membership only: all minima, ranges and widths are load-time facts.

pub const COUNT: usize = 9;

pub fn id(table: &str, column: &str) -> Option<u32> {
    Some(match (table, column) {
        ("orders", "o_orderkey") | ("lineitem", "l_orderkey") => 0,
        ("part", "p_partkey") | ("partsupp", "ps_partkey") | ("lineitem", "l_partkey") => 1,
        ("customer", "c_custkey") | ("orders", "o_custkey") => 2,
        ("supplier", "s_suppkey") | ("partsupp", "ps_suppkey") | ("lineitem", "l_suppkey") => 3,
        ("nation", "n_nationkey") | ("supplier", "s_nationkey") | ("customer", "c_nationkey") => 4,
        ("region", "r_regionkey") | ("nation", "n_regionkey") => 5,
        ("lineitem", "l_linenumber") => 6,
        ("partsupp", "ps_availqty") => 7,
        ("part", "p_size") => 8,
        _ => return None,
    })
}

/// Authored opaque host producers preserve these key identities. Their Rust
/// remains logical; the generated device upload validates and normalizes it.
pub fn host_origin(input: &str, column: &str) -> Option<(&'static str, &'static str)> {
    match (input, column) {
        ("LateLineitem", "order_key") => Some(("lineitem", "l_orderkey")),
        ("QualifiedOrders", "o_orderkey") => Some(("orders", "o_orderkey")),
        ("QualifiedOrders", "o_custkey") => Some(("orders", "o_custkey")),
        _ => None,
    }
}
