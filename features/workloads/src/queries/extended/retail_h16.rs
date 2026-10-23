//! Synthetic retail_h16 core: filtered relations are explicit inputs.
//! Dates, strings, grouping, and ordering belong to the real-dbgen peer.
use crate::Scenario;

#[allow(non_snake_case)]
pub fn check(scenario: Scenario) {
    let case = crate::extended::case("retail_h16").unwrap();
    let mut inputs = case.inputs(scenario);
    let Part = inputs.take::<(i32,)>("Part");
    let Partsupp = inputs.take::<(i32, i32)>("Partsupp");
    let BadSupplier = inputs.take::<(i32,)>("BadSupplier");
    let result = mini_linq::compile! {
        pipeline {}
        struct Query;
        relation Part(part_key: i32);
        relation Partsupp(part_key: i32, suppkey: i32);
        relation BadSupplier(suppkey: i32);
        answer(part_key, suppkey) :- Part(part_key), Partsupp(part_key, suppkey), !BadSupplier(suppkey);
    };
    case.check(scenario, result.map(|row| vec![*row.0, *row.1]).collect());
}
