use crate::Result;
use cuda_core::{CudaContext, DeviceCopy, LaunchConfig1D};
use gpu_device_columns::{Buffer, Pool};
use std::{cell::RefCell, fmt::Debug, ops::RangeInclusive, rc::Rc, sync::Arc};

mod sealed {
    pub trait Sealed {}
    impl Sealed for u64 {}
    impl Sealed for u128 {}
    impl<const L: usize> Sealed for [u64; L] {}
}

/// An unsigned packed key. Array words store their most significant lane first.
/// Lane access numbers lanes from the least significant end for all word types.
pub trait Word: sealed::Sealed + Copy + Ord + Debug + DeviceCopy {
    const BITS: u32;
    const MS_FIRST: bool;
    fn zero() -> Self;
    fn lane(&self, lane: u32) -> u64;
    fn set_lane(&mut self, lane: u32, value: u64);

    /// Lane-wise ordering avoids array intrinsics unavailable in device code.
    #[inline(always)]
    fn compare(&self, other: &Self) -> std::cmp::Ordering {
        let mut lane = Self::BITS / 64;
        while lane != 0 {
            lane -= 1;
            let left = self.lane(lane);
            let right = other.lane(lane);
            if left < right { return std::cmp::Ordering::Less; }
            if left > right { return std::cmp::Ordering::Greater; }
        }
        std::cmp::Ordering::Equal
    }

    /// Zero-extend (or truncate for u64) an unsigned native key.
    fn from_u128(value: u128) -> Self {
        let mut word = Self::zero();
        word.set_lane(0, value as u64);
        if Self::BITS > 64 { word.set_lane(1, (value >> 64) as u64); }
        word
    }
    /// Return the low 128 bits, truncating wider keys.
    fn to_u128(self) -> u128 {
        self.lane(0) as u128
            | if Self::BITS > 64 { (self.lane(1) as u128) << 64 } else { 0 }
    }
    #[inline(always)]
    fn extract(&self, shift: u32, bits: u32) -> u64 {
        let lane = shift / 64;
        let offset = shift % 64;
        let mut value = self.lane(lane) >> offset;
        if offset != 0 && bits > 64 - offset {
            value |= self.lane(lane + 1) << (64 - offset);
        }
        value & (u64::MAX >> (64 - bits))
    }
    #[inline(always)]
    fn insert(&mut self, shift: u32, bits: u32, value: u64) {
        let lane = shift / 64;
        let offset = shift % 64;
        self.set_lane(lane, self.lane(lane) | (value << offset));
        if offset != 0 && bits > 64 - offset {
            self.set_lane(lane + 1, self.lane(lane + 1) | (value >> (64 - offset)));
        }
    }
}

impl Word for u64 {
    const BITS: u32 = 64;
    const MS_FIRST: bool = false;
    fn zero() -> Self { 0 }
    #[inline(always)]
    fn lane(&self, _lane: u32) -> u64 { *self }
    #[inline(always)]
    fn set_lane(&mut self, _lane: u32, value: u64) { *self = value; }
}

impl Word for u128 {
    const BITS: u32 = 128;
    const MS_FIRST: bool = false;
    fn zero() -> Self { 0 }
    #[inline(always)]
    fn lane(&self, lane: u32) -> u64 {
        if lane == 0 { *self as u64 } else { (*self >> 64) as u64 }
    }
    #[inline(always)]
    fn set_lane(&mut self, lane: u32, value: u64) {
        let shift = lane * 64;
        *self = (*self & !((u64::MAX as u128) << shift)) | ((value as u128) << shift);
    }
}

impl<const L: usize> Word for [u64; L] {
    const BITS: u32 = {
        assert!(L > 0 && L <= u32::MAX as usize / 64);
        L as u32 * 64
    };
    const MS_FIRST: bool = true;
    fn zero() -> Self { [0; L] }
    #[inline(always)]
    fn lane(&self, lane: u32) -> u64 { self[L - 1 - lane as usize] }
    #[inline(always)]
    fn set_lane(&mut self, lane: u32, value: u64) { self[L - 1 - lane as usize] = value; }
}

