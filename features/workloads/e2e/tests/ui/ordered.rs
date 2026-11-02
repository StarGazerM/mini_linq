// A recorded native ordered read must be implemented by the host. Without
// this declaration, selection may legitimately build an ordered index.
fn main() {
    let l = std::collections::HashMap::from([((1,), vec![(2,)])]);
    let r = std::collections::HashMap::from([((1,), vec![(3,)])]);
    let _ = ::mini_linq::passes::select! {
 pipeline { ::mini_linq::passes::sequence, ::mini_linq::passes::sequential, ::mini_linq::passes::translate_pull };
        physical {
            relation l(a: i32, b: i32) host [a] reads [ordered];
            relation r(a: i32, c: i32) host [a] reads [ordered];
            left = pipe rename l@0 {a -> a, b -> b};
            right = pipe rename r@1 {a -> a, c -> c};
            j = let canon merge[a] left, right; return j as answer(a, b, c);
        }
    }.count();
}
