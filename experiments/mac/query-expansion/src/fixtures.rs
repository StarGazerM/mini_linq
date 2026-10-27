//! Deterministic synthetic inputs for the actual authored query clauses.
//! These are not dbgen data and do not establish TPC-H performance.
use crate::{h03, h09, tpch_runtime::date};
use std::collections::BTreeSet;

pub fn h03_boundaries() -> h03::Data {
    let early = date("1995-03-14");
    let cutoff = date("1995-03-15");
    let late = date("1995-03-16");
    h03::Data {
        customer: vec![
            (1, "BUILDING".into()),
            (1, "BUILDING".into()),
            (2, "AUTOMOBILE".into()),
            (3, "BUILDING".into()),
            (i64::MAX, "BUILDING".into()),
            (-7, "BUILDING".into()),
        ],
        orders: vec![
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
        lineitem: vec![
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
    }
}

pub fn h09_boundaries() -> h09::Data {
    h09::Data {
        part: vec![
            (1, "forest green".into()),
            (1, "green duplicate".into()),
            (2, "red".into()),
            (3, "greener".into()),
            (u32::MAX as i64, "green".into()),
            (4, "GREEN".into()),
        ],
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

pub fn h03_scaled(n: usize) -> h03::Data {
    let customers = (n / 32).max(1);
    let orders = (n / 4).max(1);
    let early = date("1995-03-14");
    let late = date("1995-03-16");
    h03::Data {
        customer: (0..customers)
            .map(|i| {
                (
                    i as i64 + 1,
                    if i % 3 == 0 { "BUILDING" } else { "AUTOMOBILE" }.into(),
                )
            })
            .collect(),
        orders: (0..orders)
            .map(|i| {
                (
                    i as i64 + 1,
                    (i % customers) as i64 + 1,
                    if i % 5 == 0 { late } else { early },
                )
            })
            .collect(),
        lineitem: (0..n)
            .map(|i| {
                (
                    ((i * 17) % (orders + 7)) as i64 + 1,
                    if i % 7 == 0 { early } else { late },
                )
            })
            .collect(),
    }
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
    h09::Data {
        lineitem,
        partsupp,
        part: (0..parts)
            .map(|i| {
                (
                    i as i64 + 1,
                    if i % 3 == 0 { "forest green" } else { "red" }.into(),
                )
            })
            .collect(),
        orders: (0..orders)
            .filter(|i| i % 7 != 0)
            .map(|i| (i as i64 + 1,))
            .collect(),
        supplier: (0..257).map(|i| (i as i64 + 1, i % 25)).collect(),
        nation: (0..24).map(|i| (i,)).collect(),
    }
}
