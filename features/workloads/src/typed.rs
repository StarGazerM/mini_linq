//! Typed CSV and Parquet fixtures exercised by the ordinary golden gate.
//!
//! Each case owns one CQ leaf and an independently authored SQL query. The
//! caller expands that leaf's callback into both its loaders and query. The
//! oracle also derives its input column names and SQL types from that leaf;
//! there is no parallel Rust loading schema. Fixtures include duplicate rows,
//! integers outside the i32 range, pre-epoch and leap dates, negative fixed
//! point values, and strings requiring CSV quoting.

use std::path::{Path, PathBuf};

/// One typed MiniLinq / SQL comparison.
pub struct Case {
    pub name: &'static str,
    pub purpose: &'static str,
    pub rust_path: &'static str,
    pub sql: &'static str,
    module: fn() -> syn::Result<cq_lang_cq::Program>,
}

impl Case {
    pub fn module(&self) -> syn::Result<cq_lang_cq::Program> {
        (self.module)()
    }

    pub fn path(&self, kind: &str) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join(kind)
            .join("typed")
            .join(self.name)
    }
}

macro_rules! cases {
    ($($name:ident => $purpose:literal),* $(,)?) => {
        pub static CASES: &[Case] = &[$(Case {
            name: stringify!($name),
            purpose: $purpose,
            rust_path: crate::queries::typed::$name::RUST_PATH,
            sql: include_str!(concat!("../sql/typed/", stringify!($name), ".sql")),
            module: crate::queries::typed::$name::module,
        }),*];
    };
}

cases! {
    typed_line_filter => "fixed-point and date range filters with owned text projection",
    typed_customer_orders => "i64 customer-key join with dates and fixed-point totals",
    typed_calendar_shipping => "date-key equality and duplicate input elimination",
    typed_priced_products => "fixed-point key equality including negative prices",
    typed_tagged_products => "owned string-key join and three-relation projection",
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
