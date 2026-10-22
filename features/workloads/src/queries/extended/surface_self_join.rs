use crate::Scenario;

#[allow(non_snake_case)]
pub fn check(scenario: Scenario) {
    let case = crate::extended::case("surface_self_join").unwrap();
    let mut inputs = case.inputs(scenario);
    let R = inputs.take::<(i32, i32)>("R");

    let result = mini_linq::compile! {
        pipeline {
            surface = [];
            select = mini_linq::passes::select(positional(R));
        }
        struct SurfaceSelfJoin;
        relation R(x: i32, y: i32);
        relation answer(c0: i32, c1: i32);
        answer(*x, *z + 1) :- R(x, y), R(y, z), if *x < *z;
    };
    let actual = result
        .map(|(column_0, column_1)| vec![column_0, column_1])
        .collect();
    case.check(scenario, actual);
}
