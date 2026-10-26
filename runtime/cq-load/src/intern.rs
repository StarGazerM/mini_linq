/// A caller-owned string dictionary with deterministic, zero-based `u32` codes.
///
/// Codes follow the lexicographic order of distinct source strings, independent
/// of row order and duplicates. Values are preserved exactly, including empty
/// strings, case, and whitespace. `u32::MAX` is reserved for callers that need a
/// nonmatching code for an absent query literal; it is never assigned to a value.
#[derive(Debug)]
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
    pub fn lookup(&self, value: &str) -> Option<u32> {
        self.values
            .binary_search_by(|candidate| candidate.as_str().cmp(value))
            .ok()
            .map(|index| index as u32)
    }
}

#[cfg(test)]
mod tests {
    use super::StringDictionary;

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
}
