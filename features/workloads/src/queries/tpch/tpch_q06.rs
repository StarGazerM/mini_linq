//! TPC-H-derived Q6 with exact scale-four revenue and SQL bag multiplicity.
//! The lineitem primary key survives the CQ head before the aggregate fold.

use cq_load::{Date, Decimal};
use std::{error::Error, path::Path};

pub const NAME: &str = "tpch_q06";
pub const SQL: &str = include_str!("../../../sql/tpch/q06.sql");
pub const EXPORTS: &[(&str, &str)] = &[(
    "lineitem",
    "l_orderkey,l_linenumber,l_quantity,l_extendedprice,l_discount,l_shipdate",
)];
pub const CONFIGS: &[&str] = &[
    "default",
    "radix",
    "radix_retained",
    "built_hash",
    "built_radix",
    "par",
    "row-par",
    "no-reduce",
    "generic-smallest",
    "staged",
    "transfer",
    "le",
    "le_runs",
    "blocks-plain",
    "blocks-bytes",
    "blocks-plain-par",
    "blocks-bytes-par",
];

::mini_linq::relation_loaders! {
    relation Lineitem(
        l_orderkey: i64, l_linenumber: i64, l_quantity: Decimal,
        l_extendedprice: Decimal, l_discount: Decimal, l_shipdate: Date
    );
}

::mini_linq::columnar_relation_loaders! {
    relation PlainLineitem(
        l_orderkey: i64 @ plain, l_linenumber: i64 @ plain,
        l_quantity: Decimal @ plain, l_extendedprice: Decimal @ plain,
        l_discount: Decimal @ plain, l_shipdate: Date @ plain
    );
    relation BytesLineitem(
        l_orderkey: i64 @ plain, l_linenumber: i64 @ plain,
        l_quantity: Decimal @ bytes, l_extendedprice: Decimal @ plain,
        l_discount: Decimal @ bytes, l_shipdate: Date @ bytes
    );
}

type LineitemRow = (i64, i64, Decimal, Decimal, Decimal, Date);
type PlainTable = cq_storage::scan::ScanTable<(
    cq_storage::scan::ScanColumn<i64, cq_storage::scan::Plain>,
    cq_storage::scan::ScanColumn<i64, cq_storage::scan::Plain>,
    cq_storage::scan::ScanColumn<Decimal, cq_storage::scan::Plain>,
    cq_storage::scan::ScanColumn<Decimal, cq_storage::scan::Plain>,
    cq_storage::scan::ScanColumn<Decimal, cq_storage::scan::Plain>,
    cq_storage::scan::ScanColumn<Date, cq_storage::scan::Plain>,
)>;
type BytesTable = cq_storage::scan::ScanTable<(
    cq_storage::scan::ScanColumn<i64, cq_storage::scan::Plain>,
    cq_storage::scan::ScanColumn<i64, cq_storage::scan::Plain>,
    cq_storage::scan::ScanColumn<Decimal, cq_storage::scan::Bytes>,
    cq_storage::scan::ScanColumn<Decimal, cq_storage::scan::Plain>,
    cq_storage::scan::ScanColumn<Decimal, cq_storage::scan::Bytes>,
    cq_storage::scan::ScanColumn<Date, cq_storage::scan::Bytes>,
)>;

enum LineitemInput {
    Rows(Vec<LineitemRow>),
    Plain(PlainTable),
    Bytes(BytesTable),
}

impl LineitemInput {
    fn as_slice(&self) -> &[LineitemRow] {
        let Self::Rows(rows) = self else {
            unreachable!("row configuration requires its row input")
        };
        rows.as_slice()
    }

    fn as_plain(&self) -> &PlainTable {
        let Self::Plain(table) = self else {
            unreachable!("plain blocked configuration requires its columnar input")
        };
        table
    }

    fn as_bytes(&self) -> &BytesTable {
        let Self::Bytes(table) = self else {
            unreachable!("bytes blocked configuration requires its columnar input")
        };
        table
    }
}

pub struct Data {
    lineitem: LineitemInput,
}

/// Coefficient at scale four; None is the SQL SUM result for empty input.
pub type Answer = Option<i128>;

fn scan_domains() -> [cq_storage::scan::format::Domain; 6] {
    use cq_storage::scan::{Width, format::{Domain, DomainId}};
    // The query declares logical ids 0/1/2 explicitly. Numeric domains use
    // namespace 2*id+1; no dictionary or Rust type recognition is involved.
    let domain = |namespace, carrier| Domain {
        id: DomainId { namespace, version: 0 },
        carrier,
        equality: true,
        order: true,
    };
    [
        domain(1, Width::W64), domain(1, Width::W64),
        domain(3, Width::W64), domain(3, Width::W64),
        domain(3, Width::W64), domain(5, Width::W32),
    ]
}

