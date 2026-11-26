//! Typed host views of word-run publications (CUDA `RadixLayout` and its
//! `pack.rs` key packing), read on the CPU over the runs' shared arrays.
//!
//! A [`Packing`] concatenates bounded scalar fields most-significant field
//! first; signed fields flip their sign bit, so packed keys order exactly as
//! the tuples and a fixed leading prefix occupies one inclusive key interval.
//! A [`WordLayout`] binds a [`WordRunSet`] publication to a key tuple schema
//! `K`, a value tuple schema `V` and a packing, checked against the run's
//! key words and item lanes. Republishing an unchanged set returns the same
//! publication; changed sets share the predecessor's packing.

use std::{cmp::Ordering, fmt, marker::PhantomData, ops::Deref, ops::RangeInclusive, rc::Rc};

use crate::{
    Error, Result,
    word_read::{Wide, WordRunView, compare, extract, lane},
    words::{ItemLane, ItemType, MAX_LANES, WordRunParams, WordRunSet, WordRunSnapshot},
};

/// A tuple column's logical integer domain, not its stored lane type.
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

    fn field(self) -> Field {
        Field {
            bits: self.bits(),
            signed: self.is_signed(),
            shift: 0,
        }
    }

    /// Order-preserving code: the value with its sign bit flipped when signed.
    pub fn encode(self, value: i128) -> Result<u64> {
        self.field().encode(value)
    }

    /// Inverse of [`Self::encode`].
    pub fn decode(self, code: u64) -> Result<i128> {
        if u128::from(code) > mask(self.bits()) {
            return Err(Error("encoded value exceeds column width".into()));
        }
        Ok(self.field().decode(code))
    }
}

mod sealed {
    pub trait Scalar {}
    pub trait Tuple {}
}

/// A Rust scalar type of a tuple column.
pub trait Scalar: sealed::Scalar + Copy + 'static {
    const COLUMN: Column;
}

macro_rules! scalars {
    ($($ty:ty => $column:ident),*) => {$(
        impl sealed::Scalar for $ty {}
        impl Scalar for $ty {
            const COLUMN: Column = Column::$column;
        }
    )*};
}
scalars!(u8 => U8, i8 => I8, u16 => U16, i16 => I16, u32 => U32, i32 => I32, u64 => U64,
    i64 => I64);

/// A tuple of [`Scalar`]s: a key or value schema, `()` for none.
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
tuples!(A, B, C, D, E, F, G, H, I, J, K, L);

/// Stored width and signedness of an item lane.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LaneKind {
    pub bits: u32,
    pub signed: bool,
}

impl LaneKind {
    pub const fn of(kind: ItemType) -> Self {
        match kind {
            ItemType::U32 => Self {
                bits: 32,
                signed: false,
            },
            ItemType::I32 => Self {
                bits: 32,
                signed: true,
            },
            ItemType::U64 => Self {
                bits: 64,
                signed: false,
            },
            ItemType::I64 => Self {
                bits: 64,
                signed: true,
            },
        }
    }

    /// Whether every value of `column` is stored exactly: same signedness and
    /// at least its width, or an unsigned column in a strictly wider signed
    /// lane. Never a narrowing.
    pub const fn holds(self, column: Column) -> bool {
        if self.signed == column.is_signed() {
            self.bits >= column.bits()
        } else {
            self.signed && self.bits > column.bits()
        }
    }
}

fn mask(bits: u32) -> u128 {
    if bits == 128 {
        u128::MAX
    } else {
        (1 << bits) - 1
    }
}

/// One key field: `bits` wide at bit `shift` of the packed key.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Field {
    pub bits: u32,
    pub signed: bool,
    pub shift: u32,
}

impl Field {
    fn sign(self) -> i128 {
        if self.signed { 1 << (self.bits - 1) } else { 0 }
    }

    fn encode(self, value: i128) -> Result<u64> {
        let sign = self.sign();
        if value < -sign || value > mask(self.bits) as i128 - sign {
            return Err(Error("tuple value exceeds column domain".into()));
        }
        // In the declared two's-complement width this is the sign-bit XOR.
        Ok((((value as u128) & mask(self.bits)) ^ sign as u128) as u64)
    }

