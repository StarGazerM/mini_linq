//! Scalar schemas and packed field metadata for typed radix layouts.
use crate::Column;
use cuda_core::DeviceCopy;

/// A most-significant-first packed column. Signed values use sign-bit bias.
#[repr(C)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PackedField {
    pub bits: u32,
    pub shift: u32,
    pub signed: u32,
}
// SAFETY: all fields are plain integers with no host-side ownership.
unsafe impl DeviceCopy for PackedField {}

mod sealed {
    pub trait Scalar {}
    pub trait Tuple {}
}

/// Native scalar domains accepted by a device layout.
pub trait Scalar: sealed::Scalar + 'static {
    type Logical;
    /// Preserve the scalar's storage bits in the runtime's signed numeric lane.
    fn encode(value: &Self::Logical) -> i64;
    // DECODE ?: Scalar inverse lacks a caller-level opaque-read/publication requirement.
    fn decode(value: i128) -> Self::Logical;
    const COLUMN: Column;
    const MIN: i128 = if Self::COLUMN.is_signed() {
        -(1i128 << (Self::COLUMN.bits() - 1))
    } else {
        0
    };
    const MAX: i128 = (1i128 << Self::COLUMN.bits()) - 1 + Self::MIN;
}
/// Encode an identity-domain scalar without naming its Rust type in generated code.
pub fn encode_scalar<T: Scalar<Logical = T>>(value: &T) -> i64 {
    T::encode(value)
}

macro_rules! scalars {
    ($($ty:ty => $column:ident),* $(,)?) => {$(
        impl sealed::Scalar for $ty {}
        impl Scalar for $ty {
            type Logical = Self;
            const COLUMN: Column = Column::$column;
            fn encode(value: &Self) -> i64 { *value as i64 }
            // DECODE ?: numeric Scalar inverse has no local logical-consumer evidence.
            fn decode(value: i128) -> Self { value as Self }
        }
    )*};
}
scalars!(u8 => U8, i8 => I8, u16 => U16, i16 => I16,
    u32 => U32, i32 => I32, u64 => U64, i64 => I64);
#[cfg(target_pointer_width = "32")]
scalars!(usize => U32, isize => I32);
#[cfg(target_pointer_width = "64")]
scalars!(usize => U64, isize => I64);
impl sealed::Scalar for bool {}
impl Scalar for bool {
    type Logical = bool;
    fn encode(value: &bool) -> i64 {
        *value as i64
    }
    // DECODE ?: boolean Scalar inverse needs caller evidence rather than type spelling.
    fn decode(value: i128) -> bool {
        value != 0
    }
    const COLUMN: Column = Column::U8;
    const MAX: i128 = 1;
}
impl<T: 'static, C: crate::SymbolCodec<T> + 'static> sealed::Scalar for crate::Symbol<T, C> {}
impl<T: 'static, C: crate::SymbolCodec<T> + 'static> Scalar for crate::Symbol<T, C> {
    type Logical = T;
    fn encode(value: &T) -> i64 {
        C::code(value) as i64
    }
    // DECODE ?: Symbol Scalar delegates dictionary inversion without a caller publication fact.
    fn decode(value: i128) -> T {
        C::decode(value as u32)
    }
    const COLUMN: Column = Column::U32;
}

// These domains retain their authored Rust type and methods in device expressions.
// Encoding is an identity representation: coefficients and epoch days, respectively.
impl<const SCALE: u32> sealed::Scalar for cq_load::Decimal<SCALE> {}
impl<const SCALE: u32> Scalar for cq_load::Decimal<SCALE> {
    type Logical = Self;
    const COLUMN: Column = {
        assert!(SCALE <= 18, "Decimal scale must be at most 18");
        Column::I64
    };
    fn encode(value: &Self) -> i64 {
        value.scaled()
    }
    // DECODE ?: Decimal construction needs caller read evidence despite identity storage.
    fn decode(value: i128) -> Self {
        const {
            assert!(SCALE <= 18, "Decimal scale must be at most 18");
        }
        // SAFETY: Decimal is transparent over i64; every coefficient is valid
        // at the supported scale. Avoid the host constructor's String error path.
        unsafe { core::mem::transmute::<i64, Self>(value as i64) }
    }
}
impl sealed::Scalar for cq_load::Date {}
impl Scalar for cq_load::Date {
    type Logical = Self;
    const COLUMN: Column = Column::I32;
    fn encode(value: &Self) -> i64 {
        value.days_since_epoch() as i64
    }
    // DECODE ?: epoch-day Date construction carries no caller read or publication fact.
    fn decode(value: i128) -> Self {
        Self::from_days(value as i32)
    }
}

