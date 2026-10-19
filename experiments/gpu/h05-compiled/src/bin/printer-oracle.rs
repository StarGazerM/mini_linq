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

fn closed_scalar_captures() {
    struct Metadata {
        x: i64,
        enabled: bool,
    }
    let meta = Metadata { x: -3_000_000_001, enabled: true };
    let evaluations = std::cell::Cell::new(0u32);
    let captured = || {
        evaluations.set(evaluations.get() + 1);
        4_000_000_001u64
    };
    let rows = [(1i32,), (2,), (1,)];
    macro_rules! check {
        ($($arguments:tt)*) => {{
            evaluations.set(0);
            let mut actual = cq_gpu::print! {
                arguments { $($arguments)* };
                pipeline {};
                physical {
                    relation R(a:i32) = (&rows);
                    scan = pipe rename R@0 {a -> x};
                    value = pipe bind scan with y:i64 = (meta.x);
                    flag = pipe bind value with enabled:bool = (meta.enabled);
                    counted = pipe bind flag with z:u64 = (captured());
                    output = pipe canon project counted keep {x,y,enabled,z};
                    return output as answer(x,y,enabled,z);
                }
                structures {
                    scan host scan = ::core::iter::Map<_,_> [];
                    output canon = ::cq_gpu_runtime::DeviceSet<_>;
                }
            }.to_host().expect("closed typed scalar captures");
            actual.sort_unstable();
            assert_eq!(actual, [(1, meta.x, true, 4_000_000_001), (2, meta.x, true, 4_000_000_001)]);
            assert_eq!(evaluations.get(), 1, "capture must be evaluated once, not per row or launch");
        }};
    }
    check!();
    check!(capacity(R));
    println!("closed-scalar-captures exact=true once=true bounded=true resumable=true");
}

fn composite_hash_collisions() {
    // All keys land in bucket zero even with the default 256-bucket directory.
    fn step(hash: u64, value: i64) -> u64 {
        let mut value = value as u64;
        value = (value ^ (value >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
        value = (value ^ (value >> 27)).wrapping_mul(0x94d049bb133111eb);
        (hash ^ value ^ (value >> 31)).wrapping_mul(0x100000001b3)
    }
    let collisions: Vec<i64> = (0..100_000).filter(|&y| step(step(0xcbf29ce484222325, -7), y) & 255 == 0).take(58).collect();
    assert_eq!(collisions.len(), 58);
    let mut rows: Vec<_> = (0..200).map(|z| (-7i64, collisions[0], z)).collect();
    rows.extend(collisions[1..57].iter().enumerate().map(|(z, &y)| (-7, y, z as i64)));
    let keys = [(-7, collisions[0]), (-7, collisions[22]), (-7, collisions[57]), (9, collisions[0])];
    let mut expected: Vec<_> = rows.iter().copied().filter(|&(x,y,_)| keys.contains(&(x,y))).collect();
    expected.sort_unstable();
    macro_rules! check {
        ($policy:ident; $keys:ident,$rows:ident,$expected:ident; $($arguments:tt)*) => {{
            let mut actual = cq_gpu::print! {
                arguments { $($arguments)* };
                pipeline {};
                physical {
                    relation L(a:i64,b:i64) = (&$keys);
                    relation R(a:i64,b:i64,c:i64) = (&$rows);
                    l = pipe rename L@0 {a -> x,b -> y};
                    r = let rename R@1 {a -> x,b -> y,c -> z};
                    s = pipe semi eager [x,y] l,r;
                    p = pipe canon probe first [x,y] s,r;
                    return p as answer(x,y,z);
                }
                structures {
                    l host scan = ::core::iter::Map<_,_> [];
                    r host scan = ::core::iter::Map<_,_> [];
                    r rows = ::std::vec::Vec<_>;
                    r = ::cq_gpu_runtime::HashedRun<_,_> [x,y];
                    p canon = ::cq_gpu_runtime::DeviceSet<_,::cq_gpu_runtime::Plain,::cq_gpu_runtime::$policy>;
                    l stage = batch 32;
                }
            }.to_host().expect("composite collision query");
            actual.sort_unstable();
            assert_eq!(actual, $expected);
        }};
    }
    check!(Staged; keys,rows,expected;);
    check!(Staged; keys,rows,expected; capacity(R));
    cq_gpu_runtime::reset_host_wait_count();
    check!(Fused; keys,rows,expected;);
    assert!(cq_gpu_runtime::resume_readback_count() > 1, "distinct fused output must grow and rehash");
    check!(Fused; keys,rows,expected; capacity(L));
    let keys = vec![(-7, collisions[0]); 128];
    let rows: Vec<_> = rows.into_iter().map(|(x,y,z)| (x,y,z % 7)).collect();
    let mut expected: Vec<_> = rows.iter().copied().filter(|&(x,y,_)| keys.contains(&(x,y))).collect();
    expected.sort_unstable();
    expected.dedup();
    cq_gpu_runtime::reset_host_wait_count();
    check!(Compacting; keys,rows,expected;);
    assert!(cq_gpu_runtime::resume_readback_count() > 3, "heavy multiplicity must suspend repeatedly");
    check!(Staged; keys,rows,expected;);
    cq_gpu_runtime::reset_host_wait_count();
    check!(Fused; keys,rows,expected;);
    assert_eq!(cq_gpu_runtime::compaction_readback_count(), 0);
    assert_eq!(cq_gpu_runtime::resume_readback_count(), 1, "duplicate candidates must not consume output capacity");
    println!("fused-dedup exact=true growth-rehash=true duplicates=true frontier=true");
    println!("compacting-dedup exact=true repeated-suspensions=true hash-collisions=true");
    println!("composite-hash exact=true forced-collisions=true skew=true bounded=true resumable=true");
}

fn fused_head_projection() {
    let rows = [(1i64, 2i64), (1, 3), (2, 4), (1, 2)];
    let mut actual = cq_gpu::print! {
        arguments {};
        pipeline {};
        physical {
            relation R(a:i64,b:i64) = (&rows);
            scan = pipe canon rename R@0 {a -> x,b -> y};
            return scan as answer(*x,*x);
        }
        structures {
            scan host scan = ::core::iter::Map<_,_> [];
            scan canon = ::cq_gpu_runtime::DeviceSet<_>;
            scan head = ::cq_gpu_runtime::DeviceSet<_,::cq_gpu_runtime::Plain,::cq_gpu_runtime::Fused>;
        }
    }.to_host().expect("fused repeated lossy head");
    actual.sort_unstable();
    assert_eq!(actual, [(1,1),(2,2)]);
}

fn main() {
    composite_hash_collisions();
    chained_answers();
    typed_filters();
    closed_scalar_captures();
    fused_head_projection();
    let rows = vec![
        (i64::MIN, i64::MAX, "alpha".to_owned()),
        (i64::MAX, i64::MIN, "beta".to_owned()),
        (0, -1, "alpha".to_owned()),
        (i64::MIN, i64::MAX, "alpha".to_owned()),
    ];
    let mut actual: Vec<(i64, i64, String)> = mini_linq::compile! {
        pipeline {
            structures = [::cq_rewrites::retype(dedup => ::cq_gpu_runtime::DeviceSet<_,::cq_gpu_runtime::Plain,::cq_gpu_runtime::Fused>)];
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
            p canon = ::cq_gpu_runtime::DeviceSet<_,::cq_gpu_runtime::Plain,::cq_gpu_runtime::Fused>;
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
