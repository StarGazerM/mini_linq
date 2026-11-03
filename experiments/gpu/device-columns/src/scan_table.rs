//! Retained shared-S1 scan tables for authored blocked GPU sources (P6 C).
//!
//! Every relation column keeps its immutable host S1 encoding; only the
//! columns a recorded tile scan demands are uploaded and leased, once, on the
//! table's pool lane. Generated setup checks each leased column's runtime
//! facts against its recorded Structures declaration before reading it. The
//! table never reconstructs logical rows on the device.
use crate::{
    scan::{DeviceScanColumn, DeviceScanView, ScanDeviceDescriptor, TransferBytes},
    Pool, Result,
};
use cq_storage::scan::{Family, OrdinalDecode, ScanColumn};
pub use cq_storage::scan::{format::{Domain, DomainId}, Width};
use std::{any::Any, marker::PhantomData, sync::Arc};

/// S1 family of one retained column.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FamilyId {
    Plain,
    Bytes,
}

/// Runtime facts a retained column certifies.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ColumnFacts {
    pub family: FamilyId,
    pub domain: Domain,
    pub block_rows: usize,
    pub summaries: bool,
}

trait HostOrdinals {
    fn ordinal(&self, position: usize) -> u64;
}
impl<T: OrdinalDecode, F: Family> HostOrdinals for ScanColumn<T, F> {
    fn ordinal(&self, position: usize) -> u64 {
        self.read_ordinal(position)
    }
}

struct Leased {
    descriptor: ScanDeviceDescriptor,
    transfer: TransferBytes,
    // Keeps the retained view, which orders owner release after lane use.
    _view: Box<dyn Any>,
}

/// One retained relation column: its host encoding and, when demanded, its
/// uploaded, leased device view.
pub struct ScanSlot {
    facts: ColumnFacts,
    rows: usize,
    host: Arc<dyn HostOrdinals>,
    device: Option<Leased>,
}

impl ScanSlot {
    /// Retain `column`; upload and lease it on `pool` only when `upload`.
    pub fn new<T: OrdinalDecode + 'static, F: Family + 'static>(
        pool: &Arc<Pool>,
        column: Arc<ScanColumn<T, F>>,
        family: FamilyId,
        domain: Domain,
        upload: bool,
    ) -> Result<Self> {
        let facts = ColumnFacts {
            family,
            domain,
            block_rows: column.block_rows().get(),
            summaries: column.summaries().is_some(),
        };
        let rows = column.len();
        let device = if upload {
            let uploaded = DeviceScanColumn::upload(pool.clone(), column.clone(), domain, rows)?;
            let transfer = uploaded.transfer_bytes();
            let view: DeviceScanView<T, F> = uploaded.view_on(pool.clone())?;
            // SAFETY: the descriptor is only exposed through the table's
            // unsafe accessor, whose caller launches on this lane while the
            // table (and therefore this view) is alive.
            let descriptor = unsafe { view.abi() };
            Some(Leased {
                descriptor,
                transfer,
                _view: Box::new(view),
            })
        } else {
            None
        };
        Ok(Self {
            facts,
            rows,
            host: column,
            device,
        })
    }
}

/// Typed rows over retained S1 ordinals; only the diagnostic host oracle reads
/// complete rows.
pub trait OrdinalRow: Sized + 'static {
    const WIDTH: usize;
    fn from_ordinals(slots: &[ScanSlot], position: usize) -> Self;
}

