//! Versioned S1 byte format and checked, borrowed scan descriptors.
//! No GPU codec ids or FastLanes layouts are aliases of this registry.
use super::{
    BlockHeader, BlockRows, CodecId, FamilyId, MinMax, Predicate, Preserved, VECTOR_ROWS, Width,
};
use std::sync::atomic::{AtomicU64, Ordering};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LayoutId {
    S1LittleEndian,
}

/// A provider-declared domain, including dictionary identity and version.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DomainId {
    pub namespace: u64,
    pub version: u64,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Domain {
    pub id: DomainId,
    pub carrier: Width,
    pub equality: bool,
    pub order: bool,
}

/// Process-local immutable source identity, never a mutable allocation address.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SnapshotId(u64);
impl SnapshotId {
    pub(super) fn fresh() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        let mut current = NEXT.load(Ordering::Relaxed);
        loop {
            let next = current
                .checked_add(1)
                .expect("scan snapshot identity exhausted");
            match NEXT.compare_exchange_weak(current, next, Ordering::Relaxed, Ordering::Relaxed) {
                Ok(_) => return Self(current),
                Err(actual) => current = actual,
            }
        }
    }
    pub fn get(self) -> u64 {
        self.0
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BlockFormat {
    pub version: u32,
    pub layout: LayoutId,
    pub family: FamilyId,
    pub domain: Domain,
    pub preservation: Preserved,
    pub block_rows: BlockRows,
    pub vector_rows: u32,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SummaryOrigin {
    EncodeValues,
    ValidatedImport,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SummaryProvenance {
    pub snapshot: SnapshotId,
    pub domain: DomainId,
    pub origin: SummaryOrigin,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FormatError {
    Version,
    Domain,
    Preservation,
    VectorRows,
    HeaderCount,
    HeaderRows,
    Codec,
    Width,
    Offset,
    Extent,
    Count,
    SummaryCount,
    SummaryOrigin,
    SummaryBounds,
    Payload,
    Snapshot,
    Position,
    Predicate,
}
impl std::fmt::Display for FormatError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "invalid scan format: {self:?}")
    }
}
impl std::error::Error for FormatError {}

/// Public certificates for independent loader/checker mutation tests. This is
/// not a safe view until `CheckedScanView::import` validates it and its bytes.
#[derive(Clone, Copy, Debug)]
pub struct ScanMetadata<'a> {
    pub format: BlockFormat,
    pub snapshot: SnapshotId,
    pub headers: &'a [BlockHeader],
    pub block_summaries: Option<&'a [MinMax]>,
    pub vector_summaries: Option<&'a [MinMax]>,
    pub summary_provenance: Option<SummaryProvenance>,
    pub capacity: usize,
    pub live_count: usize,
}

/// Checked byte extents and immutable owners are borrowed together. Device
/// runtimes may upload these bytes unchanged, not reinterpret a legacy codec.
#[derive(Clone, Copy, Debug)]
pub struct CheckedScanView<'a> {
    metadata: ScanMetadata<'a>,
    payload: &'a [u8],
}
fn maximum(width: Width) -> u64 {
    match width {
        Width::W0 => 0,
        Width::W8 => u8::MAX as u64,
        Width::W16 => u16::MAX as u64,
        Width::W32 => u32::MAX as u64,
        Width::W64 => u64::MAX,
    }
}
impl<'a> CheckedScanView<'a> {
    pub(super) fn encoded(
        format: BlockFormat,
        snapshot: SnapshotId,
        headers: &'a [BlockHeader],
        payload: &'a [u8],
        block_summaries: Option<&'a [MinMax]>,
        vector_summaries: Option<&'a [MinMax]>,
        capacity: usize,
        live_count: usize,
    ) -> Result<Self, FormatError> {
        let metadata = ScanMetadata {
            format,
            snapshot,
            headers,
            block_summaries,
            vector_summaries,
            capacity,
            live_count,
            summary_provenance: (block_summaries.is_some() || vector_summaries.is_some())
                .then_some(SummaryProvenance {
                    snapshot,
                    domain: format.domain.id,
                    origin: SummaryOrigin::EncodeValues,
                }),
        };
        Self::check_metadata(&metadata, payload.len())?;
        Ok(Self { metadata, payload })
    }

