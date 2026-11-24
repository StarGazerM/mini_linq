//! Immutable scan blocks. Families are authored policies, never inferred from types.
// DESIGN contract8-scan-blocks
// STORAGE §§12–13: payload and optional extrema
// originate in one input pass. Bytes permits only <=16-bit residuals; Packed
// requires separate parallel evidence. Printers must not choose these policies.
use crate::OrderKey;
use std::{marker::PhantomData, num::NonZeroU32, ops::Bound};

pub const VECTOR_ROWS: usize = 1024;

/// Summary use and physical testing are independent, explicitly selected facts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PruneMode {
    None,
    BlockMinMax,
    VectorMinMax,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TestMode {
    Encoded,
    /// Reconstruct the ordinal carrier, never an authored value or string.
    DecodeToCode,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Classification {
    Disqualified,
    Qualified,
    Ambivalent,
}
/// The checked compiler read record names the consumer of a logical value.
/// Carrier reads require no such reconstruction authorization.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LogicalRead {
    OpaqueRust { consumer: &'static str },
    FinalAnswer { consumer: &'static str },
}

#[path = "scan_format.rs"]
pub mod format;

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
    /// Initialized ABI padding; byte uploads never read Rust padding.
    pub reserved: [u8; 2],
}
#[repr(C)]
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
    vector_summary: Option<Vec<MinMax>>,
    snapshot: format::SnapshotId,
    declared_domain: Option<format::Domain>,
    marker: PhantomData<(T, F)>,
}
/// Incremental column encoder. Only one block of ordinals is buffered; extrema
/// are collected with input, never reconstructed from the encoded payload.
pub struct ScanColumnBuilder<T: OrdinalDecode, F: Family> {
    column: ScanColumn<T, F>,
    tail: Vec<u64>,
    extrema: MinMax,
}

impl<T: OrdinalDecode, F: Family> ScanColumnBuilder<T, F> {
    pub fn new(block_rows: BlockRows, summaries: bool) -> Self {
        Self::with_summaries(block_rows, summaries, false)
    }

    /// Bind the provider domain before accepting values. Legacy CPU-only
    /// builders remain unbound and cannot expose a shared device descriptor.
    pub fn with_domain(
        block_rows: BlockRows,
        block_summaries: bool,
        vector_summaries: bool,
        domain: format::Domain,
    ) -> Result<Self, format::FormatError> {
        let mut builder = Self::with_summaries(block_rows, block_summaries, vector_summaries);
        builder.column.declared_domain = Some(domain);
        builder.column.checked_view(domain, 0)?;
        Ok(builder)
    }

    /// Select summaries explicitly. Vector extrema are collected with the
    /// ordinals, including when a block contains several vectors.
    pub fn with_summaries(
        block_rows: BlockRows,
        block_summaries: bool,
        vector_summaries: bool,
    ) -> Self {
        Self {
            column: ScanColumn {
                rows: 0,
                block_rows,
                headers: Vec::new(),
                payload: AlignedBytes::default(),
                summary: block_summaries.then(Vec::new),
                vector_summary: vector_summaries.then(Vec::new),
                snapshot: format::SnapshotId::fresh(),
                declared_domain: None,
                marker: PhantomData,
            },
            tail: Vec::with_capacity(block_rows.get()),
            extrema: MinMax {
                lo: u64::MAX,
                hi: 0,
            },
        }
    }

    pub fn push(&mut self, value: T) {
        self.push_ordinal(value.ordinal());
    }

    fn push_ordinal(&mut self, ordinal: u64) {
        self.extrema.lo = self.extrema.lo.min(ordinal);
        self.extrema.hi = self.extrema.hi.max(ordinal);
        if let Some(summaries) = &mut self.column.vector_summary {
            if self.column.rows % VECTOR_ROWS == 0 {
                summaries.push(MinMax {
                    lo: ordinal,
                    hi: ordinal,
                });
            } else {
                let summary = summaries.last_mut().unwrap();
                summary.lo = summary.lo.min(ordinal);
                summary.hi = summary.hi.max(ordinal);
            }
        }
        self.tail.push(ordinal);
        self.column.rows += 1;
        if self.tail.len() == self.column.block_rows.get() {
            self.flush();
        }
    }

    pub fn extend(&mut self, values: impl IntoIterator<Item = T>) {
        for value in values {
            self.push(value);
        }
    }

    pub fn finish(mut self) -> ScanColumn<T, F> {
        self.flush();
        self.column
    }

    fn flush(&mut self) {
        if self.tail.is_empty() {
            return;
        }
        let span = self.extrema.hi - self.extrema.lo;
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
            (self.extrema.lo, compressed, CodecId::For)
        } else {
            (0, T::WIDTH, CodecId::Plain)
        };
        self.column.headers.push(BlockHeader {
            offset: self.column.payload.len as u64,
            base,
            rows: self.tail.len() as u32,
            width,
            codec,
            reserved: [0; 2],
        });
        self.column.payload.append_block(&self.tail, base, width);
        // Formal rule RT-ScanSummary: extrema enclose exactly the payload's
        // input block; collect once, never summarize a decoded column copy.
        if let Some(summary) = &mut self.column.summary {
            summary.push(self.extrema);
        }
        self.tail.clear();
        self.extrema = MinMax {
            lo: u64::MAX,
            hi: 0,
        };
    }
}

