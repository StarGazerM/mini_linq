//! H09 with its Part→Lineitem expansion executed as one batched Metal join.
//!
//! The Part/GreenNames prefix stays on the host. Its part keys are probed
//! against the source-bound GPU-built Lineitem run in one GPU dispatch sequence
//! (rank, exact u64 offsets, tiled expansion). The returned original row IDs
//! gather full-width i64 Lineitem fields; the remaining operators use the same
//! providers as the current default route: PartSupp, Orders and Nation
//! membership, Supplier multiplicity. Answers are sorted and deduplicated as
//! the default route's canonicalization does. [`execute_on_cpu`] runs the
//! identical host completion over CPU reads of the same run, isolating the
//! expansion's placement from host-code differences.
use crate::{compiled_h09::PreparedIndex, h09, manual_h09::green_parts};
use cq_mac::{
    GpuSemiJoin, IndexSpec, IndexedI64Relation, MacContext, MacError, MacRadixRun, Payload,
};
use cq_storage::{
    AdaptiveMembership, Build, DenseMembership, ExactMembership, HashIndex, IndexRead,
};
use std::collections::HashSet;

fn lineitem(index: &PreparedIndex) -> Result<&IndexedI64Relation<1>, MacError> {
    index.lineitem().ok_or_else(|| {
        MacError::CommandFailed("H09 prepared source has no green-part demand".into())
    })
}

/// The default route's operators after the Part→Lineitem expansion.
fn complete(
    data: &h09::Data,
    lineitem: &IndexedI64Relation<1>,
    row_ids: impl Iterator<Item = u32>,
) -> Vec<h09::Answer> {
    let mut row_ids = row_ids.peekable();
    if row_ids.peek().is_none() {
        return Vec::new();
    }
    let partsupp = <AdaptiveMembership<(i64, i64), 16777216> as Build<_>>::build(
        data.partsupp.iter().map(|&key| (key, ())),
    );
    let orders = <DenseMembership<i64, 16777216> as Build<_>>::build(
        data.orders.iter().map(|&(order_key,)| (order_key, ())),
    );
    let supplier = HashIndex::build(data.supplier.iter().copied());
    let nation = <DenseMembership<i32, 16777216> as Build<_>>::build(
        data.nation.iter().map(|&(nation_key,)| (nation_key, ())),
    );
    let mut answers = Vec::new();
    for row_id in row_ids {
        let row = lineitem.row(row_id);
        let (order_key, part_key, supplier_key) = (row[0], row[1], row[2]);
        if !partsupp.contains(&(part_key, supplier_key)) || !orders.contains(&order_key) {
            continue;
        }
        for nation_key in supplier.lookup(&supplier_key) {
            if nation.contains(&nation_key) {
                answers.push((order_key, part_key, supplier_key));
            }
        }
    }
    answers.sort_unstable();
    answers.dedup();
    answers
}

pub fn execute(
    context: &MacContext,
    data: &h09::Data,
    index: &PreparedIndex,
) -> Result<Vec<h09::Answer>, MacError> {
    let probes: Vec<i64> = green_parts(data).collect();
    if probes.is_empty() {
        return Ok(Vec::new());
    }
    let lineitem = lineitem(index)?;
    let join = lineitem.join_row_ids(context, &probes)?;
    Ok(complete(
        data,
        lineitem,
        join.pairs().map(|(_, row_id)| row_id),
    ))
}

/// The same completion over host reads of the same GPU-built run.
pub fn execute_on_cpu(
    data: &h09::Data,
    index: &PreparedIndex,
) -> Result<Vec<h09::Answer>, MacError> {
    let mut probes = green_parts(data).peekable();
    if probes.peek().is_none() {
        return Ok(Vec::new());
    }
    let lineitem = lineitem(index)?;
    Ok(complete(
        data,
        lineitem,
        probes.flat_map(|key| lineitem.row_ids(key).into_iter().flatten()),
    ))
}

/// GPU runs for the post-join checks, as distinct-answer semijoins: PartSupp
/// keyed by part with supplier items, Orders keys, and Supplier keys selected
/// by host Nation membership (25-row Nation, 10K-row Supplier mask). Answers
/// are distinct, so Supplier multiplicity cannot change them. Every key and
/// item must fit u32; a wider value is rejected by construction, never narrowed.
pub struct Resident {
    partsupp: MacRadixRun,
    orders: MacRadixRun,
    supplier: MacRadixRun,
}

impl Resident {
    pub fn prepare(context: &MacContext, data: &h09::Data) -> Result<Self, MacError> {
        let partsupp = context.relation_i64(data.partsupp.len(), |i| {
            let (part, supplier) = data.partsupp[i];
            [part, supplier]
        })?;
        let orders = context.relation_i64(data.orders.len(), |i| [data.orders[i].0])?;
        let supplier = context.relation_i64(data.supplier.len(), |i| [data.supplier[i].0])?;
        let nations: HashSet<i32> = data.nation.iter().map(|&(nation,)| nation).collect();
        let selection = context.relation_u32(data.supplier.len(), |i| {
            [u32::from(nations.contains(&data.supplier[i].1))]
        })?;
        let rows = IndexSpec {
            key_column: 0,
            payload: Payload::RowId,
        };
        Ok(Self {
            partsupp: context.build_radix(
                &partsupp,
                IndexSpec {
                    key_column: 0,
                    payload: Payload::Column(1),
                },
                None,
            )?,
            orders: context.build_radix(&orders, rows, None)?,
            supplier: context.build_radix(&supplier, rows, Some(&selection))?,
        })
    }
}