pub fn load(root: &Path, config: &str) -> Result<Data, Box<dyn Error>> {
    let path = root.join("lineitem.parquet");
    let blocks = cq_storage::scan::BlockRows::new(1024).unwrap();
    let lineitem = match config {
        "blocks-plain" | "blocks-plain-par" => LineitemInput::Plain(
            PlainLineitemLoader::parquet(path, Default::default(), blocks, false)?
                .bind_domains(&scan_domains())?,
        ),
        "par" | "blocks-bytes" | "blocks-bytes-par" => LineitemInput::Bytes(
            BytesLineitemLoader::parquet(path, Default::default(), blocks, false)?
                .bind_domains(&scan_domains())?,
        ),
        _ => LineitemInput::Rows(LineitemLoader::parquet(path, Default::default())?),
    };
    Ok(Data { lineitem })
}

macro_rules! query_body {
    () => {
        query_body!(as_slice; [positional(lineitem);]);
    };
    ($input:ident; [$($prefix:tt)*]) => {
        pub(super) fn execute(data: &Data) -> Vec<Answer> {
            let lineitem = data.lineitem.$input();
            let first: Date = "1994-01-01".parse().unwrap();
            let after: Date = "1995-01-01".parse().unwrap();
            let discount_low = Decimal::from_scaled(5).unwrap();
            let discount_high = Decimal::from_scaled(7).unwrap();
            let quantity_limit = Decimal::from_scaled(2400).unwrap();
            let aggregates = query! {
                $($prefix)* aggregate! {
                by (group); revenue = sum(amount);
                struct Query06;
                relation lineitem(
                    orderkey: i64 [0], linenumber: i64 [0], quantity: Decimal [1],
                    price: Decimal [1], discount: Decimal [1], shipdate: Date [2]
                );
                rows(group, orderkey, linenumber, amount) :-
                    lineitem(orderkey, linenumber, quantity, price, discount, shipdate),
                    where shipdate >= first && shipdate < after,
                    where discount >= discount_low && discount <= discount_high,
                    where quantity < quantity_limit,
                    let group: i32 = 0,
                    let amount: i128 = i128::from(price.scaled()) * i128::from(discount.scaled());
            }};
            vec![
                aggregates
                    .iter()
                    .flat_map(|(_, values)| values.iter())
                    .map(|(amount,)| *amount)
                    .next(),
            ]
        }
    };
}

// Each configuration expands the same authored query through caller-selected
// planner and checked Physical/Structures rewrites.
crate::tpch_runtime::configuration!(default, $, {}, {
    query_body!();
});
crate::tpch_runtime::configuration!(generic_smallest, $,
    { plan = ::cq_planners::order(written, fine); physical = [::cq_rewrites::canon_prefixes, ::cq_rewrites::smallest_intersections]; }, {
    query_body!();
});
crate::tpch_runtime::configuration!(transfer, $, { plan = ::cq_planners::order(transfer); }, {
    query_body!();
});
crate::tpch_runtime::configuration!(le, $, { plan = ::cq_planners::order(le); }, {
    query_body!();
});
crate::tpch_runtime::configuration!(le_runs, $, {
    plan = ::cq_planners::order(le);
}, [::cq_rewrites::sorted_runs, ::cq_rewrites::retain_lookups], {
    query_body!();
});

// Recorded representation and evaluation-unit decisions: the scan chain is
// carried as positions and its total range predicates run 1,024 rows ahead.
crate::tpch_runtime::configuration!(staged, $,
    { represent = ::mini_linq::passes::represent(positions); stage = ::mini_linq::passes::stage(batch(1024)); }, {
    query_body!();
});

// Authored scan representations; serial default and row-par retain their row host.
crate::tpch_runtime::configuration!(blocks_plain, $, {
    select = ::mini_linq::passes::select(blocks(lineitem {
        block_rows 1024; summary none;
        columns [orderkey: plain u64, linenumber: plain u64, quantity: plain u64,
            price: plain u64, discount: plain u64, shipdate: plain u32];
    }));
}, { query_body!(as_plain; []); });
crate::tpch_runtime::configuration!(blocks_bytes, $, {
    select = ::mini_linq::passes::select(blocks(lineitem {
        block_rows 1024; summary none;
        columns [orderkey: plain u64, linenumber: plain u64, quantity: bytes u64,
            price: plain u64, discount: bytes u64, shipdate: bytes u32];
    }));
}, { query_body!(as_bytes; []); });
crate::tpch_runtime::configuration!(blocks_plain_par, $, {
    select = ::mini_linq::passes::select(blocks(lineitem {
        block_rows 1024; summary none;
        columns [orderkey: plain u64, linenumber: plain u64, quantity: plain u64,
            price: plain u64, discount: plain u64, shipdate: plain u32];
    }));
    decompose = [::cq_sequence_tasks_macros::plan];
    grade = ::cq_sequence_tasks_macros::tasks;
    print = ::cq_sequence_tasks_macros::translate_tasks;
}, { query_body!(as_plain; []); });
crate::tpch_runtime::configuration!(blocks_bytes_par, $, {
    select = ::mini_linq::passes::select(blocks(lineitem {
        block_rows 1024; summary none;
        columns [orderkey: plain u64, linenumber: plain u64, quantity: bytes u64,
            price: plain u64, discount: bytes u64, shipdate: bytes u32];
    }));
    decompose = [::cq_sequence_tasks_macros::plan];
    grade = ::cq_sequence_tasks_macros::tasks;
    print = ::cq_sequence_tasks_macros::translate_tasks;
}, { query_body!(as_bytes; []); });

