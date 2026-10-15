use crate::Scenario;

#[allow(non_snake_case)]
pub fn check(scenario: Scenario) {
    let case = crate::extended::case("recursion_closure").unwrap();
    let mut inputs = case.inputs(scenario);
    let Edge = inputs.take::<(i32, i32)>("Edge");

    let result = mini_linq::compile! {
        pipeline {
            surface = [cq_ext_recursion::recursion];
            select = mini_linq::passes::select(positional(Edge));
        }
        recursion! {
            struct RecursionClosure;
            relation Edge(x: i32, y: i32);
            relation reach(x: i32, y: i32);
            reach(*x,*y) :- Edge(x, y), if *x >= 0;
            reach(*x,*z) :- reach(x, y), Edge(y, z);
        }
    };
    let actual = result.map(|(column_0, column_1,)| vec![column_0, column_1]).collect();
    case.check(scenario, actual);
}
