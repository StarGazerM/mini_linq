use crate::Scenario;

#[allow(non_snake_case)]
pub fn check(scenario: Scenario) {
    let case = crate::extended::case("union_two").unwrap();
    let mut inputs = case.inputs(scenario);
    let A = inputs.take::<(i32,)>("A");
    let B = inputs.take::<(i32,)>("B");

    let result = mini_linq::compile! {
        pipeline {
            surface = [];
            select = mini_linq::passes::select(positional(A, B));
        }
        struct UnionTwo;
        relation A(x: i32);
        relation B(x: i32);
        relation answer(x: i32);
        answer(*x) :- A(x), if *x >= 0;
        answer(*x) :- B(x), if *x >= 0;
    };
    let actual = result.map(|(column_0,)| vec![column_0]).collect();
    case.check(scenario, actual);
}
