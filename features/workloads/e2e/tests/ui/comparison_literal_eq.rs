
#[derive(PartialEq, Eq)]
struct Value(i32);
impl PartialEq<i32> for Value {
    fn eq(&self, _: &i32) -> bool { panic!("cross-type Rust comparison is not relational identity") }
}

fn main() {
    let rows = [(Value(1), 1)];
    let _ = ::mini_linq::mini_linq! {
        struct LiteralType; relation rows(key:Value, value:i32);
        answer(y) :- rows(x,y), if * x == 1;
    }.count();
}
