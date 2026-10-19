#![recursion_limit = "1024"]

fn chained_answers() {
    macro_rules! check {
        ($($arguments:tt)*) => {{
            let keys = [(-7i64,), (11,), (97,)];
            let rows = [(-7i64, 3_000_000_000i64), (-7, 3_000_000_003), (11, 3_000_000_003)];
            let answer = cq_gpu::print! {
                arguments { $($arguments)* };
                pipeline {};
                physical {
                    relation L(a:i64) = (&keys);
                    relation R(a:i64,b:i64) = (&rows);
                    l = pipe rename L@0 {a -> x};
                    b = let rename R@1 {a -> x,b -> y};
                    p = pipe canon probe first [x] l,b;
                    return p as answer(x,y);
                }
                structures {
                    l host scan = ::core::iter::Map<_,_> [];
                    b host scan = ::core::iter::Map<_,_> [];
                    b rows = ::std::vec::Vec<_>;
                    b = ::cq_storage::SortedRun<_,_,::cq_storage::BorrowedI64TupleCodec> [x];
                    p canon = ::cq_gpu_runtime::DeviceSet<_,::cq_gpu_runtime::For>;
                    p stage = batch 32 expand;
                }
            };
            let probes = [(3_000_000_003i64,), (3_000_000_001,)];
            let mut actual = cq_gpu::print! {
                arguments {};
                pipeline {};
                physical {
                    relation L(a:i64) = (&probes);
                    relation R(a:i64,b:i64) = (&answer);
                    l = pipe rename L@0 {a -> y};
                    b = let rename R@1 {a -> x,b -> y};
                    p = pipe canon probe first [y] l,b;
                    return p as answer(x,y);
                }
                structures {
                    l host scan = ::core::iter::Map<_,_> [];
                    b host scan = ::core::iter::Map<_,_> [];
                    b rows = ::std::vec::Vec<_>;
                    b = ::cq_storage::SortedRun<_,_,::cq_storage::BorrowedI64TupleCodec> [y];
                    p canon = ::cq_gpu_runtime::DeviceSet<_>;
                }
            }.to_host().expect("resident answer used as build");
            actual.sort_unstable();
            assert_eq!(actual, [(-7, 3_000_000_003), (11, 3_000_000_003)]);
        }};
    }
    check!();
    check!(capacity(R));
    println!("chained-answer-as-build exact=true bounded=true resumable=true");
}

fn typed_filters() {
    let rows = [(-3_000_000_001i64, 4_000_000_000u32), (-3_000_000_000, 4_000_000_001), (3_000_000_000, 4_000_000_002)];
    let mut actual = mini_linq::compile! {
        pipeline {
            structures = [::cq_rewrites::retype(dedup => ::cq_gpu_runtime::DeviceSet<_>)];
            print = ::cq_gpu::print;
        }
        struct TypedRange;
        relation Input(x:i64,y:u32) = (&rows);
        answer(x,y) :- Input(x,y), if x >= -3000000000, if y >= 4000000001;
    }.to_host().expect("typed range literals");
    actual.sort_unstable();
    assert_eq!(actual, rows[1..]);
    let actual = mini_linq::compile! {
        pipeline {
            structures = [::cq_rewrites::retype(dedup => ::cq_gpu_runtime::DeviceSet<_>)];
            print = ::cq_gpu::print;
        }
        struct TypedTotal;
        relation Input(x:i64,y:u32) = (&rows);
        answer(x,y) :- Input(x,y), #[total] if *x == 3000000000 && *y == 4000000002;
    }.to_host().expect("typed total literals");
    assert_eq!(actual, rows[2..]);
    println!("typed-large-literals exact=true relational=true total=true");
}

fn main() {
    chained_answers();
    typed_filters();
    let rows = vec![
        (i64::MIN, i64::MAX, "alpha".to_owned()),
        (i64::MAX, i64::MIN, "beta".to_owned()),
        (0, -1, "alpha".to_owned()),
        (i64::MIN, i64::MAX, "alpha".to_owned()),
    ];
    let mut actual: Vec<(i64, i64, String)> = mini_linq::compile! {
        pipeline {
            structures = [::cq_rewrites::retype(dedup => ::cq_gpu_runtime::DeviceSet<_>)];
            print = ::cq_gpu::print;
        }
        struct WideKeys;
        relation Input(a:i64,b:i64,name:String) = (&rows);
        answer(a,b,name) :- Input(a,b,name);
    }
    .to_host()
    .expect("wide-key answer download");
    let mut expected = rows.clone();
    expected.sort_unstable();
    expected.dedup();
    actual.sort_unstable();
    assert_eq!(
        actual, expected,
        "multiword packed-key dedup and string decoding"
    );

    let left = [(1i64,), (2,), (1,)];
    let right: Vec<_> = (0..20000i64).map(|value| (1i64, value)).collect();
    let mut expanded: Vec<(i64, i64)> = cq_gpu::print! {
        arguments { tile(256,4), frontier(1), directory(rank) };
        pipeline {};
        physical {
            relation L(a:i64) = (&left);
            relation R(a:i64,b:i64) = (&right);
            l = pipe rename L@0 {a -> x};
            b = let rename R@1 {a -> x,b -> y};
            s = pipe semi eager [x] l,b;
            p = pipe canon probe first [x] s,b;
            return p as answer(x,y);
        }
        structures {
            l host scan = ::core::iter::Map<_,_> [];
            b host scan = ::core::iter::Map<_,_> [];
            b rows = ::std::vec::Vec<_>;
            b = ::cq_storage::SortedRun<_,_,::cq_storage::BorrowedI64TupleCodec> [x];
            l join_filter = ::cq_storage::AdaptiveKeySet<i64,::cq_storage::I64Codec,16777216>
                build b [x] covering s marker ::cq_load::ColumnAt<0>;
            p canon = ::cq_gpu_runtime::DeviceSet<_>;
            s share = lookup b [x] first s force p;
            s repr = tuples witness {b [x] at s as ::cq_storage::RunCursor<'_,_>};
            p stage = batch 32 expand;
        }
    }
    .to_host()
    .expect("expanded answer download");
    expanded.sort_unstable();
    assert_eq!(
        expanded, right,
        "join filter, witness expansion, frontier and output grow/relaunch"
    );
    println!(
        "printer oracle exact=true multiword_rows={} expanded_rows={}",
        actual.len(),
        expanded.len()
    );
}