impl<T: OrdinalDecode, F: Family> ScanColumn<T, F> {
    pub fn new(values: &[T], block_rows: BlockRows, summaries: bool) -> Self {
        let mut builder = ScanColumnBuilder::new(block_rows, summaries);
        builder.extend(values.iter().copied());
        builder.finish()
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
    pub fn vector_summaries(&self) -> Option<&[MinMax]> {
        self.vector_summary.as_deref()
    }

    pub fn snapshot(&self) -> format::SnapshotId {
        self.snapshot
    }

    /// Bind an explicit provider declaration at publication without touching
    /// payload or changing occurrence identity.
    pub fn bind_domain(mut self, domain: format::Domain) -> Result<Self, format::FormatError> {
        if domain.carrier != T::WIDTH
            || domain.id.namespace == 0
            || !domain.equality
            || self.declared_domain.is_some_and(|bound| bound != domain)
        {
            return Err(format::FormatError::Domain);
        }
        self.declared_domain = Some(domain);
        Ok(self)
    }

    pub fn declared_domain(&self) -> Option<format::Domain> {
        self.declared_domain
    }

    /// Read exactly one field's physical carrier, not its logical value.
    pub fn read_ordinal(&self, position: usize) -> u64 {
        assert!(position < self.rows, "scan position outside column");
        let h = &self.headers[position / self.block_rows.get()];
        let mut ordinal = 0;
        self.visit_codes(h, position % self.block_rows.get(), 1, |_, code| {
            ordinal = h.base + code;
        });
        ordinal
    }

    fn select_rows(self, positions: &[usize]) -> Self {
        let mut builder = ScanColumnBuilder::with_summaries(
            self.block_rows,
            self.summary.is_some(),
            self.vector_summary.is_some(),
        );
        builder.column.declared_domain = self.declared_domain;
        for &position in positions {
            builder.push_ordinal(self.read_ordinal(position));
        }
        builder.finish()
    }

    /// Intersect a bounded vector in-place. No other vector's payload is read.
    /// Each pruning mode requires its declared buffer; `None` never reads summaries.
    pub fn intersect_mask(
        &self,
        start: usize,
        out: &mut [u8],
        predicate: &Predicate,
        prune: PruneMode,
        test: TestMode,
    ) -> Classification
    where
        F: EncodedPoint + EncodedWindow,
    {
        assert!(start <= self.rows && out.len() <= self.rows - start);
        assert!(
            out.len() <= VECTOR_ROWS
                && (start % VECTOR_ROWS == 0 || (out.is_empty() && start == self.rows))
        );
        if let Some(domain) = self.declared_domain {
            assert!(
                !matches!(predicate, Predicate::Window { .. }) || domain.order,
                "ordered scan predicate requires an ordered domain"
            );
        }
        // Validate even empty/dead masks at the typed boundary. Authored None
        // specializes away here instead of branching in every shared kernel call.
        match prune {
            PruneMode::None => {}
            PruneMode::BlockMinMax => assert!(self.summary.is_some(), "declared block summaries"),
            PruneMode::VectorMinMax => assert!(self.vector_summary.is_some(), "declared vector summaries"),
        }
        intersect_vector(
            &self.headers,
            self.payload(),
            self.summary.as_deref(),
            self.vector_summary.as_deref(),
            self.block_rows,
            start,
            out,
            predicate,
            prune,
            test,
        )
    }

    /// Check the encode-time declared ordinal domain, without inspecting a Rust type.
    pub fn checked_view(
        &self,
        domain: format::Domain,
        live_count: usize,
    ) -> Result<format::CheckedScanView<'_>, format::FormatError> {
        if domain.carrier != T::WIDTH || self.declared_domain != Some(domain) {
            return Err(format::FormatError::Domain);
        }
        let descriptor = format::BlockFormat {
            version: 1,
            layout: format::LayoutId::S1LittleEndian,
            family: F::ID,
            domain,
            preservation: F::PRESERVES,
            block_rows: self.block_rows,
            vector_rows: VECTOR_ROWS as u32,
        };
        // Trusted encoder output needs only descriptor/extent checks; imported
        // bytes use CheckedScanView::import, which also checks payload integrity.
        format::CheckedScanView::encoded(
            descriptor,
            self.snapshot,
            &self.headers,
            self.payload(),
            self.summaries(),
            self.vector_summaries(),
            self.rows,
            live_count,
        )
    }
    fn visit_codes(
        &self,
        h: &BlockHeader,
        start: usize,
        len: usize,
        visit: impl FnMut(usize, u64),
    ) {
        visit_codes(self.payload(), h, start, len, visit);
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
    /// Membership borrows the predicate's ordinal keys; no block-local key
    /// buffer is allocated.
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

    fn block_mask(&self, h: &BlockHeader, predicate: &TranslatedPredicate<'_>, out: &mut [u8]) {
        out.fill(1);
        intersect_codes(self.payload(), h, 0, predicate, out);
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
                TranslatedPredicate::Membership {
                    keys,
                    base: ordinal_base,
                    ..
                } => {
                    self.visit_codes(h, 0, h.rows as usize, |row, code| {
                        if keys.contains(&(ordinal_base + code)) {
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
    pub fn matches(&self, ordinal: u64) -> bool {
        match self {
            Self::Membership(keys) => keys.contains(&ordinal),
            _ => self.range().is_some_and(|(lo, hi)| lo <= ordinal && ordinal <= hi),
        }
    }

    pub fn classify(&self, bounds: MinMax) -> Classification {
        if !self.intersects(bounds.lo, bounds.hi) {
            Classification::Disqualified
        } else if match self {
            Self::Membership(keys) => bounds.lo == bounds.hi && keys.contains(&bounds.lo),
            _ => self.range().is_some_and(|(lo, hi)| lo <= bounds.lo && bounds.hi <= hi),
        } {
            Classification::Qualified
        } else {
            Classification::Ambivalent
        }
    }
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
    /// Membership retains borrowed global ordinals and the block frame instead
    /// of allocating a residual-key vector for each block.
    pub fn translate(&self, h: &BlockHeader) -> TranslatedPredicate<'_> {
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
            Self::Membership(keys) => TranslatedPredicate::Membership {
                keys,
                base: h.base,
                max,
            },
            _ => TranslatedPredicate::Range(
                self.range()
                    .map_or(CodeConstraint::Empty, |(lo, hi)| translate_range(lo, hi)),
            ),
        }
    }
}
/// A block-relative test borrowing the predicate's membership keys.
#[derive(Clone, Debug)]
pub enum TranslatedPredicate<'a> {
    Range(CodeConstraint),
    Membership {
        keys: &'a [u64],
        base: u64,
        max: u64,
    },
}
impl TranslatedPredicate<'_> {
    pub fn matches(&self, code: u64) -> bool {
        match self {
            Self::Range(r) => r.matches(code),
            Self::Membership { keys, base, max } => {
                code <= *max
                    && base
                        .checked_add(code)
                        .is_some_and(|key| keys.contains(&key))
            }
        }
    }
}

/// The single S1 width decoder shared by whole-column, vector and point reads.
fn visit_codes(
    payload: &[u8],
    h: &BlockHeader,
    start: usize,
    len: usize,
    mut visit: impl FnMut(usize, u64),
) {
    let begin = h.offset as usize + start * h.width as usize;
    let bytes = &payload[begin..begin + len * h.width as usize];
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
        Width::W16 => visit_width!(2, u16),
        Width::W32 => visit_width!(4, u32),
        Width::W64 => visit_width!(8, u64),
    }
}

/// S1's range kernel, shared with the compatibility whole-column mask API.
fn intersect_codes(
    payload: &[u8],
    h: &BlockHeader,
    start: usize,
    predicate: &TranslatedPredicate<'_>,
    out: &mut [u8],
) {
    macro_rules! range {
        ($width:literal, $ty:ty, $lo:expr, $hi:expr) => {{
            let lo = $lo as $ty;
            let hi = $hi as $ty;
            let begin = h.offset as usize + start * $width;
            let bytes = &payload[begin..begin + out.len() * $width];
            for (selected, code) in out.iter_mut().zip(bytes.chunks_exact($width)) {
                let code = <$ty>::from_le_bytes(code.try_into().unwrap());
                *selected &= ((code >= lo) & (code <= hi)) as u8;
            }
        }};
    }
    match predicate {
        TranslatedPredicate::Range(CodeConstraint::Empty) => out.fill(0),
        TranslatedPredicate::Range(CodeConstraint::All) => {}
        TranslatedPredicate::Range(CodeConstraint::Range { lo, hi }) => match h.width {
            Width::W0 => {
                if *lo != 0 {
                    out.fill(0);
                }
            }
            Width::W8 => range!(1, u8, *lo, *hi),
            Width::W16 => range!(2, u16, *lo, *hi),
            Width::W32 => range!(4, u32, *lo, *hi),
            Width::W64 => range!(8, u64, *lo, *hi),
        },
        TranslatedPredicate::Membership { keys, base, .. } => {
            visit_codes(payload, h, start, out.len(), |i, code| {
                out[i] &= keys.contains(&(*base + code)) as u8;
            });
        }
    }
}

fn intersect_vector(
    headers: &[BlockHeader],
    payload: &[u8],
    summaries: Option<&[MinMax]>,
    vector_summaries: Option<&[MinMax]>,
    block_rows: BlockRows,
    start: usize,
    out: &mut [u8],
    predicate: &Predicate,
    prune: PruneMode,
    test: TestMode,
) -> Classification {
    if out.is_empty() || !out.iter().any(|&live| live != 0) {
        return Classification::Disqualified;
    }
    let block = start / block_rows.get();
    if prune != PruneMode::None {
        let bounds = match prune {
            PruneMode::None => unreachable!(),
            PruneMode::BlockMinMax => summaries.expect("declared block summaries")[block],
            PruneMode::VectorMinMax => vector_summaries.expect("declared vector summaries")[start / VECTOR_ROWS],
        };
        match predicate.classify(bounds) {
            Classification::Disqualified => {
                out.fill(0);
                return Classification::Disqualified;
            }
            Classification::Qualified => return Classification::Qualified,
            Classification::Ambivalent => {}
        }
    }
    let h = &headers[block];
    let local = start % block_rows.get();
    match test {
        TestMode::Encoded => intersect_codes(payload, h, local, &predicate.translate(h), out),
        TestMode::DecodeToCode => match predicate {
            Predicate::Membership(keys) => {
                visit_codes(payload, h, local, out.len(), |i, code| {
                    out[i] &= keys.contains(&(h.base + code)) as u8;
                });
            }
            _ => match predicate.range() {
                None => out.fill(0),
                Some((lo, hi)) => {
                    visit_codes(payload, h, local, out.len(), |i, code| {
                        let ordinal = h.base + code;
                        out[i] &= ((lo <= ordinal) & (ordinal <= hi)) as u8;
                    });
                }
            },
        },
    }
    Classification::Ambivalent
}

/// Tuple metadata used to enforce a shared row count and block partition.
pub trait ScanColumns {
    fn layout(&self) -> (usize, BlockRows);
    fn compare_rows(&self, left: usize, right: usize) -> std::cmp::Ordering;
    fn select_rows(self, positions: &[usize]) -> Self
    where
        Self: Sized;
    fn bind_domains(self, domains: &[format::Domain]) -> Result<Self, format::FormatError>
    where
        Self: Sized;
}

/// Authored schema-slot access carries both the logical field type and its
/// selected family. It does not supply a borrowed logical row adapter.
pub trait ScanSlot<const SLOT: usize>: ScanColumns {
    type Value: OrdinalDecode;
    type Family: Family;
    fn column(&self) -> &ScanColumn<Self::Value, Self::Family>;
}

macro_rules! scan_slots {
    ([$($ty:ident $family:ident $slot:tt),+];) => {};
    ([$($ty:ident $family:ident $slot:tt),+]; $value:ident $selected:ident $index:tt $(, $rest:ident $other:ident $next:tt)*) => {
        impl<$($ty: OrdinalDecode, $family: Family),+> ScanSlot<$index>
            for ($(ScanColumn<$ty, $family>,)+)
        {
            type Value = $value;
            type Family = $selected;
            fn column(&self) -> &ScanColumn<Self::Value, Self::Family> {
                &self.$index
            }
        }
        scan_slots!([$($ty $family $slot),+]; $($rest $other $next),*);
    };
}

macro_rules! scan_columns {
    ($first:ident $family:ident $index:tt $(, $ty:ident $other:ident $slot:tt)*) => {
        impl<$first: OrdinalDecode, $family: Family, $($ty: OrdinalDecode, $other: Family),*>
            ScanColumns for (ScanColumn<$first, $family>, $(ScanColumn<$ty, $other>,)*)
        {
            fn layout(&self) -> (usize, BlockRows) {
                let rows = self.$index.len();
                let block_rows = self.$index.block_rows();
                $(
                    assert_eq!(self.$slot.len(), rows, "scan columns have different row counts");
                    assert_eq!(self.$slot.block_rows(), block_rows, "scan columns have different block sizes");
                )*
                (rows, block_rows)
            }
            fn compare_rows(&self, left: usize, right: usize) -> std::cmp::Ordering {
                self.$index.read_ordinal(left).cmp(&self.$index.read_ordinal(right))
                    $(.then_with(|| self.$slot.read_ordinal(left).cmp(&self.$slot.read_ordinal(right))))*
            }
            fn select_rows(self, positions: &[usize]) -> Self {
                (
                    self.$index.select_rows(positions),
                    $(self.$slot.select_rows(positions),)*
                )
            }
            fn bind_domains(self, domains: &[format::Domain]) -> Result<Self, format::FormatError> {
                if domains.len() != 1 $(+ { let _ = $slot; 1 })* {
                    return Err(format::FormatError::Domain);
                }
                Ok((
                    self.$index.bind_domain(domains[$index])?,
                    $(self.$slot.bind_domain(domains[$slot])?,)*
                ))
            }
        }
        scan_slots!([$first $family $index $(, $ty $other $slot)*]; $first $family $index $(, $ty $other $slot)*);
    };
}

scan_columns!(T0 F0 0);
scan_columns!(T0 F0 0, T1 F1 1);
scan_columns!(T0 F0 0, T1 F1 1, T2 F2 2);
scan_columns!(T0 F0 0, T1 F1 1, T2 F2 2, T3 F3 3);
scan_columns!(T0 F0 0, T1 F1 1, T2 F2 2, T3 F3 3, T4 F4 4);
scan_columns!(T0 F0 0, T1 F1 1, T2 F2 2, T3 F3 3, T4 F4 4, T5 F5 5);
scan_columns!(T0 F0 0, T1 F1 1, T2 F2 2, T3 F3 3, T4 F4 4, T5 F5 5, T6 F6 6);
scan_columns!(T0 F0 0, T1 F1 1, T2 F2 2, T3 F3 3, T4 F4 4, T5 F5 5, T6 F6 6, T7 F7 7);
scan_columns!(T0 F0 0, T1 F1 1, T2 F2 2, T3 F3 3, T4 F4 4, T5 F5 5, T6 F6 6, T7 F7 7, T8 F8 8);
scan_columns!(T0 F0 0, T1 F1 1, T2 F2 2, T3 F3 3, T4 F4 4, T5 F5 5, T6 F6 6, T7 F7 7, T8 F8 8, T9 F9 9);
scan_columns!(T0 F0 0, T1 F1 1, T2 F2 2, T3 F3 3, T4 F4 4, T5 F5 5, T6 F6 6, T7 F7 7, T8 F8 8, T9 F9 9, T10 F10 10);
scan_columns!(T0 F0 0, T1 F1 1, T2 F2 2, T3 F3 3, T4 F4 4, T5 F5 5, T6 F6 6, T7 F7 7, T8 F8 8, T9 F9 9, T10 F10 10, T11 F11 11);

/// Immutable columns sharing one occurrence order and block partition.
pub struct ScanTable<C> {
    columns: C,
    rows: usize,
    block_rows: BlockRows,
    snapshot: format::SnapshotId,
}

impl<C: ScanColumns> ScanTable<C> {
    pub fn new(columns: C) -> Self {
        let (rows, block_rows) = columns.layout();
        Self {
            columns,
            rows,
            block_rows,
            snapshot: format::SnapshotId::fresh(),
        }
    }

