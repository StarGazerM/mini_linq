fn main() {
    let _ = ::mini_linq::mini_linq! {
        struct Unexpanded;
        relation R(a: i32) = (&[(1,)]);
        answer(a) :- R(a), name! { a };
    };
}
