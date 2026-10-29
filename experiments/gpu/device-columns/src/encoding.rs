use crate::{Result, failure};
use std::{collections::HashMap, sync::Arc};

/// A dataset-wide shared string domain. IDs preserve equality, not lexical order.
/// Borrowing input strings allows decoded `&str` answers to outlive the query.
#[derive(Clone)]
enum Text<'data> {
    Borrowed(&'data str),
    Owned(Arc<str>),
}
impl Text<'_> {
    fn text(&self) -> &str {
        match self {
            Self::Borrowed(value) => value,
            Self::Owned(value) => value,
        }
    }
}
// DESIGN contract8-device-dictionary
// Owns single-writer first-sight codes: equality only, not lexical order (contract 8 §10).
// Sources: doc/STORAGE.md §§1,10; Data Blocks §3.3 distinguishes local dictionaries and translation.
// DO NOT compare codes as string order, join unrelated dictionaries, or recode published runs.
// Parallel code assignment is a publication decision; stable/dense codes affect routing and merges.
// Publication-point assignment is an unmeasured direction, not implemented policy (Plan P4).
// Constant translation for `where` must not insert; missing values mean empty matches.
#[derive(Clone, Default)]
pub struct Dictionary<'data> {
    ids: HashMap<&'data str, i64>,
    owned_ids: HashMap<Arc<str>, i64>,
    values: Vec<Text<'data>>,
}

impl<'data> Dictionary<'data> {
    pub fn intern(&mut self, value: &'data str) -> Result<i64> {
        if let Some(&id) = self.ids.get(value) {
            return Ok(id);
        }
        if let Some(id) = self.owned_ids.remove(value) {
            // A genuine caller-owned borrow can safely replace an owned copy
            // at the same ID, allowing both String and &'data str decoding.
            self.values[id as usize] = Text::Borrowed(value);
            self.ids.insert(value, id);
            return Ok(id);
        }
        let id = i64::try_from(self.values.len())
            .map_err(|_| failure("CQ_GPU_DICTIONARY_RANGE", "string domain exceeds i64"))?;
        self.values.push(Text::Borrowed(value));
        self.ids.insert(value, id);
        Ok(id)
    }

    // DECODE ?: borrowed dictionary inverse requires caller publication or opaque Rust read evidence.
    pub fn decode(&self, value: i64) -> Result<&'data str> {
        match usize::try_from(value)
            .ok()
            .and_then(|id| self.values.get(id))
        {
            Some(Text::Borrowed(text)) => Ok(text),
            Some(Text::Owned(_)) => Err(failure(
                "CQ_GPU_DICTIONARY_BORROW",
                "owned dictionary text requires an owned String answer",
            )),
            None => Err(failure(
                "CQ_GPU_DICTIONARY_ID",
                format!("unknown string ID {value}"),
            )),
        }
    }

    // DECODE ?: owned/borrowed dictionary lookup alone does not establish its logical read boundary.
    pub fn text(&self, value: i64) -> Result<&str> {
        usize::try_from(value)
            .ok()
            .and_then(|id| self.values.get(id))
            .map(Text::text)
            .ok_or_else(|| failure("CQ_GPU_DICTIONARY_ID", format!("unknown string ID {value}")))
    }

    pub fn intern_owned(&mut self, value: &str) -> Result<i64> {
        if let Some(&id) = self.ids.get(value).or_else(|| self.owned_ids.get(value)) {
            return Ok(id);
        }
        let id = i64::try_from(self.values.len())
            .map_err(|_| failure("CQ_GPU_DICTIONARY_RANGE", "string domain exceeds i64"))?;
        let value: Arc<str> = value.into();
        self.values.push(Text::Owned(value.clone()));
        self.owned_ids.insert(value, id);
        Ok(id)
    }

    pub fn merge(&mut self, other: &Self) -> Result<()> {
        for (index, value) in other.values.iter().enumerate() {
            if index < self.values.len() {
                if self.values[index].text() != value.text() {
                    return Err(failure(
                        "CQ_GPU_DICTIONARY_DOMAIN",
                        "resident inputs use incompatible dictionary IDs",
                    ));
                }
                if let Text::Borrowed(value) = value {
                    self.intern(value)?;
                }
            } else {
                match value {
                    Text::Borrowed(value) => {
                        self.intern(value)?;
                    }
                    Text::Owned(value) => {
                        self.intern_owned(value)?;
                    }
                }
            }
        }
        Ok(())
    }

    /// Validate a replacement dictionary without changing any encoded IDs or
    /// weakening a borrowed row's decoding lifetime.
    pub(crate) fn check_extension(&self, replacement: &Self) -> Result<()> {
        for (index, original) in self.values.iter().enumerate() {
            let Some(next) = replacement.values.get(index) else {
                return Err(failure(
                    "CQ_GPU_DICTIONARY_DOMAIN",
                    "replacement dictionary omits existing resident IDs",
                ));
            };
            if original.text() != next.text() {
                return Err(failure(
                    "CQ_GPU_DICTIONARY_DOMAIN",
                    "replacement dictionary changes existing resident IDs",
                ));
            }
            if matches!((original, next), (Text::Borrowed(_), Text::Owned(_))) {
                return Err(failure(
                    "CQ_GPU_DICTIONARY_BORROW",
                    "replacement dictionary would discard borrowed text owners",
                ));
            }
        }
        Ok(())
    }
}