    fn decode(self, code: u64) -> i128 {
        i128::from(code) - self.sign()
    }
}

#[inline(always)]
fn insert(word: &mut Wide, shift: u32, bits: u32, value: u64) {
    let index = (shift / 64) as usize;
    let offset = shift % 64;
    word[index] |= value << offset;
    if offset != 0 && bits > 64 - offset {
        word[index + 1] |= value >> (64 - offset);
    }
}

/// One packed key: `lanes` words in its run's memory layout. Ordered by
/// shape, then numerically.
#[derive(Clone, Copy, Eq, Hash, PartialEq)]
pub struct PackedKey {
    words: [u64; MAX_LANES],
    lanes: u8,
    ms_first: bool,
}

impl PackedKey {
    fn from_wide(wide: &Wide, lanes: usize, ms_first: bool) -> Self {
        let mut words = [0; MAX_LANES];
        for (index, word) in words.iter_mut().enumerate().take(lanes) {
            *word = if ms_first {
                wide[lanes - 1 - index]
            } else {
                wide[index]
            };
        }
        Self {
            words,
            lanes: lanes as u8,
            ms_first,
        }
    }

    pub fn as_slice(&self) -> &[u64] {
        &self.words[..self.lanes as usize]
    }

    pub fn ms_first(&self) -> bool {
        self.ms_first
    }
}

impl Deref for PackedKey {
    type Target = [u64];

    fn deref(&self) -> &[u64] {
        self.as_slice()
    }
}

impl fmt::Debug for PackedKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:x?}", self.as_slice())
    }
}

impl Ord for PackedKey {
    fn cmp(&self, other: &Self) -> Ordering {
        self.lanes
            .cmp(&other.lanes)
            .then(self.ms_first.cmp(&other.ms_first))
            .then_with(|| compare(self, other, self.ms_first))
    }
}

impl PartialOrd for PackedKey {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// Bounded fields concatenated most-significant field first: injective and
/// order-preserving (CUDA formal rule DL-DevicePacking). An empty schema is
/// the single empty tuple, key zero.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Packing {
    fields: Vec<Field>,
    bits: u32,
}

impl Packing {
    /// Unsigned fields of widths `1..=64`.
    pub fn new(widths: &[u32]) -> Result<Self> {
        Self::build(widths.iter().map(|&bits| (bits, false)))
    }

    /// Native-width fields of `columns`, keeping their signed domains.
    pub fn with_columns(columns: &[Column]) -> Result<Self> {
        Self::build(
            columns
                .iter()
                .map(|column| (column.bits(), column.is_signed())),
        )
    }

    /// Recorded `(bits, signed)` fields.
    pub fn with_fields(fields: &[(u32, bool)]) -> Result<Self> {
        Self::build(fields.iter().copied())
    }

    fn build(columns: impl Iterator<Item = (u32, bool)>) -> Result<Self> {
        let mut fields = Vec::with_capacity(columns.size_hint().0);
        let mut bits = 0;
        for (width, signed) in columns {
            if !(1..=64).contains(&width) {
                return Err(Error("packing requires 1..=64-bit fields".into()));
            }
            bits += width;
            if bits as usize > 64 * MAX_LANES {
                return Err(Error("packing exceeds the widest key word".into()));
            }
            fields.push(Field {
                bits: width,
                signed,
                shift: bits,
            });
        }
        for field in &mut fields {
            field.shift = bits - field.shift;
        }
        Ok(Self { fields, bits })
    }

    /// Significant key bits.
    pub fn bits(&self) -> u32 {
        self.bits
    }

    /// Fields, most significant first.
    pub fn fields(&self) -> &[Field] {
        &self.fields
    }

    pub fn len(&self) -> usize {
        self.fields.len()
    }

    pub fn is_empty(&self) -> bool {
        self.fields.is_empty()
    }

