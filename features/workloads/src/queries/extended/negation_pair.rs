use crate::Scenario;

#[allow(non_snake_case)]
pub fn check(scenario: Scenario) {
    let case = crate::extended::case("negation_pair").unwrap();
    let mut inputs = case.inputs(scenario);
    let A = inputs.take::<(i32, i32)>("A");
    let S = inputs.take::<(i32, i32)>("S");

    let result = mini_linq::compile! {
        pipeline {
            select = mini_linq::passes::select(positional(A, S));
        }
        struct NegationPair;
        relation A(x: i32, y: i32);
        relation S(x: i32, y: i32);
        relation answer(x: i32, y: i32);
        answer(*x,*y) :- A(x, y), !S(x, y);
    };
    let actual = result
        .map(|(column_0, column_1)| vec![column_0, column_1])
        .collect();
    case.check(scenario, actual);
}
