use cq_workloads::{adversarial, costs};

#[test]
fn each_decision_has_linear_or_constant_vs_adversarial_growth() {
    let runs: Vec<_> = [24, 48, 96]
        .into_iter()
        .map(|n| costs::run(n, adversarial::SEED))
        .collect();
    costs::assert_growth(&runs);
    println!(
        "{}",
        costs::table(&runs.into_iter().flatten().collect::<Vec<_>>())
    );
}

#[test]
fn generators_and_all_counter_columns_are_reproducible() {
    assert_eq!(adversarial::triangle(24, 7), adversarial::triangle(24, 7));
    assert_ne!(adversarial::triangle(24, 7), adversarial::triangle(24, 8));
    let first = costs::run(24, adversarial::SEED);
    let second = costs::run(24, adversarial::SEED);
    for (a, b) in first.iter().zip(&second) {
        assert_eq!(
            (a.query, a.config, a.scale, a.counts, a.guard_visits),
            (b.query, b.config, b.scale, b.counts, b.guard_visits)
        );
    }
}

#[test]
fn yannakakis_canon_direct_reads_and_fine_orders_keep_their_strict_bounds() {
    let runs: Vec<_> = [24, 48, 96]
        .into_iter()
        .map(|n| costs::run(n, adversarial::SEED))
        .collect();
    costs::assert_non_fan_growth(&runs);
}
