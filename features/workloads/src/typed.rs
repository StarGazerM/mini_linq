//! Typed CSV and Parquet fixtures exercised by the ordinary golden gate.
//!
//! Each case owns one literal CQ query and an independently authored SQL query.
//! Ordinary Rust schemas describe loader columns and the oracle's SQL tables.
//! Fixtures include duplicate rows,
//! integers outside the i32 range, pre-epoch and leap dates, negative fixed
//! point values, and strings requiring CSV quoting.

use std::path::{Path, PathBuf};

use crate::schema::{ColumnType, Relation};
use cq_load::{Date, Decimal};

/// A named fixture relation, independent of query syntax.
pub trait Schema: cq_load::Relation {
    const SCHEMA: Relation;
}

macro_rules! relation {
    ($schema:ident => $name:literal { $($index:literal: $column:ident: $ty:ty => $kind:ident),+ $(,)? }) => {
        pub struct $schema;

        impl cq_load::Relation for $schema {
            type Row = ($($ty,)+);
            const COLUMNS: &'static [&'static str] = &[$(stringify!($column)),+];

            fn decode(batch: &cq_load::Batch<'_>) -> cq_load::Result<Vec<Self::Row>> {
                $(let mut $column = batch.column::<$ty>($index)?.into_iter();)+
                Ok((0..batch.len())
                    .map(|_| ($($column.next().expect("decoded column length"),)+))
                    .collect())
            }
        }

        impl Schema for $schema {
            const SCHEMA: Relation = Relation {
                name: $name,
                columns: &[$((stringify!($column), ColumnType::$kind)),+],
            };
        }
    };
}

relation!(Lineitem => "Lineitem" {
    0: orderkey: i64 => I64, 1: linenumber: i32 => I32,
    2: price: Decimal => Decimal, 3: shipdate: Date => Date, 4: comment: String => Text,
});
relation!(Limits => "Limits" { 0: minimum: Decimal => Decimal, 1: earliest: Date => Date });
relation!(Customer => "Customer" { 0: customerkey: i64 => I64, 1: name: String => Text });
relation!(Orders => "Orders" {
    0: customerkey: i64 => I64, 1: orderkey: i64 => I64,
    2: orderdate: Date => Date, 3: total: Decimal => Decimal,
});
relation!(Calendar => "Calendar" { 0: shipdate: Date => Date, 1: label: String => Text });
relation!(Shipment => "Shipment" {
    0: shipdate: Date => Date, 1: orderkey: i64 => I64, 2: price: Decimal => Decimal,
});
relation!(Price => "Price" { 0: price: Decimal => Decimal, 1: label: String => Text });
relation!(PricedProduct => "Product" {
    0: price: Decimal => Decimal, 1: productkey: i64 => I64,
    2: name: String => Text, 3: released: Date => Date,
});
relation!(Tag => "Tag" { 0: tag: String => Text, 1: productkey: i64 => I64 });
relation!(Selected => "Selected" { 0: tag: String => Text });
relation!(TaggedProduct => "Product" {
    0: productkey: i64 => I64, 1: name: String => Text,
    2: price: Decimal => Decimal, 3: released: Date => Date,
});

#[derive(Clone, Copy)]
pub enum Source {
    Csv,
    #[cfg(feature = "parquet")]
    Parquet,
}

/// Load a fixture with strict headers and small batches in either source format.
pub fn load<S: Schema>(case: &str, source: Source) -> Vec<S::Row> {
    let directory = self::case(case).expect("known typed case").path("data");
    let result = match source {
        Source::Csv => cq_load::Loader::<S>::csv(
            directory.join(format!("{}.csv", S::SCHEMA.name)),
            cq_load::CsvOptions {
                headers: true,
                batch_size: 2,
                ..Default::default()
            },
        ),
        #[cfg(feature = "parquet")]
        Source::Parquet => cq_load::Loader::<S>::parquet(
            directory.join(format!("{}.parquet", S::SCHEMA.name)),
            cq_load::ParquetOptions { batch_size: 2 },
        ),
    };
    result.unwrap_or_else(|error| panic!("typed case `{case}`: {error}"))
}

/// One typed MiniLinq / SQL comparison.
pub struct Case {
    pub name: &'static str,
    pub purpose: &'static str,
    pub rust_path: &'static str,
    pub sql: &'static str,
    pub schema: &'static [Relation],
}

impl Case {
    pub fn inputs(&self) -> Vec<cq_syntax::RelationDecl> {
        crate::schema::declarations(self.schema)
    }

    pub fn path(&self, kind: &str) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join(kind)
            .join("typed")
            .join(self.name)
    }
}

macro_rules! cases {
    ($($name:ident [$($schema:ident),+] => $purpose:literal),* $(,)?) => {
        pub static CASES: &[Case] = &[$(Case {
            name: stringify!($name),
            purpose: $purpose,
            rust_path: concat!(env!("CARGO_MANIFEST_DIR"), "/src/queries/typed/", stringify!($name), ".rs"),
            sql: include_str!(concat!("../sql/typed/", stringify!($name), ".sql")),
            schema: &[$($schema::SCHEMA),+],
        }),*];
    };
}

cases! {
    typed_line_filter [Lineitem, Limits] => "fixed-point and date range filters with owned text projection",
    typed_customer_orders [Customer, Orders] => "i64 customer-key join with dates and fixed-point totals",
    typed_calendar_shipping [Calendar, Shipment] => "date-key equality and duplicate input elimination",
    typed_priced_products [Price, PricedProduct] => "fixed-point key equality including negative prices",
    typed_tagged_products [Tag, Selected, TaggedProduct] => "owned string-key join and three-relation projection",
}

pub fn case(name: &str) -> Option<&'static Case> {
    CASES.iter().find(|case| case.name == name)
}

/// Canonical CSV for typed answers, including quoted and multiline strings.
/// Sorting fields as strings makes comparison independent of output iteration
/// order; the SQL oracle applies the same serialization to its typed values.
pub fn canonical_bytes(mut rows: Vec<Vec<String>>) -> Vec<u8> {
    rows.sort();
    let mut result = String::new();
    for row in rows {
        for (index, value) in row.into_iter().enumerate() {
            if index != 0 {
                result.push(',');
            }
            if value.contains([',', '"', '\r', '\n']) {
                result.push('"');
                result.push_str(&value.replace('"', "\"\""));
                result.push('"');
            } else {
                result.push_str(&value);
            }
        }
        result.push('\n');
    }
    result.into_bytes()
}

pub fn check(name: &str, rows: Vec<Vec<String>>) {
    assert_eq!(
        rows.iter().collect::<std::collections::BTreeSet<_>>().len(),
        rows.len(),
        "typed query `{name}` returned duplicate answers",
    );
    let path = case(name)
        .expect("known typed case")
        .path("golden")
        .with_extension("csv");
    let expected =
        std::fs::read(&path).unwrap_or_else(|error| panic!("{}: {error}", path.display()));
    assert_eq!(canonical_bytes(rows), expected, "{}", path.display());
}
