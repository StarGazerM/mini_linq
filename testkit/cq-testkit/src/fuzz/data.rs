//! G2 value domains and relation shapes.

use crate::values::{ColumnType, Value};
use std::collections::{BTreeMap, BTreeSet};

/// The values one column draws from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Domain {
    /// i32 in [-3, 3]: safe for every generated arithmetic expression.
    Small,
    /// Type extremes and neighbours of zero.
    Extreme,
    /// i64 values that agree in the low 32 bits but differ above them.
    Wide,
    /// i32 row ordinals `0..n`, for tables larger than one GPU tile.
    Sequential(usize),
}

impl Domain {
    /// Only `Small` variables may feed i32 arithmetic.
    pub(super) fn arithmetic(self) -> bool {
        self == Self::Small
    }

    pub(super) fn values(self, ty: ColumnType) -> Vec<Value> {
        match (self, ty) {
            (Self::Small, ColumnType::I32) => (-3..=3).map(Value::I32).collect(),
            (Self::Extreme, ColumnType::I32) => {
                [i32::MIN, i32::MIN + 1, -1, 0, 1, i32::MAX - 1, i32::MAX]
                    .map(Value::I32)
                    .to_vec()
            }
            (Self::Sequential(n), ColumnType::I32) => (0..n as i32).map(Value::I32).collect(),
            (Self::Sequential(n), ColumnType::I64) => (0..n as i64).map(Value::I64).collect(),
            (Self::Sequential(n), ColumnType::U32) => (0..n as u32).map(Value::U32).collect(),
            (Self::Small, ColumnType::I64) => (-3..=3).map(Value::I64).collect(),
            (Self::Extreme, ColumnType::I64) => {
                [i64::MIN, i64::MIN + 1, -1, 0, 1, i64::MAX - 1, i64::MAX]
                    .map(Value::I64)
                    .to_vec()
            }
            (Self::Wide, ColumnType::I64) => {
                let mut values = Vec::new();
                for k in [-1i64, 0, 1, 3] {
                    values.extend([k, (1 << 32) + k, -(1 << 32) + k, (5 << 32) + k]);
                }
                values.into_iter().map(Value::I64).collect()
            }
            (Self::Small, ColumnType::U32) => (0..=3).map(Value::U32).collect(),
            (Self::Extreme, ColumnType::U32) => {
                [0, 1, 1 << 31, (1 << 31) + 1, u32::MAX - 1, u32::MAX]
                    .map(Value::U32)
                    .to_vec()
            }
            (Self::Small | Self::Extreme | Self::Wide, ColumnType::Str) => {
                // Equal prefixes, a prefix of another value, and differing tails.
                ["a", "ab", "abc", "abd", "abc_1", "abc_2", "b", "ba"]
                    .map(|s| Value::Str(s.to_owned()))
                    .to_vec()
            }
            (domain, ty) => panic!("no {domain:?} domain for {ty:?}"),
        }
    }
}

/// The literal values a query may print for a type: never the type minimum,
/// whose negated literal does not fit the unsigned magnitude.
pub(super) fn printable(value: &Value) -> bool {
    !matches!(value, Value::I32(i32::MIN) | Value::I64(i64::MIN))
}

/// G2 relation shapes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Shape {
    Empty,
    Single,
    AllDuplicates,
    /// All rows share the key (first column, or the declared key).
    OneKey,
    Distinct,
    /// More than one GPU tile (> 1,024 rows).
    Large,
    Random,
}

/// Data properties, inferred from rows so that hand-built cases count too.
pub(super) fn shape_features(rows: &[Vec<Value>]) -> Vec<String> {
    let distinct = rows.iter().collect::<BTreeSet<_>>().len();
    let mut features = Vec::new();
    match rows.len() {
        0 => features.push("shape-empty"),
        1 => features.push("shape-single"),
        _ => {
            if distinct == 1 {
                features.push("shape-all-duplicates");
            } else if rows.iter().all(|row| row[0] == rows[0][0]) {
                features.push("shape-one-key");
            }
            if distinct == rows.len() {
                features.push("shape-distinct");
            }
        }
    }
    if rows.len() > 1024 {
        features.push("shape-large");
    }
    features.into_iter().map(str::to_owned).collect()
}

pub(super) fn domain_features(data: &[Vec<Vec<Value>>]) -> Vec<String> {
    let mut features = Vec::new();
    let values = data.iter().flatten().flatten().collect::<BTreeSet<_>>();
    if values.iter().any(|v| {
        matches!(
            v,
            Value::I32(i32::MIN | i32::MAX)
                | Value::I64(i64::MIN | i64::MAX)
                | Value::U32(u32::MAX)
        )
    }) {
        features.push("extreme-values");
    }
    if values.iter().any(|v| v.as_i128().is_some_and(|v| v < 0)) {
        features.push("negative-values");
    }
    let mut low = BTreeMap::new();
    for value in &values {
        if let Value::I64(v) = value {
            low.entry(*v as u32)
                .or_insert_with(BTreeSet::new)
                .insert(*v);
        }
    }
    if low.values().any(|high| high.len() > 1) {
        features.push("low32-collision");
    }
    features.into_iter().map(str::to_owned).collect()
}