    /// Validate untrusted metadata and ordinal payload without rerunning the
    /// encoder or demanding its preferred compression choice. Conservative
    /// enclosing extrema and a lawful Plain block in Bytes are accepted.
    // Formal rule STR-DeviceBlocks: one shared byte layout, exact occurrence extent.
    pub fn import(mut metadata: ScanMetadata<'a>, payload: &'a [u8]) -> Result<Self, FormatError> {
        Self::check_metadata(&metadata, payload.len())?;
        let view = Self { metadata, payload };
        let mut row = 0;
        for (block, header) in metadata.headers.iter().enumerate() {
            for local in 0..header.rows as usize {
                let code = view.residual(header, local);
                let ordinal = header.base.checked_add(code).ok_or(FormatError::Payload)?;
                if ordinal > maximum(metadata.format.domain.carrier) {
                    return Err(FormatError::Payload);
                }
                for bounds in [
                    metadata.block_summaries.map(|s| s[block]),
                    metadata.vector_summaries.map(|s| s[row / VECTOR_ROWS]),
                ]
                .into_iter()
                .flatten()
                {
                    if ordinal < bounds.lo || ordinal > bounds.hi {
                        return Err(FormatError::SummaryBounds);
                    }
                }
                row += 1;
            }
        }
        if let Some(provenance) = &mut metadata.summary_provenance {
            provenance.origin = SummaryOrigin::ValidatedImport;
        }
        Ok(Self { metadata, payload })
    }

    fn check_metadata(m: &ScanMetadata<'_>, bytes: usize) -> Result<(), FormatError> {
        let f = m.format;
        if f.version != 1 {
            return Err(FormatError::Version);
        }
        if f.vector_rows != VECTOR_ROWS as u32 {
            return Err(FormatError::VectorRows);
        }
        if f.domain.id.namespace == 0
            || f.domain.carrier == Width::W0
            || !f.domain.equality
            || (f.domain.order && !f.domain.equality)
        {
            return Err(FormatError::Domain);
        }
        if !f.preservation.equality || !f.preservation.order {
            return Err(FormatError::Preservation);
        }
        if m.live_count > m.capacity {
            return Err(FormatError::Count);
        }
        if m.headers.len() != m.capacity.div_ceil(f.block_rows.get()) {
            return Err(FormatError::HeaderCount);
        }
        let mut extent = 0usize;
        let mut rows = 0usize;
        for h in m.headers {
            let expected = (m.capacity - rows).min(f.block_rows.get());
            if h.rows as usize != expected || h.rows == 0 {
                return Err(FormatError::HeaderRows);
            }
            if h.reserved != [0; 2] {
                return Err(FormatError::Version);
            }
            match h.codec {
                CodecId::Plain if h.base == 0 && h.width == f.domain.carrier => {}
                CodecId::For
                    if f.family == FamilyId::Bytes
                        && matches!(h.width, Width::W0 | Width::W8 | Width::W16)
                        && (h.width as u8) < f.domain.carrier as u8
                        && h.base <= maximum(f.domain.carrier) => {}
                _ => return Err(FormatError::Codec),
            }
            if usize::try_from(h.offset).ok() != Some(extent)
                || (h.width != Width::W0 && extent % h.width as usize != 0)
            {
                return Err(FormatError::Offset);
            }
            extent = extent
                .checked_add(
                    expected
                        .checked_mul(h.width as usize)
                        .ok_or(FormatError::Extent)?,
                )
                .ok_or(FormatError::Extent)?;
            rows = rows.checked_add(expected).ok_or(FormatError::Extent)?;
        }
        if extent != bytes {
            return Err(FormatError::Extent);
        }
        if m.block_summaries
            .is_some_and(|s| s.len() != m.headers.len())
            || m.vector_summaries
                .is_some_and(|s| s.len() != m.capacity.div_ceil(VECTOR_ROWS))
        {
            return Err(FormatError::SummaryCount);
        }
        let summaries = m.block_summaries.is_some() || m.vector_summaries.is_some();
        if summaries != m.summary_provenance.is_some() {
            return Err(FormatError::SummaryOrigin);
        }
        if let Some(p) = m.summary_provenance {
            if p.snapshot != m.snapshot || p.domain != f.domain.id {
                return Err(FormatError::SummaryOrigin);
            }
        }
        // Formal rule GPU-VectorSummary: source-value, same-domain snapshot
        // bounds; imported enclosure is additionally checked against payload.
        for bounds in m
            .block_summaries
            .into_iter()
            .chain(m.vector_summaries)
            .flatten()
        {
            if bounds.lo > bounds.hi || bounds.hi > maximum(f.domain.carrier) {
                return Err(FormatError::SummaryBounds);
            }
        }
        Ok(())
    }