    fn check_word(&self, lanes: usize) -> Result<()> {
        if lanes == 0 || lanes > MAX_LANES || self.bits as usize > 64 * lanes {
            return Err(Error("packing exceeds the selected key word".into()));
        }
        Ok(())
    }

    /// Pack mathematical integer values (`i128` covers `u64`).
    pub fn pack(&self, values: &[i128], lanes: usize, ms_first: bool) -> Result<PackedKey> {
        self.check_word(lanes)?;
        if values.len() != self.len() {
            return Err(Error("tuple arity differs from packing".into()));
        }
        let mut word = [0; MAX_LANES];
        for (&field, &value) in self.fields.iter().zip(values) {
            insert(&mut word, field.shift, field.bits, field.encode(value)?);
        }
        Ok(PackedKey::from_wide(&word, lanes, ms_first))
    }

    /// Field `index` of `key`, decoded; no checks beyond the index.
    #[inline(always)]
    pub fn field(&self, key: &[u64], ms_first: bool, index: usize) -> i128 {
        let field = self.fields[index];
        let mut word = [0; MAX_LANES];
        for (slot, value) in word.iter_mut().enumerate().take(key.len()) {
            *value = lane(key, ms_first, slot);
        }
        field.decode(extract(&word, field.shift, field.bits))
    }

    /// Every field of `key`; keys with bits outside the schema are rejected.
    pub fn unpack(&self, key: &[u64], ms_first: bool) -> Result<Vec<i128>> {
        self.check_word(key.len())?;
        for index in self.bits as usize / 64..key.len() {
            let used = self.bits.saturating_sub(index as u32 * 64);
            if lane(key, ms_first, index) & !(mask(used) as u64) != 0 {
                return Err(Error("packed key has bits outside its schema".into()));
            }
        }
        Ok((0..self.len())
            .map(|index| self.field(key, ms_first, index))
            .collect())
    }

    fn range(
        &self,
        prefix: impl ExactSizeIterator<Item = i128>,
        lanes: usize,
        ms_first: bool,
    ) -> Result<Option<RangeInclusive<PackedKey>>> {
        self.check_word(lanes)?;
        if prefix.len() > self.len() {
            return Err(Error("prefix is longer than the tuple schema".into()));
        }
        let mut low = [0; MAX_LANES];
        let mut remaining = self.bits;
        for (&field, value) in self.fields.iter().zip(prefix) {
            let Ok(code) = field.encode(value) else {
                return Ok(None);
            };
            insert(&mut low, field.shift, field.bits, code);
            remaining = field.shift;
        }
        let mut high = low;
        for (index, word) in high.iter_mut().enumerate() {
            let bits = remaining.saturating_sub(index as u32 * 64).min(64);
            if bits != 0 {
                *word |= u64::MAX >> (64 - bits);
            }
        }
        Ok(Some(
            PackedKey::from_wide(&low, lanes, ms_first)
                ..=PackedKey::from_wide(&high, lanes, ms_first),
        ))
    }

    /// Inclusive key interval of the leading fields bound to `prefix` (every
    /// schema key when empty); no successor of the maximum is formed.
    pub fn prefix_range(
        &self,
        prefix: &[i128],
        lanes: usize,
        ms_first: bool,
    ) -> Result<RangeInclusive<PackedKey>> {
        self.range(prefix.iter().copied(), lanes, ms_first)?
            .ok_or_else(|| Error("tuple value exceeds column domain".into()))
    }

    /// CUDA `device::encode_prefix`: the interval of the leading fields bound
    /// to `values`, each an i64 bit pattern (a 64-bit unsigned field reads it
    /// as u64). `None` when a value lies outside its field, so no key matches.
    /// Panics when `values` is longer than the schema or the word too narrow.
    pub fn encode_prefix(
        &self,
        values: &[i64],
        lanes: usize,
        ms_first: bool,
    ) -> Option<RangeInclusive<PackedKey>> {
        let prefix = self.fields.iter().zip(values).map(|(field, &value)| {
            if field.bits == 64 && !field.signed {
                i128::from(value as u64)
            } else {
                i128::from(value)
            }
        });
        assert_eq!(prefix.len(), values.len(), "prefix longer than schema");
        self.range(prefix, lanes, ms_first)
            .expect("packing fits the key word")
    }
}

