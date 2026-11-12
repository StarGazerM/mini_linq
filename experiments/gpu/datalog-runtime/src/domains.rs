//! Execution-local equality components and their load-time device adaptation.
//!
//! Dictionary codes are dense in global `SymbolCodec::code` order, not string
//! order. Only distinct dictionary entries are inspected on the host. Signed
//! components retain their native values and use the smallest signed field that
//! contains the observed range. Neither kind changes generic payload packing.
use crate::{Buffer, Packing, Pool, Result, SymbolCodec, Word};
use cq_storage::{CodeDomain, OrderedCodeDomain};
use cuda_core::LaunchConfig1D;
use std::{marker::PhantomData, sync::Arc};

const MISSING: u32 = u32::MAX;

/// Membership collected while loading a compiler-inferred equality component.
pub enum DomainInput {
    /// Strictly increasing, distinct global symbol handles. No sorting occurs here.
    Dictionary(Vec<u32>),
    /// Actual inclusive numeric bounds, or no observations. Values stay signed.
    Signed(Option<(i64, i64)>),
}

struct Dictionary {
    inverse: Vec<u32>,
    base: u32,
    bits: u32,
}

fn dictionary_bits(cardinality: usize) -> Result<u32> {
    if cardinality >= MISSING as usize {
        return Err("dictionary cardinality reaches the reserved u32 sentinel".into());
    }
    Ok((u32::BITS - (cardinality as u32).saturating_sub(1).leading_zeros()).max(1))
}

impl Dictionary {
    fn new(inverse: Vec<u32>) -> Result<Self> {
        let bits = dictionary_bits(inverse.len())?;
        if inverse.windows(2).any(|pair| pair[0] >= pair[1]) {
            return Err("dictionary handles must be sorted and distinct".into());
        }
        Ok(Self { base: inverse.first().copied().unwrap_or(0), inverse, bits })
    }

    fn forward(&self) -> Result<Vec<u32>> {
        let span = self.inverse.last().map_or(0, |&last| u64::from(last) - u64::from(self.base) + 1);
        let span = usize::try_from(span).map_err(|_| "dictionary forward span exceeds host address space")?;
        let mut forward = Vec::new();
        forward.try_reserve_exact(span)?;
        forward.resize(span, MISSING);
        for (dense, &global) in self.inverse.iter().enumerate() {
            forward[(global - self.base) as usize] = dense as u32;
        }
        Ok(forward)
    }

    fn exact(&self, global: u32) -> u32 {
        self.inverse.binary_search(&global).map_or(MISSING, |code| code as u32)
    }

    fn encode_required(&self, global: u32) -> Result<u32> {
        let code = self.exact(global);
        if code == MISSING {
            return Err("required symbol is absent from its equality component".into());
        }
        Ok(code)
    }

    fn global(&self, code: u32) -> Result<u32> {
        self.inverse.get(code as usize).copied().ok_or_else(|| "invalid component dictionary code".into())
    }
}

fn signed_bits(range: Option<(i64, i64)>) -> Result<u32> {
    let Some((minimum, maximum)) = range else { return Ok(1) };
    if minimum > maximum {
        return Err("signed component minimum exceeds maximum".into());
    }
    for bits in 1..=32 {
        let sign = 1_i64 << (bits - 1);
        if minimum >= -sign && maximum < sign {
            return Ok(bits);
        }
    }
    Err("signed component requires more than 32 key bits".into())
}

enum Metadata {
    Dictionary(Arc<Dictionary>),
    Signed { range: Option<(i64, i64)>, bits: u32 },
}

impl Metadata {
    fn new(input: DomainInput) -> Result<Self> {
        match input {
            DomainInput::Dictionary(handles) => Ok(Self::Dictionary(Arc::new(Dictionary::new(handles)?))),
            DomainInput::Signed(range) => Ok(Self::Signed { range, bits: signed_bits(range)? }),
        }
    }

    fn field(&self) -> (u32, bool) {
        match self {
            Self::Dictionary(dictionary) => (dictionary.bits, false),
            Self::Signed { bits, .. } => (*bits, true),
        }
    }

    fn dictionary(&self) -> Result<&Arc<Dictionary>> {
        match self {
            Self::Dictionary(dictionary) => Ok(dictionary),
            Self::Signed { .. } => Err("signed components retain native values, not symbol codes".into()),
        }
    }
}

struct Entry {
    metadata: Metadata,
    forward: Option<Buffer<u32>>,
}

