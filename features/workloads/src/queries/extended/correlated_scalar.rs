use crate::Scenario;

#[allow(non_snake_case)]
pub fn check(scenario: Scenario) {
    let case = crate::extended::case("correlated_scalar").unwrap();
    let mut inputs = case.inputs(scenario);
    let A = inputs.take::<(i32,)>("A");
    let S = inputs.take::<(i32, i32, i32)>("S");

    let limit = 2;

    let result = mini_linq::compile! {
        pipeline {
            surface = [cq_ext_correlated::correlated];
            select = mini_linq::passes::select(positional(A, S));
        }
        struct CorrelatedScalar;
        relation A(x: i32);
        relation S(x: i32, y: i32, id: i32);
        relation answer(x: i32, v: i32);
        answer(*x, v) :- A(x), scalar!{ v: i32=(0); select (*y); S(x, y, id) }, where v > limit;
    };
    let actual = result
        .map(|(column_0, column_1)| vec![column_0, column_1])
        .collect();
    case.check(scenario, actual);
}