    pub fn metadata(self) -> ScanMetadata<'a> {
        self.metadata
    }
    pub fn payload(self) -> &'a [u8] {
        self.payload
    }
    /// The S1 C ABI contains no implicit padding. These are initialized bytes,
    /// not a translation to the legacy GPU word-offset header.
    pub fn header_bytes(self) -> &'a [u8] {
        let headers = self.metadata.headers;
        const {
            assert!(std::mem::size_of::<BlockHeader>() == 24);
        }
        unsafe {
            std::slice::from_raw_parts(headers.as_ptr().cast(), std::mem::size_of_val(headers))
        }
    }
    pub fn summary_bytes(summaries: &'a [MinMax]) -> &'a [u8] {
        const {
            assert!(std::mem::size_of::<MinMax>() == 16);
        }
        unsafe {
            std::slice::from_raw_parts(summaries.as_ptr().cast(), std::mem::size_of_val(summaries))
        }
    }
    fn residual(self, h: &BlockHeader, row: usize) -> u64 {
        let width = h.width as usize;
        let start = h.offset as usize + row * width;
        let mut bytes = [0; 8];
        bytes[..width].copy_from_slice(&self.payload[start..start + width]);
        u64::from_le_bytes(bytes)
    }
    pub fn check_predicate(
        self,
        domain: DomainId,
        predicate: &Predicate,
    ) -> Result<(), FormatError> {
        if domain != self.metadata.format.domain.id {
            return Err(FormatError::Domain);
        }
        if matches!(predicate, Predicate::Window { .. }) && !self.metadata.format.domain.order {
            return Err(FormatError::Predicate);
        }
        Ok(())
    }
    pub fn filter_mask(
        self,
        domain: DomainId,
        predicate: &Predicate,
        out: &mut [u8],
    ) -> Result<(), FormatError> {
        self.check_predicate(domain, predicate)?;
        if out.len() != self.metadata.live_count {
            return Err(FormatError::Count);
        }
        let mut row = 0;
        for h in self.metadata.headers {
            let translated = predicate.translate(h);
            for local in 0..(h.rows as usize).min(out.len() - row) {
                out[row] = translated.matches(self.residual(h, local)) as u8;
                row += 1;
            }
            if row == out.len() {
                break;
            }
        }
        Ok(())
    }
    /// Late carrier reads preserve global ordinal codes, including duplicate or
    /// reordered positions; they do not reconstruct authored Rust values.
    // Formal rule GPU-LateRead: positions refer to this retained source snapshot.
    pub fn read_positions(
        self,
        snapshot: SnapshotId,
        positions: &[usize],
        out: &mut [u64],
    ) -> Result<(), FormatError> {
        if snapshot != self.metadata.snapshot {
            return Err(FormatError::Snapshot);
        }
        if positions.len() != out.len() || positions.iter().any(|&p| p >= self.metadata.live_count)
        {
            return Err(FormatError::Position);
        }
        for (&position, target) in positions.iter().zip(out) {
            let h = &self.metadata.headers[position / self.metadata.format.block_rows.get()];
            *target = h.base + self.residual(h, position % self.metadata.format.block_rows.get());
        }
        Ok(())
    }
}