/// A tuple of scalar columns in recorded layout order, including `()`.
/// Each tuple supports up to sixteen columns; key and value schemas concatenate.
pub trait ScalarTuple: sealed::Tuple + 'static {
    const COLUMNS: &'static [Column];
}
impl sealed::Tuple for () {}
impl ScalarTuple for () {
    const COLUMNS: &'static [Column] = &[];
}
macro_rules! tuples {
    ($first:ident $(, $rest:ident)*) => {
        impl<$first: Scalar, $($rest: Scalar),*> sealed::Tuple for ($first, $($rest,)*) {}
        impl<$first: Scalar, $($rest: Scalar),*> ScalarTuple for ($first, $($rest,)*) {
            const COLUMNS: &'static [Column] = &[$first::COLUMN, $($rest::COLUMN),*];
        }
        tuples!($($rest),*);
    };
    () => {};
}
tuples!(A, B, C, D, E, F, G, H, I, J, K, L, M, N, O, P);


#[cfg(test)]
mod tests {
    use super::*;
    use crate::Packing;
    use std::mem::{align_of, size_of};

    #[test]
    fn scalar_encoding_preserves_identity_storage_bits() {
        assert_eq!(encode_scalar(&u64::MAX), -1);
        // DECODE P: diagnostic unsigned identity assertion exposes the reconstructed maximum.
        assert_eq!(<u64 as Scalar>::decode(-1), u64::MAX);
        assert_eq!(encode_scalar(&i64::MIN), i64::MIN);
        assert_eq!(encode_scalar(&true), 1);
        assert_eq!(encode_scalar(&false), 0);
    }

    #[test]
    fn decimal_and_date_layouts_preserve_authored_scalar_domains() {
        use cq_load::{Date, Decimal};

        type Amount = Decimal<2>;
        assert_eq!(size_of::<Amount>(), size_of::<i64>());
        assert_eq!(align_of::<Amount>(), align_of::<i64>());
        assert_eq!(size_of::<Date>(), size_of::<i32>());
        assert_eq!(align_of::<Date>(), align_of::<i32>());
        assert_eq!(
            <(Amount, Date) as ScalarTuple>::COLUMNS,
            &[Column::I64, Column::I32]
        );
        for coefficient in [i64::MIN, -12345, 0, 12345, i64::MAX] {
            // DECODE P: diagnostic Decimal fixture constructs the expected coefficient answer.
            let amount = Amount::from_scaled(coefficient).unwrap();
            assert_eq!(encode_scalar(&amount), coefficient);
            // DECODE P: diagnostic Decimal round trip exposes each reconstructed coefficient.
            let decoded = <Amount as Scalar>::decode(coefficient as i128);
            assert_eq!(decoded, amount);
            assert_eq!(decoded.scaled(), coefficient);
        }
        // DECODE P: diagnostic scale-18 fixture constructs the expected fractional answer.
        let fine = Decimal::<18>::from_scaled(-1).unwrap();
        // DECODE P: diagnostic fine-scale round trip exposes a reconstructed Decimal.
        assert_eq!(
            <Decimal<18> as Scalar>::decode(encode_scalar(&fine) as i128),
            fine
        );
        for days in [i32::MIN, -1, 0, 1, i32::MAX] {
            // DECODE P: diagnostic Date fixture constructs the expected epoch-day answer.
            let date = Date::from_days(days);
            assert_eq!(encode_scalar(&date), days as i64);
            // DECODE P: diagnostic Date round trip exposes each reconstructed date.
            assert_eq!(<Date as Scalar>::decode(days as i128), date);
        }
        // DECODE P: diagnostic Decimal ordering assertion exposes both inverse values.
        assert!(<Amount as Scalar>::decode(-1) < <Amount as Scalar>::decode(0));
        // DECODE P: diagnostic Date ordering assertion exposes both inverse dates.
        assert!(<Date as Scalar>::decode(-1) < <Date as Scalar>::decode(0));
    }

