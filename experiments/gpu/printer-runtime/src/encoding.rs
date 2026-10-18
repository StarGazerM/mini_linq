use crate::{Result, failure};
use std::collections::HashMap;

/// A query-local shared string domain. IDs preserve equality, not lexical order.
/// Borrowing input strings allows decoded `&str` answers to outlive the query.
#[derive(Default)]
pub struct Dictionary<'data> {
    ids: HashMap<&'data str, i64>,
    values: Vec<&'data str>,
}

impl<'data> Dictionary<'data> {
    pub fn intern(&mut self, value: &'data str) -> Result<i64> {
        if let Some(&id) = self.ids.get(value) {
            return Ok(id);
        }
        let id = i64::try_from(self.values.len())
            .map_err(|_| failure("CQ_GPU_DICTIONARY_RANGE", "string domain exceeds i64"))?;
        self.values.push(value);
        self.ids.insert(value, id);
        Ok(id)
    }

    pub fn decode(&self, value: i64) -> Result<&'data str> {
        usize::try_from(value)
            .ok()
            .and_then(|id| self.values.get(id).copied())
            .ok_or_else(|| failure("CQ_GPU_DICTIONARY_ID", format!("unknown string ID {value}")))
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
integers!(
    i8, i16, i32, i64, i128, isize, u8, u16, u32, u64, u128, usize
);

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
        Ok(dictionary.decode(value)?.to_owned())
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
        assert!(
            encode_value(u64::MAX, &mut dictionary)
                .unwrap_err()
                .to_string()
                .contains("CQ_GPU_INTEGER_RANGE")
        );
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
}
