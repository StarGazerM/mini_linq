use super::*;
pub fn invalid_sources() -> InvalidSources {
    InvalidSources {
        cq: quote! {
            struct BadCq;
            relation R(c0: i32);
            answer(x) :- R(y);
        },
        index_requirements: quote! {
            struct BadIndexes;
            relation R(c0: i32);
            answer(x) :- R(x);
            relational {
                r0 = rename R {c0 -> x};
                r1 = project r0 keep {x};
                output r1 as answer(x);
            }
            indexes { R[0]; R[0]; }
        },
        rust_access_plan: quote! {
            answer(x) => ([x]) :-
                R(x) => if (rows.contains(&[x]));
        },
    }
}

pub fn missing_required_index() -> TokenStream {
    quote! {
        pub struct MissingRequiredIndex;
        relation Seed(c0: i32);
        relation Edge(c0: i32, c1: i32);
        answer(x, z) :- Seed(x), Edge(x, z);
        relational {
            r0 = rename Seed {c0 -> x};
            r1 = rename Edge {c0 -> x, c1 -> z};
            r2 = natural_join r0 with r1;
            r3 = project r2 keep {x, z};
            output r3 as answer(x, z);
        }
        indexes {}
    }
}
