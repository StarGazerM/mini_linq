#[test]
fn field_flow_matches_all_expected_relations() {
    // One complete evaluation keeps device work serial within this test binary.
    // CUDA initialization failures are errors, not skipped comparisons.
    let _pool = gpu_datalog_runtime::thread_pool().expect("initialize CUDA");
    let actual = srd_doop_gpu::run(srd_doop_shared::fixture::facts());
    srd_doop_shared::fixture::assert_results(actual);
}
