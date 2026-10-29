use std::collections::HashMap;

use crate::{Load, Relation};

/// A sorted, column-local string code. Codes from different columns cannot be
/// compared as strings, even when their numeric values happen to match.
#[repr(transparent)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct StringCode(pub u32);

impl StringCode {
    /// An absent query literal; never assigned to a dictionary value.
    pub const MAX: Self = Self(u32::MAX);
}

impl cq_storage::DenseOrdinal for StringCode {
    type Ordinal = u64;

    #[inline]
    fn dense_ordinal(&self) -> Self::Ordinal {
        u64::from(self.0)
    }
}

impl cq_storage::KeyCodec<StringCode> for cq_storage::I64Codec {
    #[inline]
    fn ordinal(self, key: &StringCode) -> Option<u128> {
        Some(u128::from(key.0))
    }

    fn total(self) -> bool {
        true
    }
}

/// A caller-owned string dictionary with deterministic, zero-based `u32` codes.
///
/// Codes follow the lexicographic order of distinct source strings, independent
/// of row order and duplicates. Values are preserved exactly, including empty
/// strings, case, and whitespace. `u32::MAX` is reserved for callers that need a
/// nonmatching code for an absent query literal; it is never assigned to a value.
#[derive(Debug, PartialEq, Eq)]
pub struct StringDictionary {
    values: Vec<String>,
}

impl StringDictionary {
    /// Builds a dictionary, copying each distinct string only once.
    ///
    /// Panics if more than `u32::MAX` distinct values are supplied.
    pub fn new<'a>(values: impl IntoIterator<Item = &'a str>) -> Self {
        let mut values: Vec<&str> = values.into_iter().collect();
        values.sort_unstable();
        values.dedup();
        assert!(
            values.len() <= u32::MAX as usize,
            "string dictionary exceeds u32 codes"
        );
        Self {
            values: values.into_iter().map(str::to_owned).collect(),
        }
    }

    /// Returns the source value's code, or `None` when the value is absent.
    // Encoding only: search stored dictionary entries for a code.
    pub fn lookup(&self, value: &str) -> Option<u32> {
        self.values
            .binary_search_by(|candidate| candidate.as_str().cmp(value))
            .ok()
            .map(|index| index as u32)
    }
}

/// Load-time dictionaries keyed by declaration column name.
///
/// Names must uniquely identify a column across the loaded relations. A column
/// is registered once; registering identical contents again is harmless, but
/// changing its contents panics rather than invalidating already encoded rows.
#[derive(Debug, Default)]
pub struct Dictionaries {
    columns: HashMap<String, StringDictionary>,
}

impl Dictionaries {
    /// Register all source values for a column before encoding any of its rows.
    pub fn insert<'a>(&mut self, column: &str, values: impl IntoIterator<Item = &'a str>) {
        let dictionary = StringDictionary::new(values);
        if let Some(existing) = self.columns.get(column) {
            assert_eq!(
                existing, &dictionary,
                "dictionary column {column:?} changed"
            );
        } else {
            self.columns.insert(column.to_owned(), dictionary);
        }
    }

    /// Borrow a registered dictionary for generated load-time field encoding.
    pub fn get(&self, column: &str) -> Option<&StringDictionary> {
        self.columns.get(column)
    }

    /// Translate an equality or membership literal without inserting it.
    /// An absent column or value returns the nonmatching sentinel, not an
    /// insertion rank for ordered comparisons against an absent value.
    pub fn literal(&self, column: &str, value: &str) -> StringCode {
        self.get(column)
            .and_then(|dictionary| dictionary.lookup(value))
            .map_or(StringCode::MAX, StringCode)
    }

    /// Borrow source text for an explicit opaque Rust read.
    ///
    /// Panics for an unregistered column, invalid code, or missing sentinel.
    pub fn read(&self, column: &str, code: StringCode) -> &str {
        // DECODE R: Dictionaries::read borrows a dictionary value for an explicit opaque Rust consumer.
        &self.columns[column].values[code.0 as usize]
    }

    /// Materialize an owned string only at answer publication.
    pub fn publish(&self, column: &str, code: StringCode) -> String {
        // DECODE P: Dictionaries::publish materializes owned text at the caller's explicit answer boundary.
        self.columns[column].values[code.0 as usize].clone()
    }

    /// Prepare a unary code relation by testing each distinct value exactly
    /// once in sorted order. Call during loading, not once per source row.
    ///
    /// An unregistered column produces an empty relation.
    pub fn select(
        &self,
        column: &str,
        mut predicate: impl FnMut(&str) -> bool,
    ) -> Vec<(StringCode,)> {
        self.get(column)
            .into_iter()
            .flat_map(|dictionary| dictionary.values.iter().enumerate())
            .filter_map(|(index, value)| {
                // DECODE R: the load-time dictionary predicate reads each distinct stored string once.
                predicate(value).then_some((StringCode(index as u32),))
            })
            .collect()
    }
}

