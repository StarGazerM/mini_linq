//! Synthetic retail_h22 core: filtered relations are explicit inputs.
//! Dates, strings, grouping, and ordering belong to the real-dbgen peer.
use crate::Scenario;

#[allow(non_snake_case)]
pub fn check(scenario: Scenario) {
    let case = crate::extended::case("retail_h22").unwrap();
    let mut inputs = case.inputs(scenario);
    let Customer = inputs.take::<(i32,)>("Customer");
    let Orders = inputs.take::<(i32, i32)>("Orders");
    let result = mini_linq::compile! {
        pipeline {}
        struct Query;
        relation Customer(custkey: i32);
        relation Orders(order_key: i32, custkey: i32);
        answer(custkey) :- Customer(custkey), !Orders(_, custkey);
    };
    case.check(scenario, result.map(|row| vec![*row.0]).collect());
}
