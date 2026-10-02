use std::collections::BTreeSet;

// Only normalize columns DuckDB actually typed as DECIMAL. Numeric-looking
// VARCHAR values remain distinct. No floating parsing, rounding or tolerance.
pub(crate) fn normalize_decimal(value: String) -> String {
    let unsigned = value.strip_prefix('-').unwrap_or(&value);
    if let Some((whole, fraction)) = unsigned.split_once('.') {
        if !whole.is_empty()
            && !fraction.is_empty()
            && whole
                .bytes()
                .chain(fraction.bytes())
                .all(|b| b.is_ascii_digit())
        {
            let normalized = value.trim_end_matches('0').trim_end_matches('.');
            return if normalized == "-0" {
                "0".to_owned()
            } else {
                normalized.to_owned()
            };
        }
    }
    value
}

pub(crate) fn normalize_set(
    rows: Vec<Vec<String>>,
    decimal_columns: &BTreeSet<usize>,
) -> BTreeSet<Vec<String>> {
    rows.into_iter()
        .map(|row| {
            row.into_iter()
                .enumerate()
                .map(|(column, value)| {
                    if decimal_columns.contains(&column) {
                        normalize_decimal(value)
                    } else {
                        value
                    }
                })
                .collect()
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn decimal_equality_does_not_round_or_use_float() {
        assert_eq!(
            normalize_decimal("12345678901234567890.123400".into()),
            "12345678901234567890.1234"
        );
        assert_ne!(
            normalize_decimal("1.000000000000000001".into()),
            normalize_decimal("1.000000000000000002".into())
        );
        assert_eq!(normalize_decimal("-0.00".into()), "0");
        let rows = vec![vec!["1.00".into()], vec!["1.0".into()]];
        assert_eq!(normalize_set(rows.clone(), &BTreeSet::new()).len(), 2);
        assert_eq!(normalize_set(rows, &BTreeSet::from([0])).len(), 1);
    }
}
