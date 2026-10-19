use crate::{failure, Result};
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
#[derive(Clone, Default)]
pub struct Dictionary<'data> {
    ids: HashMap<&'data str, i64>,
    owned_ids: HashMap<Arc<str>, i64>,
    values: Vec<Text<'data>>,
}

impl<'data> Dictionary<'data> {
    pub fn intern(&mut self, value: &'data str) -> Result<i64> {
        if let Some(&id) = self.ids.get(value).or_else(|| self.owned_ids.get(value)) {
            return Ok(id);
        }
        let id = i64::try_from(self.values.len())
            .map_err(|_| failure("CQ_GPU_DICTIONARY_RANGE", "string domain exceeds i64"))?;
        self.values.push(Text::Borrowed(value));
        self.ids.insert(value, id);
        Ok(id)
    }

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
}

pub trait EncodeValue<'data> {
    fn encode(self, dictionary: &mut Dictionary<'data>) -> Result<i64>;
}

pub trait DecodeValue<'data>: Sized {
    fn decode(value: i64, dictionary: &Dictionary<'data>) -> Result<Self>;
}

pub fn encode_value<'data, T: EncodeValue<'data>>(
    value: T,
    dictionary: &mut Dictionary<'data>,
) -> Result<i64> {
    value.encode(dictionary)
}

pub fn decode_value<'data, T: DecodeValue<'data>>(
    value: i64,
    dictionary: &Dictionary<'data>,
) -> Result<T> {
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
            fn decode(value: i64, _: &Dictionary<'data>) -> Result<Self> {
                <$ty>::try_from(value).map_err(|_| failure("CQ_GPU_DECODE_RANGE",
                    concat!("encoded value is outside ", stringify!($ty))))
            }
        }
    )*};
}
integers!(i8, i16, i32, i64, i128, isize, u8, u16, u32, u64, u128, usize);

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
    fn decode(value: i64, dictionary: &Dictionary<'data>) -> Result<Self> {
        dictionary.decode(value)
    }
}
impl<'data> DecodeValue<'data> for String {
    fn decode(value: i64, dictionary: &Dictionary<'data>) -> Result<Self> {
        Ok(dictionary.text(value)?.to_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn integral_encoding_is_checked_and_reversible() {
        let mut dictionary = Dictionary::default();
        assert_eq!(encode_value(i64::MIN, &mut dictionary).unwrap(), i64::MIN);
        assert_eq!(
            decode_value::<u32>(u32::MAX as i64, &dictionary).unwrap(),
            u32::MAX
        );
        assert!(encode_value(u64::MAX, &mut dictionary)
            .unwrap_err()
            .to_string()
            .contains("CQ_GPU_INTEGER_RANGE"));
        assert!(decode_value::<u8>(256, &dictionary).is_err());
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
            assert!(dictionary.decode(-1).is_err());
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
            decode_value::<String>(first, &first_relation).unwrap(),
            "shared"
        );
        assert_eq!(
            decode_value::<String>(second, &first_relation).unwrap(),
            "second"
        );
        assert!(decode_value::<&str>(first, &first_relation).is_err());
        let mut incompatible = Dictionary::default();
        incompatible.intern_owned("different").unwrap();
        assert!(first_relation.merge(&incompatible).is_err());
    }
}