/// A tuple column's logical integer domain, not its packed storage type.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Column {
    U8,
    I8,
    U16,
    I16,
    U32,
    I32,
    U64,
    I64,
}

impl Column {
    pub const fn bits(self) -> u32 {
        match self {
            Self::U8 | Self::I8 => 8,
            Self::U16 | Self::I16 => 16,
            Self::U32 | Self::I32 => 32,
            Self::U64 | Self::I64 => 64,
        }
    }

    pub const fn is_signed(self) -> bool {
        matches!(self, Self::I8 | Self::I16 | Self::I32 | Self::I64)
    }

    /// Flip the sign bit for signed columns, preserving numeric order.
    pub fn encode(self, value: i128) -> Result<u64> {
        Ok(Field { bits: self.bits(), signed: self.is_signed(), shift: 0 }.encode(value)? as u64)
    }

    pub fn decode(self, encoded: u64) -> Result<i128> {
        if encoded as u128 > mask(self.bits()) {
            return Err("encoded value exceeds column width".into());
        }
        Ok(Field { bits: self.bits(), signed: self.is_signed(), shift: 0 }.decode(encoded as u128))
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct Field {
    bits: u32,
    signed: bool,
    shift: u32,
}

impl Field {
    fn encode(self, value: i128) -> Result<u128> {
        let sign = if self.signed { 1_i128 << (self.bits - 1) } else { 0 };
        let minimum = -sign;
        let maximum = mask(self.bits) as i128 - sign;
        if value < minimum || value > maximum {
            return Err("tuple value exceeds column domain".into());
        }
        // In the declared two's-complement width this is exactly sign-bit XOR.
        Ok(((value as u128) & mask(self.bits)) ^ sign as u128)
    }

    fn decode(self, encoded: u128) -> i128 {
        if self.signed {
            encoded as i128 - (1_i128 << (self.bits - 1))
        } else {
            encoded as i128
        }
    }
}

fn mask(bits: u32) -> u128 {
    if bits == 128 { u128::MAX } else { (1_u128 << bits) - 1 }
}

/// Formal rule DL-DevicePacking:
/// Concatenating bounded column encodings most-significant-column first is
/// injective and preserves tuple lexicographic order. Signed columns flip
/// their sign bit; a fixed leading prefix occupies one inclusive key interval.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Packing {
    fields: Vec<Field>,
    bits: u32,
}

impl Packing {
    /// Construct unsigned columns with widths in `1..=64`.
    /// An empty schema represents the single empty tuple with key zero.
    pub fn new(widths: &[u32]) -> Result<Self> {
        Self::build(widths.iter().map(|&bits| (bits, false)))
    }

    /// Construct native-width columns, retaining their signed numeric domains.
    pub fn with_columns(columns: &[Column]) -> Result<Self> {
        Self::build(columns.iter().map(|column| (column.bits(), column.is_signed())))
    }

    pub(crate) fn build(columns: impl Iterator<Item = (u32, bool)>) -> Result<Self> {
        let mut fields = Vec::with_capacity(columns.size_hint().0);
        let mut bits = 0_u32;
        for (width, signed) in columns {
            if !(1..=64).contains(&width) {
                return Err("packing requires 1..=64-bit columns".into());
            }
            bits = bits.checked_add(width).filter(|&bits| bits <= u32::MAX - 63)
                .ok_or("packing bit width exceeds metadata domain")?;
            fields.push(Field { bits: width, signed, shift: bits });
        }
        for field in &mut fields {
            field.shift = bits - field.shift;
        }
        Ok(Self { fields, bits })
    }

    pub fn bits(&self) -> u32 {
        self.bits
    }

    /// Minimal storage width, rounded up to at least one complete u64 lane.
    pub fn word_bits(&self) -> u32 {
        self.bits.div_ceil(64).max(1) * 64
    }

    pub fn widths(&self) -> impl ExactSizeIterator<Item = u32> + '_ {
        self.fields.iter().map(|field| field.bits)
    }