/// A program execution's immutable component dictionaries and device tables.
/// IDs are positions in `inputs`; no process-global catalog is installed.
pub struct DomainCatalog {
    module: domain_kernels::LoadedModule,
    pool: Arc<Pool>,
    entries: Vec<Entry>,
}

/// Packed candidates whose domain validation is pending on their pool stream.
/// Only admission/sort can expose a published batch: its existing count readback
/// consumes the validation flag, so repacking adds no query-time synchronization.
pub struct DomainBatch<W: Word> {
    pub(crate) words: Buffer<W>,
    pub(crate) validation: Option<Buffer<u64>>,
    pub(crate) packing: Packing,
}

impl<W: Word> DomainBatch<W> {
    pub fn len(&self) -> usize { self.words.len() }
    pub fn is_empty(&self) -> bool { self.words.is_empty() }

    /// Reorder fields without discarding the original validation dependency.
    pub fn permuted(self, order: &[usize], block_threads: u32) -> Result<Self> {
        let words = self.packing.permute_device(&self.words, self.len(), order, block_threads)?;
        let packing = self.packing.permute(order)?;
        Ok(Self { words, validation: self.validation, packing })
    }
}

impl DomainCatalog {
    pub fn new(pool: Arc<Pool>, inputs: Vec<DomainInput>) -> Result<Self> {
        if inputs.len() > u32::MAX as usize {
            return Err("component count exceeds u32 IDs".into());
        }
        let mut entries = Vec::with_capacity(inputs.len());
        for input in inputs {
            let metadata = Metadata::new(input)?;
            let forward = match &metadata {
                Metadata::Dictionary(dictionary) => Some(Buffer::from_host_async(pool.clone(), &dictionary.forward()?)?),
                Metadata::Signed { .. } => None,
            };
            entries.push(Entry { metadata, forward });
        }
        let module = unsafe { domain_kernels::load(pool.context())? };
        Ok(Self { module, pool, entries })
    }

    fn entry(&self, id: u32) -> Result<&Entry> {
        self.entries.get(id as usize).ok_or_else(|| "unknown equality component ID".into())
    }

    /// Ordered component IDs determine the tuple's runtime field widths.
    pub fn packing(&self, components: &[u32]) -> Result<Packing> {
        let fields = components.iter().map(|&id| Ok(self.entry(id)?.metadata.field())).collect::<Result<Vec<_>>>()?;
        Packing::with_fields(&fields)
    }

    /// Own a typed symbol-domain handle, independent of the catalog's lifetime.
    /// Panics for an invalid ID or a signed component (which has no symbol codec).
    pub fn domain<T, C: SymbolCodec<T>>(&self, id: u32) -> DomainRef<T, C> {
        let dictionary = self.entry(id).and_then(|entry| entry.metadata.dictionary())
            .expect("symbol domain requires a valid dictionary component");
        DomainRef { dictionary: dictionary.clone(), marker: PhantomData }
    }

    /// Restore a whole output column with one inverse-table gather.
    /// The table and packed-field descriptor are resolved once, outside the
    /// row loop; the codec receives the complete owned global-code column.
    pub fn decode_packed_column<T, C: SymbolCodec<T>, W: Word>(
        &self, id: u32, packing: &Packing, column: usize, words: &[W],
    ) -> Result<Vec<T>> {
        let dictionary = self.entry(id)?.metadata.dictionary()?;
        let field = packing.packed_fields().nth(column)
            .ok_or("output column is outside its packing")?;
        if packing.bits() > W::BITS || field.signed != 0 || field.bits != dictionary.bits {
            return Err("output column packing differs from its recorded symbol domain".into());
        }
        // DECODE P: final-output bulk gather resolves one component inverse table per column.
        let codes = words.iter().map(|word| {
            dictionary.global(word.extract(field.shift, field.bits) as u32)
        }).collect::<Result<Vec<_>>>()?;
        let values = C::decode_many(codes);
        if values.len() != words.len() {
            return Err("symbol codec changed the output column length".into());
        }
        Ok(values)
    }

    /// Required constants must belong to this component, not merely the codec.
    pub fn encode_required<T, C: SymbolCodec<T>>(&self, id: u32, value: &T) -> Result<u32> {
        self.entry(id)?.metadata.dictionary()?.encode_required(C::code(value))
    }

    pub fn bits(&self, id: u32) -> Result<u32> {
        Ok(self.entry(id)?.metadata.field().0)
    }

