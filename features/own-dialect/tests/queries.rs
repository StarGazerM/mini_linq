use std::collections::BTreeSet;

#[test]
#[allow(non_snake_case)]
fn filtered_storage_reconstructs_constants_in_both_printers() {
    let L = [(1,), (2,), (3,)];
    let R = [(7, 1, 10), (8, 2, 20), (7, 3, 30), (7, 1, 10)];
    macro_rules! recorded {
        ($($printer:ident)::+) => {
            $($printer)::+! {
                pipeline {};
                physical {
                    relation L(k:i32); relation R(a:i32,k:i32,value:i32);
                    l = pipe rename L {k -> k}; r = pipe rename R {a -> a,k -> k,value -> value};
                    filtered = let point select r if (a = 7);
                    joined = pipe probe first [k] l,filtered;
                    out = pipe canon project joined keep {a,k}; return out as answer(a,k);
                }
                structures {
                    l host scan = ::core::iter::Map<_,_> [];
                    r host scan = ::core::iter::Map<_,_> [];
                    r host filtered = ::core::iter::Map<_,_> [a];
                    filtered = ::cq_storage::HashIndex<'_,_,_> [k] payload [k] constants {a = 7};
                    out canon = ::std::collections::HashSet<_>;
                }
            }
        };
    }
    let push: BTreeSet<_> = recorded!(own_push::push).map(|(a, k)| (*a, *k)).collect();
    let pull: BTreeSet<_> = recorded!(mini_linq::passes::print_pull)
        .map(|(a, k)| (*a, *k))
        .collect();
    assert_eq!(push, [(7, 1), (7, 3)].into_iter().collect());
    assert_eq!(push, pull);
}

macro_rules! partial {
    ($printer:path, $r:expr, $s:expr) => {{
        #[allow(non_snake_case)]
        let R = $r;
        #[allow(non_snake_case)]
        let S = $s;
        mini_linq::compile! {
            pipeline { print = $printer; }
            struct Partial;
            relation R(x: i32);
            relation S(w: i32);
            answer(x, z) :- R(x), S(w), if *x != 0 && *w > 1,
                let y: i32 = 10 / *x, if *y > 1, let z: i32 = *y + 1;
        }
        .map(|(x, z)| vec![*x, z])
        .collect::<cq_testkit::Rows>()
    }};
}

#[test]
fn push_chain_preserves_guard_and_binds_computed_columns_as_references() {
    fn reference(r: &[(i32,)], s: &[(i32,)]) -> cq_testkit::Rows {
        let mut answers = cq_testkit::Rows::new();
        for (x,) in r {
            for (w,) in s {
                if *x != 0 && *w > 1 {
                    let y = 10 / *x;
                    if y > 1 {
                        answers.insert(vec![*x, y + 1]);
                    }
                }
            }
        }
        answers
    }
    for r in [vec![], vec![(0,), (2,)], vec![(2,), (2,), (20,)]] {
        for s in [vec![], vec![(1,)], vec![(3,), (3,)]] {
            cq_testkit::harness::check_refinement(
                || reference(&r, &s),
                || partial!(own_push::push, r.as_slice(), s.as_slice()),
            )
            .unwrap();
            assert_eq!(
                partial!(own_push::push, r.as_slice(), s.as_slice()),
                partial!(mini_linq::passes::print_pull, r.as_slice(), s.as_slice()),
            );
        }
    }
}

// No Clone, Copy or Hash implementation: host columns remain references and
// the written ordered representations impose only their own trait bounds.
#[derive(Debug, PartialEq, Eq, PartialOrd, Ord)]
struct Value(i32);

type Retained<T> = std::collections::VecDeque<T>;

