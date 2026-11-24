//! Deterministic synthetic inputs for the actual authored query clauses.
//! These are not dbgen data and do not establish TPC-H performance. String
//! columns are interned exactly as the workload loaders do: one dictionary per
//! declared column, and H09's `GreenNames` selected once per distinct name.
use crate::{h03, h09, tpch_runtime::date};
use cq_load::{Date, Dictionaries, StringCode};
use std::collections::BTreeSet;

fn intern(dictionaries: &mut Dictionaries, column: &str, values: &[&str]) -> Vec<StringCode> {
    dictionaries.insert(column, values.iter().copied());
    let dictionary = dictionaries.get(column).expect("registered column");
    values
        .iter()
        .map(|value| StringCode(dictionary.lookup(value).expect("registered value")))
        .collect()
}

/// H03 input whose Customer market segments are interned in row order.
pub fn h03_data(
    customer: &[(i64, &str)],
    orders: Vec<(i64, i64, Date)>,
    lineitem: Vec<(i64, Date)>,
) -> h03::Data {
    let mut dictionaries = Dictionaries::default();
    let segments: Vec<_> = customer.iter().map(|(_, segment)| *segment).collect();
    let codes = intern(&mut dictionaries, "c_mktsegment", &segments);
    h03::Data {
        customer: customer.iter().map(|(key, _)| *key).zip(codes).collect(),
        orders,
        lineitem,
        dictionaries,
    }
}

/// The relations of H09 other than Part, which [`h09_data`] interns.
#[derive(Clone)]
pub struct H09Rows {
    pub lineitem: Vec<h09::Lineitem>,
    pub partsupp: Vec<h09::PartSupp>,
    pub supplier: Vec<h09::Supplier>,
    pub orders: Vec<h09::Orders>,
    pub nation: Vec<h09::Nation>,
}

/// H09 input with interned Part names and the loader's `GreenNames` relation.
pub fn h09_data(part: &[(i64, &str)], rows: H09Rows) -> h09::Data {
    let mut dictionaries = Dictionaries::default();
    let names: Vec<_> = part.iter().map(|(_, name)| *name).collect();
    let codes = intern(&mut dictionaries, "p_name", &names);
    let green_names =
        cq_load::dictionary_where(&dictionaries, "p_name", |value| value.contains("green"));
    h09::Data {
        lineitem: rows.lineitem,
        partsupp: rows.partsupp,
        part: part.iter().map(|(key, _)| *key).zip(codes).collect(),
        supplier: rows.supplier,
        orders: rows.orders,
        nation: rows.nation,
        green_names,
        dictionaries,
    }
}

pub const H03_BOUNDARY_CUSTOMERS: &[(i64, &str)] = &[
    (1, "BUILDING"),
    (1, "BUILDING"),
    (2, "AUTOMOBILE"),
    (3, "BUILDING"),
    (i64::MAX, "BUILDING"),
    (-7, "BUILDING"),
];

fn h03_boundary_rows() -> (Vec<h03::Orders>, Vec<h03::Lineitem>) {
    let early = date("1995-03-14");
    let cutoff = date("1995-03-15");
    let late = date("1995-03-16");
    (
        vec![
            (10, 1, early),
            (10, 3, early),
            (10, 1, early),
            (11, 2, early),
            (12, 1, cutoff),
            (13, 3, late),
            (14, 999, early),
            (u32::MAX as i64, i64::MAX, early),
            (15, -7, early),
        ],
        vec![
            (10, late),
            (10, late),
            (10, cutoff),
            (11, late),
            (12, late),
            (13, late),
            (14, late),
            (u32::MAX as i64, late),
            (15, late),
            (16, late),
        ],
    )
}

pub fn h03_boundaries() -> h03::Data {
    let (orders, lineitem) = h03_boundary_rows();
    h03_data(H03_BOUNDARY_CUSTOMERS, orders, lineitem)
}

