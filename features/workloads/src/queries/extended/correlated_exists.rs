use crate::Scenario;

#[allow(non_snake_case)]
pub fn check(scenario: Scenario) {
    let case = crate::extended::case("correlated_exists").unwrap();
    let mut inputs = case.inputs(scenario);
    let A = inputs.take::<(i32,)>("A");
    let S = inputs.take::<(i32, i32)>("S");

    let limit = 2;

    let result = mini_linq::compile! {
        pipeline {
            surface = [cq_ext_correlated::correlated];
            select = mini_linq::passes::select(positional(A, S));
        }
        struct CorrelatedExists;
        relation A(x: i32);
        relation S(x: i32, y: i32);
        relation answer(x: i32);
        answer(*x) :- A(x), exists!{ S(x, y), where y > limit };
    };
    let actual = result.map(|(column_0,)| vec![column_0]).collect();
    case.check(scenario, actual);
}