#[test]
#[allow(non_snake_case)]
fn authored_structures_control_build_retention_and_nested_canonical_states() {
    let R = [
        (Value(1), Value(10)),
        (Value(1), Value(10)),
        (Value(1), Value(11)),
        (Value(2), Value(20)),
    ];
    macro_rules! recorded {
        ($($printer:ident)::+) => {
            $($printer)::+! {
                pipeline {};
                physical {
                    relation R(a: Value,b: Value);
                    r = let canon rename R@0 { a -> a, b -> b };
                    p = pipe project r keep { a };
                    j = pipe probe first[a] p,r;
                    o = pipe canon project j keep { a,b };
                    return o as answer(b,a);
                }
                structures {
                    r host scan = ::core::iter::Map<_,_> [];
                    r = ::std::collections::BTreeMap<_,::std::vec::Vec<_>> [a];
                    r rows = Retained<_>;
                    r canon = ::std::collections::BTreeSet<_>;
                    o canon = ::std::collections::BTreeSet<_>;
                }
            }
        };
    }
    let push: Vec<(&Value, &Value)> = recorded!(own_push::push).collect();
    let pull: Vec<(&Value, &Value)> = recorded!(mini_linq::passes::print_pull).collect();
    let values = |rows: &[(&Value, &Value)]| {
        rows.iter()
            .map(|(b, a)| (b.0, a.0))
            .collect::<BTreeSet<_>>()
    };
    assert_eq!(values(&push), BTreeSet::from([(10, 1), (11, 1), (20, 2)]));
    assert_eq!(values(&push), values(&pull));
    assert_eq!(push.len(), 3);
    for (b, a) in push {
        assert!(R.iter().any(|(ra, _)| std::ptr::eq(ra, a)));
        assert!(R.iter().any(|(_, rb)| std::ptr::eq(rb, b)));
    }
}

struct HeadState<T>(BTreeSet<T>);
impl<T: Ord> HeadState<T> {
    fn new() -> Self {
        Self(BTreeSet::new())
    }
    fn insert(&mut self, key: T) -> bool {
        self.0.insert(key)
    }
}

#[test]
#[allow(non_snake_case)]
fn computed_head_deduplicates_its_image_with_the_recorded_type() {
    let R = [(1,), (1,), (3,), (4,)];
    macro_rules! image {
        ($($printer:ident)::+) => {
            $($printer)::+! {
                pipeline {};
                physical {
                    relation R(x: i32);
                    r = pipe rename R@0 { x -> x };
                    o = pipe canon project r keep { x };
                    return o as answer(*x % 2);
                }
                structures {
                    r host scan = ::core::iter::Map<_,_> [];
                    o canon = ::std::collections::BTreeSet<_>;
                    o head = HeadState<_>;
                }
            }
        };
    }
    let push: Vec<_> = image!(own_push::push).collect();
    let pull: Vec<_> = image!(mini_linq::passes::print_pull).collect();
    assert_eq!(push.len(), 2);
    assert_eq!(
        push.into_iter().collect::<BTreeSet<_>>(),
        BTreeSet::from([(0,), (1,)])
    );
    assert_eq!(
        pull.into_iter().collect::<BTreeSet<_>>(),
        BTreeSet::from([(0,), (1,)])
    );
}

#[test]
fn exists_consumes_the_full_source_and_retains_only_nonemptiness() {
    fn checked(x: &i32) -> bool {
        assert_ne!(*x, 0);
        true
    }
    macro_rules! exists {
        ($($printer:ident)::+, $rows:expr) => {{
            #[allow(non_snake_case)]
            let R = $rows;
            $($printer)::+! {
                pipeline {};
                physical {
                    relation R(x: i32);
                    r = pipe rename R@0 { x -> x };
                    g = pipe guard r if (checked(x));
                    e = exists canon project g keep {};
                    return e as answer();
                }
                structures {
                    r host scan = ::core::iter::Map<_,_> [];
                    e canon = ::std::collections::BTreeSet<_>;
                    e rows = ::cq_storage::Exists;
                }
            }
            .collect::<BTreeSet<_>>()
        }};
    }
    for rows in [vec![], vec![(1,), (2,)], vec![(1,), (0,)]] {
        let comparison = cq_testkit::harness::check_refinement(
            || {
                let mut nonempty = false;
                for (x,) in &rows {
                    if checked(x) {
                        nonempty = true;
                    }
                }
                if nonempty {
                    BTreeSet::from([()])
                } else {
                    BTreeSet::new()
                }
            },
            || exists!(own_push::push, rows.as_slice()),
        )
        .unwrap();
        if rows.iter().any(|&(x,)| x == 0) {
            assert!(matches!(
                comparison.target,
                cq_testkit::harness::Outcome::Panicked(_)
            ));
        }
        cq_testkit::harness::check_refinement(
            || exists!(mini_linq::passes::print_pull, rows.as_slice()),
            || exists!(own_push::push, rows.as_slice()),
        )
        .unwrap();
    }
}

