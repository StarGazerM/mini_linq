//! CUDA Sequence versus the Sequential translation of the same Physical H05.
//! Both execute on identical encoded rows from /data/tpch-gpu/sf1. The CPU
//! reference uses default structures and lowering, not the GPU Sequence terms.
#![recursion_limit = "1024"]

use cq_gpu_runtime::{DeviceColumns, Pool};
use cq_testkit::conformance::assert_answers_equal;
use gpu_compress::{Catalog, Source};
use gpu_compress_host::tpch::schema;
use std::{error::Error, path::Path};

#[allow(dead_code)]
#[path = "support/h05_program.rs"]
mod h05_program;
#[path = "support/h05_rows.rs"]
mod h05_rows;

use h05_rows::Rows;

type Result<T, E = Box<dyn Error>> = std::result::Result<T, E>;
type Answer = (u32, u32, u32);

struct HostRows {
    customer: Vec<(u32, u32)>,
    orders: Vec<(u32, u32, i32)>,
    lineitem: Vec<(u32, u32)>,
    supplier: Vec<(u32, u32)>,
    nation: Vec<(u32, u32)>,
    region: Vec<(u32, i32)>,
}

impl HostRows {
    fn download(data: &Rows) -> Result<Self> {
        // Download the actual query bindings: no second source parser, column
        // ordering, or dictionary encoding can diverge between the targets.
        Ok(Self {
            customer: data.customer.to_host()?,
            orders: data.orders.to_host()?,
            lineitem: data.lineitem.to_host()?,
            supplier: data.supplier.to_host()?,
            nation: data.nation.to_host()?,
            region: data.region.to_host()?,
        })
    }
}

fn cuda(data: &Rows) -> DeviceColumns<'_, Answer> {
    cq_gpu_sequence_test_macros::h05!(1048576)
}

fn sequential(data: &HostRows) -> Vec<Answer> {
    cq_gpu_sequence_test_macros::h05_sequential_physical_reference!(1048576)
        .map(|(order, supplier, nation)| (*order, *supplier, *nation))
        .collect()
}

fn planned_a(data: &Rows) -> DeviceColumns<'_, Answer> {
    cq_gpu_sequence_test_macros::planned_h05!(A)
}

fn planned_b(data: &Rows) -> DeviceColumns<'_, Answer> {
    cq_gpu_sequence_test_macros::planned_h05!(B)
}

fn planned_r(data: &Rows) -> DeviceColumns<'_, Answer> {
    cq_gpu_sequence_test_macros::planned_h05!(R)
}

fn sequential_a(data: &HostRows) -> Vec<Answer> {
    cq_gpu_sequence_test_macros::h05_physical_reference!(A)
        .map(|(order, supplier, nation)| (*order, *supplier, *nation))
        .collect()
}

fn sequential_b(data: &HostRows) -> Vec<Answer> {
    cq_gpu_sequence_test_macros::h05_physical_reference!(B)
        .map(|(order, supplier, nation)| (*order, *supplier, *nation))
        .collect()
}

fn sequential_r(data: &HostRows) -> Vec<Answer> {
    cq_gpu_sequence_test_macros::h05_physical_reference!(R)
        .map(|(order, supplier, nation)| (*order, *supplier, *nation))
        .collect()
}

#[test]
fn h05_cuda_matches_sequential_physical_reference_on_sf1() -> Result<()> {
    let directory = Path::new("/data/tpch-gpu/sf1");
    if !directory.is_dir() {
        eprintln!(
            "SKIP h05_cuda_matches_sequential_physical_reference_on_sf1: {} is missing",
            directory.display()
        );
        return Ok(());
    }

    let source = Source::load_manifest(directory)?;
    let constant = |name: &str| -> Result<i32> {
        Ok(i32::try_from(
            source.manifest()["constants"][name]
                .as_i64()
                .ok_or_else(|| format!("missing manifest constant {name}"))?,
        )?)
    };
    assert_eq!(
        (constant("date_start_days")?, constant("date_end_days")?),
        (h05_program::DATE_START, h05_program::DATE_END),
        "H05 fixture date literals must match the sf1 input manifest"
    );
    let pool = Pool::new(0, 0, 0)?;
    let catalog = Catalog::compress(&pool, &source, &schema())?;
    assert_eq!(constant("asia_id")?, h05_program::ASIA);
    assert_eq!(catalog.text_id("region.r_name", "ASIA")?, h05_program::ASIA);
    let resident = Rows::bind(&catalog)?;
    drop(source);
    let host = HostRows::download(&resident)?;

    let expected = sequential(&host);
    let actual = cuda(&resident).to_host()?;
    // Both translations retain the fixture's return order (order, supplier,
    // nation) and use the same dense dictionaries, so no answer remapping is
    // needed. H05 explicitly canonicalizes its answer: compare supports.
    assert_answers_equal(
        "Sequential Physical H05 reference (sf1)",
        expected,
        "CUDA Sequence H05 (sf1)",
        actual,
        true,
    );
    for (name, expected, actual) in [
        ("A", sequential_a(&host), planned_a(&resident).to_host()?),
        ("B", sequential_b(&host), planned_b(&resident).to_host()?),
        ("R", sequential_r(&host), planned_r(&resident).to_host()?),
    ] {
        assert_answers_equal(
            &format!("Sequential Physical H05 {name} (sf1)"),
            expected,
            &format!("planned CUDA H05 {name} (sf1)"),
            actual,
            true,
        );
    }
    Ok(())
}
