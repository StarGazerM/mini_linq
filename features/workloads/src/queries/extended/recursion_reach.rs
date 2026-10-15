use crate::Scenario;

#[allow(non_snake_case)]
pub fn check(scenario: Scenario) {
    let case = crate::extended::case("recursion_reach").unwrap();
    let mut inputs = case.inputs(scenario);
    let Seed = inputs.take::<(i32,)>("Seed");
    let Edge = inputs.take::<(i32, i32)>("Edge");

    let result = mini_linq::compile! {
        pipeline {
            surface = [cq_ext_recursion::recursion];
            select = mini_linq::passes::select(positional(Seed, Edge));
        }
        recursion! {
            struct RecursionReach;
            relation Seed(x: i32);
            relation Edge(x: i32, y: i32);
            relation reach(x: i32);
            reach(*y) :- Seed(x), Edge(x, y);
            reach(*z) :- reach(y), Edge(y, z);
        }
    };
    let actual = result.map(|(column_0,)| vec![column_0]).collect();
    case.check(scenario, actual);
}
