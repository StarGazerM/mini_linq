use crate::Scenario;

#[allow(non_snake_case)]
pub fn check(scenario: Scenario) {
    let case = crate::extended::case("surface_constant").unwrap();
    let mut inputs = case.inputs(scenario);
    let R = inputs.take::<(i32, i32)>("R");

    let result = mini_linq::compile! {
        pipeline {
            surface = [];
            select = mini_linq::passes::select(positional(R));
        }
        struct SurfaceConstant;
        relation R(x: i32, tag: i32);
        relation answer(c0: i32);
        answer(*x) :- R(x, 7);
    };
    let actual = result.map(|(column_0,)| vec![column_0]).collect();
    case.check(scenario, actual);
}