/// Host scalar publication encoding. Decimal retains its signed scaled
/// coefficient, Date retains epoch days, and text retains dataset dictionary IDs.
pub trait EncodeValue<'data> {
    fn encode(self, dictionary: &mut Dictionary<'data>) -> Result<i64>;
}

/// Reconstruct a typed Rust scalar at a host read or answer publication boundary.
pub trait DecodeValue<'data>: Sized {
    // DECODE ?: logical scalar inverse contract is shared by host reads and answer publication.
    fn decode(value: i64, dictionary: &Dictionary<'data>) -> Result<Self>;
}

pub fn encode_value<'data, T: EncodeValue<'data>>(
    value: T,
    dictionary: &mut Dictionary<'data>,
) -> Result<i64> {
    value.encode(dictionary)
}

// DECODE ?: generic typed reconstruction needs the caller read fact; a Rust type is not evidence.
pub fn decode_value<'data, T: DecodeValue<'data>>(
    value: i64,
    dictionary: &Dictionary<'data>,
) -> Result<T> {
    // DECODE ?: trait dispatch inherits the generic caller boundary.
    T::decode(value, dictionary)
}

macro_rules! integers {
    ($($ty:ty),* $(,)?) => {$ (
        impl<'data> EncodeValue<'data> for $ty {
            fn encode(self, _: &mut Dictionary<'data>) -> Result<i64> {
                i64::try_from(self).map_err(|_| failure("CQ_GPU_INTEGER_RANGE",
                    concat!(stringify!($ty), " value is not losslessly representable as i64")))
            }
        }
        impl<'data> EncodeValue<'data> for &$ty {
            fn encode(self, dictionary: &mut Dictionary<'data>) -> Result<i64> {
                (*self).encode(dictionary)
            }
        }
        impl<'data> DecodeValue<'data> for $ty {
            // DECODE ?: integer carrier inverse serves generic typed reads and answers.
            fn decode(value: i64, _: &Dictionary<'data>) -> Result<Self> {
                <$ty>::try_from(value).map_err(|_| failure("CQ_GPU_DECODE_RANGE",
                    concat!("encoded value is outside ", stringify!($ty))))
            }
        }
    )*};
}
integers!(
    i8, i16, i32, i64, i128, isize, u8, u16, u32, u64, u128, usize
);