    /// Formal rule LD-ColumnSet: retain one complete encoded row per tuple.
    /// A sorted-unique input returns unchanged without allocating. Otherwise
    /// only row positions are sorted; columns are rebuilt only if duplicates
    /// exist, without decoding logical tuples or disturbing distinct inputs.
    pub fn into_set(self) -> Self {
        if (1..self.rows).all(|row| {
            self.columns.compare_rows(row - 1, row) == std::cmp::Ordering::Less
        }) {
            return self;
        }
        let mut positions: Vec<_> = (0..self.rows).collect();
        positions.sort_unstable_by(|&left, &right| self.columns.compare_rows(left, right));
        positions.dedup_by(|left, right| {
            self.columns.compare_rows(*left, *right) == std::cmp::Ordering::Equal
        });
        if positions.len() == self.rows {
            return self;
        }
        Self::new(self.columns.select_rows(&positions))
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

    pub fn columns(&self) -> &C {
        &self.columns
    }

    pub fn into_columns(self) -> C {
        self.columns
    }

    pub fn snapshot(&self) -> format::SnapshotId {
        self.snapshot
    }

    /// Bind once at loader publication. Domain declarations remain attached to
    /// the same immutable columns and positions, without copying their payload.
    pub fn bind_domains(mut self, domains: &[format::Domain]) -> Result<Self, format::FormatError> {
        self.columns = self.columns.bind_domains(domains)?;
        Ok(self)
    }

    pub fn column<const SLOT: usize>(
        &self,
    ) -> &ScanColumn<<C as ScanSlot<SLOT>>::Value, <C as ScanSlot<SLOT>>::Family>
    where
        C: ScanSlot<SLOT>,
    {
        <C as ScanSlot<SLOT>>::column(&self.columns)
    }

    pub fn encode_operand<const SLOT: usize>(&self, value: &<C as ScanSlot<SLOT>>::Value) -> u64
    where
        C: ScanSlot<SLOT>,
    {
        value.ordinal()
    }

    fn check_position(&self, position: ScanPosition<'_>) {
        assert_eq!(position.snapshot, self.snapshot, "scan position belongs to another table");
        assert!(position.index < self.rows, "scan position outside table");
    }

    pub fn read_ordinal<const SLOT: usize>(&self, position: ScanPosition<'_>) -> u64
    where
        C: ScanSlot<SLOT>,
    {
        self.check_position(position);
        self.column::<SLOT>().read_ordinal(position.index)
    }

    /// Reconstruct only the demanded field at its authorized consumer boundary.
    pub fn read_value<const SLOT: usize>(
        &self,
        position: ScanPosition<'_>,
        authorization: LogicalRead,
    ) -> <C as ScanSlot<SLOT>>::Value
    where
        C: ScanSlot<SLOT>,
    {
        let consumer = match authorization {
            LogicalRead::OpaqueRust { consumer } | LogicalRead::FinalAnswer { consumer } => consumer,
        };
        assert!(!consumer.is_empty(), "logical read requires its recorded consumer");
        // DECODE ?: ScanTable::read_value is a generic field inverse; caller records must authorize the named opaque Rust or final-answer consumer.
        <C as ScanSlot<SLOT>>::Value::from_ordinal(self.read_ordinal::<SLOT>(position))
    }

    pub fn tile<'table, 'mask>(
        &'table self,
        start: usize,
        vector_rows: usize,
        mask: &'mask mut ScanMask,
    ) -> ScanTile<'table, 'mask, C> {
        assert_eq!(vector_rows, VECTOR_ROWS, "supported provider vector geometry");
        assert!(start <= self.rows && (start % VECTOR_ROWS == 0 || start == self.rows));
        let len = VECTOR_ROWS.min(self.rows - start);
        mask.bytes[..len].fill(1);
        ScanTile { table: self, mask, start, len }
    }

    /// Lazy source scan, retaining one reusable vector mask and its source.
    /// Tasks pass disjoint aligned ranges; the last range may end at table len.
    pub fn scan_positions<F>(
        &self,
        range: std::ops::Range<usize>,
        vector_rows: usize,
        configure: F,
    ) -> ScanPositions<'_, C, F>
    where
        F: FnMut(&mut ScanTile<'_, '_, C>),
    {
        assert_eq!(vector_rows, VECTOR_ROWS, "supported provider vector geometry");
        assert!(range.start <= range.end && range.end <= self.rows);
        assert!(range.start % VECTOR_ROWS == 0 || range.start == self.rows);
        assert!(range.end % VECTOR_ROWS == 0 || range.end == self.rows);
        ScanPositions {
            table: self,
            configure,
            mask: ScanMask::default(),
            next: range.start,
            end: range.end,
            start: range.start,
            cursor: 0,
            len: 0,
        }
    }

    /// Preserve the incoming occurrence order and multiplicity. Cache only one
    /// vector's exact predicate mask; opaque iterator boundaries remain lazy.
    pub fn filter_positions<'table, I, F>(
        &'table self,
        input: I,
        vector_rows: usize,
        configure: F,
    ) -> FilterRows<'table, C, I::IntoIter, impl FnMut(&ScanPosition<'table>) -> ScanPosition<'table>, F>
    where
        I: IntoIterator<Item = ScanPosition<'table>>,
        F: FnMut(&mut ScanTile<'_, '_, C>),
    {
        self.filter_rows(input, vector_rows, |position| *position, configure)
    }

