use std::collections::HashMap;

use hashbrown::HashTable;

use crate::{Batch, Load, Relation, Result};

/// A sorted, column-local string code. Codes from different columns cannot be
/// compared as strings, even when their numeric values happen to match.
#[repr(transparent)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct StringCode(pub u32);

impl StringCode {
    /// An absent query literal; never assigned to a dictionary value.
    pub const MAX: Self = Self(u32::MAX);
}

impl cq_storage::OrderKey for StringCode {
    type Ordinal = u64;

    #[inline]
    fn ordinal(&self) -> Self::Ordinal {
        u64::from(self.0)
    }
}

impl cq_storage::scan::OrdinalDecode for StringCode {
    const WIDTH: cq_storage::scan::Width = cq_storage::scan::Width::W32;
    fn from_ordinal(ordinal: u64) -> Self {
        Self(ordinal as u32)
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
        let mut interner = StringInterner::default();
        for value in values {
            interner.encode(value);
        }
        interner.finish()
    }

    /// Encode borrowed strings in one input pass, allocating text only for
    /// distinct dictionary entries. Finalization sorts those entries and remaps
    /// integer codes; it never hashes or searches the source strings again.
    pub fn encode<'a>(values: impl IntoIterator<Item = &'a str>) -> (Self, Vec<StringCode>) {
        let mut interner = StringInterner::default();
        let mut codes: Vec<_> = values
            .into_iter()
            .map(|value| interner.encode(value))
            .collect();
        let dictionary = interner.finish();
        for code in &mut codes {
            code.0 = interner.remap[code.0 as usize];
        }
        (dictionary, codes)
    }

    /// Returns the source value's code, or `None` when the value is absent.
    // Encoding only: search stored dictionary entries for a code.
    pub fn lookup(&self, value: &str) -> Option<u32> {
        self.values
            .binary_search_by(|candidate| candidate.as_str().cmp(value))
            .ok()
            .map(|index| index as u32)
    }

    /// The number of stored values that sort `before` a query value.
    // Encoding only: partition stored dictionary entries around a query value.
    fn rank(&self, mut before: impl FnMut(&str) -> bool) -> StringCode {
        StringCode(self.values.partition_point(|candidate| before(candidate)) as u32)
    }
}

/// The dictionary is the column's code domain: codes follow the sorted order of
/// distinct values, so it supports ordered comparisons. Absent values map to
/// [`StringCode::MAX`] for equality and to their insertion rank for bounds.
impl cq_storage::CodeDomain for StringDictionary {
    type Value = str;
    type Code = StringCode;
    fn exact(&self, value: &str) -> StringCode {
        self.lookup(value).map_or(StringCode::MAX, StringCode)
    }
}

impl cq_storage::OrderedCodeDomain for StringDictionary {
    fn lower(&self, value: &str) -> StringCode {
        self.rank(|candidate| candidate < value)
    }
    fn upper(&self, value: &str) -> StringCode {
        self.rank(|candidate| candidate <= value)
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
        self.register(column, dictionary);
    }