struct Publication {
    snapshot: Rc<WordRunSnapshot>,
    // Constant per packing: republished successors share it.
    packing: Rc<Packing>,
    params: WordRunParams,
    item_types: Rc<[ItemType]>,
}

/// An immutable publication of a [`WordRunSet`] typed by key tuple `K` and
/// value tuple `V` (CUDA formal rule GPU-RadixLayout). Reads borrow the
/// snapshot's shared arrays; clones share the publication.
pub struct WordLayout<K, V> {
    publication: Rc<Publication>,
    marker: PhantomData<fn() -> (K, V)>,
}

impl<K, V> Clone for WordLayout<K, V> {
    fn clone(&self) -> Self {
        Self {
            publication: Rc::clone(&self.publication),
            marker: PhantomData,
        }
    }
}

impl<K, V> fmt::Debug for WordLayout<K, V> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("WordLayout")
            .field("snapshot", &self.snapshot_id())
            .field("packing", self.packing())
            .field("runs", &self.publication.snapshot.run_lengths())
            .finish_non_exhaustive()
    }
}

impl<K: ScalarTuple, V: ScalarTuple> WordLayout<K, V> {
    /// Native-width packing of `K`'s columns in order.
    pub fn derive_packing() -> Result<Packing> {
        Packing::with_columns(K::COLUMNS)
    }

    /// Packing of loaded domain widths within `K`'s columns.
    pub fn derive_packing_with_fields(fields: &[(u32, bool)]) -> Result<Packing> {
        let packing = Packing::with_fields(fields)?;
        Self::check_key(&packing)?;
        Ok(packing)
    }

    fn check_key(packing: &Packing) -> Result<()> {
        if packing.len() != K::COLUMNS.len()
            || !packing
                .fields()
                .iter()
                .zip(K::COLUMNS)
                .all(|(field, column)| {
                    field.bits <= column.bits() && field.signed == column.is_signed()
                })
        {
            return Err(Error(
                "layout key fields must match the key schema's arity and signedness and fit its columns"
                    .into(),
            ));
        }
        Ok(())
    }

    /// Check `packing` against `K`, the run's key word and sort window, and
    /// the item lanes against `V`.
    pub fn check_schema(
        packing: &Packing,
        params: WordRunParams,
        item_types: &[ItemType],
    ) -> Result<()> {
        Self::check_key(packing)?;
        if packing.bits() as usize > params.lanes * 64 || packing.bits() > params.active_bits {
            return Err(Error(
                "layout key packing exceeds the key word or its sorted bits".into(),
            ));
        }
        if item_types.len() != V::COLUMNS.len()
            || !item_types
                .iter()
                .zip(V::COLUMNS)
                .all(|(&kind, &column)| LaneKind::of(kind).holds(column))
        {
            return Err(Error(
                "layout item lanes do not hold its value tuple schema".into(),
            ));
        }
        Ok(())
    }

    /// Publish `set`'s current snapshot with key `packing`.
    pub fn publish(set: &WordRunSet, packing: Packing) -> Result<Self> {
        Self::check_schema(&packing, set.params(), set.item_types())?;
        Ok(Self {
            publication: Rc::new(Publication {
                snapshot: set.snapshot(),
                packing: Rc::new(packing),
                params: set.params(),
                item_types: set.item_types().into(),
            }),
            marker: PhantomData,
        })
    }