// Host publication codecs preserve the scalar's identity representation. These
// traits serve typed Rust reads/answers, not a separate staging conversion.
impl<'data, const SCALE: u32> EncodeValue<'data> for cq_load::Decimal<SCALE> {
    fn encode(self, _: &mut Dictionary<'data>) -> Result<i64> {
        Ok(self.scaled())
    }
}
impl<'data, const SCALE: u32> EncodeValue<'data> for &cq_load::Decimal<SCALE> {
    fn encode(self, dictionary: &mut Dictionary<'data>) -> Result<i64> {
        (*self).encode(dictionary)
    }
}
impl<'data, const SCALE: u32> DecodeValue<'data> for cq_load::Decimal<SCALE> {
    // DECODE ?: Decimal inverse is host-only but its caller must establish the allowed read.
    fn decode(value: i64, _: &Dictionary<'data>) -> Result<Self> {
        // DECODE ?: coefficient-to-Decimal construction inherits the host caller boundary.
        Self::from_scaled(value).map_err(|message| failure("CQ_GPU_DECODE_RANGE", message))
    }
}
impl<'data> EncodeValue<'data> for cq_load::Date {
    fn encode(self, _: &mut Dictionary<'data>) -> Result<i64> {
        Ok(i64::from(self.days_since_epoch()))
    }
}
impl<'data> EncodeValue<'data> for &cq_load::Date {
    fn encode(self, dictionary: &mut Dictionary<'data>) -> Result<i64> {
        (*self).encode(dictionary)
    }
}
impl<'data> DecodeValue<'data> for cq_load::Date {
    // DECODE ?: Date inverse is host-only but its caller must establish publication or opaque read.
    fn decode(value: i64, dictionary: &Dictionary<'data>) -> Result<Self> {
        // DECODE ?: checked day-carrier inverse and Date construction inherit the host caller boundary.
        decode_value::<i32>(value, dictionary).map(Self::from_days)
    }
}