pub fn run(data: &Data, config: &str) -> Vec<Answer> {
    match config {
        "default" => default::execute(data),
        "radix" => radix::execute(data),
        "radix_retained" => radix_retained::execute(data),
        "built_hash" => built_hash::execute(data),
        "built_radix" => built_radix::execute(data),
        "par" => blocks_bytes_par::execute(data),
        "row-par" => par::execute(data),
        "staged" => staged::execute(data),
        "no-reduce" => no_reduce::execute(data),
        "generic-smallest" => generic_smallest::execute(data),
        "transfer" => transfer::execute(data),
        "le" => le::execute(data),
        "le_runs" => le_runs::execute(data),
        "blocks-plain" => blocks_plain::execute(data),
        "blocks-bytes" => blocks_bytes::execute(data),
        "blocks-plain-par" => blocks_plain_par::execute(data),
        "blocks-bytes-par" => blocks_bytes_par::execute(data),
        _ => panic!("unknown Q6 configuration: {config}"),
    }
}

pub fn canonical(rows: &[Answer]) -> Vec<Vec<String>> {
    rows.iter()
        .map(|value| vec![value.map_or_else(|| "\\N".into(), fixed_four)])
        .collect()
}

fn fixed_four(value: i128) -> String {
    let sign = if value < 0 { "-" } else { "" };
    let magnitude = value.unsigned_abs();
    format!("{sign}{}.{:04}", magnitude / 10_000, magnitude % 10_000)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input(rows: &[LineitemRow], config: &str) -> Data {
        fn column<T: cq_storage::scan::OrdinalDecode, F: cq_storage::scan::Family>(
            rows: &[LineitemRow],
            value: impl Fn(&LineitemRow) -> T,
        ) -> cq_storage::scan::ScanColumn<T, F> {
            let mut builder = cq_storage::scan::ScanColumnBuilder::new(
                cq_storage::scan::BlockRows::new(1024).unwrap(),
                false,
            );
            builder.extend(rows.iter().map(value));
            builder.finish()
        }
        let lineitem = match config {
            "blocks-plain" | "blocks-plain-par" => LineitemInput::Plain(
                cq_storage::scan::ScanTable::new((
                    column(rows, |r| r.0), column(rows, |r| r.1),
                    column(rows, |r| r.2), column(rows, |r| r.3),
                    column(rows, |r| r.4), column(rows, |r| r.5),
                )).bind_domains(&scan_domains()).unwrap(),
            ),
            "par" | "blocks-bytes" | "blocks-bytes-par" => LineitemInput::Bytes(
                cq_storage::scan::ScanTable::new((
                    column(rows, |r| r.0), column(rows, |r| r.1),
                    column(rows, |r| r.2), column(rows, |r| r.3),
                    column(rows, |r| r.4), column(rows, |r| r.5),
                )).bind_domains(&scan_domains()).unwrap(),
            ),
            _ => LineitemInput::Rows(rows.to_vec()),
        };
        Data { lineitem }
    }

    #[test]
    fn row_identity_and_filter_boundaries_preserve_sql_sum() {
        let decimal = |value| Decimal::from_scaled(value).unwrap();
        let row = |number, quantity, discount, date: &str| {
            (
                1,
                number,
                decimal(quantity),
                decimal(10_000),
                decimal(discount),
                date.parse().unwrap(),
            )
        };
        let rows = [
                row(1, 2300, 5, "1994-01-01"),
                row(2, 2300, 5, "1994-01-01"),
                row(3, 2300, 7, "1994-12-31"),
                row(4, 2400, 5, "1994-01-01"),
                row(5, 2300, 4, "1994-01-01"),
                row(6, 2300, 8, "1994-01-01"),
                row(7, 2300, 5, "1995-01-01"),
                row(8, 2300, 5, "1993-12-31"),
        ];
        for config in CONFIGS {
            let data = input(&rows, config);
            assert_eq!(run(&data, config), [Some(170_000)]);
            assert_eq!(canonical(&run(&data, config)), [vec!["17.0000"]]);
            assert_eq!(run(&input(&[], config), config), [None]);
            assert_eq!(canonical(&[None]), [vec!["\\N"]]);
        }
    }

    #[test]
    fn blocked_routes_cover_full_and_partial_vectors_without_losing_occurrences() {
        let mut rows: Vec<_> = (0..2057)
            .map(|line| (
                1,
                line,
                Decimal::from_scaled(2300).unwrap(),
                Decimal::from_scaled(10_000).unwrap(),
                Decimal::from_scaled(5).unwrap(),
                "1994-01-01".parse().unwrap(),
            ))
            .collect();
        rows.push(rows[0]);
        for config in ["default", "par", "row-par", "blocks-plain", "blocks-bytes",
            "blocks-plain-par", "blocks-bytes-par"] {
            assert_eq!(run(&input(&rows, config), config), [Some(2057 * 50_000)]);
        }
    }
}
