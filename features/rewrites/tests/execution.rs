mod representations;

mod from_ix {
    cq_rewrite_test_driver::corpus_tests! {
        ix; pipeline {
            mini_linq::passes::lower,
            cq_rewrites::canon_prefixes,
            cq_rewrites::smallest_intersections,
            cq_rewrites::lazy_semijoins,
            mini_linq::passes::select,
            mini_linq::passes::sequence, mini_linq::passes::sequential, mini_linq::passes::translate_pull
        }
    }
}

mod from_physical {
    cq_rewrite_test_driver::corpus_tests! {
        physical; pipeline {
            cq_rewrites::canon_prefixes,
            cq_rewrites::smallest_intersections,
            cq_rewrites::lazy_semijoins,
            mini_linq::passes::select,
            mini_linq::passes::sequence, mini_linq::passes::sequential, mini_linq::passes::translate_pull
        }
    }
}

type IntersectionRun = fn(&[(i32,)], &[(i32, i32)], &[(i32, i32)], &[(i32, i32)]) -> cq_testkit::Rows;

macro_rules! intersection_runner {
    ($name:ident; $($rewrites:tt)*) => {
        #[allow(non_snake_case)]
        fn $name(L: &[(i32,)], R: &[(i32, i32)], S: &[(i32, i32)], T: &[(i32, i32)]) -> cq_testkit::Rows {
            cq_rewrite_test_driver::three_way! {
                pipeline {
                    $($rewrites)*
                    mini_linq::passes::select,
                    mini_linq::passes::sequence,
                    mini_linq::passes::sequential,
                    mini_linq::passes::translate_pull
                }
            }.map(|(a, v)| vec![*a, *v]).collect()
        }
    };
}

intersection_runner!(three_way_default;);
intersection_runner!(three_way_all; cq_rewrites::smallest_intersections,);
intersection_runner!(three_way_site; cq_rewrites::smallest_intersections(smallest(R@1,S@2,T@3)),);

#[test]
fn three_way_smallest_executes_each_provider_as_minimum() {
    let wide: Vec<_> = (0..128).map(|v| (1, v)).collect();
    let l = [(1,)];
    for minimum in 0..3 {
        let mut inputs = [wide.clone(), wide.clone(), wide.clone()];
        inputs[minimum] = vec![(1, 7)];
        let expected = cq_testkit::Rows::from([vec![1, 7]]);
        for run in [three_way_all as IntersectionRun, three_way_site] {
            cq_storage::counters::reset();
            assert_eq!(run(&l, &inputs[0], &inputs[1], &inputs[2]), expected);
            // Counts plus one enumeration and membership checks, not a
            // binary intersection that still enumerates 128 values when T wins.
            assert!(cq_storage::counters::snapshot().lookups < 20);
        }
        assert_eq!(three_way_default(&l, &inputs[0], &inputs[1], &inputs[2]), expected);
        for input in &mut inputs {
            input.extend(input.clone());
        }
        for run in [three_way_default as IntersectionRun, three_way_all, three_way_site] {
            assert_eq!(run(&l, &inputs[0], &inputs[1], &inputs[2]), expected);
            assert!(run(&[], &inputs[0], &inputs[1], &inputs[2]).is_empty());
            for empty in 0..3 {
                let mut empty_inputs = inputs.clone();
                empty_inputs[empty].clear();
                assert!(run(&l, &empty_inputs[0], &empty_inputs[1], &empty_inputs[2]).is_empty());
            }
            assert!(run(&l, &[(1, 1)], &[(1, 2)], &[(1, 3)]).is_empty());
        }
    }
}
