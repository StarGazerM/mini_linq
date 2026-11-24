use crate::Scenario;

#[allow(non_snake_case)]
pub fn check(scenario: Scenario) {
    let case = crate::extended::case("aggregate_two_keys").unwrap();
    let mut inputs = case.inputs(scenario);
    let Sale = inputs.take::<(i32, i32, i32, i32)>("Sale");

    let result = mini_linq::compile! {
        pipeline {
            surface = [cq_ext_aggregate::aggregate];
            select = mini_linq::passes::select(positional(Sale));
        }
        aggregate! {
            by (d, k); n = count(); s = sum(a);
            struct AggregateTwoKeys;
            relation Sale(d: i32, k: i32, id: i32, a: i32);
            rows(d, k, id, a) :- Sale(d, k, id, a), where d > 0;
        }
    };
    let actual = result
        .iter()
        .flat_map(|(key, payload)| {
            payload
                .iter()
                .map(move |value| vec![key.0, key.1, i32::try_from(value.0).unwrap(), value.1])
        })
        .collect();
    case.check(scenario, actual);
}
