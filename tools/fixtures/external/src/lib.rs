#[test]
fn packaged_facade_accepts_an_external_rewrite() {
    #[derive(PartialEq, Eq, PartialOrd, Ord)]
    struct Key(i32);
    let rows = [(Key(1),), (Key(1),), (Key(2),)];
    let answer: Vec<_> = mini_linq::compile! {
        pipeline { structures = [external_pass_fixture::accept_sort(a + (b * c))]; }
        struct Q; relation rows(x: Key); answer(x) :- rows(x);
    }
    .collect();
    assert_eq!(answer.len(), 2);
    assert!(std::ptr::eq(answer[0].0, &rows[0].0));
}

#[test]
fn packaged_testkit_supplies_corpus_oracles_and_authored_rewrites() {
    let scenario = cq_testkit::SCENARIOS
        .iter()
        .find(|scenario| scenario.name == "triangle")
        .unwrap();
    let case = scenario.case().unwrap();
    assert_eq!(
        cq_testkit::cq_semantics::evaluate_cq(&case.parse_cq().unwrap(), &case.database()),
        case.expected_rows()
    );
    let source = scenario.parse_physical().unwrap();
    let target = cq_testkit::rewrites::merged(&source).unwrap();
    cq_testkit::physical::contract::check(&target).unwrap();
    cq_testkit::physical::check_rewrite(&source, &target).unwrap();
}
