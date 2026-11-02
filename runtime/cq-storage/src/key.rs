//! One encoding law for dense membership, rank directories, and radix runs.
//!
//! [`OrderKey`] covers the entire logical domain, including absent query keys.
//! Codecs are sealed representation policies, not independent encoding laws.

/// Unsigned arithmetic in an encoding's native word width.
///
/// Ordering, subtraction, and conversion must be exact integer operations.
/// Bitmap construction checks the complete span before converting to `usize`.
pub trait OrdinalWord:
    Copy + Ord + Send + Sync + std::ops::Sub<Output = Self> + TryInto<usize> + Into<u128>
{
}

impl OrdinalWord for u64 {}
impl OrdinalWord for u128 {}

/// Formal rule IX-OrderKey: one total order embedding for every index operation.
/// A total, injective, order-preserving encoding of the entire key domain.
///
/// For every pair of keys, including keys absent from a provider,
/// `a.cmp(b) == a.ordinal().cmp(&b.ordinal())`. Thus ordinal equality is exactly
/// key equality. The same encoding serves membership, interval/rank reads, and
/// radix directories; providers may require a particular native ordinal width.
/// This is a forward encoding only, with no ordinal-to-key reconstruction.
pub trait OrderKey: Ord {
    type Ordinal: OrdinalWord;
    fn ordinal(&self) -> Self::Ordinal;
}

impl OrderKey for i64 {
    type Ordinal = u64;

    #[inline]
    fn ordinal(&self) -> u64 {
        *self as u64 ^ (1 << 63)
    }
}

impl OrderKey for i32 {
    type Ordinal = u64;

    #[inline]
    fn ordinal(&self) -> u64 {
        u64::from(*self as u32 ^ (1 << 31))
    }
}

impl OrderKey for u32 {
    type Ordinal = u64;

    #[inline]
    fn ordinal(&self) -> u64 {
        u64::from(*self)
    }
}

impl OrderKey for u64 {
    type Ordinal = u64;

    #[inline]
    fn ordinal(&self) -> u64 {
        *self
    }
}

impl OrderKey for u128 {
    type Ordinal = u128;

    #[inline]
    fn ordinal(&self) -> u128 {
        *self
    }
}

/// Signed integers: flip the sign bit of the 64-bit two's-complement value.
macro_rules! signed_order_key {
    ($($t:ty),*) => {$(
        impl OrderKey for $t {
            type Ordinal = u64;

            #[inline]
            fn ordinal(&self) -> u64 {
                *self as i64 as u64 ^ (1 << 63)
            }
        }
    )*};
}
signed_order_key!(i8, i16, isize);

/// Unsigned integers, `char` (code point order) and `bool` (`false < true`)
/// widen losslessly to `u64`.
macro_rules! unsigned_order_key {
    ($($t:ty),*) => {$(
        impl OrderKey for $t {
            type Ordinal = u64;

            #[inline]
            fn ordinal(&self) -> u64 {
                *self as u64
            }
        }
    )*};
}
unsigned_order_key!(u8, u16, usize, char, bool);

impl<T: OrderKey + ?Sized> OrderKey for &T {
    type Ordinal = T::Ordinal;

    #[inline]
    fn ordinal(&self) -> Self::Ordinal {
        T::ordinal(*self)
    }
}

impl<T: OrderKey> OrderKey for (T,) {
    type Ordinal = T::Ordinal;

    #[inline]
    fn ordinal(&self) -> Self::Ordinal {
        self.0.ordinal()
    }
}

mod sealed {
    pub trait Policy<K> {}
}

/// A sealed representation policy for optional use of [`OrderKey`].
///
/// [`NoDense`] disables ordinal storage for any key. The enabled policies
/// delegate to the key's one canonical encoding, widened losslessly to `u128`.
/// Implement [`OrderKey`] on a custom key rather than defining another codec.
/// Policies are either total or disabled; partial encodings are not supported.
pub trait KeyCodec<K>: Copy + sealed::Policy<K> {
    fn ordinal(self, key: &K) -> Option<u128>;
    fn total(self) -> bool;
}

