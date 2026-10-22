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
