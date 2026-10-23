//! Synthetic retail_h04_not_exists core: filtered relations are explicit inputs.
//! Dates, strings, grouping, and ordering belong to the real-dbgen peer.
use crate::Scenario;

#[allow(non_snake_case)]
pub fn check(scenario: Scenario) {
    let case = crate::extended::case("retail_h04_not_exists").unwrap();
    let mut inputs = case.inputs(scenario);
    let Orders = inputs.take::<(i32,)>("Orders");
    let LateLineitem = inputs.take::<(i32,)>("LateLineitem");
    let result = mini_linq::compile! {
        pipeline {}
        struct Query;
        relation Orders(order_key: i32);
        relation LateLineitem(order_key: i32);
        answer(order_key) :- Orders(order_key), !LateLineitem(order_key);
    };
    case.check(scenario, result.map(|row| vec![*row.0]).collect());
}
