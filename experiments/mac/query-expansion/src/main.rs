use cq_mac::MacContext;
use mac_query_expansion::{ManualResult, fixtures, h03, h09, manual_h03, manual_h09};
use std::{error::Error, time::Instant};

type Result<T, E = Box<dyn Error>> = std::result::Result<T, E>;

fn check<T: Ord + std::fmt::Debug>(
    name: &str,
    mut expected: Vec<T>,
    mut actual: ManualResult<T>,
    compiled_ms: f64,
    manual_ms: f64,
) -> ManualResult<T> {
    expected.sort_unstable();
    actual.answers.sort_unstable();
    assert_eq!(actual.answers, expected, "{name}: complete answer mismatch");
    println!(
        "{name}: answers={} witnesses={} index_builds={} indexed_rows={} index_probes={} payload_gathers={} compiled_ms={compiled_ms:.3} manual_ms={manual_ms:.3}",
        actual.answers.len(),
        actual.witnesses,
        actual.index_builds,
        actual.indexed_rows,
        actual.index_probes,
        actual.payload_gathers
    );
    actual
}

fn run_h03(
    context: &MacContext,
    name: &str,
    data: &h03::Data,
) -> Result<ManualResult<h03::Answer>> {
    let start = Instant::now();
    let expected = h03::compiled(data);
    let compiled_ms = start.elapsed().as_secs_f64() * 1000.0;
    let prepared = manual_h03::Prepared::new(context, data)?;
    let start = Instant::now();
    let actual = manual_h03::execute(context, data, &prepared)?;
    let manual_ms = start.elapsed().as_secs_f64() * 1000.0;
    let start = Instant::now();
    let repeated = manual_h03::execute(context, data, &prepared)?;
    let repeat_ms = start.elapsed().as_secs_f64() * 1000.0;
    assert_eq!(
        actual.answers, repeated.answers,
        "{name}: repeated answer order"
    );
    assert_eq!(
        actual.witnesses, repeated.witnesses,
        "{name}: repeated witnesses"
    );
    let ingestion_ms = prepared.ingestion.as_secs_f64() * 1000.0;
    println!(
        "{name}: ingestion_ms={ingestion_ms:.3} source_bytes={} ingestion_plus_first_ms={:.3} repeat_ms={repeat_ms:.3}",
        prepared.source_bytes(),
        ingestion_ms + manual_ms
    );
    Ok(check(name, expected, actual, compiled_ms, manual_ms))
}

fn run_h09(
    context: &MacContext,
    name: &str,
    data: &h09::Data,
) -> Result<ManualResult<h09::Answer>> {
    let start = Instant::now();
    let expected = h09::compiled(data);
    let compiled_ms = start.elapsed().as_secs_f64() * 1000.0;
    let prepared = manual_h09::Prepared::new(context, data)?;
    let start = Instant::now();
    let first = manual_h09::execute(context, data, &prepared, &mut MacContext::build)?;
    let manual_ms = start.elapsed().as_secs_f64() * 1000.0;
    let start = Instant::now();
    let repeated = manual_h09::execute(context, data, &prepared, &mut MacContext::build)?;
    let repeat_ms = start.elapsed().as_secs_f64() * 1000.0;
    assert_eq!(
        first.result.answers, repeated.result.answers,
        "{name}: repeated answer order"
    );
    assert_eq!(
        first.result.witnesses, repeated.result.witnesses,
        "{name}: repeated witnesses"
    );
    let ingestion_ms = prepared.ingestion.as_secs_f64() * 1000.0;
    println!(
        "{name}: ingestion_ms={ingestion_ms:.3} source_bytes={} ingestion_plus_first_ms={:.3} construction_ms={:.3} repeat_ms={repeat_ms:.3} repeat_construction_ms={:.3}",
        prepared.source_bytes(),
        ingestion_ms + manual_ms,
        first.construction.as_secs_f64() * 1000.0,
        repeated.construction.as_secs_f64() * 1000.0
    );
    Ok(check(name, expected, first.result, compiled_ms, manual_ms))
}

