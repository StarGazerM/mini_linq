// A triangle has an actual candidate intersection, not an inactive pass flag.
fn main() {
    let R = [(1_i32, 2_i32)];
    let S = [(2_i32, 3_i32)];
    let T = [(3_i32, 1_i32)];
    let _ = mini_linq::compile! {
        pipeline {
            plan = cq_planners::order(written, coarse);
            physical = [cq_rewrites::smallest_intersections];
            print_input = structures;
            print = own_push::push;
        }
        struct Triangle;
        relation R(x:i32,y:i32); relation S(x:i32,y:i32); relation T(x:i32,y:i32);
        answer(x,y,z) :- R(x,y), S(y,z), T(z,x);
    }.count();
}
