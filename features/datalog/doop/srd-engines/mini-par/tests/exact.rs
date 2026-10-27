#[test]
fn exact_field_flow_all_relations() {
    srd_doop_shared::fixture::assert_results(srd_doop_mini_par::run(
        srd_doop_shared::fixture::facts(),
    ));
}
