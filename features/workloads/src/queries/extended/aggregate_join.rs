use crate::Scenario;

#[allow(non_snake_case)]
pub fn check(scenario: Scenario) {
    let case = crate::extended::case("aggregate_join").unwrap();
    let mut inputs = case.inputs(scenario);
    let Sale = inputs.take::<(i32, i32, i32)>("Sale");
    let Allowed = inputs.take::<(i32,)>("Allowed");

    let result = mini_linq::compile! {
        pipeline {
            surface = [cq_ext_aggregate::aggregate];
            select = mini_linq::passes::select(positional(Sale, Allowed));
        }
        aggregate! {
            by (d); s = sum(a); lo = min(a);
            struct AggregateJoin;
            relation Sale(d: i32, id: i32, a: i32);
            relation Allowed(d: i32);
            rows(d, id, a) :- Sale(d, id, a), Allowed(d);
        }
    };
    let actual = result.iter().flat_map(|(key, payload)| {
        payload.iter().map(move |value| vec![
            key.0,
            value.0,
            value.1,
        ])
    }).collect();
    case.check(scenario, actual);
}