    /// Device field descriptors in this packing's most-significant-first order.
    pub fn packed_fields(&self) -> impl ExactSizeIterator<Item = crate::layout::PackedField> + '_ {
        self.fields.iter().map(|field| crate::layout::PackedField {
            bits: field.bits, shift: field.shift, signed: field.signed as u32,
        })
    }

    pub fn len(&self) -> usize {
        self.fields.len()
    }

    pub fn is_empty(&self) -> bool {
        self.fields.is_empty()
    }

    fn check_word<W: Word>(&self) -> Result<()> {
        if self.bits > W::BITS {
            return Err("packing exceeds the selected word width".into());
        }
        Ok(())
    }

    /// Pack mathematical integer values; `i128` includes the full `u64` domain.
    pub fn pack<W: Word>(&self, values: &[i128]) -> Result<W> {
        self.check_word::<W>()?;
        if values.len() != self.len() {
            return Err("tuple arity differs from packing".into());
        }
        let mut packed = W::zero();
        for (&field, &value) in self.fields.iter().zip(values) {
            packed.insert(field.shift, field.bits, field.encode(value)? as u64);
        }
        Ok(packed)
    }

    pub fn unpack<W: Word>(&self, packed: W) -> Result<Vec<i128>> {
        self.check_word::<W>()?;
        for lane in self.bits / 64..W::BITS / 64 {
            let used = self.bits.saturating_sub(lane * 64);
            if packed.lane(lane) & !(mask(used) as u64) != 0 {
                return Err("packed key contains bits outside its schema".into());
            }
        }
        Ok(self.fields.iter().map(|field| {
            field.decode(packed.extract(field.shift, field.bits) as u128)
        }).collect())
    }

    /// Inclusive bounds avoid an unrepresentable successor of the maximum key.
    /// An empty prefix covers every key in this schema, not unused high bits.
    pub fn prefix_range<W: Word>(&self, prefix: &[i128]) -> Result<RangeInclusive<W>> {
        self.check_word::<W>()?;
        if prefix.len() > self.len() {
            return Err("prefix is longer than the tuple schema".into());
        }
        let mut lower = W::zero();
        let mut remaining = self.bits;
        for (&field, &value) in self.fields.iter().zip(prefix) {
            lower.insert(field.shift, field.bits, field.encode(value)? as u64);
            remaining = field.shift;
        }
        let mut upper = lower;
        for lane in 0..remaining.div_ceil(64) {
            let bits = (remaining - lane * 64).min(64);
            upper.set_lane(lane, upper.lane(lane) | (u64::MAX >> (64 - bits)));
        }
        Ok(lower..=upper)
    }

    fn check_order(&self, order: &[usize]) -> Result<()> {
        if order.len() != self.len() {
            return Err("permutation arity differs from packing".into());
        }
        let mut seen = vec![false; self.len()];
        for &index in order {
            if index >= self.len() || seen[index] {
                return Err("column order is not a permutation".into());
            }
            seen[index] = true;
        }
        Ok(())
    }

    /// `order[destination_column]` selects its original source column.
    pub fn permute(&self, order: &[usize]) -> Result<Self> {
        self.check_order(order)?;
        Self::build(order.iter().map(|&index| {
            let field = self.fields[index];
            (field.bits, field.signed)
        }))
    }

    /// Permute column bit fields on the input buffer's stream without a host
    /// round trip. Rows retain their original order; the result is not sorted.
    /// The initialized input prefix must contain keys packed with this schema.
    /// Every storage lane is retained, including fields crossing lane boundaries;
    /// field widths and shifts use the same u32 metadata as host packing.
    pub fn permute_device<W: Word>(
        &self,
        input: &Buffer<W>,
        rows: usize,
        order: &[usize],
    ) -> Result<Buffer<W>> {
        self.check_word::<W>()?;
        self.check_order(order)?;
        if rows > input.len() {
            return Err("permutation rows exceed input capacity".into());
        }
        let pool = input.pool();
        let output = unsafe { Buffer::uninitialized(pool.clone(), rows)? };
        if rows == 0 {
            return Ok(output);
        }
        let mut descriptors = vec![0_u32; order.len() * 3];
        let mut destination_shift = self.bits;
        for (destination, &source) in order.iter().enumerate() {
            let field = self.fields[source];
            destination_shift -= field.bits;
            descriptors[destination * 3] = field.shift;
            descriptors[destination * 3 + 1] = field.bits;
            descriptors[destination * 3 + 2] = destination_shift;
        }
        let descriptors = Buffer::from_host_async(pool.clone(), &descriptors[..order.len() * 3])?;
        let module = module(pool)?;
        let initial = module.prepare_permute::<W>(LaunchConfig1D::new(1, 256, 0))?;
        let grid = cq_gpu_runtime::launch_grid_for_kernel(initial.function(), rows.div_ceil(256), 256, 0)?;
        let launch = LaunchConfig1D::new(grid, 256, 0);
        module.permute::<W>(
            pool.stream(),
            &module.prepare_permute::<W>(launch)?,
            input.ptr().cast(),
            output.ptr().cast(),
            rows as u64,
            descriptors.ptr(),
            order.len() as u32,
        )?;
        Ok(output)
    }
}

