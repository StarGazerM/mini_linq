#[test]
fn every_generated_case_is_accounted_for() {
    let mut cases = 0;
    let mut rejected = 0;
    let mut configurations = 0;
    let mut names = std::collections::BTreeSet::new();
    for line in include_str!(concat!(env!("OUT_DIR"), "/cases.tsv"))
        .lines()
        .skip(1)
    {
        let fields: Vec<_> = line.split('\t').collect();
        assert!(
            names.insert((fields[0], fields[1])),
            "duplicate case: {line}"
        );
        configurations += 1;
        match fields[2] {
            "run" => cases += fields[3].parse::<usize>().unwrap(),
            "compile-fail" => {
                assert!(!fields[4].is_empty(), "rejection needs a reason");
                rejected += 1;
            }
            _ => panic!("unclassified case: {line}"),
        }
    }
    assert_eq!(configurations, 4 * 864 + 16 * 27);
    assert_eq!(cases.to_string(), env!("E2E_DIFFERENTIAL_CASES"));
    assert_eq!(rejected.to_string(), env!("E2E_COMPILE_FAIL_CASES"));
    println!("{cases} differential scenario cases; {rejected} rejected configurations");
}
