// A one-column native BTreeMap key cannot satisfy a two-column merge prefix.
fn main() {
    let ABC = std::collections::BTreeMap::from([((1_i32,), vec![(2_i32, 3_i32)])]);
    let BCD = std::collections::BTreeMap::from([((2_i32,), vec![(3_i32, 4_i32)])]);
    let ACD = std::collections::BTreeMap::from([((1_i32,), vec![(3_i32, 4_i32)])]);
    let ABD = std::collections::BTreeMap::from([((1_i32,), vec![(2_i32, 4_i32)])]);
    let _ = mini_linq::compile! {
        pipeline {
            plan = cq_planners::order(hypergraph);
            physical = [cq_rewrites::merge_joins];
        }
        struct LoomisWhitney;
        relation ABC(a:i32,b:i32,c:i32);
        relation BCD(b:i32,c:i32,d:i32);
        relation ACD(a:i32,c:i32,d:i32);
        relation ABD(a:i32,b:i32,d:i32);
        answer(a,b,c,d) :- ABC(a,b,c), BCD(b,c,d), ACD(a,c,d), ABD(a,b,d);
    }.count();
}
