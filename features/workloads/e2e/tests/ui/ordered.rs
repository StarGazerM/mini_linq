// An actually consumed native ordered window requires OrderedRead.
// Merge readers instead use their selected ordered fallback.
fn main() {
    let l = std::collections::HashMap::from([((1,), vec![(2,)])]);
    let _ = ::mini_linq::passes::select! {
 pipeline { ::mini_linq::passes::sequence, ::mini_linq::passes::sequential, ::mini_linq::passes::translate_pull };
        physical {
            relation l(a: i32, b: i32) host [a] reads [ordered];
            left = pipe rename l@0 {a -> a, b -> b};
            j = let canon range select left if (a > 0); return j as answer(a, b);
        }
    }.count();
}