    #[test]
    fn typed_key_schema_and_field_abi() {
        use crate::RadixLayout;
        assert_eq!(size_of::<PackedField>(), 12);
        type Layout = RadixLayout<(i32, u64, u16), (), u128, u64, ()>;
        assert_eq!(Layout::derive_packing().unwrap(),
            Packing::with_columns(&[Column::I32, Column::U64, Column::U16]).unwrap());
        assert_eq!(RadixLayout::<(), (), u64, u64, ()>::derive_packing().unwrap().bits(), 0);
    }

    fn key_fields<W: crate::Word>(columns: &[Column], rows: &[Vec<i128>]) {
        use crate::radix_run::{device, RadixKeysView, RadixLayoutView, RadixRunView};
        let packing = Packing::with_columns(columns).unwrap();
        let fields = packing.packed_fields().collect::<Vec<_>>();
        let words = rows.iter().map(|row| packing.pack::<W>(row).unwrap()).collect::<Vec<_>>();
        let offsets = (0..=rows.len() as u64).collect::<Vec<_>>();
        let run = RadixRunView {
            keys: words.as_ptr(), offsets: offsets.as_ptr(), directory: std::ptr::null(),
            keys_len: rows.len() as u64, items: rows.len() as u64, radix_bits: 0, unit_offsets: 0,
        };
        let runs = [run];
        let view = RadixLayoutView {
            keys: RadixKeysView {
                runs: runs.as_ptr(), run_count: 1, reserved: 0,
                items: rows.len() as u64, snapshot: 1,
            },
            fields: fields.as_ptr(), field_count: fields.len() as u32, bits: packing.bits(),
            count: std::ptr::null(),
        };
        for (position, row) in rows.iter().enumerate() {
            for (column, &expected) in row.iter().enumerate() {
                assert_eq!(unsafe { device::decode_key(view, 0, position as u64, column as u32) }, expected as i64);
            }
            assert_eq!(unsafe { device::locate_key(view.keys, position as u64) }, (0, position as u64));
            let prefix = row.iter().map(|&value| value as i64).collect::<Vec<_>>();
            for length in 0..=row.len() {
                let (low, high, valid) = unsafe { device::encode_prefix(view, prefix.as_ptr(), length as u32) };
                assert!(valid);
                let first = rows.partition_point(|candidate| candidate[..length] < row[..length]);
                let last = rows.partition_point(|candidate| candidate[..length] <= row[..length]);
                assert_eq!(unsafe { device::interval(run, low, high) }, (first as u64, last as u64));
            }
        }
        assert_eq!(unsafe { device::locate_key(view.keys, rows.len() as u64) }, (1, 0));
    }

    #[test]
    fn radix_key_decode_and_encoded_prefix_preserve_full_domains() {
        key_fields::<u64>(&[Column::I64], &[vec![i64::MIN as i128], vec![i64::MAX as i128]]);
        key_fields::<u128>(&[Column::I8, Column::U64, Column::I32], &[
            vec![-128, u64::MAX as i128, i32::MIN as i128],
            vec![127, 0, i32::MAX as i128],
        ]);
        let rows = vec![
            vec![0, i64::MIN as i128, 0], vec![0, -1, u64::MAX as i128],
            vec![0, i64::MAX as i128, 1], vec![255, i64::MAX as i128, u64::MAX as i128],
        ];
        key_fields::<[u64; 3]>(&[Column::U8, Column::I64, Column::U64], &rows);
        let rows = rows.into_iter().map(|mut row| {
            row.push(u64::MAX as i128 - row[0]); row
        }).collect::<Vec<_>>();
        key_fields::<[u64; 4]>(&[Column::U8, Column::I64, Column::U64, Column::U64], &rows);
    }
}