#[test]
#[allow(non_snake_case)]
fn native_and_fallback_point_range_reads_agree_with_pull() {
    macro_rules! windows {
        ($printer:path, $rows:expr, $low:expr, $high:expr) => {{
            let R = $rows;
            let low = $low;
            let high = $high;
            mini_linq::compile! {
                pipeline { print = $printer; }
                struct Window;
                relation R(x: i32,y: i32);
                answer(x,y) :- R(x,y), if *x >= low && *x < high, if *y == 7;
            }
            .map(|(x, y)| (*x, *y))
            .collect::<BTreeSet<_>>()
        }};
    }
    let rows = [(1, 7), (2, 7), (2, 7), (3, 8), (4, 7)];
    let ordered = std::collections::BTreeMap::from([
        ((1,), vec![(7,)]),
        ((2,), vec![(7,), (7,)]),
        ((3,), vec![(8,)]),
        ((4,), vec![(7,)]),
    ]);
    for (low, high) in [(0, 5), (2, 4), (4, 2), (2, 2)] {
        let expected = rows
            .iter()
            .copied()
            .filter(|(x, y)| *x >= low && *x < high && *y == 7)
            .collect::<BTreeSet<_>>();
        assert_eq!(
            windows!(own_push::push, rows.as_slice(), low, high),
            expected
        );
        assert_eq!(windows!(own_push::push, &ordered, low, high), expected);
        assert_eq!(
            windows!(mini_linq::passes::print_pull, &ordered, low, high),
            expected
        );
    }
    macro_rules! points {
        ($printer:path, $rows:expr) => {{
            let R = $rows;
            mini_linq::compile! {
                pipeline { print = $printer; }
                struct Point;
                relation R(x: i32,y: i32);
                answer(x,y) :- R(x,y), if *x == 2;
            }
            .map(|(x, y)| (*x, *y))
            .collect::<BTreeSet<_>>()
        }};
    }
    assert_eq!(
        points!(own_push::push, rows.as_slice()),
        BTreeSet::from([(2, 7)])
    );
    assert_eq!(
        points!(own_push::push, &ordered),
        points!(mini_linq::passes::print_pull, &ordered)
    );
}

#[test]
#[allow(non_snake_case)]
fn full_depth_curried_probes_and_prefix_membership_use_written_levels() {
    let R = [(1, 10, 100), (1, 11, 101), (1, 10, 100), (2, 20, 200)];
    let L = [(1, 10), (1, 11), (1, 12), (2, 20), (3, 30)];
    macro_rules! curried {
        ($($printer:ident)::+) => {
            $($printer)::+! {
                pipeline {};
                physical {
                    relation R(a: i32,b: i32,c: i32);
                    relation L(a: i32,b: i32);
                    r = let rename R@0 { a -> a,b -> b,c -> c };
                    l = pipe rename L@1 { a -> a,b -> b };
                    p = pipe project l keep { a };
                    m = pipe semi eager[a] p,r;
                    n = pipe probe first[a] m,l;
                    j = pipe probe first[a,b] n,r;
                    o = pipe canon project j keep { a,b,c };
                    return o as answer(a,b,c);
                }
                structures {
                    r host scan = ::core::iter::Map<_,_> [];
                    l host scan = ::core::iter::Map<_,_> [];
                    l host n = ::cq_storage::HashIndex<'_,_,_> [a];
                    r = ::cq_storage::Trie<'_,_,::cq_storage::HashIndex<'_,_,_>> [a][b];
                    r rows = ::std::vec::Vec<_>;
                    o canon = ::std::collections::BTreeSet<_>;
                }
            }
        };
    }
    let push = curried!(own_push::push)
        .map(|(a, b, c)| (*a, *b, *c))
        .collect::<BTreeSet<_>>();
    let pull = curried!(mini_linq::passes::print_pull)
        .map(|(a, b, c)| (*a, *b, *c))
        .collect::<BTreeSet<_>>();
    assert_eq!(
        push,
        BTreeSet::from([(1, 10, 100), (1, 11, 101), (2, 20, 200)])
    );
    assert_eq!(push, pull);
}