/// Prepare an ordinary unary input relation from a column's dictionary.
/// Evaluate opaque string work once per distinct entry during loading.
pub fn dictionary_where(
    dictionaries: &Dictionaries,
    column: &str,
    predicate: impl FnMut(&str) -> bool,
) -> Vec<(StringCode,)> {
    dictionaries.select(column, predicate)
}

/// Load-time field conversion selected by traits, not by Rust type spelling.
///
/// Every [`Load`] type is preserved unchanged. [`StringCode`] instead loads
/// input-only [`String`] values, prepares a sorted dictionary, and encodes them.
/// It intentionally does not implement `Load`: a code needs its column's
/// complete dictionary and cannot be parsed independently from one source cell.
pub trait InternedField: Sized {
    type Source: Load;

    fn prepare<'a>(
        column: &str,
        values: impl Iterator<Item = &'a Self::Source>,
        dictionaries: &mut Dictionaries,
    ) where
        Self::Source: 'a;

    fn intern(value: Self::Source, dictionary: Option<&StringDictionary>) -> Self;
}

impl<T: Load> InternedField for T {
    type Source = T;

    fn prepare<'a>(_: &str, _: impl Iterator<Item = &'a T>, _: &mut Dictionaries)
    where
        T: 'a,
    {
    }

    fn intern(value: T, _: Option<&StringDictionary>) -> Self {
        value
    }
}

impl InternedField for StringCode {
    type Source = String;

    fn prepare<'a>(
        column: &str,
        values: impl Iterator<Item = &'a String>,
        dictionaries: &mut Dictionaries,
    ) where
        String: 'a,
    {
        dictionaries.insert(column, values.map(String::as_str));
    }

    fn intern(value: String, dictionary: Option<&StringDictionary>) -> Self {
        Self(
            dictionary
                .expect("string column dictionary was not prepared")
                .lookup(&value)
                .expect("source value missing from prepared dictionary"),
        )
    }
}

/// A generated relation whose source schema may differ from its encoded rows.
/// Source strings exist only while loading; returned relation rows own codes.
pub trait InternedRelation {
    type Row;
    type Source: Relation;

    fn intern(
        rows: Vec<<Self::Source as Relation>::Row>,
        dictionaries: &mut Dictionaries,
    ) -> Vec<Self::Row>;
}

#[cfg(test)]
mod tests {
    use super::{Dictionaries, InternedField, StringCode, StringDictionary};

    #[test]
    fn code_capabilities_select_dense_membership_and_ordered_keysets() {
        use cq_storage::{
            AdaptiveKeySet, AdaptiveMembership, AutoDense, ExactIntervalMembership, ExactMembership,
        };

        type Code = StringCode;
        let codes: [Code; 3] = [StringCode(0), StringCode(2), StringCode(2)];
        let membership = cq_storage::build!(
            AdaptiveMembership<_, 64>,
            codes.iter().map(|code| ((code,), ())),
        );
        let membership: cq_storage::DenseMembership<(&Code,), 64> = membership;
        assert!(membership.is_dense());
        assert!(membership.contains(&(&StringCode(0),)));
        assert!(membership.contains(&(&StringCode(2),)));
        assert!(!membership.contains(&(&StringCode(1),)));
        assert!(!membership.contains(&(&StringCode::MAX,)));

        let ordered = cq_storage::build!(
            AdaptiveKeySet<_, AutoDense, 64>,
            codes.into_iter().map(|code| (code, ())),
        );
        let ordered: AdaptiveKeySet<Code, cq_storage::I64Codec, 64> = ordered;
        assert_eq!(ordered.retained_bytes(), std::mem::size_of::<u64>());
        assert!(ordered.any_in(&StringCode(1), &StringCode(2)));
        assert!(!ordered.any_in(&StringCode(1), &StringCode(1)));
        assert!(!ordered.contains(&StringCode::MAX));
        assert!(!ordered.any_in(&StringCode(3), &StringCode::MAX));
    }

    #[test]
    fn codes_are_sorted_distinct_and_independent_of_row_order() {
        let values = ["RAIL", "AIR REG", "AIR", "RAIL", "AIR"];
        let forward = StringDictionary::new(values);
        let reversed = StringDictionary::new(values.into_iter().rev());
        for (code, value) in ["AIR", "AIR REG", "RAIL"].into_iter().enumerate() {
            assert_eq!(forward.lookup(value), Some(code as u32));
            assert_eq!(reversed.lookup(value), Some(code as u32));
        }
        assert_eq!(forward.lookup("SHIP"), None);
    }

