//! Run with: cargo run --manifest-path features/rewrites/Cargo.toml --example radix_runs
use std::collections::BTreeSet;

macro_rules! execute {
    ($left:expr, $right:expr; $($module:ident :: $advice:ident $(($args:tt))?),* $(,)?) => {
        mini_linq::passes::select! {
            pipeline {
                $($module :: $advice $(($args))?,)*
                mini_linq::passes::sequence,
                mini_linq::passes::sequential,
                mini_linq::passes::translate_pull
            };
            physical {
                relation L(a:i64) = ($left);
                relation R(a:i64,b:i64) = ($right);
                l = pipe rename L@0 {a -> x};
                r = let rename R@1 {a -> x,b -> y};
                s = pipe semi eager[x] l,r;
                p = pipe probe first[x] s,r;
                out = pipe canon project p keep {x,y};
                return out as answer(x,y);
            }
        }.map(|(key, value)| (*key, *value)).collect::<BTreeSet<_>>()
    };
}

fn main() {
    let left: Vec<_> = (-12_i64..80)
        .map(|key| (key * key * key,))
        .chain([(i64::MIN,), (i64::MAX,)])
        .collect();
    let right: Vec<_> = (-9_i64..65)
        .flat_map(|key| {
            [
                (key * key * key, key),
                (key * key * key, key + 1),
                (key * key * key, key),
            ]
        })
        .chain([(i64::MIN, -1), (i64::MAX, 1)])
        .collect();
    let expected: BTreeSet<_> = left
        .iter()
        .flat_map(|&(key,)| {
            right
                .iter()
                .filter_map(move |&(other, value)| (key == other).then_some((key, value)))
        })
        .collect();
    let results = [
        ("default hash", execute!(&left, &right;)),
        (
            "RadixRun immediate",
            execute!(&left, &right; cq_rewrites::radix_runs),
        ),
        (
            "RadixRun retained",
            execute!(&left, &right; cq_rewrites::radix_runs, cq_rewrites::retain_lookups),
        ),
        (
            "RadixRun staged32",
            execute!(&left, &right; cq_rewrites::radix_runs, cq_rewrites::expand_lookups(32)),
        ),
        (
            "RadixRun retained+staged32",
            execute!(&left, &right; cq_rewrites::radix_runs, cq_rewrites::retain_lookups, cq_rewrites::expand_lookups(32)),
        ),
    ];
    for (mode, actual) in results {
        assert_eq!(actual, expected, "{mode}");
        println!("{mode}: {} exact answers", actual.len());
    }
}