macro_rules! ordinal_rows {
    ($($name:ident $index:tt),+) => {
        impl<$($name: OrdinalDecode + 'static),+> OrdinalRow for ($($name,)+) {
            const WIDTH: usize = [$($index),+].len();
            fn from_ordinals(slots: &[ScanSlot], position: usize) -> Self {
                ($($name::from_ordinal(slots[$index].host.ordinal(position)),)+)
            }
        }
    };
}
ordinal_rows!(A 0);
ordinal_rows!(A 0, B 1);
ordinal_rows!(A 0, B 1, C 2);
ordinal_rows!(A 0, B 1, C 2, D 3);
ordinal_rows!(A 0, B 1, C 2, D 3, E 4);
ordinal_rows!(A 0, B 1, C 2, D 3, E 4, G 5);
ordinal_rows!(A 0, B 1, C 2, D 3, E 4, G 5, H 6);
ordinal_rows!(A 0, B 1, C 2, D 3, E 4, G 5, H 6, I 7);
ordinal_rows!(A 0, B 1, C 2, D 3, E 4, G 5, H 6, I 7, J 8);
ordinal_rows!(A 0, B 1, C 2, D 3, E 4, G 5, H 6, I 7, J 8, K 9);
ordinal_rows!(A 0, B 1, C 2, D 3, E 4, G 5, H 6, I 7, J 8, K 9, L 10);
ordinal_rows!(A 0, B 1, C 2, D 3, E 4, G 5, H 6, I 7, J 8, K 9, L 10, M 11);

/// A retained blocked relation on one pool lane.
pub struct DeviceScanTable<Row> {
    pool: Arc<Pool>,
    rows: usize,
    slots: Vec<ScanSlot>,
    _row: PhantomData<fn() -> Row>,
}

impl<Row: OrdinalRow> DeviceScanTable<Row> {
    pub fn new(pool: Arc<Pool>, rows: usize, slots: Vec<ScanSlot>) -> Result<Self> {
        if slots.len() != Row::WIDTH {
            return Err("scan table width differs from its row type".into());
        }
        if slots.iter().any(|slot| slot.rows != rows) {
            return Err("scan table columns differ in length".into());
        }
        Ok(Self {
            pool,
            rows,
            slots,
            _row: PhantomData,
        })
    }
    pub fn pool(&self) -> &Arc<Pool> {
        &self.pool
    }
    pub fn len(&self) -> usize {
        self.rows
    }
    pub fn is_empty(&self) -> bool {
        self.rows == 0
    }
    /// Reject a column whose runtime facts differ from the recorded ones.
    pub fn check(&self, slot: usize, expected: ColumnFacts) -> Result<()> {
        let actual = self.slots.get(slot).ok_or("scan table slot out of range")?;
        if actual.facts != expected {
            return Err(format!(
                "scan table slot {slot}: recorded {expected:?}, retained {:?}",
                actual.facts
            )
            .into());
        }
        if actual.device.is_none() {
            return Err(format!("scan table slot {slot} was not uploaded").into());
        }
        Ok(())
    }
    /// Initialized H2D extents of every uploaded column.
    pub fn transfer_bytes(&self) -> TransferBytes {
        let mut sum = TransferBytes {
            payload: 0,
            headers: 0,
            block_summaries: 0,
            vector_summaries: 0,
        };
        for leased in self.slots.iter().filter_map(|slot| slot.device.as_ref()) {
            sum.payload += leased.transfer.payload;
            sum.headers += leased.transfer.headers;
            sum.block_summaries += leased.transfer.block_summaries;
            sum.vector_summaries += leased.transfer.vector_summaries;
        }
        sum
    }
    /// # Safety
    /// Launch only on this table's pool lane while the table is alive; never
    /// read beyond the descriptor's live count.
    pub unsafe fn descriptor(&self, slot: usize) -> Result<ScanDeviceDescriptor> {
        self.slots
            .get(slot)
            .and_then(|slot| slot.device.as_ref())
            .map(|leased| leased.descriptor)
            .ok_or_else(|| format!("scan table slot {slot} was not uploaded").into())
    }
    /// Diagnostic complete-row read of the retained host encoding for the
    /// independent Sequential oracle; not a GPU answer or relational read.
    pub fn to_host(&self) -> Result<Vec<Row>> {
        // DECODE ?: the independent Sequential oracle reconstructs logical input rows from retained host S1 ordinals; it is diagnostic, not GPU publication.
        Ok((0..self.rows)
            .map(|position| Row::from_ordinals(&self.slots, position))
            .collect())
    }
}