    /// Significant tuple width without constructing a second packing descriptor.
    pub fn total_bits(&self, components: &[u32]) -> Result<u32> {
        components.iter().try_fold(0u32, |bits, &id| {
            bits.checked_add(self.bits(id)?).ok_or_else(|| "domain tuple width exceeds u32".into())
        })
    }

    /// Remap uploaded global symbols on the device, preserving row order and W.
    /// Signed source fields contain native values in `Packing`'s sign-ordinal
    /// representation; only their physical width changes. Source initialization
    /// must be ordered on this catalog's pool stream. One error word, never rows,
    /// is read back before returning an initialized result.
    pub fn remap<W: Word>(&self, input: &Buffer<W>, source: &Packing, components: &[u32]) -> Result<Buffer<W>> {
        let batch = self.repack(input, source, components, false)?;
        if let Some(validation) = &batch.validation {
            let result = validation.to_host()?;
            crate::record_readback();
            if result[1] != 0 {
                return Err("uploaded value is outside its recorded equality component".into());
            }
        }
        Ok(batch.words)
    }

    /// Seal already-domain-coded candidate rows at their recorded field widths.
    /// This preserves codes; it does not look up or reconstruct global symbols.
    pub fn repack_codes<W: Word>(&self, input: &Buffer<W>, source: &Packing, components: &[u32]) -> Result<DomainBatch<W>> {
        self.repack(input, source, components, true)
    }

    fn repack<W: Word>(&self, input: &Buffer<W>, source: &Packing, components: &[u32], dense: bool) -> Result<DomainBatch<W>> {
        let target = self.packing(components)?;
        if source.len() != components.len() {
            return Err("component count differs from source packing arity".into());
        }
        if source.bits() > W::BITS || target.bits() > W::BITS {
            return Err("domain remap packing exceeds the selected word width".into());
        }
        if !self.pool.shares_memory(input.pool()) || self.pool.stream().cu_stream() != input.pool().stream().cu_stream() {
            return Err("domain remap requires the catalog's memory and stream".into());
        }
        // Eleven u64 words per column: source shift/width, target shift/width,
        // kind, forward base/length/pointer, signed minimum/maximum/empty.
        let mut descriptors = Vec::with_capacity(components.len() * 11);
        for ((source, target), &id) in source.packed_fields().zip(target.packed_fields()).zip(components) {
            let entry = self.entry(id)?;
            descriptors.extend_from_slice(&[source.shift as u64, source.bits as u64, target.shift as u64, target.bits as u64]);
            match &entry.metadata {
                Metadata::Dictionary(dictionary) => {
                    if source.signed != 0 || source.bits > 32 {
                        return Err("dictionary source must contain unsigned global u32 handles".into());
                    }
                    if dense {
                        descriptors.extend_from_slice(&[2, 0, dictionary.inverse.len() as u64, 0, 0, 0, 0]);
                    } else {
                        let forward = entry.forward.as_ref().expect("dictionary forward table");
                        descriptors.extend_from_slice(&[0, dictionary.base as u64, forward.len() as u64, forward.cu_deviceptr(), 0, 0, 0]);
                    }
                }
                Metadata::Signed { range, .. } => {
                    if source.signed == 0 || source.bits > 32 {
                        return Err("signed component requires a signed key field of at most 32 bits".into());
                    }
                    let (minimum, maximum) = range.unwrap_or((0, 0));
                    descriptors.extend_from_slice(&[1, 0, 0, 0, minimum as u64, maximum as u64, range.is_none() as u64]);
                }
            }
        }
        let output = if components.is_empty() {
            Buffer::zeroed(self.pool.clone(), input.len())?
        } else {
            unsafe { Buffer::uninitialized(self.pool.clone(), input.len())? }
        };
        if input.is_empty() || components.is_empty() {
            return Ok(DomainBatch { words: output, validation: None, packing: target });
        }
        let descriptors = Buffer::from_host_async(self.pool.clone(), &descriptors)?;
        // Slot zero receives the unique count at the existing admission barrier.
        // Slot one is the sticky error flag; all eight bytes start initialized.
        let validation = Buffer::<u64>::zeroed(self.pool.clone(), 2)?;
        let error = unsafe { validation.ptr().add(1).cast::<u32>() };
        let module = &self.module;
        let initial = module.prepare_remap::<W>(LaunchConfig1D::new(1, 256, 0))?;
        let grid = cq_gpu_runtime::launch_grid_for_kernel(initial.function(), input.len().div_ceil(256), 256, 0)?;
        let launch = module.prepare_remap::<W>(LaunchConfig1D::new(grid, 256, 0))?;
        module.remap::<W>(self.pool.stream(), &launch, input.ptr().cast(), output.ptr().cast(), input.len() as u64, descriptors.ptr(), components.len() as u32, error)?;
        Ok(DomainBatch { words: output, validation: Some(validation), packing: target })
    }
}