/// Part→Lineitem join, the PartSupp, Orders and Supplier⋉Nation checks, and
/// the answer gather/sort/dedup run on the GPU as one graph with a single
/// host readback (the join's pair count); only distinct answers return.
/// `green` is the Part keys passing Q9's `p_name LIKE '%green%'` prefilter
/// ([`green_parts`]). LIKE is outside the supported query language, so callers
/// evaluate it once and it is not part of the measured query.
pub fn execute_resident(
    context: &MacContext,
    green: &[i64],
    index: &PreparedIndex,
    resident: &Resident,
) -> Result<Vec<h09::Answer>, MacError> {
    if green.is_empty() {
        return Ok(Vec::new());
    }
    let lineitem = lineitem(index)?;
    // Lineitem columns: 0 order, 1 part, 2 supplier. Answers are (order,
    // part, supplier), deduplicated in that lexicographic order. The whole
    // chain is one GPU graph; only the join's pair count and the distinct
    // answers cross to the host.
    let steps = [
        GpuSemiJoin {
            key_column: 1,
            item_column: Some(2),
            run: &resident.partsupp,
        },
        GpuSemiJoin {
            key_column: 0,
            item_column: None,
            run: &resident.orders,
        },
        GpuSemiJoin {
            key_column: 2,
            item_column: None,
            run: &resident.supplier,
        },
    ];
    let answers = lineitem.join_semijoin_distinct(context, green, &steps, [0, 1, 2])?;
    Ok(answers
        .tuples()
        .iter()
        .map(|&[order, part, supplier]| (order, part, supplier))
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{compiled_h09, fixtures, manual_h09::Prepared};

    #[test]
    fn metal_join_h09_matches_the_authored_default_route() {
        let context = MacContext::new().unwrap();
        let mut parts = fixtures::H09_BOUNDARY_PARTS.to_vec();
        // Out-of-domain green probes are exact misses, not narrowed keys.
        parts.extend([(-1, "green absent"), (u32::MAX as i64 + 1, "green absent")]);
        for data in [
            fixtures::h09_data(&parts, fixtures::h09_boundary_rows()),
            fixtures::h09_scaled(65_539),
        ] {
            let mut expected = h09::compiled(&data);
            expected.sort_unstable();
            let source = Prepared::new(&context, &data).unwrap();
            let index =
                compiled_h09::build(&context, &source, compiled_h09::Construction::Gpu).unwrap();
            assert_eq!(execute(&context, &data, &index).unwrap(), expected);
            assert_eq!(execute_on_cpu(&data, &index).unwrap(), expected);
        }
        let no_green = fixtures::h09_no_green(fixtures::h09_boundary_rows());
        let source = Prepared::new(&context, &no_green).unwrap();
        let index =
            compiled_h09::build(&context, &source, compiled_h09::Construction::Gpu).unwrap();
        assert!(execute(&context, &no_green, &index).unwrap().is_empty());
        let mut rows = fixtures::h09_boundary_rows();
        rows.lineitem.clear();
        let empty = fixtures::h09_data(fixtures::H09_BOUNDARY_PARTS, rows);
        let source = Prepared::new(&context, &empty).unwrap();
        let index =
            compiled_h09::build(&context, &source, compiled_h09::Construction::Gpu).unwrap();
        assert!(execute(&context, &empty, &index).unwrap().is_empty());
    }

    #[test]
    fn resident_semijoin_chain_matches_the_authored_default_route() {
        let context = MacContext::new().unwrap();
        let in_domain = |value: i64| u32::try_from(value).is_ok();
        let mut rows = fixtures::h09_boundary_rows();
        rows.lineitem
            .retain(|&(o, p, s)| in_domain(o) && in_domain(p) && in_domain(s));
        rows.lineitem
            .extend([(104, 3, 8), (105, 4, 7), (106, 1, 8)]);
        rows.partsupp.retain(|&(p, s)| in_domain(p) && in_domain(s));
        rows.partsupp.push((3, 8));
        rows.orders.retain(|&(o,)| in_domain(o));
        rows.orders.extend([(104,), (106,)]);
        rows.supplier.retain(|&(s, _)| in_domain(s));
        let mut parts = fixtures::H09_BOUNDARY_PARTS.to_vec();
        parts.extend([(-1, "green absent"), (u32::MAX as i64 + 1, "green absent")]);
        for data in [
            fixtures::h09_data(&parts, rows),
            fixtures::h09_scaled(65_539),
        ] {
            let mut expected = h09::compiled(&data);
            expected.sort_unstable();
            assert!(!expected.is_empty());
            let source = Prepared::new(&context, &data).unwrap();
            let index =
                compiled_h09::build(&context, &source, compiled_h09::Construction::Gpu).unwrap();
            let resident = Resident::prepare(&context, &data).unwrap();
            assert_eq!(
                execute_resident(&context, &green_parts(&data).collect::<Vec<_>>(), &index, &resident).unwrap(),
                expected
            );
        }
        // Wide PartSupp/Orders/Supplier keys cannot be GPU runs: rejected.
        assert!(Resident::prepare(&context, &fixtures::h09_boundaries()).is_err());
    }
}
