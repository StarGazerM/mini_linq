// A two-column merge key uses its recorded fallback even when the raw host
// cursor has a shorter key; the fallback preserves complete joined rows.
fn main() {
    let l = std::collections::BTreeMap::from([((1_i32,), vec![(2_i32, 3_i32)])]);
    let r = std::collections::BTreeMap::from([((1_i32,), vec![(2_i32, 4_i32)])]);
    let rows: Vec<_> = mini_linq::passes::select! {
        pipeline {
            mini_linq::passes::sequence,
            mini_linq::passes::sequential,
            mini_linq::passes::translate_pull
        };
        physical {
            relation l(a:i32,b:i32,c:i32) host [a,b] reads [ordered];
            relation r(a:i32,b:i32,d:i32) host [a,b] reads [ordered];
            left = pipe rename l {a -> a, b -> b, c -> c};
            right = pipe rename r {a -> a, b -> b, d -> d};
            j = let canon merge[a,b] left, right;
            return j as answer(a,b,c,d);
        }
    }.map(|(a,b,c,d)| (*a,*b,*c,*d)).collect();
    assert_eq!(rows, [(1,2,3,4)]);
}
