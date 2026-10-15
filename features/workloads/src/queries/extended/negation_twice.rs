use crate::Scenario;

#[allow(non_snake_case)]
pub fn check(scenario: Scenario) {
    let case = crate::extended::case("negation_twice").unwrap();
    let mut inputs = case.inputs(scenario);
    let A = inputs.take::<(i32,)>("A");
    let B = inputs.take::<(i32, i32)>("B");
    let C = inputs.take::<(i32, i32)>("C");

    let result = mini_linq::compile! {
        pipeline {
            surface = [cq_ext_negation::negation];
            select = mini_linq::passes::select(positional(A, B, C));
        }
        struct NegationTwice;
        relation A(x: i32);
        relation B(x: i32, y: i32);
        relation C(x: i32, y: i32);
        relation answer(x: i32);
        answer(*x) :- A(x), neg!{ B(x, _) }, neg!{ C(x, _) };
    };
    let actual = result.map(|(column_0,)| vec![column_0]).collect();
    case.check(scenario, actual);
}
