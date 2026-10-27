use cq_storage::{
    AdaptiveKeySet, Build, ExactMembership, HashIndex, I64Codec, IndexRead, JoinFilterPredicate,
    ReadPredicate,
};
use std::{
    collections::HashSet,
    error::Error,
    io,
    time::{Duration, Instant},
};

/// Full numeric Orders rows, ingested once without index-specific packing.
pub struct Prepared {
    source: cq_mac::Relation,
    pub ingestion: Duration,
}

impl Prepared {
    pub fn new(
        context: &cq_mac::MacContext,
        data: &crate::h03::Data,
    ) -> Result<Self, Box<dyn Error>> {
        let start = Instant::now();
        let source = context.relation_i64(data.orders.len(), |i| {
            let (order, customer, date) = data.orders[i];
            [order, customer, i64::from(date.days_since_epoch())]
        })?;
        Ok(Self {
            source,
            ingestion: start.elapsed(),
        })
    }

    pub fn source_bytes(&self) -> usize {
        self.source.len() * self.source.columns() * size_of::<i64>()
    }
}

pub fn execute(
    context: &cq_mac::MacContext,
    data: &crate::h03::Data,
    prepared: &Prepared,
) -> Result<crate::ManualResult<crate::h03::Answer>, Box<dyn Error>> {
    let cutoff = crate::tpch_runtime::date("1995-03-15");

    // Expanded r2: CPU Customer(customer_key -> ()), after the string predicate.
    // Retain every occurrence: membership reduces Orders, but the final lookup
    // must still enumerate duplicate Customer witnesses before canon.
    let customers: HashIndex<'_, i64, ()> = HashIndex::build(
        data.customer
            .iter()
            .filter(|(_, segment)| segment == "BUILDING")
            .map(|(customer_key, _)| (*customer_key, ())),
    );

    // The CPU opaque predicate/semijoin writes only a shared selection mask.
    // GPU compaction projects the selected keys and ORIGINAL source row IDs.
    let numeric = prepared.source.i64_values()?;
    let cutoff_days = i64::from(cutoff.days_since_epoch());
    let mut distinct_order_keys = HashSet::new();
    let selection = context.relation_u32(prepared.source.len(), |row_id| {
        let offset = row_id * 3;
        let selected =
            numeric[offset + 2] < cutoff_days && customers.contains(&numeric[offset + 1]);
        if selected {
            // This is the original CPU join-filter operator, not index packing.
            distinct_order_keys.insert(numeric[offset]);
        }
        [u32::from(selected)]
    })?;
    let orders = context.build(
        &prepared.source,
        cq_mac::IndexSpec {
            key_column: 0,
            payload: cq_mac::Payload::RowId,
        },
        Some(&selection),
    )?;
    let indexed_rows = orders.len();

    // Preserve the expanded optional r4 join filter, including its unknown
    // population argument (None), rather than substituting source row count.
    // Distinct keys come from exactly the selected r3 build, never raw Orders.
    let join_filter = cq_storage::selective_build(distinct_order_keys.len(), None).then(|| {
        AdaptiveKeySet::<i64, I64Codec, 67108864>::from_keys(distinct_order_keys, I64Codec)
    });
    let predicate = JoinFilterPredicate(join_filter.as_ref());

    let mut result = crate::ManualResult {
        answers: Vec::new(),
        index_builds: 1,
        indexed_rows,
        index_probes: 0,
        payload_gathers: 0,
        witnesses: 0,
    };
    let mut seen = HashSet::new();
    // Realize the positional Lineitem scan in source order on the CPU. The
    // date test and conservative filter precede each exact shared-buffer probe;
    // only the selected Orders index was built on the GPU, not Lineitem.
    for (order_key, ship_date) in &data.lineitem {
        if *ship_date <= cutoff || !predicate.test(order_key) {
            continue;
        }
        let key = encode_order_key(*order_key, "Lineitem.l_orderkey")?;
        result.index_probes += 1;
        for row_id in orders.lookup(&key) {
            let customer_key = &numeric[row_id as usize * 3 + 1];
            result.payload_gathers += 1;
            for () in customers.lookup(customer_key) {
                result.witnesses += 1;
                let answer = (*order_key, *customer_key);
                if seen.insert(answer) {
                    result.answers.push(answer);
                }
            }
        }
    }
    Ok(result)
}

fn encode_order_key(key: i64, column: &str) -> Result<u32, io::Error> {
    u32::try_from(key).map_err(|_| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("H03 {column} key {key} is outside the GPU u32 key domain"),
        )
    })
}