/// The H03 boundary rows with no BUILDING customer.
pub fn h03_no_building() -> h03::Data {
    let customers: Vec<_> = H03_BOUNDARY_CUSTOMERS
        .iter()
        .map(|&(key, _)| (key, "AUTOMOBILE"))
        .collect();
    let (orders, lineitem) = h03_boundary_rows();
    h03_data(&customers, orders, lineitem)
}

/// The given H09 relations with no green Part name.
pub fn h09_no_green(rows: H09Rows) -> h09::Data {
    let red: Vec<_> = H09_BOUNDARY_PARTS
        .iter()
        .map(|&(key, _)| (key, "red"))
        .collect();
    h09_data(&red, rows)
}

pub const H09_BOUNDARY_PARTS: &[(i64, &str)] = &[
    (1, "forest green"),
    (1, "green duplicate"),
    (2, "red"),
    (3, "greener"),
    (u32::MAX as i64, "green"),
    (4, "GREEN"),
];

pub fn h09_boundary_rows() -> H09Rows {
    H09Rows {
        lineitem: vec![
            (100, 1, 7),
            (100, 1, 7),
            (101, 1, 8),
            (102, 1, 9),
            (-1, 3, i64::MAX),
            (200, 2, 7),
            (300, u32::MAX as i64, -9),
            (999, 1, 7),
            (103, 1, 10),
        ],
        partsupp: vec![
            (1, 7),
            (1, 7),
            (1, 8),
            (3, i64::MAX),
            (u32::MAX as i64, -9),
            (1, 10),
        ],
        orders: vec![(100,), (100,), (101,), (-1,), (300,), (103,)],
        supplier: vec![
            (7, 1),
            (7, 1),
            (8, 2),
            (i64::MAX, i32::MIN),
            (-9, 3),
            (10, 999),
        ],
        nation: vec![(1,), (1,), (2,), (i32::MIN,), (3,)],
    }
}

pub fn h09_boundaries() -> h09::Data {
    h09_data(H09_BOUNDARY_PARTS, h09_boundary_rows())
}

pub fn h03_scaled(n: usize) -> h03::Data {
    let customers = (n / 32).max(1);
    let orders = (n / 4).max(1);
    let early = date("1995-03-14");
    let late = date("1995-03-16");
    let customer: Vec<_> = (0..customers)
        .map(|i| {
            (
                i as i64 + 1,
                if i % 3 == 0 { "BUILDING" } else { "AUTOMOBILE" },
            )
        })
        .collect();
    h03_data(
        &customer,
        (0..orders)
            .map(|i| {
                (
                    i as i64 + 1,
                    (i % customers) as i64 + 1,
                    if i % 5 == 0 { late } else { early },
                )
            })
            .collect(),
        (0..n)
            .map(|i| {
                (
                    ((i * 17) % (orders + 7)) as i64 + 1,
                    if i % 7 == 0 { early } else { late },
                )
            })
            .collect(),
    )
}

pub fn h09_scaled(n: usize) -> h09::Data {
    let parts = (n / 32).max(1);
    let orders = (n / 4).max(1);
    let lineitem: Vec<_> = (0..n)
        .map(|i| {
            (
                (i % (orders + 7)) as i64 + 1,
                ((i * 13) % parts) as i64 + 1,
                ((i * 17) % 257) as i64 + 1,
            )
        })
        .collect();
    let partsupp = lineitem
        .iter()
        .enumerate()
        .filter(|(i, _)| i % 11 != 0)
        .map(|(_, &(_, part, supplier))| (part, supplier))
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    let part: Vec<_> = (0..parts)
        .map(|i| {
            (
                i as i64 + 1,
                if i % 3 == 0 { "forest green" } else { "red" },
            )
        })
        .collect();
    h09_data(
        &part,
        H09Rows {
            lineitem,
            partsupp,
            orders: (0..orders)
                .filter(|i| i % 7 != 0)
                .map(|i| (i as i64 + 1,))
                .collect(),
            supplier: (0..257).map(|i| (i as i64 + 1, i % 25)).collect(),
            nation: (0..24).map(|i| (i,)).collect(),
        },
    )
}