#[test]
#[allow(non_snake_case)]
fn written_lazy_host_membership_and_fallback_type_are_preserved() {
    let R = [(1,), (2,), (2,), (3,)];
    let S = [(2,), (4,), (2,)];
    macro_rules! lazy {
        ($($printer:ident)::+) => {
            $($printer)::+! {
                pipeline {};
                physical {
                    relation R(x: i32);
                    relation S(x: i32);
                    r = pipe rename R@0 { x -> x };
                    s = pipe rename S@1 { x -> x };
                    m = pipe semi lazy[x] r,s;
                    o = pipe canon project m keep { x };
                    return o as answer(x);
                }
                structures {
                    r host scan = ::core::iter::Map<_,_> [];
                    s host scan = ::core::iter::Map<_,_> [];
                    s host m = ::cq_storage::dispatch::KeySet<_,false> [x];
                    o canon = ::std::collections::BTreeSet<_>;
                }
            }
        };
    }
    let push: Vec<_> = lazy!(own_push::push).collect();
    let pull: Vec<_> = lazy!(mini_linq::passes::print_pull).collect();
    assert_eq!(push, [(&2,)]);
    assert_eq!(push, pull);
}

#[test]
#[allow(non_snake_case)]
fn nested_stream_canonical_boundaries_have_separate_state() {
    let R = [(1, 10), (1, 11), (1, 10), (2, 20)];
    let rows: Vec<_> = own_push::push! {
        pipeline {};
        physical {
            relation R(a: i32,b: i32);
            r = pipe canon rename R@0 { a -> a,b -> b };
            o = pipe canon project r keep { a };
            return o as answer(a);
        }
        structures {
            r host scan = ::core::iter::Map<_,_> [];
            r canon = ::std::collections::BTreeSet<_>;
            o canon = ::std::collections::BTreeSet<_>;
        }
    }
    .collect();
    assert_eq!(rows.len(), 2);
    assert_eq!(
        rows.into_iter().map(|(a,)| *a).collect::<BTreeSet<_>>(),
        BTreeSet::from([1, 2])
    );
}

#[test]
#[allow(non_snake_case)]
fn wider_join_answers_use_grouped_keys_and_keep_flat_rows() {
    let R = [(1, 2, 3, 4, 5, 6, 7), (1, 2, 3, 4, 5, 6, 7)];
    let S = [(8, 9, 10, 11, 12, 13), (8, 9, 10, 11, 12, 14)];
    macro_rules! wide {
        ($printer:path) => {
            mini_linq::compile! {
                pipeline { print = $printer; }
                struct Wide;
                relation R(a: i32,b: i32,c: i32,d: i32,e: i32,f: i32,g: i32);
                relation S(h: i32,i: i32,j: i32,k: i32,l: i32,m: i32);
                answer(a,b,c,d,e,f,g,h,i,j,k,l,m) :- R(a,b,c,d,e,f,g), S(h,i,j,k,l,m);
            }
            .map(|(a, b, c, d, e, f, g, h, i, j, k, l, m)| {
                vec![*a, *b, *c, *d, *e, *f, *g, *h, *i, *j, *k, *l, *m]
            })
            .collect::<Vec<_>>()
        };
    }
    let push = wide!(own_push::push);
    let pull = wide!(mini_linq::passes::print_pull);
    let expected = BTreeSet::from([
        vec![1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13],
        vec![1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 14],
    ]);
    assert_eq!(push.len(), 2);
    assert_eq!(push.into_iter().collect::<BTreeSet<_>>(), expected);
    assert_eq!(pull.into_iter().collect::<BTreeSet<_>>(), expected);
}

#[test]
fn unit_has_one_empty_answer() {
    let push: Vec<()> = own_push::push! {
        pipeline {};
        physical { u = pipe unit; return u as answer(); }
        structures {}
    }
    .collect();
    let pull: Vec<()> = mini_linq::passes::print_pull! {
        pipeline {};
        physical { u = pipe unit; return u as answer(); }
        structures {}
    }
    .collect();
    assert_eq!(push, [()]);
    assert_eq!(push, pull);
}
