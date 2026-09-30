#[path = "wiring.rs"]
mod mini_linq;
fn main() {
    let _ = ::mini_linq::cq_to_ra! {
        struct Unexpanded;
        relation R(a: i32) = (&[(1,)]);
        answer(a) :- R(a), name! { a };
    };
}