    fn register(&mut self, column: &str, dictionary: StringDictionary) {
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

    /// The code domain of a column, named in query declarations as
    /// `col: StringCode in (dictionaries.domain("col"))`.
    /// An unregistered column has the empty domain: no value is present.
    pub fn domain(&self, column: &str) -> &StringDictionary {
        static EMPTY: StringDictionary = StringDictionary { values: Vec::new() };
        self.get(column).unwrap_or(&EMPTY)
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

struct InternEntry {
    value: String,
    hash: u64,
    code: u32,
}

/// Load-time state for a string column; intermediate codes never escape loading.
///
/// Each source string is hashed once. Cached hashes also avoid rereading text
/// when the table grows. Only distinct strings own allocations. Sorted final
/// codes are independent of encounter order and randomized hash-table layout.
#[derive(Default)]
pub struct StringInterner {
    index: HashTable<u32>,
    entries: Vec<InternEntry>,
    hasher: ahash::RandomState,
    remap: Vec<u32>,
}

impl StringInterner {
    fn encode(&mut self, value: &str) -> StringCode {
        let hash = self.hasher.hash_one(value);
        let entries = &mut self.entries;
        let next = entries.len();
        let entry = self
            .index
            .entry(
                hash,
                |&code| entries[code as usize].value == value,
                |&code| entries[code as usize].hash,
            )
            .or_insert_with(|| {
                assert!(
                    next < u32::MAX as usize,
                    "string dictionary exceeds u32 codes"
                );
                entries.push(InternEntry {
                    value: value.to_owned(),
                    hash,
                    code: next as u32,
                });
                next as u32
            });
        StringCode(*entry.get())
    }

    fn finish(&mut self) -> StringDictionary {
        // The lookup table stores only indices, so growth never moves text
        // descriptors. Distinct values are already dense and ready to sort.
        self.index = HashTable::new();
        let mut entries = std::mem::take(&mut self.entries);
        entries.sort_unstable_by(|left, right| left.value.cmp(&right.value));
        self.remap.resize(entries.len(), 0);
        let values = entries
            .into_iter()
            .enumerate()
            .map(|(code, entry)| {
                self.remap[entry.code as usize] = code as u32;
                entry.value
            })
            .collect();
        StringDictionary { values }
    }
}

/// Load-time field conversion selected by traits, not by Rust type spelling.
///
/// Ordinary [`Load`] values pass through unchanged. [`StringCode`] borrows
/// source cells directly into column-local state; its [`String`] source type is
/// used only by callers supplying already-owned in-memory fixture rows.
pub trait InternedField: Sized {
    type Source: Load;
    type State: Default;

    fn encode(value: Self::Source, state: &mut Self::State) -> Self;
    fn column(batch: &Batch<'_>, index: usize, state: &mut Self::State) -> Result<Vec<Self>>;
    fn finish(column: &str, state: &mut Self::State, dictionaries: &mut Dictionaries);
    fn remap(&mut self, state: &Self::State);
}

impl<T: Load> InternedField for T {
    type Source = T;
    type State = ();

    fn encode(value: T, _: &mut ()) -> Self {
        value
    }
    fn column(batch: &Batch<'_>, index: usize, _: &mut ()) -> Result<Vec<Self>> {
        batch.column(index)
    }
    fn finish(_: &str, _: &mut (), _: &mut Dictionaries) {}
    fn remap(&mut self, _: &()) {}
}

impl InternedField for StringCode {
    type Source = String;
    type State = StringInterner;

    fn encode(value: String, state: &mut StringInterner) -> Self {
        state.encode(&value)
    }
    fn column(batch: &Batch<'_>, index: usize, state: &mut StringInterner) -> Result<Vec<Self>> {
        batch.strings(index, |value| state.encode(value))
    }
    fn finish(column: &str, state: &mut StringInterner, dictionaries: &mut Dictionaries) {
        dictionaries.register(column, state.finish());
    }
    fn remap(&mut self, state: &StringInterner) {
        self.0 = state.remap[self.0 as usize];
    }
}

/// A generated relation with column-local state used only during loading.
/// File readers borrow source text; returned relation rows own final codes.
pub trait InternedRelation {
    type Row;
    type Source: Relation;
    type State;

    fn state() -> Self::State;

    fn intern(
        rows: Vec<<Self::Source as Relation>::Row>,
        dictionaries: &mut Dictionaries,
    ) -> Vec<Self::Row>;
    fn decode(batch: &Batch<'_>, state: &mut Self::State) -> Result<Vec<Self::Row>>;
    fn finish(rows: &mut [Self::Row], state: Self::State, dictionaries: &mut Dictionaries);
}

#[cfg(test)]
mod tests {
    use super::{Dictionaries, InternedField, StringCode, StringDictionary};

    #[test]
    fn borrowed_encoding_matches_sorted_dictionary_across_growth_and_permutation() {
        let values: Vec<_> = (0..8192)
            .map(|i| format!("{}-é-{}", i % 997, i % 19))
            .collect();
        let mut source: Vec<_> = values.iter().map(String::as_str).collect();
        source.extend(["", " ", "a", "a\0", "a\0b", "a ", "A", "é", "東京", ""]);
        for reverse in [false, true] {
            if reverse {
                source.reverse();
            }
            let (dictionary, codes) = StringDictionary::encode(source.iter().copied());
            let mut sorted = source.clone();
            sorted.sort_unstable();
            sorted.dedup();
            assert_eq!(dictionary.values, sorted);
            for (value, code) in source.iter().zip(codes) {
                assert_eq!(code.0 as usize, sorted.binary_search(value).unwrap());
            }
        }
        let (dictionary, codes) = StringDictionary::encode([]);
        assert!(dictionary.values.is_empty());
        assert!(codes.is_empty());
    }

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
        use cq_storage::domain::exact;
        let mut dictionaries = Dictionaries::default();
        dictionaries.insert("name", ["z", "", "a", "a"]);
        dictionaries.insert("other", ["a", "z"]);
        assert_eq!(exact(dictionaries.domain("name"), ""), StringCode(0));
        assert_eq!(exact(dictionaries.domain("name"), "a"), StringCode(1));
        assert_eq!(exact(dictionaries.domain("other"), "a"), StringCode(0));
        assert_eq!(exact(dictionaries.domain("name"), "missing"), StringCode::MAX);
        assert_eq!(exact(dictionaries.domain("absent"), "a"), StringCode::MAX);
        dictionaries.insert("name", ["a", "z", ""]);
        // DECODE R: the dictionary diagnostic borrows a stored string for its exact-value assertion.
        assert_eq!(dictionaries.read("name", StringCode(1)), "a");
        // DECODE P: the dictionary diagnostic publishes a stored string to its owned-value assertion.
        assert_eq!(dictionaries.publish("name", StringCode(2)), "z");
        assert!(exact(dictionaries.domain("name"), "z") < StringCode::MAX);
    }

    #[test]
    fn ordered_bounds_translate_comparisons_to_codes() {
        use cq_storage::domain::{lower, upper};
        let dictionary = StringDictionary::new(["b", "d", "f"]);
        let codes = [StringCode(0), StringCode(1), StringCode(2)];
        let values = ["b", "d", "f"];
        for probe in ["", "a", "b", "c", "d", "e", "f", "g", "é"] {
            let (low, high) = (lower(&dictionary, probe), upper(&dictionary, probe));
            for (code, value) in codes.iter().zip(values) {
                assert_eq!(*code < low, value < probe, "{value} < {probe}");
                assert_eq!(*code >= low, value >= probe, "{value} >= {probe}");
                assert_eq!(*code < high, value <= probe, "{value} <= {probe}");
                assert_eq!(*code >= high, value > probe, "{value} > {probe}");
            }
        }
        let empty = Dictionaries::default();
        assert_eq!(lower(empty.domain("absent"), "a"), StringCode(0));
        assert_eq!(upper(empty.domain("absent"), "a"), StringCode(0));
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
        assert_eq!(cq_storage::domain::exact(dictionaries.domain("type"), "missing"), StringCode::MAX);
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
        assert_eq!(cq_storage::domain::exact(dictionaries.domain("empty"), ""), StringCode::MAX);
    }

    #[test]
    fn string_code_dense_ordinals_use_native_words() {
        use cq_storage::{DenseMembership, ExactMembership, OrderKey};

        let ordinal: u64 = StringCode::MAX.ordinal();
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
        let mut state = super::StringInterner::default();
        let mut codes: Vec<_> = values
            .into_iter()
            .map(|value| StringCode::encode(value, &mut state))
            .collect();
        StringCode::finish("name", &mut state, &mut dictionaries);
        for code in &mut codes {
            code.remap(&state);
        }
        assert_eq!(codes, [StringCode(1), StringCode(0), StringCode(1)]);
        assert_eq!(i64::encode(42, &mut ()), 42);
        assert!(f64::encode(f64::NAN, &mut ()).is_nan());
    }
}
