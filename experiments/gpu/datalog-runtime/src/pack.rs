use crate::Result;
use cuda_core::{CudaContext, DeviceCopy, LaunchConfig1D};
use gpu_device_columns::{Buffer, Pool};
use std::{cell::RefCell, fmt::Debug, ops::RangeInclusive, rc::Rc, sync::Arc};

mod sealed {
    pub trait Sealed {}
    impl Sealed for u64 {}
    impl Sealed for u128 {}
}

/// The two unsigned, native device representations of packed tuples.
pub trait Word: sealed::Sealed + Copy + Ord + Default + Debug + DeviceCopy {
    const BITS: u32;
    /// Convert the low `Self::BITS` bits. Packing checks the width first.
    fn from_u128(value: u128) -> Self;
    fn to_u128(self) -> u128;
}

impl Word for u64 {
    const BITS: u32 = 64;
    fn from_u128(value: u128) -> Self {
        value as u64
    }
    fn to_u128(self) -> u128 {
        self as u128
    }
}

impl Word for u128 {
    const BITS: u32 = 128;
    fn from_u128(value: u128) -> Self {
        value
    }
    fn to_u128(self) -> u128 {
        self
    }
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
    /// Construct unsigned columns with widths in `1..=64`, totaling at most 128.
    /// An empty schema represents the single empty tuple with key zero.
    pub fn new(widths: &[u32]) -> Result<Self> {
        Self::build(widths.iter().map(|&bits| (bits, false)))
    }

    /// Construct native-width columns, retaining their signed numeric domains.
    pub fn with_columns(columns: &[Column]) -> Result<Self> {
        Self::build(columns.iter().map(|column| (column.bits(), column.is_signed())))
    }

    fn build(columns: impl Iterator<Item = (u32, bool)>) -> Result<Self> {
        let mut fields = Vec::with_capacity(columns.size_hint().0.min(128));
        let mut bits = 0;
        for (width, signed) in columns {
            if !(1..=64).contains(&width) || width > 128 - bits {
                return Err("packing requires 1..=64-bit columns totaling at most 128 bits".into());
            }
            bits += width;
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

    /// Minimal native word width: u64 through 64 total bits, otherwise u128.
    pub fn word_bits(&self) -> u32 {
        if self.bits <= 64 { 64 } else { 128 }
    }

    pub fn widths(&self) -> impl ExactSizeIterator<Item = u32> + '_ {
        self.fields.iter().map(|field| field.bits)
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
        let mut packed = 0;
        for (&field, &value) in self.fields.iter().zip(values) {
            packed |= field.encode(value)? << field.shift;
        }
        Ok(W::from_u128(packed))
    }

    pub fn unpack<W: Word>(&self, packed: W) -> Result<Vec<i128>> {
        self.check_word::<W>()?;
        let packed = packed.to_u128();
        if packed > mask(self.bits) {
            return Err("packed key contains bits outside its schema".into());
        }
        Ok(self.fields.iter().map(|field| {
            field.decode((packed >> field.shift) & mask(field.bits))
        }).collect())
    }

    /// Inclusive bounds avoid an unrepresentable successor of `u128::MAX`.
    /// An empty prefix covers every key in this schema, not unused high bits.
    pub fn prefix_range<W: Word>(&self, prefix: &[i128]) -> Result<RangeInclusive<W>> {
        self.check_word::<W>()?;
        if prefix.len() > self.len() {
            return Err("prefix is longer than the tuple schema".into());
        }
        let mut lower = 0;
        let mut remaining = self.bits;
        for (&field, &value) in self.fields.iter().zip(prefix) {
            lower |= field.encode(value)? << field.shift;
            remaining = field.shift;
        }
        Ok(W::from_u128(lower)..=W::from_u128(lower | mask(remaining)))
    }

    fn check_order(&self, order: &[usize]) -> Result<()> {
        if order.len() != self.len() {
            return Err("permutation arity differs from packing".into());
        }
        let mut seen = 0_u128;
        for &index in order {
            if index >= self.len() || seen & (1_u128 << index) != 0 {
                return Err("column order is not a permutation".into());
            }
            seen |= 1_u128 << index;
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
        // At least one bit per field bounds the descriptor to 128 columns.
        let mut descriptors = [0_u32; 128 * 3];
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
        let launch = LaunchConfig1D::new(rows.div_ceil(256).clamp(1, 2272) as u32, 256, 0);
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
        input: *const u64,
        output: *mut u64,
        rows: u64,
        descriptors: *const u32,
        columns: u32,
    ) {
        let lanes = W::BITS / 64;
        let mut row = thread::index_1d().get() as u64;
        let stride = thread::gridDim_x() as u64 * 256;
        while row < rows {
            let offset = row as usize * lanes as usize;
            let mut source = unsafe { *input.add(offset) } as u128;
            if lanes == 2 {
                source |= (unsafe { *input.add(offset + 1) } as u128) << 64;
            }
            let mut packed = 0_u128;
            let mut column = 0;
            while column < columns as usize {
                let source_shift = unsafe { *descriptors.add(column * 3) };
                let width = unsafe { *descriptors.add(column * 3 + 1) };
                let destination_shift = unsafe { *descriptors.add(column * 3 + 2) };
                let field = (source >> source_shift) & ((1_u128 << width) - 1);
                packed |= field << destination_shift;
                column += 1;
            }
            unsafe {
                *output.add(offset) = packed as u64;
                if lanes == 2 {
                    *output.add(offset + 1) = (packed >> 64) as u64;
                }
            }
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
        for widths in [&[0][..], &[65], &[64, 64, 1]] {
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
}