/// The same wrapper type serves every dictionary component of a logical type.
/// Ordering is global codec-handle ordering, preserved by the dense inverse.
pub struct DomainRef<T, C> {
    dictionary: Arc<Dictionary>,
    marker: PhantomData<fn() -> (T, C)>,
}

impl<T, C: SymbolCodec<T>> DomainRef<T, C> {
    /// Inverse for an explicitly selected logical Rust read or final publication.
    pub fn decode(&self, code: u32) -> Result<T> {
        // DECODE ?: component view inversion serves both Rust reads and final publication.
        Ok(C::decode(self.dictionary.global(code)?))
    }

    /// Encode a required head or opaque-boundary value, rejecting other domains.
    pub fn encode_required(&self, value: &T) -> Result<u32> {
        self.dictionary.encode_required(C::code(value))
    }
}

impl<T, C> Clone for DomainRef<T, C> {
    fn clone(&self) -> Self {
        Self { dictionary: self.dictionary.clone(), marker: PhantomData }
    }
}

impl<T, C: SymbolCodec<T>> CodeDomain for DomainRef<T, C> {
    type Value = T;
    type Code = u32;
    fn exact(&self, value: &T) -> u32 {
        self.dictionary.exact(C::code(value))
    }
}

impl<T, C: SymbolCodec<T>> OrderedCodeDomain for DomainRef<T, C> {
    fn lower(&self, value: &T) -> u32 {
        let global = C::code(value);
        self.dictionary.inverse.partition_point(|&handle| handle < global) as u32
    }
    fn upper(&self, value: &T) -> u32 {
        let global = C::code(value);
        self.dictionary.inverse.partition_point(|&handle| handle <= global) as u32
    }
}


#[cuda_host::cuda_module]
mod domain_kernels {
    use cuda_device::atomic::{AtomicOrdering, DeviceAtomicU32};
    use cuda_device::{kernel, launch_bounds, launch_contract, thread};