thread_local! {
    static CACHED_MODULE: RefCell<Option<(Arc<CudaContext>, Rc<packing_kernels::LoadedModule>)>> =
        const { RefCell::new(None) };
}

fn module(pool: &Pool) -> Result<Rc<packing_kernels::LoadedModule>> {
    CACHED_MODULE.with(|cache| {
        if let Some((context, module)) = cache.borrow().as_ref() {
            if Arc::ptr_eq(context, pool.context()) {
                return Ok(module.clone());
            }
        }
        let module = Rc::new(unsafe { packing_kernels::load(pool.context())? });
        *cache.borrow_mut() = Some((pool.context().clone(), module.clone()));
        Ok(module)
    })
}

#[cuda_host::cuda_module]
mod packing_kernels {
    use cuda_device::{kernel, launch_bounds, launch_contract, thread};

    #[kernel]
    #[launch_bounds(256)]
    #[launch_contract(domain = 1, block = (256, 1, 1))]
    pub fn permute<W: crate::Word>(
        input: *const W,
        output: *mut W,
        rows: u64,
        descriptors: *const u32,
        columns: u32,
    ) {
        let mut row = thread::index_1d().get() as u64;
        let stride = thread::gridDim_x() as u64 * 256;
        while row < rows {
            let source = unsafe { *input.add(row as usize) };
            let mut packed = W::zero();
            let mut column = 0;
            while column < columns as usize {
                let source_shift = unsafe { *descriptors.add(column * 3) };
                let width = unsafe { *descriptors.add(column * 3 + 1) };
                let destination_shift = unsafe { *descriptors.add(column * 3 + 2) };
                let field = source.extract(source_shift, width);
                packed.insert(destination_shift, width, field);
                column += 1;
            }
            unsafe { *output.add(row as usize) = packed; }
            row += stride;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn random(state: &mut u64) -> u64 {
        *state ^= *state << 13;
        *state ^= *state >> 7;
        *state ^= *state << 17;
        *state
    }

    fn tuple(packing: &Packing, state: &mut u64) -> Vec<i128> {
        packing.fields.iter().map(|field| {
            field.decode(random(state) as u128 & mask(field.bits))
        }).collect()
    }

    fn properties<W: Word>(packing: &Packing, state: &mut u64) {
        for iteration in 0..2048 {
            let left = tuple(packing, state);
            let mut right = tuple(packing, state);
            let prefix_len = iteration % (packing.len() + 1);
            if iteration % 2 == 0 {
                right[..prefix_len].copy_from_slice(&left[..prefix_len]);
            }
            let a = packing.pack::<W>(&left).unwrap();
            let b = packing.pack::<W>(&right).unwrap();
            assert_eq!(packing.unpack(a).unwrap(), left);
            assert_eq!(packing.unpack(b).unwrap(), right);
            assert_eq!(a.cmp(&b), left.cmp(&right));
            assert_eq!(a == b, left == right);
            let range = packing.prefix_range::<W>(&left[..prefix_len]).unwrap();
            assert!(range.contains(&a));
            assert_eq!(range.contains(&b), left[..prefix_len] == right[..prefix_len]);
        }
    }

    #[test]
    fn random_signed_unsigned_order_injectivity_and_prefixes() {
        let columns = [Column::U8, Column::I8, Column::U16, Column::I16,
            Column::U32, Column::I32, Column::U64, Column::I64];
        let mut state = 0x17e6_98a2_0cd5_b341;
        for column in columns {
            properties::<u64>(&Packing::with_columns(&[column]).unwrap(), &mut state);
        }
        for first in columns {
            for second in columns {
                let packing = Packing::with_columns(&[first, second]).unwrap();
                properties::<u128>(&packing, &mut state);
                if packing.bits() <= 64 {
                    properties::<u64>(&packing, &mut state);
                }
            }
        }
        for widths in [&[1, 7, 13, 32, 11][..], &[63, 1, 64], &[1; 128], &[]] {
            let packing = Packing::new(widths).unwrap();
            properties::<u128>(&packing, &mut state);
            if packing.bits() <= 64 {
                properties::<u64>(&packing, &mut state);
            }
        }
    }

    #[test]
    fn signed_boundaries_flip_sign_bit() {
        for column in [Column::I8, Column::I16, Column::I32, Column::I64] {
            let sign = 1_i128 << (column.bits() - 1);
            for value in [-sign, -1, 0, sign - 1] {
                let encoded = column.encode(value).unwrap();
                assert_eq!(encoded as u128, value as u128 & mask(column.bits()) ^ sign as u128);
                assert_eq!(column.decode(encoded).unwrap(), value);
            }
            assert_eq!(column.encode(-sign).unwrap(), 0);
            assert_eq!(column.encode(sign - 1).unwrap() as u128, mask(column.bits()));
            assert!(column.encode(-sign - 1).is_err());
            assert!(column.encode(sign).is_err());
        }
        for column in [Column::U8, Column::U16, Column::U32, Column::U64] {
            let maximum = mask(column.bits()) as i128;
            assert_eq!(column.decode(column.encode(maximum).unwrap()).unwrap(), maximum);
            assert!(column.encode(-1).is_err());
            assert!(column.encode(maximum + 1).is_err());
        }
        assert!(Column::U8.decode(256).is_err());
    }

    #[test]
    fn full_width_and_empty_prefix_bounds() {
        let packing = Packing::with_columns(&[Column::U64, Column::I64]).unwrap();
        assert_eq!(packing.prefix_range::<u128>(&[]).unwrap(), 0..=u128::MAX);
        let maximum = [u64::MAX as i128, i64::MAX as i128];
        assert_eq!(packing.pack::<u128>(&maximum).unwrap(), u128::MAX);
        assert_eq!(packing.prefix_range::<u128>(&maximum).unwrap(), u128::MAX..=u128::MAX);
        let prefix = packing.prefix_range::<u128>(&maximum[..1]).unwrap();
        assert_eq!(*prefix.start(), (u64::MAX as u128) << 64);
        assert_eq!(*prefix.end(), u128::MAX);
        let empty = Packing::new(&[]).unwrap();
        assert_eq!(empty.pack::<u64>(&[]).unwrap(), 0);
        assert_eq!(empty.unpack(0_u128).unwrap(), Vec::<i128>::new());
        assert_eq!(empty.prefix_range::<u128>(&[]).unwrap(), 0..=0);
    }

    #[test]
    fn permutations_retain_domains_and_prefix_membership() {
        let packing = Packing::with_columns(&[Column::I16, Column::U64, Column::I8, Column::U32]).unwrap();
        let order = [2, 0, 3, 1];
        let permuted = packing.permute(&order).unwrap();
        assert_eq!(permuted, Packing::with_columns(&[Column::I8, Column::I16, Column::U32, Column::U64]).unwrap());
        assert_eq!(permuted.permute(&[1, 3, 0, 2]).unwrap(), packing);
        let mut state = 0xb104_9dc8_6a17_521e;
        properties::<u128>(&permuted, &mut state);
        for _ in 0..2048 {
            let original = tuple(&packing, &mut state);
            let values: Vec<_> = order.iter().map(|&index| original[index]).collect();
            let key = permuted.pack::<u128>(&values).unwrap();
            assert_eq!(permuted.unpack(key).unwrap(), values);
            for prefix in 0..=values.len() {
                assert!(permuted.prefix_range::<u128>(&values[..prefix]).unwrap().contains(&key));
            }
        }
    }

    #[test]
    fn invalid_layouts_values_words_and_permutations_are_rejected() {
        for widths in [&[0][..], &[65]] {
            assert!(Packing::new(widths).is_err());
        }
        let wide = Packing::new(&[64, 64]).unwrap();
        assert!(wide.pack::<u64>(&[0, 0]).is_err());
        assert!(wide.unpack(0_u64).is_err());
        assert!(wide.prefix_range::<u64>(&[]).is_err());
        let packing = Packing::with_columns(&[Column::I8, Column::U8]).unwrap();
        assert!(packing.pack::<u64>(&[128, 0]).is_err());
        assert!(packing.pack::<u64>(&[0, -1]).is_err());
        assert!(packing.pack::<u64>(&[0]).is_err());
        assert!(packing.unpack(1_u64 << 16).is_err());
        assert!(packing.prefix_range::<u64>(&[0, 0, 0]).is_err());
        assert!(packing.prefix_range::<u64>(&[128]).is_err());
        for order in [&[0][..], &[0, 0], &[1, 2]] {
            assert!(packing.permute(order).is_err());
        }
    }

    #[test]
    fn arbitrary_lane_packing_matches_native_words() {
        let mut state = 0xa5c7_1e29_0b83_d641;
        for widths in [&[64][..], &[1, 63, 64], &[64, 17, 64, 47], &[7, 64, 64, 64, 57]] {
            let packing = Packing::new(widths).unwrap();
            properties::<[u64; 4]>(&packing, &mut state);
            if packing.bits() <= 192 {
                properties::<[u64; 3]>(&packing, &mut state);
            }
            if packing.bits() <= 128 {
                for _ in 0..128 {
                    let row = tuple(&packing, &mut state);
                    let native = packing.pack::<u128>(&row).unwrap();
                    assert_eq!(packing.pack::<[u64; 3]>(&row).unwrap(),
                        [0, (native >> 64) as u64, native as u64]);
                    assert_eq!(packing.pack::<[u64; 4]>(&row).unwrap(),
                        [0, 0, (native >> 64) as u64, native as u64]);
                    assert_eq!(packing.pack::<[u64; 2]>(&row).unwrap(),
                        [(native >> 64) as u64, native as u64]);
                    if packing.bits() <= 64 {
                        assert_eq!(packing.pack::<[u64; 1]>(&row).unwrap(),
                            [packing.pack::<u64>(&row).unwrap()]);
                    }
                }
            }
        }
        let signed = Packing::with_columns(&[Column::I64, Column::U8, Column::I64,
            Column::U64, Column::I16]).unwrap();
        properties::<[u64; 4]>(&signed, &mut state);
        let full = Packing::new(&[64; 4]).unwrap();
        assert_eq!(full.word_bits(), 256);
        assert_eq!(full.prefix_range::<[u64; 4]>(&[]).unwrap(), [0; 4]..=[u64::MAX; 4]);
        assert_eq!(full.pack::<[u64; 4]>(&[u64::MAX as i128; 4]).unwrap(), [u64::MAX; 4]);
        assert!(full.pack::<[u64; 3]>(&[0; 4]).is_err());
        let narrow = Packing::new(&[64, 1]).unwrap();
        assert!(narrow.unpack([1_u64, 0, 0]).is_err());
        assert!(narrow.unpack([0_u64, 2, 0]).is_err());

        // Neither column count nor array length has the former 128-bit cap.
        let many = Packing::new(&[1; 257]).unwrap();
        let order = (0..257).rev().collect::<Vec<_>>();
        assert_eq!(many.permute(&order).unwrap(), many);
        properties::<[u64; 5]>(&many, &mut state);
        assert_eq!(Packing::new(&[64; 33]).unwrap().pack::<[u64; 33]>(&[7; 33]).unwrap(), [7; 33]);
    }
}
