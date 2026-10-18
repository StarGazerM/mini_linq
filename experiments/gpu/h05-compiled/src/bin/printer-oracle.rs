#![recursion_limit = "1024"]

fn main() {
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
    .collect();
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
    .collect();
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
