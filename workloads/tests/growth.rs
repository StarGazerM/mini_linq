use cq_workloads::{adversarial, costs};

#[test]
fn checked_shapes_have_recorded_linear_constant_or_quadratic_growth() {
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