    /// Filter carrier tuples without losing fields computed at earlier opaque
    /// boundaries. The projection extracts an existing source occurrence only.
    pub fn filter_rows<'table, I, P, F>(
        &'table self,
        input: I,
        vector_rows: usize,
        position: P,
        configure: F,
    ) -> FilterRows<'table, C, I::IntoIter, P, F>
    where
        I: IntoIterator,
        P: FnMut(&I::Item) -> ScanPosition<'table>,
        F: FnMut(&mut ScanTile<'_, '_, C>),
    {
        assert_eq!(vector_rows, VECTOR_ROWS, "supported provider vector geometry");
        FilterRows {
            table: self,
            input: input.into_iter(),
            position,
            configure,
            mask: ScanMask::default(),
            start: None,
        }
    }
}

/// A base occurrence issued by a retained table, distinct from logical Rows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ScanPosition<'table> {
    snapshot: format::SnapshotId,
    index: usize,
    source: PhantomData<&'table ()>,
}
impl ScanPosition<'_> {
    pub fn index(self) -> usize {
        self.index
    }
    pub fn snapshot(self) -> format::SnapshotId {
        self.snapshot
    }
}

/// Exactly one byte per vector lane; no relation-sized staging or position Vec.
pub struct ScanMask {
    bytes: [u8; VECTOR_ROWS],
}
impl Default for ScanMask {
    fn default() -> Self {
        Self { bytes: [0; VECTOR_ROWS] }
    }
}

