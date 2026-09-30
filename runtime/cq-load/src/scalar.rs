use crate::Load;
use std::fmt;
use std::str::FromStr;

macro_rules! primitives {
    ($($ty:ty => $array:ident),* $(,)?) => {$ (
        impl Load for $ty {
            fn load(field: &str) -> Result<Self, String> {
                field.parse().map_err(|error: <$ty as FromStr>::Err| error.to_string())
            }

            #[cfg(feature = "parquet")]
            fn load_arrow(column: &dyn arrow_array::Array) -> Result<Vec<Self>, crate::ColumnError> {
                if let Some(array) = column.as_any().downcast_ref::<arrow_array::$array>() {
                    crate::parquet_source::native_values(array.iter())
                } else {
                    crate::parquet_source::parse_arrow(column, Self::load)
                }
            }
        }
    )* };
}

primitives! {
    bool => BooleanArray,
    i8 => Int8Array, i16 => Int16Array, i32 => Int32Array, i64 => Int64Array,
    u8 => UInt8Array, u16 => UInt16Array, u32 => UInt32Array, u64 => UInt64Array,
    f64 => Float64Array,
}

macro_rules! owned_strings {
    ($($ty:ty),* $(,)?) => {$ (
        impl Load for $ty {
            fn load(field: &str) -> Result<Self, String> {
                Ok(field.into())
            }

            #[cfg(feature = "parquet")]
            fn load_arrow(column: &dyn arrow_array::Array) -> Result<Vec<Self>, crate::ColumnError> {
                crate::parquet_source::string_values(column, Self::load)
            }
        }
    )* };
}

owned_strings!(String, Box<str>, std::sync::Arc<str>, std::rc::Rc<str>);

/// Exact fixed-point value with an `i64` coefficient and compile-time scale.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Decimal<const SCALE: u32 = 2>(i64);

impl<const SCALE: u32> Decimal<SCALE> {
    pub fn from_scaled(value: i64) -> Result<Self, String> {
        if SCALE > 18 {
            return Err("Decimal scale must be at most 18".into());
        }
        Ok(Self(value))
    }

    pub const fn scaled(self) -> i64 {
        self.0
    }

    #[cfg(feature = "parquet")]
    pub(crate) fn rescale(value: i128, scale: i8) -> Result<Self, String> {
        if SCALE > 18 {
            return Err("Decimal scale must be at most 18".into());
        }
        let difference = i64::from(SCALE) - i64::from(scale);
        let scaled = if difference >= 0 {
            let power = 10_i128
                .checked_pow(difference as u32)
                .ok_or("decimal scale overflow")?;
            value.checked_mul(power).ok_or("decimal overflow")?
        } else {
            let divisor = 10_i128
                .checked_pow((-difference) as u32)
                .ok_or("decimal scale overflow")?;
            if value % divisor != 0 {
                return Err(format!("decimal loses precision at scale {SCALE}"));
            }
            value / divisor
        };
        Self::from_scaled(i64::try_from(scaled).map_err(|_| "decimal exceeds i64 range")?)
    }
}

impl<const SCALE: u32> Load for Decimal<SCALE> {
    fn load(field: &str) -> Result<Self, String> {
        if SCALE > 18 {
            return Err("Decimal scale must be at most 18".into());
        }
        let (negative, unsigned) = if let Some(rest) = field.strip_prefix('-') {
            (true, rest)
        } else {
            (false, field.strip_prefix('+').unwrap_or(field))
        };
        let (whole, fraction) = unsigned.split_once('.').unwrap_or((unsigned, ""));
        if whole.is_empty()
            || !whole.bytes().all(|byte| byte.is_ascii_digit())
            || !fraction.bytes().all(|byte| byte.is_ascii_digit())
        {
            return Err("expected a decimal number".into());
        }
        let width = SCALE as usize;
        if fraction.len() > width
            && !fraction.as_bytes()[width..]
                .iter()
                .all(|byte| *byte == b'0')
        {
            return Err(format!("decimal loses precision at scale {SCALE}"));
        }
        let mut coefficient = 0_i128;
        for digit in whole
            .bytes()
            .chain(fraction.bytes().take(width))
            .chain(std::iter::repeat_n(
                b'0',
                width.saturating_sub(fraction.len()),
            ))
        {
            coefficient = coefficient
                .checked_mul(10)
                .and_then(|value| value.checked_add(i128::from(digit - b'0')))
                .ok_or("decimal overflow")?;
        }
        if negative {
            coefficient = -coefficient;
        }
        Self::from_scaled(i64::try_from(coefficient).map_err(|_| "decimal exceeds i64 range")?)
    }

