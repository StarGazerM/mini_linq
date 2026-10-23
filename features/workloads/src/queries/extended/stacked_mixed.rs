use crate::Scenario;

#[allow(non_snake_case)]
pub fn check(scenario: Scenario) {
    let case = crate::extended::case("stacked_mixed").unwrap();
    let mut inputs = case.inputs(scenario);
    let A = inputs.take::<(i32,)>("A");
    let B = inputs.take::<(i32,)>("B");
    let Blocked = inputs.take::<(i32, i32)>("Blocked");
    let Hidden = inputs.take::<(i32, i32)>("Hidden");

    let result = mini_linq::compile! {
        pipeline {
            select = mini_linq::passes::select(positional(A, B, Blocked, Hidden));
        }
        struct StackedMixed;
        relation A(x: i32);
        relation B(x: i32);
        relation Blocked(x: i32, y: i32);
        relation Hidden(x: i32, y: i32);
        relation answer(x: i32);
        answer(*x) :- A(x), !Blocked(x, _), !Hidden(x, _);
        answer(*x) :- B(x), !Hidden(x, _);
    };
    let actual = result.map(|(column_0,)| vec![column_0]).collect();
    case.check(scenario, actual);
}
