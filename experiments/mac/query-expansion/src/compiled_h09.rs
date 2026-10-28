//! Explicit prepared Lineitem binding for the unchanged authored H09 query.
//! CPU/GPU placement changes construction only; query() has one concrete host type.
use crate::{h09, manual_h09::Prepared};
use cq_mac::{IndexedI64Relation, MacContext, MacError};
use cq_storage::{
    IndexRead, RunCursor,
    dispatch::IndexedColumn,
    rows::{Columns, Rows},
};

#[derive(Clone, Copy, Debug)]
pub enum Construction {
    Cpu,
    Gpu,
}

/// Owns a completed index together with the exact immutable Lineitem source.
/// No-green preparation has no source/index and remains valid only for no-green
/// query inputs. Other relations are borrowed afresh for each execution.
pub struct PreparedIndex {
    lineitem: Option<IndexedI64Relation<1>>,
}

pub fn build(
    context: &MacContext,
    source: &Prepared,
    mode: Construction,
) -> Result<PreparedIndex, MacError> {
    let lineitem = source
        .source()
        .map(|source| match mode {
            Construction::Cpu => context.prepare_radix_i64_on_cpu::<1>(source),
            Construction::Gpu => context.prepare_radix_i64::<1>(source),
        })
        .transpose()?;
    Ok(PreparedIndex { lineitem })
}

impl PreparedIndex {
    pub fn indexed_rows(&self) -> usize {
        self.lineitem.as_ref().map_or(0, IndexedI64Relation::len)
    }
}

#[cfg(test)]
#[derive(Default)]
struct Reads {
    scans: std::cell::Cell<usize>,
    probes: std::cell::Cell<usize>,
}

struct LineitemHost<'a> {
    index: Option<&'a IndexedI64Relation<1>>,
    #[cfg(test)]
    reads: &'a Reads,
}

impl<'a> Rows for &'a LineitemHost<'_> {
    type Row = (&'a i64, &'a i64, &'a i64);
    fn rows(self) -> impl Iterator<Item = Self::Row> {
        #[cfg(test)]
        self.reads.scans.set(self.reads.scans.get() + 1);
        self.index
            .into_iter()
            .flat_map(|index| index.values().chunks_exact(3))
            .map(|row| (&row[0], &row[1], &row[2]))
    }
}

#[derive(Clone, Copy)]
struct LineitemRead<'a> {
    index: Option<&'a IndexedI64Relation<1>>,
    #[cfg(test)]
    reads: &'a Reads,
}

impl<'a> IndexedColumn<1> for &'a LineitemHost<'_> {
    type Read = LineitemRead<'a>;
    fn indexed_column(&self) -> Self::Read {
        LineitemRead {
            index: self.index,
            #[cfg(test)]
            reads: self.reads,
        }
    }
}

#[derive(Clone, Copy)]
struct Payload<'a> {
    order: &'a i64,
    supplier: &'a i64,
}
impl<'a> Columns for Payload<'a> {
    type Row = (&'a i64, &'a i64);
    fn columns(self) -> Self::Row {
        (self.order, self.supplier)
    }
}

#[derive(Clone, Copy)]
struct PayloadCursor<'a> {
    values: &'a [i64],
    ids: Option<RunCursor<'a, u32>>,
}
impl<'a> Iterator for PayloadCursor<'a> {
    type Item = Payload<'a>;
    fn next(&mut self) -> Option<Self::Item> {
        let row = self.ids.as_mut()?.next()? as usize * 3;
        Some(Payload {
            order: &self.values[row],
            supplier: &self.values[row + 2],
        })
    }
    fn size_hint(&self) -> (usize, Option<usize>) {
        let remaining = self.ids.as_ref().map_or(0, ExactSizeIterator::len);
        (remaining, Some(remaining))
    }
}
impl ExactSizeIterator for PayloadCursor<'_> {}
impl std::iter::FusedIterator for PayloadCursor<'_> {}

