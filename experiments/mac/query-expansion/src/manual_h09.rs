use std::{
    collections::HashSet,
    error::Error,
    io,
    time::{Duration, Instant},
};

use cq_storage::{
    AdaptiveMembership, Build, DenseMembership, ExactMembership, HashIndex, IndexRead,
};

/// Full-width Lineitem storage shared by all executions and index projections.
pub struct Prepared {
    source: Option<cq_mac::Relation>,
    pub ingestion: Duration,
}

/// Part keys whose interned name is in the loader's `GreenNames` relation.
pub fn green_parts(data: &crate::h09::Data) -> impl Iterator<Item = i64> + '_ {
    let green: HashSet<_> = data.green_names.iter().map(|(name,)| *name).collect();
    data.part
        .iter()
        .filter(move |(_, name)| green.contains(name))
        .map(|(part_key, _)| *part_key)
}

/// Whether the Part/GreenNames prefix demands any Lineitem probe.
pub(crate) fn has_green_demand(data: &crate::h09::Data) -> bool {
    green_parts(data).next().is_some()
}

impl Prepared {
    pub fn new(
        context: &cq_mac::MacContext,
        data: &crate::h09::Data,
    ) -> Result<Self, Box<dyn Error>> {
        if !has_green_demand(data) {
            return Ok(Self {
                source: None,
                ingestion: Duration::ZERO,
            });
        }
        let start = Instant::now();
        let source = context.relation_i64(data.lineitem.len(), |i| {
            let (order, part, supplier) = data.lineitem[i];
            [order, part, supplier]
        })?;
        Ok(Self {
            source: Some(source),
            ingestion: start.elapsed(),
        })
    }

    pub(crate) fn source(&self) -> Option<&cq_mac::Relation> {
        self.source.as_ref()
    }

    pub fn source_bytes(&self) -> usize {
        self.source
            .as_ref()
            .map_or(0, |s| s.len() * s.columns() * size_of::<i64>())
    }
}

pub struct Execution {
    pub result: crate::ManualResult<crate::h09::Answer>,
    pub construction: Duration,
}

fn gpu_part_key(value: i64, column: &str) -> Result<u32, io::Error> {
    u32::try_from(value).map_err(|_| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("H09 {column} value {value} is outside the GPU u32 key domain"),
        )
    })
}

pub fn execute(
    context: &cq_mac::MacContext,
    data: &crate::h09::Data,
    prepared: &Prepared,
    construct: &mut dyn FnMut(
        &cq_mac::MacContext,
        &cq_mac::Relation,
        cq_mac::IndexSpec,
        Option<&cq_mac::Relation>,
    ) -> Result<cq_mac::MacRun, cq_mac::MacError>,
) -> Result<Execution, Box<dyn Error>> {
    let mut result = crate::ManualResult {
        answers: Vec::new(),
        index_builds: 0,
        indexed_rows: 0,
        index_probes: 0,
        payload_gathers: 0,
        witnesses: 0,
    };

    // The Part/GreenNames join precedes every other join. Peeking demands only
    // its first match, so an empty green selection never builds.
    let mut green_parts = green_parts(data).peekable();
    if green_parts.peek().is_none() {
        return Ok(Execution {
            result,
            construction: Duration::ZERO,
        });
    }

    // The Lineitem part-key read: GPU projection over the retained full numeric
    // Lineitem. Original source row IDs retain wide signed order/supplier payloads.
    let source = prepared.source.as_ref().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "H09 prepared source has no green-part demand",
        )
    })?;
    let build_start = Instant::now();
    let lineitem = construct(
        context,
        source,
        cq_mac::IndexSpec {
            key_column: 1,
            payload: cq_mac::Payload::RowId,
        },
        None,
    )?;
    let construction = build_start.elapsed();
    let numeric = source.i64_values()?;
    result.index_builds = 1;
    result.indexed_rows = lineitem.len();

    // The current default route's remaining providers: PartSupp, Orders and
    // Nation are membership reads; Supplier is the only multiplicity-preserving
    // index, supplying its i32 nation key. Composite keys stay (i64, i64).
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
    let mut seen = HashSet::new();

    for part_key in green_parts {
        let key = gpu_part_key(part_key, "Part.p_partkey")?;
        result.index_probes += 1;
        // The exact lookup reads the completed shared buffer on the CPU.
        for row_id in lineitem.lookup(&key) {
            let offset = row_id as usize * 3;
            let (order_key, source_part_key, supplier_key) =
                (numeric[offset], numeric[offset + 1], numeric[offset + 2]);
            result.payload_gathers += 1;
            if !partsupp.contains(&(source_part_key, supplier_key)) || !orders.contains(&order_key)
            {
                continue;
            }
            // Preserve the original i32 nation-key payload and its multiplicity.
            for nation_key in supplier.lookup(&supplier_key) {
                if nation.contains(&nation_key) {
                    result.witnesses += 1;
                    let answer = (order_key, part_key, supplier_key);
                    if seen.insert(answer) {
                        result.answers.push(answer);
                    }
                }
            }
        }
    }
    Ok(Execution {
        result,
        construction,
    })
}