pub struct ScanTile<'table, 'mask, C> {
    table: &'table ScanTable<C>,
    mask: &'mask mut ScanMask,
    start: usize,
    len: usize,
}
impl<'table, C: ScanColumns> ScanTile<'table, '_, C> {
    pub fn start(&self) -> usize {
        self.start
    }
    pub fn len(&self) -> usize {
        self.len
    }
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }
    pub fn mask(&self) -> &[u8] {
        &self.mask.bytes[..self.len]
    }
    pub fn intersect<const SLOT: usize>(
        &mut self,
        predicate: &Predicate,
        prune: PruneMode,
        test: TestMode,
    ) -> Classification
    where
        C: ScanSlot<SLOT>,
        <C as ScanSlot<SLOT>>::Family: EncodedPoint + EncodedWindow,
    {
        self.table.column::<SLOT>().intersect_mask(
            self.start,
            &mut self.mask.bytes[..self.len],
            predicate,
            prune,
            test,
        )
    }
    /// Exact cross-column/disjunctive carrier predicate on currently live lanes.
    pub fn retain(&mut self, mut predicate: impl FnMut(ScanPosition<'table>) -> bool) {
        for (local, live) in self.mask.bytes[..self.len].iter_mut().enumerate() {
            if *live != 0 {
                *live = predicate(ScanPosition {
                    snapshot: self.table.snapshot,
                    index: self.start + local,
                    source: PhantomData,
                }) as u8;
            }
        }
    }
    pub fn positions(&self) -> impl Iterator<Item = ScanPosition<'table>> + '_ {
        self.mask.bytes[..self.len].iter().enumerate().filter_map(|(local, &live)| {
            (live != 0).then_some(ScanPosition {
                snapshot: self.table.snapshot,
                index: self.start + local,
                source: PhantomData,
            })
        })
    }
}