impl<'a> IndexRead for LineitemRead<'a> {
    type Key = (i64,);
    type Item = Payload<'a>;
    type Cursor = PayloadCursor<'a>;
    fn lookup(&self, key: &Self::Key) -> Self::Cursor {
        #[cfg(test)]
        self.reads.probes.set(self.reads.probes.get() + 1);
        PayloadCursor {
            values: self.index.map_or(&[], IndexedI64Relation::values),
            ids: self.index.and_then(|index| index.row_ids(key.0)),
        }
    }
}
impl cq_storage::ExactMembership<(i64,)> for LineitemRead<'_> {
    fn contains(&self, key: &(i64,)) -> bool {
        self.index
            .and_then(|index| index.row_ids(key.0))
            .is_some_and(|ids| ids.len() != 0)
    }
}

// One compiled function for both construction choices, not two handwritten joins.
#[inline(never)]
#[allow(non_snake_case)]
fn query(data: &h09::Data, lineitem: &LineitemHost<'_>) -> Vec<h09::Answer> {
    h09::with_hosts!(
        lineitem,
        data.partsupp.as_slice(),
        data.part.as_slice(),
        data.supplier.as_slice(),
        data.orders.as_slice(),
        data.nation.as_slice(),
    )
}

/// The Lineitem relation is the pinned prepared source, not data.lineitem.
/// All other relations and the opaque Part predicate retain the authored query.
pub fn execute(data: &h09::Data, index: &PreparedIndex) -> Result<Vec<h09::Answer>, MacError> {
    if index.lineitem.is_none() && data.part.iter().any(|(_, name)| name.contains("green")) {
        return Err(MacError::CommandFailed(
            "H09 prepared source has no green-part demand".into(),
        ));
    }
    #[cfg(test)]
    let reads = Reads::default();
    let host = LineitemHost {
        index: index.lineitem.as_ref(),
        #[cfg(test)]
        reads: &reads,
    };
    Ok(query(data, &host))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixtures;

    fn canonical(mut rows: Vec<h09::Answer>) -> Vec<h09::Answer> {
        rows.sort_unstable();
        rows
    }

    #[test]
    fn compiled_h09_uses_bound_column_index_without_fallback_and_preserves_wide_payloads() {
        let context = MacContext::new().unwrap();
        let mut data = fixtures::h09_boundaries();
        // Out-of-domain probe keys are absent, not narrowed into a real key.
        data.part.extend([
            (-1, "green absent".into()),
            (u32::MAX as i64 + 1, "green absent".into()),
        ]);
        let expected = canonical(h09::compiled(&data));
        let source = Prepared::new(&context, &data).unwrap();
        let cpu = build(&context, &source, Construction::Cpu).unwrap();
        let gpu = build(&context, &source, Construction::Gpu).unwrap();
        drop(source);
        drop(context);
        // Payloads must still come from the pinned shared source, not this vector.
        data.lineitem.clear();
        cq_mac::memory::clear_cache().unwrap();
        for index in [cpu, gpu] {
            let reads = Reads::default();
            let host = LineitemHost {
                index: index.lineitem.as_ref(),
                reads: &reads,
            };
            assert_eq!(canonical(query(&data, &host)), expected);
            assert_eq!(
                reads.scans.get(),
                0,
                "prepared source was rebuilt through a fallback scan"
            );
            assert_eq!(
                reads.probes.get(),
                data.part
                    .iter()
                    .filter(|(_, name)| name.contains("green"))
                    .count()
            );
            assert!(expected.contains(&(-1, 3, i64::MAX)));
            assert_eq!(canonical(execute(&data, &index).unwrap()), expected);
        }
    }

    #[test]
    fn compiled_h09_handles_empty_sources_and_no_demand_without_ingesting_invalid_rows() {
        let context = MacContext::new().unwrap();
        let mut data = fixtures::h09_boundaries();
        data.lineitem.clear();
        let source = Prepared::new(&context, &data).unwrap();
        for mode in [Construction::Cpu, Construction::Gpu] {
            let index = build(&context, &source, mode).unwrap();
            assert!(execute(&data, &index).unwrap().is_empty());
        }
        data.part
            .iter_mut()
            .for_each(|(_, name)| *name = "red".into());
        data.lineitem.push((i64::MIN, -1, i64::MAX));
        let source = Prepared::new(&context, &data).unwrap();
        assert_eq!(source.source_bytes(), 0);
        for mode in [Construction::Cpu, Construction::Gpu] {
            let index = build(&context, &source, mode).unwrap();
            assert!(execute(&data, &index).unwrap().is_empty());
            data.part.push((1, "green".into()));
            assert!(execute(&data, &index).is_err());
            data.part.pop();
        }
    }
}