#[derive(Clone, Copy, Default)]
pub struct NoDense;
impl<K> sealed::Policy<K> for NoDense {}
impl<K> KeyCodec<K> for NoDense {
    #[inline]
    fn ordinal(self, _: &K) -> Option<u128> {
        None
    }

    fn total(self) -> bool {
        false
    }
}

/// Enable the canonical [`OrderKey`] encoding for scalar or custom keys.
#[derive(Clone, Copy, Default)]
pub struct I64Codec;
impl<K: OrderKey> sealed::Policy<K> for I64Codec {}
impl<K: OrderKey> KeyCodec<K> for I64Codec {
    #[inline]
    fn ordinal(self, key: &K) -> Option<u128> {
        Some(key.ordinal().into())
    }

    fn total(self) -> bool {
        true
    }
}

/// Enable the same canonical encoding for borrowed singleton tuple keys.
#[derive(Clone, Copy, Default)]
pub struct BorrowedI64TupleCodec;
impl<K: OrderKey + ?Sized> sealed::Policy<(&K,)> for BorrowedI64TupleCodec {}
impl<K: OrderKey + ?Sized> KeyCodec<(&K,)> for BorrowedI64TupleCodec {
    #[inline]
    fn ordinal(self, key: &(&K,)) -> Option<u128> {
        Some(key.ordinal().into())
    }

    fn total(self) -> bool {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn check<K: OrderKey + std::fmt::Debug>(sample: &[K]) {
        for a in sample {
            for b in sample {
                assert_eq!(a.cmp(b), a.ordinal().cmp(&b.ordinal()), "{a:?}, {b:?}");
            }
            let ordinal: u128 = a.ordinal().into();
            assert_eq!(I64Codec.ordinal(a), Some(ordinal));
            assert_eq!(BorrowedI64TupleCodec.ordinal(&(a,)), Some(ordinal));
            assert_eq!(OrderKey::ordinal(&a).into(), ordinal);
            assert_eq!(OrderKey::ordinal(&(a,)).into(), ordinal);
            assert_eq!(NoDense.ordinal(a), None);
        }
    }

    #[test]
    fn integer_boundaries_and_absent_keys_obey_one_order_law() {
        check(&[i64::MIN, i64::MIN + 1, -1, 0, 1, i64::MAX - 1, i64::MAX]);
        check(&[i32::MIN, i32::MIN + 1, -1, 0, 1, i32::MAX - 1, i32::MAX]);
        check(&[0u32, 1, 63, 64, u32::MAX - 1, u32::MAX]);
        check(&[0u64, 1, 63, 64, u64::MAX - 1, u64::MAX]);
        check(&[0u128, 1, u64::MAX as u128, 1 << 64, 1 << 80, u128::MAX]);
        check(&[i8::MIN, -1, 0, 1, i8::MAX]);
        check(&[i16::MIN, -1, 0, 1, i16::MAX]);
        check(&[isize::MIN, -1, 0, 1, isize::MAX]);
        check(&[0u8, 1, u8::MAX]);
        check(&[0u16, 1, u16::MAX]);
        check(&[0usize, 1, usize::MAX]);
        check(&['\0', 'a', 'z', '\u{10FFFF}']);
        check(&[false, true]);
        assert_eq!(i32::MIN.ordinal(), 0);
        assert_eq!(i32::MAX.ordinal(), u64::from(u32::MAX));
        assert_eq!(i64::MIN.ordinal(), 0);
        assert_eq!(i64::MAX.ordinal(), u64::MAX);
    }

    #[test]
    fn deterministic_full_width_samples_preserve_order() {
        let mut state = 0x243f_6a88_85a3_08d3u64;
        let mut unsigned = Vec::new();
        for _ in 0..128 {
            state = state.wrapping_mul(6364136223846793005).wrapping_add(1);
            unsigned.push(state);
        }
        check(&unsigned);
        check(
            &unsigned
                .iter()
                .map(|&value| value as i64)
                .collect::<Vec<_>>(),
        );
        check(
            &unsigned
                .iter()
                .map(|&value| value as i32)
                .collect::<Vec<_>>(),
        );
        check(
            &unsigned
                .iter()
                .map(|&value| value as u32)
                .collect::<Vec<_>>(),
        );
    }
}
