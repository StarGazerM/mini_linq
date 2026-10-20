//! Stored schema of the projected TPC-H Q5 export (`compare/export_tpch.py`):
//! five join domains, every join key a `u32` domain ID, Region.name (interned
//! text) and the order date at their declared `i32` type.

use crate::{Kind, Schema};

pub fn schema() -> Schema {
    let id = |domain: &str| Kind::Id(domain.to_string());
    let columns = |list: &[(&str, Kind)]| list.iter().map(|(c, k)| (c.to_string(), k.clone())).collect();
    let members = |list: &[&str]| list.iter().map(|m| m.to_string()).collect();
    Schema {
        domains: vec![
            ("orderkey".into(), members(&["orders.o_orderkey", "lineitem.l_orderkey"])),
            ("custkey".into(), members(&["customer.c_custkey", "orders.o_custkey"])),
            ("suppkey".into(), members(&["supplier.s_suppkey", "lineitem.l_suppkey"])),
            ("nationkey".into(), members(&["nation.n_nationkey", "customer.c_nationkey", "supplier.s_nationkey"])),
            ("regionkey".into(), members(&["region.r_regionkey", "nation.n_regionkey"])),
        ],
        tables: vec![
            ("region".into(), columns(&[("r_regionkey", id("regionkey")), ("r_name", Kind::I32)])),
            ("nation".into(), columns(&[("n_nationkey", id("nationkey")), ("n_regionkey", id("regionkey"))])),
            ("customer".into(), columns(&[("c_custkey", id("custkey")), ("c_nationkey", id("nationkey"))])),
            (
                "orders".into(),
                columns(&[("o_orderkey", id("orderkey")), ("o_custkey", id("custkey")), ("o_orderdate", Kind::I32)]),
            ),
            ("lineitem".into(), columns(&[("l_orderkey", id("orderkey")), ("l_suppkey", id("suppkey"))])),
            ("supplier".into(), columns(&[("s_suppkey", id("suppkey")), ("s_nationkey", id("nationkey"))])),
        ],
    }
}