pub struct ScanPositions<'table, C, F> {
    table: &'table ScanTable<C>,
    configure: F,
    mask: ScanMask,
    next: usize,
    end: usize,
    start: usize,
    cursor: usize,
    len: usize,
}
impl<'table, C: ScanColumns, F: FnMut(&mut ScanTile<'_, '_, C>)> Iterator
    for ScanPositions<'table, C, F>
{
    type Item = ScanPosition<'table>;
    fn next(&mut self) -> Option<Self::Item> {
        loop {
            while self.cursor < self.len {
                let local = self.cursor;
                self.cursor += 1;
                if self.mask.bytes[local] != 0 {
                    return Some(ScanPosition {
                        snapshot: self.table.snapshot,
                        index: self.start + local,
                        source: PhantomData,
                    });
                }
            }
            if self.next == self.end {
                return None;
            }
            self.start = self.next;
            self.len = VECTOR_ROWS.min(self.end - self.next);
            self.next += self.len;
            self.cursor = 0;
            let mut tile = self.table.tile(self.start, VECTOR_ROWS, &mut self.mask);
            (self.configure)(&mut tile);
        }
    }
}

pub struct FilterRows<'table, C, I, P, F> {
    table: &'table ScanTable<C>,
    input: I,
    position: P,
    configure: F,
    mask: ScanMask,
    start: Option<usize>,
}
impl<'table, C, I, P, F> Iterator for FilterRows<'table, C, I, P, F>
where
    C: ScanColumns,
    I: Iterator,
    P: FnMut(&I::Item) -> ScanPosition<'table>,
    F: FnMut(&mut ScanTile<'_, '_, C>),
{
    type Item = I::Item;
    fn next(&mut self) -> Option<Self::Item> {
        for item in self.input.by_ref() {
            let position = (self.position)(&item);
            self.table.check_position(position);
            let start = position.index / VECTOR_ROWS * VECTOR_ROWS;
            if self.start != Some(start) {
                let mut tile = self.table.tile(start, VECTOR_ROWS, &mut self.mask);
                (self.configure)(&mut tile);
                self.start = Some(start);
            }
            if self.mask.bytes[position.index - start] != 0 {
                return Some(item);
            }
        }
        None
    }
}