impl<'data> EncodeValue<'data> for bool {
    fn encode(self, _: &mut Dictionary<'data>) -> Result<i64> {
        Ok(self as i64)
    }
}
impl<'data> EncodeValue<'data> for &bool {
    fn encode(self, dictionary: &mut Dictionary<'data>) -> Result<i64> {
        (*self).encode(dictionary)
    }
}
impl<'data> DecodeValue<'data> for bool {
    // DECODE ?: boolean carrier inverse requires caller logical-consumption evidence.
    fn decode(value: i64, _: &Dictionary<'data>) -> Result<Self> {
        match value {
            0 => Ok(false),
            1 => Ok(true),
            _ => Err(failure(
                "CQ_GPU_DECODE_RANGE",
                "boolean value is not 0 or 1",
            )),
        }
    }
}
impl<'data> EncodeValue<'data> for &'data str {
    fn encode(self, dictionary: &mut Dictionary<'data>) -> Result<i64> {
        dictionary.intern(self)
    }
}
impl<'data> EncodeValue<'data> for &&'data str {
    fn encode(self, dictionary: &mut Dictionary<'data>) -> Result<i64> {
        dictionary.intern(self)
    }
}
impl<'data> EncodeValue<'data> for &'data String {
    fn encode(self, dictionary: &mut Dictionary<'data>) -> Result<i64> {
        dictionary.intern(self.as_str())
    }
}
impl<'data> DecodeValue<'data> for &'data str {
    // DECODE ?: borrowed string decoder delegates dictionary inversion without caller evidence.
    fn decode(value: i64, dictionary: &Dictionary<'data>) -> Result<Self> {
        // DECODE ?: borrowed dictionary resolution inherits its generic caller boundary.
        dictionary.decode(value)
    }
}
impl<'data> DecodeValue<'data> for String {
    // DECODE ?: owned string decoder lacks a caller-level publication fact.
    fn decode(value: i64, dictionary: &Dictionary<'data>) -> Result<Self> {
        // DECODE ?: dictionary resolution and String allocation require the eventual logical-read fact.
        Ok(dictionary.text(value)?.to_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decimal_publication_preserves_coefficients_and_scale() {
        fn round_trip<const SCALE: u32>() {
            let mut dictionary = Dictionary::default();
            for coefficient in [i64::MIN, -12345, 0, 12345, i64::MAX] {
                // DECODE P: diagnostic Decimal fixture constructs the expected coefficient answer.
                let value = cq_load::Decimal::<SCALE>::from_scaled(coefficient).unwrap();
                assert_eq!(encode_value(value, &mut dictionary).unwrap(), coefficient);
                assert_eq!(encode_value(&value, &mut dictionary).unwrap(), coefficient);
                assert_eq!(
                    // DECODE P: diagnostic Decimal round trip exposes the reconstructed scalar.
                    decode_value::<cq_load::Decimal<SCALE>>(coefficient, &dictionary).unwrap(),
                    value
                );
            }
        }
        round_trip::<0>();
        round_trip::<2>();
        round_trip::<18>();
        // DECODE P: diagnostic unsupported scale checks logical decode rejection.
        assert!(decode_value::<cq_load::Decimal<19>>(0, &Dictionary::default()).is_err());
    }

    #[test]
    fn date_publication_preserves_epoch_days_and_checks_range() {
        let mut dictionary = Dictionary::default();
        for days in [i32::MIN, -1, 0, 1, i32::MAX] {
            // DECODE P: diagnostic Date fixture constructs the expected epoch-day answer.
            let value = cq_load::Date::from_days(days);
            assert_eq!(encode_value(value, &mut dictionary).unwrap(), i64::from(days));
            assert_eq!(encode_value(&value, &mut dictionary).unwrap(), i64::from(days));
            // DECODE P: diagnostic Date round trip exposes the reconstructed date.
            assert_eq!(decode_value::<cq_load::Date>(i64::from(days), &dictionary).unwrap(), value);
        }
        // DECODE P: diagnostic lower day overflow checks logical decode rejection.
        assert!(decode_value::<cq_load::Date>(i64::from(i32::MIN) - 1, &dictionary).is_err());
        // DECODE P: diagnostic upper day overflow checks logical decode rejection.
        assert!(decode_value::<cq_load::Date>(i64::from(i32::MAX) + 1, &dictionary).is_err());
    }

    #[test]
    fn scalar_row_codec_preserves_published_dictionary_ids() {
        use crate::RowCodec;
        type Row = (cq_load::Decimal<2>, cq_load::Date, String);

        let row: Row = (
            // DECODE P: diagnostic row fixture constructs its Decimal field.
            cq_load::Decimal::from_scaled(-12345).unwrap(),
            // DECODE P: diagnostic row fixture constructs its Date field.
            cq_load::Date::from_days(-1),
            String::from("published"),
        );
        let mut dictionary = Dictionary::default();
        dictionary.intern_owned("earlier").unwrap();
        let id = dictionary.intern_owned(&row.2).unwrap();
        let mut encoded = [0; 3];
        row.encode(&mut dictionary, &mut encoded).unwrap();
        assert_eq!(encoded, [-12345, -1, id]);
        // DECODE P: diagnostic row inverse exposes the reconstructed scalar-and-symbol answer.
        assert_eq!(Row::decode(&encoded, &dictionary).unwrap(), row);
        assert_eq!(dictionary.intern_owned("published").unwrap(), id);
        // DECODE P: diagnostic dictionary lookup exposes the preserved earlier symbol.
        assert_eq!(dictionary.text(0).unwrap(), "earlier");
    }

    #[test]
    fn integral_encoding_is_checked_and_reversible() {
        let mut dictionary = Dictionary::default();
        assert_eq!(encode_value(i64::MIN, &mut dictionary).unwrap(), i64::MIN);
        assert_eq!(
            // DECODE P: diagnostic integer round trip exposes the reconstructed maximum.
            decode_value::<u32>(u32::MAX as i64, &dictionary).unwrap(),
            u32::MAX
        );
        assert!(
            encode_value(u64::MAX, &mut dictionary)
                .unwrap_err()
                .to_string()
                .contains("CQ_GPU_INTEGER_RANGE")
        );
        // DECODE P: diagnostic integer overflow checks typed decode rejection.
        assert!(decode_value::<u8>(256, &dictionary).is_err());
        // DECODE P: diagnostic invalid boolean checks typed decode rejection.
        assert!(decode_value::<bool>(2, &dictionary).is_err());
    }

    #[test]
    fn dictionary_shares_ids_and_retains_borrowed_answers() {
        let text = String::from("shared");
        let answer = {
            let mut dictionary = Dictionary::default();
            let a = encode_value(&text, &mut dictionary).unwrap();
            let b = encode_value("shared", &mut dictionary).unwrap();
            assert_eq!(a, b);
            // DECODE P: diagnostic negative dictionary ID checks answer rejection.
            assert!(dictionary.decode(-1).is_err());
            // DECODE P: diagnostic borrowed answer is exposed for lifetime and equality assertions.
            decode_value::<&str>(a, &dictionary).unwrap()
        };
        assert_eq!(answer, text);
    }

    #[test]
    fn owned_dictionary_survives_rows_and_shares_dataset_ids() {
        let mut dictionary = Dictionary::default();
        let first = {
            let temporary = String::from("shared");
            dictionary.intern_owned(&temporary).unwrap()
        };
        let mut first_relation = dictionary.clone();
        assert_eq!(dictionary.intern_owned("shared").unwrap(), first);
        let second = dictionary.intern_owned("second").unwrap();
        first_relation.merge(&dictionary).unwrap();
        assert_eq!(
            // DECODE P: diagnostic first owned symbol exposes the merged-dictionary answer.
            decode_value::<String>(first, &first_relation).unwrap(),
            "shared"
        );
        assert_eq!(
            // DECODE P: diagnostic second owned symbol exposes the appended-dictionary answer.
            decode_value::<String>(second, &first_relation).unwrap(),
            "second"
        );
        // DECODE P: diagnostic owned-only dictionary checks borrowed answer rejection.
        assert!(decode_value::<&str>(first, &first_relation).is_err());
        let mut incompatible = Dictionary::default();
        incompatible.intern_owned("different").unwrap();
        assert!(first_relation.merge(&incompatible).is_err());
    }

    #[test]
    fn common_dictionary_preserves_ids_and_prefers_real_borrows() {
        let mut owned = Dictionary::default();
        assert_eq!(owned.intern_owned("shared").unwrap(), 0);
        let mut common = owned.clone();
        assert_eq!(common.intern("shared").unwrap(), 0);
        assert_eq!(common.intern("second").unwrap(), 1);
        // DECODE P: diagnostic common dictionary exposes a promoted borrowed answer.
        assert_eq!(common.decode(0).unwrap(), "shared");
        owned.check_extension(&common).unwrap();
        assert!(common.check_extension(&owned).is_err());

        let mut merged = owned.clone();
        merged.merge(&common).unwrap();
        // DECODE P: diagnostic merged first symbol exposes its retained borrowed answer.
        assert_eq!(merged.decode(0).unwrap(), "shared");
        // DECODE P: diagnostic merged second symbol exposes its extended borrowed answer.
        assert_eq!(merged.decode(1).unwrap(), "second");
        assert_eq!(merged.intern_owned("shared").unwrap(), 0);
        common.check_extension(&merged).unwrap();

        let mut changed = Dictionary::default();
        changed.intern("different").unwrap();
        assert!(owned.check_extension(&changed).is_err());
        assert!(owned.check_extension(&Dictionary::default()).is_err());
        let mut borrowed = Dictionary::default();
        borrowed.intern("shared").unwrap();
        assert!(
            borrowed
                .check_extension(&owned)
                .unwrap_err()
                .to_string()
                .contains("CQ_GPU_DICTIONARY_BORROW")
        );
    }
}