    #[cfg(feature = "parquet")]
    fn load_arrow(column: &dyn arrow_array::Array) -> Result<Vec<Self>, crate::ColumnError> {
        use arrow_schema::DataType;
        if let (Some(array), DataType::Decimal128(_, scale)) = (
            column
                .as_any()
                .downcast_ref::<arrow_array::Decimal128Array>(),
            column.data_type(),
        ) {
            array
                .iter()
                .enumerate()
                .map(|(row, value)| {
                    let value = value.ok_or_else(|| crate::ColumnError::null(row))?;
                    Self::rescale(value, *scale).map_err(|message| crate::ColumnError {
                        row,
                        value: crate::parquet_source::decimal_text(value, *scale),
                        message,
                    })
                })
                .collect()
        } else {
            crate::parquet_source::parse_arrow(column, Self::load)
        }
    }
}

impl<const SCALE: u32> FromStr for Decimal<SCALE> {
    type Err = String;
    fn from_str(field: &str) -> Result<Self, Self::Err> {
        Self::load(field)
    }
}

impl<const SCALE: u32> fmt::Display for Decimal<SCALE> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if SCALE > 18 {
            return Err(fmt::Error);
        }
        if self.0 < 0 {
            write!(f, "-")?;
        }
        let magnitude = self.0.unsigned_abs();
        let divisor = 10_u64.pow(SCALE);
        write!(f, "{}", magnitude / divisor)?;
        if SCALE > 0 {
            write!(
                f,
                ".{:0width$}",
                magnitude % divisor,
                width = SCALE as usize
            )?;
        }
        Ok(())
    }
}

/// Gregorian date represented by signed days since 1970-01-01.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Date(i32);

impl Date {
    pub const fn from_days(days_since_epoch: i32) -> Self {
        Self(days_since_epoch)
    }

    pub const fn days_since_epoch(self) -> i32 {
        self.0
    }
}

impl Load for Date {
    fn load(field: &str) -> Result<Self, String> {
        let bytes = field.as_bytes();
        if bytes.len() != 10
            || bytes[4] != b'-'
            || bytes[7] != b'-'
            || !bytes
                .iter()
                .enumerate()
                .all(|(i, byte)| i == 4 || i == 7 || byte.is_ascii_digit())
        {
            return Err("expected ISO date YYYY-MM-DD".into());
        }
        let year: i32 = field[..4].parse().map_err(|_| "invalid year")?;
        let month: i32 = field[5..7].parse().map_err(|_| "invalid month")?;
        let day: i32 = field[8..].parse().map_err(|_| "invalid day")?;
        let leap = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
        let max_day = match month {
            1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
            4 | 6 | 9 | 11 => 30,
            2 if leap => 29,
            2 => 28,
            _ => return Err("date month outside 1..12".into()),
        };
        if !(1..=max_day).contains(&day) {
            return Err("date day outside month's range".into());
        }
        // March-based Gregorian eras make leap-day handling uniform.
        let adjusted_year = year - i32::from(month <= 2);
        let era = adjusted_year.div_euclid(400);
        let year_of_era = adjusted_year - era * 400;
        let march_month = month + if month > 2 { -3 } else { 9 };
        let day_of_year = (153 * march_month + 2) / 5 + day - 1;
        let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
        Ok(Self(era * 146097 + day_of_era - 719468))
    }

    #[cfg(feature = "parquet")]
    fn load_arrow(column: &dyn arrow_array::Array) -> Result<Vec<Self>, crate::ColumnError> {
        if let Some(array) = column.as_any().downcast_ref::<arrow_array::Date32Array>() {
            crate::parquet_source::native_values(array.iter().map(|value| value.map(Self)))
        } else {
            crate::parquet_source::parse_arrow(column, Self::load)
        }
    }
}

impl FromStr for Date {
    type Err = String;
    fn from_str(field: &str) -> Result<Self, Self::Err> {
        Self::load(field)
    }
}

impl fmt::Display for Date {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let shifted = i64::from(self.0) + 719468;
        let era = shifted.div_euclid(146097);
        let day_of_era = shifted - era * 146097;
        let year_of_era =
            (day_of_era - day_of_era / 1460 + day_of_era / 36524 - day_of_era / 146096) / 365;
        let mut year = year_of_era + era * 400;
        let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
        let march_month = (5 * day_of_year + 2) / 153;
        let day = day_of_year - (153 * march_month + 2) / 5 + 1;
        let month = march_month + if march_month < 10 { 3 } else { -9 };
        year += i64::from(month <= 2);
        write!(f, "{year:04}-{month:02}-{day:02}")
    }
}
