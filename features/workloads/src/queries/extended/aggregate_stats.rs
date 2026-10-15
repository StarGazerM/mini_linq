use crate::Scenario;

#[allow(non_snake_case)]
pub fn check(scenario: Scenario) {
    let case = crate::extended::case("aggregate_stats").unwrap();
    let mut inputs = case.inputs(scenario);
    let Sale = inputs.take::<(i32, i32, i32)>("Sale");

    let result = mini_linq::compile! {
        pipeline {
            surface = [cq_ext_aggregate::aggregate];
            select = mini_linq::passes::select(positional(Sale));
        }
        aggregate! {
            by (d); n = count(); s = sum(a); lo = min(a); hi = max(a);
            struct AggregateStats;
            relation Sale(d: i32, id: i32, a: i32);
            rows(d, id, a) :- Sale(d, id, a), if *d > 0;
        }
    };
    let actual = result.iter().flat_map(|(key, payload)| {
        payload.iter().map(move |value| vec![
            key.0,
            i32::try_from(value.0).unwrap(),
            value.1,
            value.2,
            value.3,
        ])
    }).collect();
    case.check(scenario, actual);
}
