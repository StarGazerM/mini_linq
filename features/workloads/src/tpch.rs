//! Source inventory for standalone TPC-H-derived benchmark executables.
//!
//! These kernels load real dbgen projections and compile independently in the
//! release profile. They do not replace the synthetic teaching query catalog.

#[derive(Clone, Copy, Debug)]
pub struct Case {
    pub name: &'static str,
    /// Repository-relative source path consumed by the isolated benchmark crate.
    pub rust_path: &'static str,
    /// Separate authored SQL peer; retail kernels embed their SQL in Rust.
    pub sql_path: Option<&'static str>,
    /// Canonical full-query number. Inspired positive retail cores have no ID.
    pub tpch_query: Option<u8>,
}

pub const COVERAGE: &str = include_str!("../tpch/coverage.tsv");
pub const NUMERIC_CONTRACT: &str = include_str!("../tpch/numeric_contract.txt");

pub const CASES: &[Case] = &[
    Case {
        name: "retail_h01",
        rust_path: "workloads/src/queries/retail/tpch/retail_h01.rs",
        sql_path: None,
        tpch_query: None,
    },
    Case {
        name: "retail_h02",
        rust_path: "workloads/src/queries/retail/tpch/retail_h02.rs",
        sql_path: None,
        tpch_query: None,
    },
    Case {
        name: "retail_h11",
        rust_path: "workloads/src/queries/retail/tpch/retail_h11.rs",
        sql_path: None,
        tpch_query: None,
    },
    Case {
        name: "retail_h15",
        rust_path: "workloads/src/queries/retail/tpch/retail_h15.rs",
        sql_path: None,
        tpch_query: None,
    },
    Case {
        name: "retail_h20",
        rust_path: "workloads/src/queries/retail/tpch/retail_h20.rs",
        sql_path: None,
        tpch_query: None,
    },
    Case {
        name: "retail_h06",
        rust_path: "workloads/src/queries/retail/tpch/retail_h06.rs",
        sql_path: None,
        tpch_query: None,
    },
    Case {
        name: "retail_h10",
        rust_path: "workloads/src/queries/retail/tpch/retail_h10.rs",
        sql_path: None,
        tpch_query: None,
    },
    Case {
        name: "retail_h12",
        rust_path: "workloads/src/queries/retail/tpch/retail_h12.rs",
        sql_path: None,
        tpch_query: None,
    },
    Case {
        name: "retail_h13",
        rust_path: "workloads/src/queries/retail/tpch/retail_h13.rs",
        sql_path: None,
        tpch_query: None,
    },
    Case {
        name: "retail_h14",
        rust_path: "workloads/src/queries/retail/tpch/retail_h14.rs",
        sql_path: None,
        tpch_query: None,
    },
    Case {
        name: "retail_h18",
        rust_path: "workloads/src/queries/retail/tpch/retail_h18.rs",
        sql_path: None,
        tpch_query: None,
    },
    Case {
        name: "retail_h19",
        rust_path: "workloads/src/queries/retail/tpch/retail_h19.rs",
        sql_path: None,
        tpch_query: None,
    },
    Case {
        name: "retail_h04",
        rust_path: "workloads/src/queries/retail/tpch/retail_h04.rs",
        sql_path: None,
        tpch_query: None,
    },
    Case {
        name: "retail_h04_not_exists",
        rust_path: "workloads/src/queries/retail/tpch/retail_h04_not_exists.rs",
        sql_path: None,
        tpch_query: None,
    },
    Case {
        name: "retail_h16",
        rust_path: "workloads/src/queries/retail/tpch/retail_h16.rs",
        sql_path: None,
        tpch_query: None,
    },
    Case {
        name: "retail_h22",
        rust_path: "workloads/src/queries/retail/tpch/retail_h22.rs",
        sql_path: None,
        tpch_query: None,
    },
    Case {
        name: "retail_h03",
        rust_path: "workloads/src/queries/retail/tpch/retail_h03.rs",
        sql_path: None,
        tpch_query: None,
    },
    Case {
        name: "retail_h05",
        rust_path: "workloads/src/queries/retail/tpch/retail_h05.rs",
        sql_path: None,
        tpch_query: None,
    },
    Case {
        name: "retail_h07",
        rust_path: "workloads/src/queries/retail/tpch/retail_h07.rs",
        sql_path: None,
        tpch_query: None,
    },
    Case {
        name: "retail_h07_block",
        rust_path: "workloads/src/queries/retail/tpch/retail_h07_block.rs",
        sql_path: None,
        tpch_query: None,
    },
    Case {
        name: "retail_h08",
        rust_path: "workloads/src/queries/retail/tpch/retail_h08.rs",
        sql_path: None,
        tpch_query: None,
    },
    Case {
        name: "retail_h09",
        rust_path: "workloads/src/queries/retail/tpch/retail_h09.rs",
        sql_path: None,
        tpch_query: None,
    },
    Case {
        name: "retail_h21_positive",
        rust_path: "workloads/src/queries/retail/tpch/retail_h21_positive.rs",
        sql_path: None,
        tpch_query: None,
    },
    Case {
        name: "retail_d19",
        rust_path: "workloads/src/queries/retail/tpch/retail_d19.rs",
        sql_path: None,
        tpch_query: None,
    },
    Case {
        name: "retail_d27",
        rust_path: "workloads/src/queries/retail/tpch/retail_d27.rs",
        sql_path: None,
        tpch_query: None,
    },
    Case {
        name: "retail_d72_inner",
        rust_path: "workloads/src/queries/retail/tpch/retail_d72_inner.rs",
        sql_path: None,
        tpch_query: None,
    },
    Case {
        name: "retail_d85",
        rust_path: "workloads/src/queries/retail/tpch/retail_d85.rs",
        sql_path: None,
        tpch_query: None,
    },
    Case {
        name: "tpch_q06",
        rust_path: "workloads/src/queries/tpch/tpch_q06.rs",
        sql_path: Some("workloads/sql/tpch/q06.sql"),
        tpch_query: Some(6),
    },
    Case {
        name: "tpch_q19",
        rust_path: "workloads/src/queries/tpch/tpch_q19.rs",
        sql_path: Some("workloads/sql/tpch/q19.sql"),
        tpch_query: Some(19),
    },
];
