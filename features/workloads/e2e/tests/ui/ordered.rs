#[path = "wiring.rs"]
mod mini_linq;
fn main() {
    let l = std::collections::HashMap::from([((1,), vec![(2,)])]);
    let r = std::collections::HashMap::from([((1,), vec![(3,)])]);
    let _ = ::mini_linq::select_structures! {
        physical {
            relation l(a: i32, b: i32); relation r(a: i32, c: i32);
            left = pipe rename l {a -> a, b -> b};
            right = pipe rename r {a -> a, c -> c};
            j = pipe canon merge[a] left, right; return j as answer(a, b, c);
        } policy {join = merge;}
    }.count();
}