    #[kernel]
    #[launch_bounds(256)]
    #[launch_contract(domain = 1, block = (256, 1, 1))]
    pub fn remap<W: crate::Word>(input: *const W, output: *mut W, rows: u64, descriptors: *const u64, columns: u32, error: *mut u32) {
        let mut row = thread::index_1d().get() as u64;
        let stride = thread::gridDim_x() as u64 * 256;
        while row < rows {
            let source = unsafe { *input.add(row as usize) };
            let mut packed = W::zero();
            let mut column = 0_usize;
            while column < columns as usize {
                let descriptor = unsafe { descriptors.add(column * 11) };
                let source_shift = unsafe { *descriptor } as u32;
                let source_bits = unsafe { *descriptor.add(1) } as u32;
                let target_shift = unsafe { *descriptor.add(2) } as u32;
                let target_bits = unsafe { *descriptor.add(3) } as u32;
                let value = source.extract(source_shift, source_bits);
                let mut valid = true;
                let mut encoded = 0_u64;
                if unsafe { *descriptor.add(4) } == 0 {
                    let base = unsafe { *descriptor.add(5) };
                    let len = unsafe { *descriptor.add(6) };
                    if value < base || value - base >= len {
                        valid = false;
                    } else {
                        let forward = unsafe { *descriptor.add(7) } as *const u32;
                        let dense = unsafe { *forward.add((value - base) as usize) };
                        if dense == u32::MAX {
                            valid = false;
                        } else {
                            encoded = dense as u64;
                        }
                    }
                } else if unsafe { *descriptor.add(4) } == 2 {
                    valid = value < unsafe { *descriptor.add(6) };
                    encoded = value;
                } else {
                    // DECODE I: rebase a 32-bit sign ordinal between runtime field widths.
                    let native = (value as u32).wrapping_sub(1_u32 << (source_bits - 1)) as i32;
                    let minimum = unsafe { *descriptor.add(8) } as i32;
                    let maximum = unsafe { *descriptor.add(9) } as i32;
                    if unsafe { *descriptor.add(10) } != 0 || native < minimum || native > maximum {
                        valid = false;
                    } else {
                        encoded = native.wrapping_add(1_i32 << (target_bits - 1)) as u32 as u64;
                    }
                }
                if valid {
                    packed.insert(target_shift, target_bits, encoded);
                } else {
                    unsafe { DeviceAtomicU32::from_ptr(error).store(1, AtomicOrdering::Relaxed) };
                }
                column += 1;
            }
            unsafe { *output.add(row as usize) = packed };
            row += stride;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Handle;
    impl SymbolCodec<u32> for Handle {
        fn code(value: &u32) -> u32 { *value }
        // DECODE P: test publication exposes the original global handle.
        fn decode(code: u32) -> u32 { code }
    }

    fn domain(handles: Vec<u32>) -> DomainRef<u32, Handle> {
        DomainRef { dictionary: Arc::new(Dictionary::new(handles).unwrap()), marker: PhantomData }
    }

    #[test]
    fn dictionaries_preserve_ordinals_and_trim_forward_tables() {
        let dictionary = Dictionary::new(vec![100, 102, 105]).unwrap();
        assert_eq!(dictionary.bits, 2);
        assert_eq!(dictionary.base, 100);
        assert_eq!(dictionary.forward().unwrap(), [0, MISSING, 1, MISSING, MISSING, 2]);
        for (dense, global) in [(0, 100), (1, 102), (2, 105)] {
            assert_eq!(dictionary.encode_required(global).unwrap(), dense);
            assert_eq!(dictionary.global(dense).unwrap(), global);
        }
        assert!(dictionary.global(3).is_err());
        assert!(dictionary.global(MISSING).is_err());
        assert!(Dictionary::new(vec![2, 1]).is_err());
        assert!(Dictionary::new(vec![1, 1]).is_err());
    }

    #[test]
    fn exact_and_ordered_bounds_include_absent_values() {
        let first = domain(vec![10, 20, 40]);
        let second = domain(vec![20, 30]);
        for value in [0, 10, 15, 20, 30, 40, u32::MAX] {
            for (dense, stored) in [10, 20, 40].into_iter().enumerate() {
                assert_eq!(stored < value, (dense as u32) < first.lower(&value));
                assert_eq!(stored <= value, (dense as u32) < first.upper(&value));
            }
        }
        assert_eq!(first.exact(&20), 1);
        assert_eq!(second.exact(&20), 0);
        assert_eq!(first.exact(&30), MISSING);
        assert!(first.encode_required(&30).is_err());
        assert_eq!(second.encode_required(&30).unwrap(), 1);
        assert_eq!(first.decode(1).unwrap(), 20);
        assert_eq!(second.decode(1).unwrap(), 30);
        assert!(first.decode(MISSING).is_err());
        let owned = first.clone();
        drop(first);
        assert_eq!(owned.exact(&40), 2);
    }

    #[test]
    fn empty_and_maximum_global_handle_are_not_sentinels() {
        let empty = domain(vec![]);
        assert_eq!(empty.dictionary.bits, 1);
        assert!(empty.dictionary.forward().unwrap().is_empty());
        assert_eq!(empty.exact(&0), MISSING);
        assert_eq!(empty.lower(&0), 0);
        assert_eq!(empty.upper(&u32::MAX), 0);
        assert!(empty.dictionary.encode_required(0).is_err());
        let last = domain(vec![u32::MAX]);
        assert_eq!(last.dictionary.base, u32::MAX);
        assert_eq!(last.dictionary.forward().unwrap(), [0]);
        assert_eq!(last.exact(&u32::MAX), 0);
        assert_eq!(last.upper(&u32::MAX), 1);
    }

    #[test]
    fn dictionary_width_and_cardinality_boundaries() {
        for (count, bits) in [(0, 1), (1, 1), (2, 1), (3, 2), (4, 2), (5, 3), (256, 8), (257, 9)] {
            assert_eq!(dictionary_bits(count).unwrap(), bits);
        }
        assert_eq!(dictionary_bits(u32::MAX as usize - 1).unwrap(), 32);
        assert!(dictionary_bits(u32::MAX as usize).is_err());
    }

    #[test]
    fn signed_components_keep_minimal_native_widths() {
        for (range, bits) in [(None, 1), (Some((0, 0)), 1), (Some((-1, 0)), 1), (Some((1, 1)), 2), (Some((-128, 127)), 8), (Some((-129, 127)), 9), (Some((i32::MIN as i64, i32::MAX as i64)), 32)] {
            let metadata = Metadata::new(DomainInput::Signed(range)).unwrap();
            assert_eq!(metadata.field(), (bits, true));
            assert!(metadata.dictionary().is_err());
            let packing = Packing::with_fields(&[metadata.field()]).unwrap();
            if let Some((minimum, maximum)) = range {
                for native in [minimum, maximum] {
                    let word = packing.pack::<u64>(&[native as i128]).unwrap();
                    // DECODE P: test numeric publication preserves native values.
                    assert_eq!(packing.unpack(word).unwrap(), [native as i128]);
                }
            }
        }
        for range in [(2, 1), (i64::MIN, 0), (0, i32::MAX as i64 + 1)] {
            assert!(signed_bits(Some(range)).is_err());
        }
    }

    #[test]
    fn device_remap_mixes_symbols_and_signed_values() -> Result<()> {
        let pool = Pool::new(0, 0, 0)?;
        let catalog = DomainCatalog::new(pool.clone(), vec![DomainInput::Dictionary(vec![100, 102]), DomainInput::Signed(Some((-2, 3)))])?;
        let source = Packing::with_fields(&[(32, false), (32, true)])?;
        let words = [[100, -2], [102, 3]].into_iter().map(|row| source.pack::<u64>(&row)).collect::<Result<Vec<_>>>()?;
        let input = Buffer::from_host(pool.clone(), &words)?;
        let output = catalog.remap(&input, &source, &[0, 1])?.to_host()?;
        let target = catalog.packing(&[0, 1])?;
        assert_eq!(target.bits(), 4);
        assert_eq!(output, [target.pack::<u64>(&[0, -2])?, target.pack::<u64>(&[1, 3])?]);
        assert_eq!(catalog.decode_packed_column::<u32, Handle, u64>(0, &target, 0, &output)?, [100, 102]);
        assert!(catalog.encode_required::<u32, Handle>(0, &101).is_err());
        for row in [[99, 0], [101, 0], [103, 0], [100, 4]] {
            let input = Buffer::from_host(pool.clone(), &[source.pack::<u64>(&row)?])?;
            assert!(catalog.remap(&input, &source, &[0, 1]).is_err());
        }
        let empty = Buffer::<u64>::from_host(pool, &[])?;
        assert!(catalog.remap(&empty, &source, &[0, 1])?.is_empty());
        Ok(())
    }

    #[test]
    fn device_remap_checks_word_bounds_and_signed_extremes() -> Result<()> {
        let pool = Pool::new(0, 0, 0)?;
        let catalog = DomainCatalog::new(pool.clone(), vec![
            DomainInput::Signed(Some((i32::MIN as i64, i32::MAX as i64))),
            DomainInput::Dictionary(vec![u32::MAX]),
            DomainInput::Signed(None),
            DomainInput::Dictionary(vec![]),
        ])?;
        let source = Packing::with_fields(&[(32, false), (32, true)])?;
        let target = catalog.packing(&[1, 0])?;
        for value in [i32::MIN as i128, -1, 0, i32::MAX as i128] {
            let word = source.pack::<u128>(&[u32::MAX as i128, value])?;
            let input = Buffer::from_host(pool.clone(), &[word])?;
            let output = catalog.remap(&input, &source, &[1, 0])?.to_host()?;
            assert_eq!(output, [target.pack::<u128>(&[0, value])?]);
            let word = source.pack::<[u64; 2]>(&[u32::MAX as i128, value])?;
            let input = Buffer::from_host(pool.clone(), &[word])?;
            let output = catalog.remap(&input, &source, &[1, 0])?.to_host()?;
            assert_eq!(output, [target.pack::<[u64; 2]>(&[0, value])?]);
        }
        let empty = Buffer::<u64>::from_host(pool.clone(), &[])?;
        let overwide = Packing::with_fields(&[(32, false), (64, true)])?;
        assert!(catalog.remap(&empty, &overwide, &[1, 0]).is_err());
        let narrow = Packing::with_fields(&[(1, true); 3])?;
        assert!(catalog.remap(&empty, &narrow, &[0, 0, 0]).is_err());
        assert!(catalog.bits(4).is_err());
        assert!(catalog.packing(&[4]).is_err());
        assert!(catalog.decode_packed_column::<u32, Handle, u64>(0, &Packing::new(&[1])?, 0, &[]).is_err());
        assert!(catalog.encode_required::<u32, Handle>(0, &0).is_err());
        for (component, signed) in [(2, true), (3, false)] {
            let source = Packing::with_fields(&[(1, signed)])?;
            let input = Buffer::from_host(pool.clone(), &[source.pack::<u64>(&[0])?])?;
            assert!(catalog.remap(&input, &source, &[component]).is_err());
        }
        Ok(())
    }
}