fn main() -> Result<()> {
    let n: usize = std::env::args()
        .nth(1)
        .map(|arg| arg.parse())
        .transpose()?
        .unwrap_or(65_539);
    let start = Instant::now();
    let context = MacContext::new()?;
    println!(
        "Metal context setup_ms={:.3}",
        start.elapsed().as_secs_f64() * 1000.0
    );
    let boundary03 = fixtures::h03_boundaries();
    let boundary09 = fixtures::h09_boundaries();
    let result03 = run_h03(&context, "H03/boundaries", &boundary03)?;
    assert_eq!(
        result03.answers,
        [(10, 1), (10, 3), (15, -7), (u32::MAX as i64, i64::MAX)]
    );
    assert_eq!(result03.witnesses, 12);
    let result09 = run_h09(&context, "H09/boundaries", &boundary09)?;
    assert_eq!(
        result09.answers,
        [
            (-1, 3, i64::MAX),
            (100, 1, 7),
            (101, 1, 8),
            (300, u32::MAX as i64, -9)
        ]
    );
    assert_eq!(result09.witnesses, 68);

    let mut empty03 = boundary03.clone();
    empty03.customer.clear();
    empty03.orders.clear();
    empty03.lineitem.clear();
    run_h03(&context, "H03/empty", &empty03)?;
    let mut no_match03 = boundary03.clone();
    no_match03
        .customer
        .iter_mut()
        .for_each(|(_, segment)| *segment = "AUTOMOBILE".into());
    run_h03(&context, "H03/no-building", &no_match03)?;

    let mut empty09 = boundary09.clone();
    empty09.lineitem.clear();
    run_h09(&context, "H09/empty-lineitem", &empty09)?;
    let mut no_match09 = boundary09.clone();
    no_match09
        .part
        .iter_mut()
        .for_each(|(_, name)| *name = "red".into());
    let no_match = run_h09(&context, "H09/no-green", &no_match09)?;
    assert_eq!(no_match.index_builds, 0, "preserve lazy index construction");
    assert_eq!(no_match.indexed_rows, 0);

    run_h03(&context, "H03/scaled", &fixtures::h03_scaled(n))?;
    run_h09(&context, "H09/scaled", &fixtures::h09_scaled(n))?;

    // The authored i64 domain is wider than the GPU key domain. Demonstrate
    // the boundary rather than silently truncating or disguising a fallback.
    let mut wide03 = boundary03.clone();
    wide03.orders.push((
        i64::MAX,
        1,
        mac_query_expansion::tpch_runtime::date("1995-03-14"),
    ));
    wide03.lineitem.push((
        i64::MAX,
        mac_query_expansion::tpch_runtime::date("1995-03-16"),
    ));
    assert!(h03::compiled(&wide03).contains(&(i64::MAX, 1)));
    let prepared03 = manual_h03::Prepared::new(&context, &wide03)?;
    assert!(manual_h03::execute(&context, &wide03, &prepared03).is_err());
    let mut wide09 = boundary09.clone();
    wide09.part.push((-1, "green".into()));
    wide09.lineitem.push((100, -1, 7));
    wide09.partsupp.push((-1, 7));
    assert!(h09::compiled(&wide09).contains(&(100, -1, 7)));
    let prepared09 = manual_h09::Prepared::new(&context, &wide09)?;
    assert!(manual_h09::execute(&context, &wide09, &prepared09, &mut MacContext::build).is_err());
    println!(
        "Full signed/wide payloads preserved via row IDs; unsupported GPU keys rejected explicitly."
    );
    println!(
        "Exact authored-query comparisons passed. Synthetic inputs; first/repeated query timings include builds and CPU joins, exclude separately reported one-time ingestion/context setup/input generation. Not a crossover benchmark."
    );
    Ok(())
}
