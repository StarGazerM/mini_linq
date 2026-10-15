use crate::Scenario;

#[allow(non_snake_case)]
pub fn check(scenario: Scenario) {
    let case = crate::extended::case("recursion_reverse").unwrap();
    let mut inputs = case.inputs(scenario);
    let Goal = inputs.take::<(i32,)>("Goal");
    let Edge = inputs.take::<(i32, i32)>("Edge");

    let result = mini_linq::compile! {
        pipeline {
            surface = [cq_ext_recursion::recursion];
            select = mini_linq::passes::select(positional(Goal, Edge));
        }
        recursion! {
            struct RecursionReverse;
            relation Goal(x: i32);
            relation Edge(x: i32, y: i32);
            relation reach(x: i32);
            reach(*y) :- Goal(x), Edge(y, x);
            reach(*z) :- reach(y), Edge(z, y);
        }
    };
    let actual = result.map(|(column_0,)| vec![column_0]).collect();
    case.check(scenario, actual);
}
