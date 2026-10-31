//! Immutable scan blocks. Families are authored policies, never inferred from types.
// DESIGN contract8-scan-blocks
// STORAGE §§12–13: payload and optional extrema
// originate in one input pass. Bytes permits only <=16-bit residuals; Packed
// requires separate parallel evidence. Printers must not choose these policies.
use crate::OrderKey;
use std::{marker::PhantomData, num::NonZeroU32, ops::Bound};

pub const VECTOR_ROWS: usize = 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BlockRows(NonZeroU32);
impl BlockRows {
    pub fn new(rows: u32) -> Option<Self> {
        NonZeroU32::new(rows)
            .filter(|n| n.get() % VECTOR_ROWS as u32 == 0)
            .map(Self)
    }
    pub fn get(self) -> usize {
        self.0.get() as usize
    }
}

#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Width {
    W0 = 0,
    W8 = 1,
    W16 = 2,
    W32 = 4,
    W64 = 8,
}
#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CodecId {
    Plain = 0,
    For = 1,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FamilyId {
    Plain,
    Bytes,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Preserved {
    pub equality: bool,
    pub order: bool,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BlockHeader {
    pub offset: u64,
    pub base: u64,
    pub rows: u32,
    pub width: Width,
    pub codec: CodecId,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MinMax {
    pub lo: u64,
    pub hi: u64,
}

/// Inverse of the canonical order embedding on its image. WIDTH holds every
/// ordinal of this type; reconstruction is required only for encoded values.
pub trait OrdinalDecode: OrderKey<Ordinal = u64> + Copy {
    const WIDTH: Width;
    fn from_ordinal(ordinal: u64) -> Self;
}
impl OrdinalDecode for i64 {
    const WIDTH: Width = Width::W64;
    fn from_ordinal(o: u64) -> Self {
        (o ^ (1 << 63)) as i64
    }
}
impl OrdinalDecode for i32 {
    const WIDTH: Width = Width::W32;
    fn from_ordinal(o: u64) -> Self {
        (o as u32 ^ (1 << 31)) as i32
    }
}
impl OrdinalDecode for u32 {
    const WIDTH: Width = Width::W32;
    fn from_ordinal(o: u64) -> Self {
        o as u32
    }
}
impl OrdinalDecode for u64 {
    const WIDTH: Width = Width::W64;
    fn from_ordinal(o: u64) -> Self {
        o
    }
}
mod sealed {
    pub trait Sealed {}
}
pub trait Family: sealed::Sealed {
    const ID: FamilyId;
    const PRESERVES: Preserved = Preserved {
        equality: true,
        order: true,
    };
}
pub trait EncodedPoint: Family {}
pub trait EncodedWindow: Family {}
pub struct Plain;
pub struct Bytes;
impl sealed::Sealed for Plain {}
impl sealed::Sealed for Bytes {}
impl Family for Plain {
    const ID: FamilyId = FamilyId::Plain;
}
impl Family for Bytes {
    const ID: FamilyId = FamilyId::Bytes;
}
impl EncodedPoint for Plain {}
impl EncodedPoint for Bytes {}
impl EncodedWindow for Plain {}
impl EncodedWindow for Bytes {}
pub fn family<F: Family, T: OrdinalDecode>(_: &ScanColumn<T, F>) {}

/// One contiguous allocation with at least eight-byte alignment. Headers use
/// byte offsets; final-word padding is not part of the payload.
#[derive(Default)]
pub struct AlignedBytes {
    words: Vec<u64>,
    len: usize,
}
impl AlignedBytes {
    pub fn as_bytes(&self) -> &[u8] {
        // SAFETY: initialized u64 storage is valid as bytes; len excludes padding.
        unsafe { std::slice::from_raw_parts(self.words.as_ptr().cast(), self.len) }
    }
    fn append_block(&mut self, ordinals: &[u64], base: u64, width: Width) {
        if width == Width::W0 {
            return;
        }
        let size = ordinals
            .len()
            .checked_mul(width as usize)
            .expect("scan block overflow");
        let end = self.len.checked_add(size).expect("scan payload overflow");
        self.words.resize(end.div_ceil(8), 0);
        // SAFETY: resized initialized storage contains end bytes; exclusive borrow.
        let target =
            unsafe { std::slice::from_raw_parts_mut(self.words.as_mut_ptr().cast::<u8>(), end) };
        for (bytes, ordinal) in target[self.len..end]
            .chunks_exact_mut(width as usize)
            .zip(ordinals)
        {
            bytes.copy_from_slice(&(ordinal - base).to_le_bytes()[..width as usize]);
        }
        self.len = end;
    }
}

/// Formal rule RT-ScanCodec: concatenated vector decodes equal input occurrences.
pub struct ScanColumn<T: OrdinalDecode, F: Family> {
    rows: usize,
    block_rows: BlockRows,
    headers: Vec<BlockHeader>,
    payload: AlignedBytes,
    summary: Option<Vec<MinMax>>,
    marker: PhantomData<(T, F)>,
}
impl<T: OrdinalDecode, F: Family> ScanColumn<T, F> {
    pub fn new(values: &[T], block_rows: BlockRows, summaries: bool) -> Self {
        let blocks = values.len().div_ceil(block_rows.get());
        let mut result = Self {
            rows: values.len(),
            block_rows,
            headers: Vec::with_capacity(blocks),
            payload: AlignedBytes::default(),
            summary: summaries.then(|| Vec::with_capacity(blocks)),
            marker: PhantomData,
        };
        let mut scratch = Vec::with_capacity(values.len().min(block_rows.get()));
        for block in values.chunks(block_rows.get()) {
            scratch.clear();
            let mut extrema = MinMax {
                lo: u64::MAX,
                hi: 0,
            };
            // Formal rule RT-ScanSummary: extrema enclose exactly the payload's
            // input block; collect once, never summarize a decoded column copy.
            for value in block {
                let ordinal = value.ordinal();
                extrema.lo = extrema.lo.min(ordinal);
                extrema.hi = extrema.hi.max(ordinal);
                scratch.push(ordinal);
            }
            let span = extrema.hi - extrema.lo;
            let compressed = if span == 0 {
                Width::W0
            } else if span <= u8::MAX as u64 {
                Width::W8
            } else {
                Width::W16
            };
            let use_for = F::ID == FamilyId::Bytes
                && span <= u16::MAX as u64
                && (compressed as u8) < T::WIDTH as u8;
            let (base, width, codec) = if use_for {
                (extrema.lo, compressed, CodecId::For)
            } else {
                (0, T::WIDTH, CodecId::Plain)
            };
            result.headers.push(BlockHeader {
                offset: result.payload.len as u64,
                base,
                rows: block.len() as u32,
                width,
                codec,
            });
            result.payload.append_block(&scratch, base, width);
            if let Some(summary) = &mut result.summary {
                summary.push(extrema);
            }
        }
        result
    }
    pub fn len(&self) -> usize {
        self.rows
    }
    pub fn is_empty(&self) -> bool {
        self.rows == 0
    }
    pub fn block_rows(&self) -> BlockRows {
        self.block_rows
    }
    pub fn headers(&self) -> &[BlockHeader] {
        &self.headers
    }
    pub fn summaries(&self) -> Option<&[MinMax]> {
        self.summary.as_deref()
    }
    pub fn payload(&self) -> &[u8] {
        self.payload.as_bytes()
    }
    fn visit_codes(
        &self,
        h: &BlockHeader,
        start: usize,
        len: usize,
        mut visit: impl FnMut(usize, u64),
    ) {
        let begin = h.offset as usize + start * h.width as usize;
        let bytes = &self.payload()[begin..begin + len * h.width as usize];
        macro_rules! visit_width {
            ($width:literal, $ty:ty) => {
                for (i, chunk) in bytes.chunks_exact($width).enumerate() {
                    visit(i, <$ty>::from_le_bytes(chunk.try_into().unwrap()) as u64);
                }
            };
        }
        match h.width {
            Width::W0 => {
                for i in 0..len {
                    visit(i, 0);
                }
            }
            Width::W8 => {
                for (i, &code) in bytes.iter().enumerate() {
                    visit(i, code as u64);
                }
            }
            Width::W16 => {
                visit_width!(2, u16);
            }
            Width::W32 => {
                visit_width!(4, u32);
            }
            Width::W64 => {
                visit_width!(8, u64);
            }
        }
    }
    /// Decode at most one vector into caller-owned storage; returns its length.
    pub fn decode_vector(&self, block: usize, vector: usize, out: &mut [T]) -> usize {
        let h = &self.headers[block];
        let start = vector
            .checked_mul(VECTOR_ROWS)
            .expect("vector offset overflow");
        assert!(start <= h.rows as usize);
        let len = VECTOR_ROWS.min(h.rows as usize - start);
        assert!(out.len() >= len);
        self.visit_codes(h, start, len, |i, code| {
            out[i] = T::from_ordinal(h.base + code)
        });
        len
    }
    /// Write one selection byte per occurrence (0 or 1), without logical decode.
    /// Width and predicate dispatch occur once per block, outside the tight loop.
    pub fn filter_mask(&self, predicate: &Predicate, out: &mut [u8])
    where
        F: EncodedPoint + EncodedWindow,
    {
        assert_eq!(out.len(), self.rows);
        for (b, h) in self.headers.iter().enumerate() {
            let start = b * self.block_rows.get();
            let selected = &mut out[start..start + h.rows as usize];
            if self
                .summary
                .as_ref()
                .is_some_and(|s| !predicate.intersects(s[b].lo, s[b].hi))
            {
                selected.fill(0);
                continue;
            }
            let translated = predicate.translate(h);
            self.block_mask(h, &translated, selected);
        }
    }

    fn block_mask(&self, h: &BlockHeader, predicate: &TranslatedPredicate, out: &mut [u8]) {
        let bytes = &self.payload()
            [h.offset as usize..h.offset as usize + h.rows as usize * h.width as usize];
        macro_rules! range {
            ($width:literal, $ty:ty, $lo:expr, $hi:expr) => {{
                let lo = $lo as $ty;
                let hi = $hi as $ty;
                for (selected, code) in out.iter_mut().zip(bytes.chunks_exact($width)) {
                    let code = <$ty>::from_le_bytes(code.try_into().unwrap());
                    *selected = ((code >= lo) & (code <= hi)) as u8;
                }
            }};
        }
        match predicate {
            TranslatedPredicate::Range(CodeConstraint::Empty) => out.fill(0),
            TranslatedPredicate::Range(CodeConstraint::All) => out.fill(1),
            TranslatedPredicate::Range(CodeConstraint::Range { lo, hi }) => match h.width {
                Width::W0 => out.fill((*lo == 0) as u8),
                Width::W8 => range!(1, u8, *lo, *hi),
                Width::W16 => range!(2, u16, *lo, *hi),
                Width::W32 => range!(4, u32, *lo, *hi),
                Width::W64 => range!(8, u64, *lo, *hi),
            },
            TranslatedPredicate::Membership(keys) => {
                self.visit_codes(h, 0, h.rows as usize, |i, code| {
                    out[i] = keys.contains(&code) as u8
                });
            }
        }
    }

    /// Exact ascending occurrence positions without reconstructing logical values.
    pub fn filter(&self, predicate: &Predicate, mut emit: impl FnMut(usize))
    where
        F: EncodedPoint + EncodedWindow,
    {
        for (b, h) in self.headers.iter().enumerate() {
            if self
                .summary
                .as_ref()
                .is_some_and(|s| !predicate.intersects(s[b].lo, s[b].hi))
            {
                continue;
            }
            let translated = predicate.translate(h);
            let base = b * self.block_rows.get();
            // Hoist the predicate variant out of the occurrence loop as well as width.
            match translated {
                TranslatedPredicate::Range(CodeConstraint::Empty) => {}
                TranslatedPredicate::Range(CodeConstraint::All) => {
                    for row in 0..h.rows as usize {
                        emit(base + row);
                    }
                }
                TranslatedPredicate::Range(CodeConstraint::Range { lo, hi }) => {
                    self.visit_codes(h, 0, h.rows as usize, |row, code| {
                        if (lo <= code) & (code <= hi) {
                            emit(base + row);
                        }
                    });
                }
                TranslatedPredicate::Membership(keys) => {
                    self.visit_codes(h, 0, h.rows as usize, |row, code| {
                        if keys.contains(&code) {
                            emit(base + row);
                        }
                    });
                }
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CodeConstraint {
    Empty,
    All,
    Range { lo: u64, hi: u64 },
}
impl CodeConstraint {
    pub fn matches(&self, code: u64) -> bool {
        match *self {
            Self::Empty => false,
            Self::All => true,
            Self::Range { lo, hi } => lo <= code && code <= hi,
        }
    }
}
/// Constants already translated once on the host into canonical ordinals.
#[derive(Clone, Debug)]
pub enum Predicate {
    Point(u64),
    Window { lo: Bound<u64>, hi: Bound<u64> },
    Membership(Vec<u64>),
}
impl Predicate {
    fn range(&self) -> Option<(u64, u64)> {
        match self {
            Self::Point(c) => Some((*c, *c)),
            Self::Window { lo, hi } => {
                let lo = match *lo {
                    Bound::Included(v) => v,
                    Bound::Excluded(v) => v.checked_add(1)?,
                    Bound::Unbounded => 0,
                };
                let hi = match *hi {
                    Bound::Included(v) => v,
                    Bound::Excluded(v) => v.checked_sub(1)?,
                    Bound::Unbounded => u64::MAX,
                };
                (lo <= hi).then_some((lo, hi))
            }
            Self::Membership(_) => None,
        }
    }
    fn intersects(&self, lo: u64, hi: u64) -> bool {
        match self {
            Self::Membership(keys) => keys.iter().any(|k| lo <= *k && *k <= hi),
            _ => self.range().is_some_and(|(a, b)| a <= hi && lo <= b),
        }
    }
    /// Formal rule RT-Translate: residual tests iff the logical predicate holds,
    /// including absent constants and open endpoints at the ordinal extremes.
    pub fn translate(&self, h: &BlockHeader) -> TranslatedPredicate {
        let max = match h.width {
            Width::W0 => 0,
            Width::W8 => u8::MAX as u64,
            Width::W16 => u16::MAX as u64,
            Width::W32 => u32::MAX as u64,
            Width::W64 => u64::MAX,
        };
        let hi = h.base.saturating_add(max);
        let translate_range = |lo: u64, upper: u64| {
            if lo > hi || upper < h.base {
                CodeConstraint::Empty
            } else if lo <= h.base && upper >= hi {
                CodeConstraint::All
            } else {
                CodeConstraint::Range {
                    lo: lo.saturating_sub(h.base),
                    hi: upper.min(hi) - h.base,
                }
            }
        };
        match self {
            Self::Membership(keys) => TranslatedPredicate::Membership(
                keys.iter()
                    .filter_map(|k| k.checked_sub(h.base).filter(|v| *v <= max))
                    .collect(),
            ),
            _ => TranslatedPredicate::Range(
                self.range()
                    .map_or(CodeConstraint::Empty, |(lo, hi)| translate_range(lo, hi)),
            ),
        }
    }
}
#[derive(Clone, Debug)]
pub enum TranslatedPredicate {
    Range(CodeConstraint),
    Membership(Vec<u64>),
}
impl TranslatedPredicate {
    pub fn matches(&self, code: u64) -> bool {
        match self {
            Self::Range(r) => r.matches(code),
            Self::Membership(keys) => keys.contains(&code),
        }
    }
}
