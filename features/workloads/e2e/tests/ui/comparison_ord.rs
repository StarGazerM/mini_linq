
#[derive(PartialEq, Eq)]
struct EqOnly(i32);

fn main() {
    let rows = [(EqOnly(1), 1)];
    let cutoff = EqOnly(2);
    let _ = ::mini_linq::mini_linq! {
        struct MissingOrd; relation rows(key:EqOnly, value:i32);
        answer(y) :- rows(x,y), if * x < cutoff;
    }.count();
}
