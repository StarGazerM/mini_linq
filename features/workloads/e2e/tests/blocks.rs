#![allow(non_snake_case)]

#[test]
fn both_printers_build_independent_union_indices_directly() {
    let L = [(1_i32,), (2,)];
    let A = [(1_i32, 10_i32), (1, 10), (2, 20)];
    let B = [(1_i32, 10_i32), (3, 30)];
    macro_rules! run { ($printer:path) => { mini_linq::passes::select! {
        pipeline {$printer};
        physical {
            relation L(a:i32); relation A(a:i32,b:i32); relation B(a:i32,b:i32);
            l = pipe rename L@0 {a -> x}; a = pipe rename A@1.0.0 {a -> x,b -> y}; b = pipe rename B@1.1.0 {a -> x,b -> y};
            d = let canon union a,b block @1 free[x] interface[y];
            j = pipe probe first [x] l,d; q = pipe project j keep {y}; out = pipe canon semi eager [y] q,d; return out as answer(y);
        }
    }.map(|(y,)|*y).collect::<std::collections::BTreeSet<_>>() }; }
    let pull = run!(mini_linq::passes::print_pull);
    let push = run!(own_push::push);
    assert_eq!(pull, push);
    assert_eq!(pull, std::collections::BTreeSet::from([10, 20]));
}

#[test]
fn both_printers_keep_curried_union_indices_and_a_recorded_scan() {
    let L = [(1_i32, 10_i32), (2, 20), (9, 90)];
    let A = [(1_i32, 10_i32, 100_i32), (1, 11, 101), (2, 20, 200)];
    let B = [(1_i32, 10_i32, 100_i32), (3, 30, 300)];
    macro_rules! run { ($printer:path) => { mini_linq::passes::select! {
        pipeline {$printer};
        physical {
            relation L(a:i32,b:i32); relation A(a:i32,b:i32,c:i32); relation B(a:i32,b:i32,c:i32);
            l = pipe rename L@0 {a -> x,b -> y}; a = pipe rename A@1.0.0 {a -> x,b -> y,c -> z}; b = pipe rename B@1.1.0 {a -> x,b -> y,c -> z};
            d = let canon union a,b block @1 free[x,y] interface[z];
            p = pipe project l keep {x}; checked = pipe semi eager [x] p,d;
            c = pipe probe first [x] checked,l; j = pipe probe first [x,y] c,d;
            aux = let project d keep {x,y}; out = pipe canon semi eager [x,y] j,aux;
            result = pipe canon project out keep {x,z}; return result as answer(x,z);
        }
    }.map(|(x,z)|(*x,*z)).collect::<std::collections::BTreeSet<_>>() }; }
    let pull = run!(mini_linq::passes::print_pull);
    let push = run!(own_push::push);
    assert_eq!(pull, push);
    assert_eq!(pull, std::collections::BTreeSet::from([(1, 100), (2, 200)]));
}

#[test]
fn prefix_let_domains_execute_once_and_preserve_all_planners() {
    let R = [(0_i32,), (1,), (2,), (3,)];
    macro_rules! run {
        ($($plan:tt)*) => {{
            let rows: std::collections::BTreeSet<_> = mini_linq::compile! {
                pipeline {plan = cq_planners::order($($plan)*);}
                struct Q; relation R(x:i32);
                answer(x) :- R(x), let k:i32 = std::convert::identity(*x), {if *k == 1} | {if *k == 2};
            }.map(|(x,)|*x).collect();
            assert_eq!(rows,std::collections::BTreeSet::from([1,2]));
        }};
    }
    run!(written, coarse);
    run!(written, fine);
    run!(hypergraph);
    run!(prefix connected,coarse);
    run!(prefix connected,fine);
    run!(prefix greedy,coarse);
    run!(prefix greedy,fine);
    run!(elimination, coarse);
    run!(elimination, fine);
}

#[test]
fn fallback_test_block_uses_each_empty_prefix_row_with_native_hosts() {
    let r0 = std::collections::BTreeMap::from([((1_i32,), vec![()]), ((3,), vec![()])]);
    let r1 = std::collections::BTreeMap::from([((3_i32,), vec![(-3_i32, 2_i32)])]);
    let r2 = std::collections::BTreeMap::from([((1_i32,), vec![(-3_i32, 2_i32)])]);
    let answers: std::collections::BTreeSet<_> = mini_linq::compile! {
        pipeline {
            plan = cq_planners::order(elimination, coarse);
            physical = [cq_rewrites::canon_prefixes, cq_rewrites::lazy_semijoins, cq_rewrites::merge_joins];
            structures = [cq_workload_passes::sorted_payload];
        }
        struct Q;
        relation r0(c0:i32); relation r1(c0:i32,c1:i32,c2:i32); relation r2(c0:i32,c1:i32,c2:i32);
        answer(x) :- r0(x), {r1(x,a,b)} else {r2(x,c,d), if 10 / x.to_owned() >= 0};
    }.map(|(x,)|*x).collect();
    assert_eq!(answers, std::collections::BTreeSet::from([1, 3]));
}
