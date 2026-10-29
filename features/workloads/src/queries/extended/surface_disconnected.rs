use crate::Scenario;

#[allow(non_snake_case)]
pub fn check(scenario: Scenario) {
    let case = crate::extended::case("surface_disconnected").unwrap();
    let mut inputs = case.inputs(scenario);
    let R = inputs.take::<(i32, i32)>("R");
    let S = inputs.take::<(i32,)>("S");

    let result = mini_linq::compile! {
        pipeline {
            surface = [];
            select = mini_linq::passes::select(positional(R, S));
        }
        struct SurfaceDisconnected;
        relation R(x: i32, unused: i32);
        relation S(y: i32);
        relation answer(c0: i32);
        answer(*difference) :- R(x, _), S(y), let difference: i32 = *x - *y,
            where x > y;
    };
    let actual = result.map(|(column_0,)| vec![column_0]).collect();
    case.check(scenario, actual);
}
