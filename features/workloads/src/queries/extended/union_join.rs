use crate::Scenario;

#[allow(non_snake_case)]
pub fn check(scenario: Scenario) {
    let case = crate::extended::case("union_join").unwrap();
    let mut inputs = case.inputs(scenario);
    let A = inputs.take::<(i32,)>("A");
    let B = inputs.take::<(i32,)>("B");
    let C = inputs.take::<(i32,)>("C");
    let D = inputs.take::<(i32,)>("D");

    let result = mini_linq::compile! {
        pipeline {
            surface = [];
            select = mini_linq::passes::select(positional(A, B, C, D));
        }
        struct UnionJoin;
        relation A(x: i32);
        relation B(x: i32);
        relation C(x: i32);
        relation D(x: i32);
        relation answer(x: i32);
        answer(*x) :- A(x), B(x);
        answer(*x) :- C(x), D(x);
    };
    let actual = result.map(|(column_0,)| vec![column_0]).collect();
    case.check(scenario, actual);
}
