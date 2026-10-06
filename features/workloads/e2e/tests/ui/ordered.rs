fn main() {
    let l = std::collections::HashMap::from([((1,), vec![(2,)])]);
    let r = std::collections::HashMap::from([((1,), vec![(3,)])]);
    let _ = ::mini_linq::passes::select! {
 pipeline { ::mini_linq::passes::print_pull };
        physical {
            relation l(a: i32, b: i32); relation r(a: i32, c: i32);
            left = pipe rename l@0 {a -> a, b -> b};
            right = pipe rename r@1 {a -> a, c -> c};
            j = pipe canon merge[a] left, right; return j as answer(a, b, c);
        }
    }.count();
}