    /// The successor over `set`'s current snapshot: this layout itself when
    /// the snapshot is unchanged, otherwise a publication sharing this
    /// layout's packing. `set` must have this layout's params and lanes.
    pub fn republish(&self, set: &WordRunSet) -> Result<Self> {
        let publication = &self.publication;
        if set.params() != publication.params || set.item_types() != &*publication.item_types {
            return Err(Error(
                "republished word run set has other params or item lanes".into(),
            ));
        }
        let snapshot = set.snapshot();
        if Rc::ptr_eq(&snapshot, &publication.snapshot) {
            return Ok(self.clone());
        }
        Ok(Self {
            publication: Rc::new(Publication {
                snapshot,
                packing: Rc::clone(&publication.packing),
                params: publication.params,
                item_types: Rc::clone(&publication.item_types),
            }),
            marker: PhantomData,
        })
    }
}

/// One value column of a publication, borrowed per run in stable order.
#[derive(Clone, Debug)]
pub struct ItemColumnView<'a, T> {
    runs: Vec<&'a [T]>,
    snapshot: u64,
    column: usize,
}

impl<'a, T: Copy> ItemColumnView<'a, T> {
    pub fn snapshot_id(&self) -> u64 {
        self.snapshot
    }

    pub fn column(&self) -> usize {
        self.column
    }

    pub fn run_count(&self) -> usize {
        self.runs.len()
    }

    /// Run `run`'s column, indexed by that run's item positions.
    pub fn run(&self, run: usize) -> &'a [T] {
        self.runs[run]
    }

    #[inline(always)]
    pub fn get(&self, run: usize, position: usize) -> T {
        self.runs[run][position]
    }
}

impl<K, V> WordLayout<K, V> {
    pub fn snapshot(&self) -> &Rc<WordRunSnapshot> {
        &self.publication.snapshot
    }

    pub fn snapshot_id(&self) -> u64 {
        self.publication.snapshot.id()
    }

    pub fn packing(&self) -> &Packing {
        &self.publication.packing
    }

    pub fn params(&self) -> WordRunParams {
        self.publication.params
    }

    pub fn item_types(&self) -> &[ItemType] {
        &self.publication.item_types
    }

    pub fn run_count(&self) -> usize {
        self.publication.snapshot.run_count()
    }

    /// Occurrences across all runs.
    pub fn len(&self) -> usize {
        self.publication.snapshot.item_count()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Borrowed run views, oldest first, for [`crate::word_read`] reads.
    pub fn views(&self) -> Vec<WordRunView<'_>> {
        self.publication
            .snapshot
            .runs()
            .iter()
            .map(WordRunView::from)
            .collect()
    }

    /// Value column `column`, whose lane type must be exactly `T`.
    pub fn item_column<T: ItemLane>(&self, column: usize) -> Result<ItemColumnView<'_, T>> {
        match self.publication.item_types.get(column) {
            Some(&kind) if kind == T::TYPE => {}
            Some(_) => {
                return Err(Error(
                    "item column lane differs from the requested element type".into(),
                ));
            }
            None => return Err(Error("item column outside the value tuple".into())),
        }
        let runs = self
            .publication
            .snapshot
            .runs()
            .iter()
            .map(|run| run.items::<T>(column))
            .collect::<Result<_>>()?;
        Ok(ItemColumnView {
            runs,
            snapshot: self.snapshot_id(),
            column,
        })
    }

    /// `values` packed in this layout's key word.
    pub fn pack(&self, values: &[i128]) -> Result<PackedKey> {
        let params = self.publication.params;
        self.packing().pack(values, params.lanes, params.ms_first)
    }

    /// The tuple of a key read from this layout.
    pub fn unpack(&self, key: &[u64]) -> Result<Vec<i128>> {
        self.packing().unpack(key, self.publication.params.ms_first)
    }

    /// Inclusive packed-key interval of a key prefix (all keys when empty).
    pub fn prefix_interval(&self, prefix: &[i128]) -> Result<RangeInclusive<PackedKey>> {
        let params = self.publication.params;
        self.packing()
            .prefix_range(prefix, params.lanes, params.ms_first)
    }

    /// [`Packing::encode_prefix`] in this layout's key word.
    pub fn encode_prefix(&self, values: &[i64]) -> Option<RangeInclusive<PackedKey>> {
        let params = self.publication.params;
        self.packing()
            .encode_prefix(values, params.lanes, params.ms_first)
    }
}