#[cfg(test)]
mod bounded_payload_laws {
    use super::*;

    #[test]
    fn declared_granularity_controls_classification_and_payload_reads() {
        let mut builder = ScanColumnBuilder::<u64, Plain>::with_summaries(
            BlockRows::new(2048).unwrap(), true, true,
        );
        builder.extend(std::iter::repeat_n(7, VECTOR_ROWS));
        builder.extend(std::iter::repeat_n(9, VECTOR_ROWS));
        let mut column = builder.finish();
        for test in [TestMode::Encoded, TestMode::DecodeToCode] {
            for (point, vector_class, live) in [
                (7, Classification::Qualified, 1),
                (9, Classification::Disqualified, 0),
            ] {
                for (mode, class) in [
                    (PruneMode::None, Classification::Ambivalent),
                    (PruneMode::BlockMinMax, Classification::Ambivalent),
                    (PruneMode::VectorMinMax, vector_class),
                ] {
                    let mut mask = [1; VECTOR_ROWS];
                    assert_eq!(column.intersect_mask(0, &mut mask, &Predicate::Point(point), mode, test), class);
                    assert!(mask.iter().all(|&lane| lane == live));
                }
            }
        }
        // Both buffers remain present; only selected vector bounds skip reads.
        column.payload = AlignedBytes::default();
        for test in [TestMode::Encoded, TestMode::DecodeToCode] {
            for point in [7, 9] {
                let mut mask = [1; VECTOR_ROWS];
                column.intersect_mask(0, &mut mask, &Predicate::Point(point), PruneMode::VectorMinMax, test);
                for mode in [PruneMode::None, PruneMode::BlockMinMax] {
                    assert!(std::panic::catch_unwind(|| {
                        let mut mask = [1; VECTOR_ROWS];
                        column.intersect_mask(0, &mut mask, &Predicate::Point(point), mode, test);
                    }).is_err());
                }
            }
        }
    }

