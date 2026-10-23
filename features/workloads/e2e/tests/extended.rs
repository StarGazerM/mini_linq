use cq_workloads::{Scenario, queries::extended};

#[test]
fn retail_h04() {
    for scenario in Scenario::ALL {
        extended::retail_h04::check(scenario);
    }
}

#[test]
fn retail_h04_not_exists() {
    for scenario in Scenario::ALL {
        extended::retail_h04_not_exists::check(scenario);
    }
}

#[test]
fn retail_h16() {
    for scenario in Scenario::ALL {
        extended::retail_h16::check(scenario);
    }
}

#[test]
fn retail_h22() {
    for scenario in Scenario::ALL {
        extended::retail_h22::check(scenario);
    }
}

#[test]
fn aggregate_join() {
    for scenario in Scenario::ALL {
        extended::aggregate_join::check(scenario);
    }
}

#[test]
fn aggregate_stats() {
    for scenario in Scenario::ALL {
        extended::aggregate_stats::check(scenario);
    }
}

#[test]
fn aggregate_two_keys() {
    for scenario in Scenario::ALL {
        extended::aggregate_two_keys::check(scenario);
    }
}

#[test]
fn correlated_exists() {
    for scenario in Scenario::ALL {
        extended::correlated_exists::check(scenario);
    }
}

#[test]
fn correlated_parameter() {
    for scenario in Scenario::ALL {
        extended::correlated_parameter::check(scenario);
    }
}

#[test]
fn correlated_scalar() {
    for scenario in Scenario::ALL {
        extended::correlated_scalar::check(scenario);
    }
}

#[test]
fn negation_key() {
    for scenario in Scenario::ALL {
        extended::negation_key::check(scenario);
    }
}

#[test]
fn negation_pair() {
    for scenario in Scenario::ALL {
        extended::negation_pair::check(scenario);
    }
}

#[test]
fn negation_twice() {
    for scenario in Scenario::ALL {
        extended::negation_twice::check(scenario);
    }
}

#[test]
fn recursion_closure() {
    for scenario in Scenario::ALL {
        extended::recursion_closure::check(scenario);
    }
}

#[test]
fn recursion_reach() {
    for scenario in Scenario::ALL {
        extended::recursion_reach::check(scenario);
    }
}

#[test]
fn recursion_reverse() {
    for scenario in Scenario::ALL {
        extended::recursion_reverse::check(scenario);
    }
}

#[test]
fn stacked_antijoin() {
    for scenario in Scenario::ALL {
        extended::stacked_antijoin::check(scenario);
    }
}

#[test]
fn stacked_mixed() {
    for scenario in Scenario::ALL {
        extended::stacked_mixed::check(scenario);
    }
}

#[test]
fn surface_arithmetic_guard() {
    for scenario in Scenario::ALL {
        extended::surface_arithmetic_guard::check(scenario);
    }
}

#[test]
fn surface_boolean_guard() {
    for scenario in Scenario::ALL {
        extended::surface_boolean_guard::check(scenario);
    }
}

#[test]
fn surface_compare() {
    for scenario in Scenario::ALL {
        extended::surface_compare::check(scenario);
    }
}

#[test]
fn surface_computed_head() {
    for scenario in Scenario::ALL {
        extended::surface_computed_head::check(scenario);
    }
}

#[test]
fn surface_constant() {
    for scenario in Scenario::ALL {
        extended::surface_constant::check(scenario);
    }
}

#[test]
fn surface_disconnected() {
    for scenario in Scenario::ALL {
        extended::surface_disconnected::check(scenario);
    }
}

#[test]
fn surface_let_chain() {
    for scenario in Scenario::ALL {
        extended::surface_let_chain::check(scenario);
    }
}

#[test]
fn surface_let_sum() {
    for scenario in Scenario::ALL {
        extended::surface_let_sum::check(scenario);
    }
}

#[test]
fn surface_negative_constant() {
    for scenario in Scenario::ALL {
        extended::surface_negative_constant::check(scenario);
    }
}

#[test]
fn surface_repeat() {
    for scenario in Scenario::ALL {
        extended::surface_repeat::check(scenario);
    }
}

#[test]
fn surface_self_join() {
    for scenario in Scenario::ALL {
        extended::surface_self_join::check(scenario);
    }
}

#[test]
fn surface_wildcards() {
    for scenario in Scenario::ALL {
        extended::surface_wildcards::check(scenario);
    }
}

#[test]
fn union_join() {
    for scenario in Scenario::ALL {
        extended::union_join::check(scenario);
    }
}

#[test]
fn union_project() {
    for scenario in Scenario::ALL {
        extended::union_project::check(scenario);
    }
}

#[test]
fn union_two() {
    for scenario in Scenario::ALL {
        extended::union_two::check(scenario);
    }
}