    #[test]
    fn values_remain_exact_and_missing_values_never_alias_source_codes() {
        let dictionary = StringDictionary::new(["air", "AIR ", "AIR", ""]);
        for (code, value) in ["", "AIR", "AIR ", "air"].into_iter().enumerate() {
            assert_eq!(dictionary.lookup(value), Some(code as u32));
        }
        assert_eq!(dictionary.lookup(" AIR"), None);
        let empty = StringDictionary::new(std::iter::empty());
        assert_eq!(empty.lookup("AIR"), None);
        assert_eq!(empty.lookup(""), None);
    }

    #[test]
    fn dictionaries_keep_column_order_and_missing_literals_stable() {
        let mut dictionaries = Dictionaries::default();
        dictionaries.insert("name", ["z", "", "a", "a"]);
        dictionaries.insert("other", ["a", "z"]);
        assert_eq!(dictionaries.literal("name", ""), StringCode(0));
        assert_eq!(dictionaries.literal("name", "a"), StringCode(1));
        assert_eq!(dictionaries.literal("other", "a"), StringCode(0));
        assert_eq!(dictionaries.literal("name", "missing"), StringCode::MAX);
        assert_eq!(dictionaries.literal("absent", "a"), StringCode::MAX);
        dictionaries.insert("name", ["a", "z", ""]);
        // DECODE R: the dictionary diagnostic borrows a stored string for its exact-value assertion.
        assert_eq!(dictionaries.read("name", StringCode(1)), "a");
        // DECODE P: the dictionary diagnostic publishes a stored string to its owned-value assertion.
        assert_eq!(dictionaries.publish("name", StringCode(2)), "z");
        assert!(dictionaries.literal("name", "z") < StringCode::MAX);
    }

    #[test]
    fn selection_tests_each_distinct_value_once_without_inserting_literals() {
        let mut dictionaries = Dictionaries::default();
        dictionaries.insert("type", ["red", "blue", "red", "green"]);
        let mut visited = Vec::new();
        let selected = super::dictionary_where(&dictionaries, "type", |value| {
            visited.push(value.to_owned());
            value.ends_with('e')
        });
        assert_eq!(visited, ["blue", "green", "red"]);
        assert_eq!(selected, [(StringCode(0),)]);
        assert_eq!(dictionaries.literal("type", "missing"), StringCode::MAX);
        assert_eq!(dictionaries.select("type", |_| true).len(), 3);
        assert!(
            dictionaries
                .select("absent", |_| panic!("no values"))
                .is_empty()
        );
        dictionaries.insert("empty", []);
        assert!(
            dictionaries
                .select("empty", |_| panic!("no values"))
                .is_empty()
        );
        assert_eq!(dictionaries.literal("empty", ""), StringCode::MAX);
    }

    #[test]
    fn string_code_dense_ordinals_use_native_words() {
        use cq_storage::{DenseMembership, DenseOrdinal, ExactMembership};

        let ordinal: u64 = StringCode::MAX.dense_ordinal();
        assert_eq!(ordinal, u64::from(u32::MAX));
        let set = DenseMembership::<_, 64>::from_dense_keys([StringCode(0), StringCode(63)]);
        assert!(set.is_dense());
        assert!(set.contains(&StringCode(0)));
        assert!(set.contains(&StringCode(63)));
        assert!(!set.contains(&StringCode(1)));
        assert!(!set.contains(&StringCode::MAX));
    }

    #[test]
    #[should_panic(expected = "dictionary column")]
    fn registration_cannot_invalidate_existing_codes() {
        let mut dictionaries = Dictionaries::default();
        dictionaries.insert("name", ["z"]);
        dictionaries.insert("name", ["a", "z"]);
    }

    #[test]
    fn field_traits_encode_strings_and_move_scalars_unchanged() {
        let values = ["z".to_owned(), "a".to_owned(), "z".to_owned()];
        let mut dictionaries = Dictionaries::default();
        StringCode::prepare("name", values.iter(), &mut dictionaries);
        let dictionary = dictionaries.get("name");
        let codes: Vec<_> = values
            .into_iter()
            .map(|value| StringCode::intern(value, dictionary))
            .collect();
        assert_eq!(codes, [StringCode(1), StringCode(0), StringCode(1)]);
        assert_eq!(i64::intern(42, None), 42);
        assert!(f64::intern(f64::NAN, None).is_nan());
    }
}