    #[test]
    fn missing_summary_capability_is_rejected_even_for_dead_lanes() {
        for (block, vector) in [(false, false), (true, false), (false, true)] {
            let mut builder = ScanColumnBuilder::<u64, Plain>::with_summaries(
                BlockRows::new(1024).unwrap(), block, vector,
            );
            builder.extend([7]);
            let column = builder.finish();
            for (mode, available) in [(PruneMode::BlockMinMax, block), (PruneMode::VectorMinMax, vector)] {
                for lanes in [0, 1] {
                    let result = std::panic::catch_unwind(|| {
                        let mut mask = [0];
                        column.intersect_mask(0, &mut mask[..lanes], &Predicate::Point(7), mode, TestMode::Encoded)
                    });
                    assert_eq!(result.is_ok(), available);
                }
            }
        }
    }

    #[test]
    fn runtime_rejects_unrecorded_vector_geometry() {
        let table = ScanTable::new((ScanColumn::<u64, Plain>::new(
            &[7], BlockRows::new(1024).unwrap(), false,
        ),));
        for vector_rows in [0, 512, 2048] {
            assert!(std::panic::catch_unwind(|| {
                table.scan_positions(0..1, vector_rows, |_| {});
            }).is_err());
            assert!(std::panic::catch_unwind(|| {
                table.filter_positions(std::iter::empty(), vector_rows, |_| {});
            }).is_err());
            assert!(std::panic::catch_unwind(|| {
                table.filter_rows(std::iter::empty::<ScanPosition<'_>>(), vector_rows, |position| *position, |_| {});
            }).is_err());
            assert!(std::panic::catch_unwind(|| {
                table.tile(0, vector_rows, &mut ScanMask::default());
            }).is_err());
        }
        assert_eq!(table.scan_positions(0..1, VECTOR_ROWS, |_| {}).count(), 1);
        let positions = table.scan_positions(0..1, VECTOR_ROWS, |_| {});
        assert_eq!(table.filter_positions(positions, VECTOR_ROWS, |_| {}).count(), 1);
    }


    fn header(rows: u32) -> BlockHeader {
        BlockHeader {
            offset: 0,
            base: 0,
            rows,
            width: Width::W64,
            codec: CodecId::Plain,
            reserved: [0; 2],
        }
    }

    #[test]
    fn qualification_and_disqualification_never_access_payload() {
        // Deliberately unavailable payload: any attempted decode would panic.
        // The enclosing summary alone certifies either decision.
        let headers = [header(1024)];
        let summaries = [MinMax { lo: 7, hi: 7 }];
        for test in [TestMode::Encoded, TestMode::DecodeToCode] {
            let mut mask = [1; VECTOR_ROWS];
            mask[4] = 0;
            assert_eq!(
                intersect_vector(
                    &headers, &[], Some(&summaries), None, BlockRows::new(1024).unwrap(),
                    0, &mut mask, &Predicate::Point(7), PruneMode::BlockMinMax, test,
                ),
                Classification::Qualified,
            );
            assert_eq!(mask[4], 0);
            assert_eq!(mask.iter().map(|&live| live as usize).sum::<usize>(), 1023);
            assert_eq!(
                intersect_vector(
                    &headers, &[], Some(&summaries), None, BlockRows::new(1024).unwrap(),
                    0, &mut mask, &Predicate::Point(8), PruneMode::BlockMinMax, test,
                ),
                Classification::Disqualified,
            );
            assert!(mask.iter().all(|&live| live == 0));
        }
    }

    #[test]
    fn vector_never_touches_another_vectors_payload_or_omitted_summary() {
        let headers = [header(2048)];
        // Only the requested first vector is backed. Header/summary accesses
        // outside the selected protocol also fail on the empty summary slice.
        let bytes = vec![0; VECTOR_ROWS * 8];
        for test in [TestMode::Encoded, TestMode::DecodeToCode] {
            let mut mask = [1; VECTOR_ROWS];
            intersect_vector(
                &headers, &bytes, Some(&[]), Some(&[]), BlockRows::new(2048).unwrap(),
                0, &mut mask, &Predicate::Point(0), PruneMode::None, test,
            );
            assert!(mask.iter().all(|&live| live == 1));
        }
    }
    #[test]
    fn demanded_point_read_does_not_touch_other_columns() {
        let needed = ScanColumn::<u64, Plain>::new(&[5, 8], BlockRows::new(1024).unwrap(), false);
        let mut unused = ScanColumn::<u64, Plain>::new(&[99, 99], BlockRows::new(1024).unwrap(), false);
        unused.payload = AlignedBytes::default();
        let table = ScanTable::new((needed, unused));
        let position = table.scan_positions(0..2, VECTOR_ROWS, |_| {}).nth(1).unwrap();
        assert_eq!(table.read_ordinal::<0>(position), 8);
        assert_eq!(
            table.read_value::<0>(position, LogicalRead::FinalAnswer { consumer: "answer" }),
            8,
        );
    }
}
