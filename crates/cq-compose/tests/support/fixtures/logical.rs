use super::*;
pub fn cq_to_relational_plan_and_indexes() -> Vec<LoweringFixture> {
    vec![
        LoweringFixture {
            source: quote! {
                pub struct TriangleProgram;
                relation R(c0: i32, c1: i32);
                relation S(c0: i32, c1: i32);
                relation T(c0: i32, c1: i32);
                triangle(x, y, z) :- R(x, y), S(y, z), T(z, x);
            },
            expected: triangle_index_requirements(),
        },
        LoweringFixture {
            source: quote! {
                struct NonLeadingKey;
                relation Seed(c0: i32);
                relation Edge(c0: i32, c1: i32);
                answer(x, z) :- Seed(x), Edge(z, x);
            },
            expected: quote! {
                struct NonLeadingKey;
                relation Seed(c0: i32);
                relation Edge(c0: i32, c1: i32);
                answer(x, z) :- Seed(x), Edge(z, x);
                relational {
                    r0 = rename Seed {c0 -> x};
                    r1 = rename Edge {c0 -> z, c1 -> x};
                    r2 = natural_join r0 with r1;
                    r3 = project r2 keep {x, z};
                    output r3 as answer(x, z);
                }
                indexes { Edge[1]; }
            },
        },
        LoweringFixture {
            source: quote! {
                struct RawNames;
                relation r#type(c0: i32);
                relation Edge(c0: i32, c1: i32);
                answer(x, z) :- r#type(r#x), Edge(z, x);
            },
            expected: quote! {
                struct RawNames;
                relation r#type(c0: i32);
                relation Edge(c0: i32, c1: i32);
                answer(x, z) :- r#type(r#x), Edge(z, x);
                relational {
                    r0 = rename r#type {c0 -> r#x};
                    r1 = rename Edge {c0 -> z, c1 -> x};
                    r2 = natural_join r0 with r1;
                    r3 = project r2 keep {x, z};
                    output r3 as answer(x, z);
                }
                indexes { Edge[1]; }
            },
        },
        LoweringFixture {
            source: quote! {
                struct SelfJoinProgram;
                relation R(c0: i32, c1: i32);
                path2(x, y, z) :- R(x, y), R(y, z);
            },
            expected: self_join_index_requirements(),
        },
        LoweringFixture {
            source: quote! {
                pub struct ReusedIndexProgram;
                relation Seeds(c0: i32, c1: i32);
                relation Edge(c0: i32, c1: i32);
                answer(a, b, x, y) :- Seeds(a, b), Edge(a, x), Edge(b, y);
            },
            expected: reused_index_requirements(),
        },
        LoweringFixture {
            source: quote! {
                struct MultiColumnProgram;
                relation Pair(c0: i32, c1: i32);
                relation Fact(c0: i32, c1: i32, c2: i32, c3: i32);
                answer(a, b, c, d) :- Pair(a, b), Fact(c, b, a, d);
            },
            expected: multi_column_index_requirements(),
        },
    ]
}

pub(super) fn triangle_index_requirements() -> TokenStream {
    quote! {
        pub struct TriangleProgram;
        relation R(c0: i32, c1: i32);
        relation S(c0: i32, c1: i32);
        relation T(c0: i32, c1: i32);
        triangle(x, y, z) :- R(x, y), S(y, z), T(z, x);
        relational {
            r0 = rename R {c0 -> x, c1 -> y};
            r1 = rename S {c0 -> y, c1 -> z};
            r2 = natural_join r0 with r1;
            r3 = rename T {c0 -> z, c1 -> x};
            r4 = natural_join r2 with r3;
            r5 = project r4 keep {x, y, z};
            output r5 as triangle(x, y, z);
        }
        indexes { S[0]; T[0, 1]; }
    }
}

pub(super) fn row_only_index_requirements() -> TokenStream {
    quote! {
        pub struct RowOnlyProgram;
        relation Rows(c0: i32, c1: i32, c2: i32);
        answer(z, x, y) :- Rows(x, y, z);
        relational {
            r0 = rename Rows {c0 -> x, c1 -> y, c2 -> z};
            r1 = project r0 keep {z, x, y};
            output r1 as answer(z, x, y);
        }
        indexes {}
    }
}

pub(super) fn unused_input_index_requirements() -> TokenStream {
    quote! {
        pub struct UnusedInputProgram;
        relation Rows(c0: i32);
        relation Unused(c0: i32, c1: i32);
        answer(x) :- Rows(x);
        relational {
            r0 = rename Rows {c0 -> x};
            r1 = project r0 keep {x};
            output r1 as answer(x);
        }
        indexes {}
    }
}

pub(super) fn extra_index_requirements() -> TokenStream {
    quote! {
        pub struct ExtraIndexProgram;
        relation Rows(c0: i32);
        relation Edge(c0: i32, c1: i32);
        answer(x) :- Rows(x);
        relational {
            r0 = rename Rows {c0 -> x};
            r1 = project r0 keep {x};
            output r1 as answer(x);
        }
        // Written full-key first on purpose: proper-key builders still have to
        // borrow relation1 before index0 takes ownership of that HashSet.
        indexes { Edge[0, 1]; Edge[0]; Edge[1]; }
    }
}

pub(super) fn scan_proper_full_index_requirements() -> TokenStream {
    quote! {
        pub struct ScanProperFullProgram;
        relation R(c0: i32, c1: i32);
        answer(x, y, z) :- R(x, y), R(y, z), R(x, y);
        relational {
            r0 = rename R {c0 -> x, c1 -> y};
            r1 = rename R {c0 -> y, c1 -> z};
            r2 = natural_join r0 with r1;
            r3 = rename R {c0 -> x, c1 -> y};
            r4 = natural_join r2 with r3;
            r5 = project r4 keep {x, y, z};
            output r5 as answer(x, y, z);
        }
        // Full-key is written first on purpose. The proper-key map must borrow
        // relation0 before index0 takes ownership, and index0 also supplies the scan.
        indexes { R[0, 1]; R[0]; }
    }
}

pub(super) fn multiple_proper_indexes_index_requirements() -> TokenStream {
    quote! {
        pub struct MultipleProperIndexesProgram;
        relation Seed(c0: i32);
        relation Edge(c0: i32, c1: i32);
        answer(x) :- Seed(x);
        relational {
            r0 = rename Seed {c0 -> x};
            r1 = project r0 keep {x};
            output r1 as answer(x);
        }
        indexes { Edge[0]; Edge[1]; }
    }
}

pub(super) fn non_leading_membership_index_requirements() -> TokenStream {
    quote! {
        pub struct NonLeadingMembershipProgram;
        relation Seed(c0: i32);
        relation Edge(c0: i32, c1: i32);
        relation Keep(c0: i32, c1: i32);
        answer(x, z) :- Seed(x), Edge(z, x), Keep(z, x);
        relational {
            r0 = rename Seed {c0 -> x};
            r1 = rename Edge {c0 -> z, c1 -> x};
            r2 = natural_join r0 with r1;
            r3 = rename Keep {c0 -> z, c1 -> x};
            r4 = natural_join r2 with r3;
            r5 = project r4 keep {x, z};
            output r5 as answer(x, z);
        }
        indexes { Edge[1]; Keep[0, 1]; }
    }
}

pub(super) fn reused_index_requirements() -> TokenStream {
    quote! {
        pub struct ReusedIndexProgram;
        relation Seeds(c0: i32, c1: i32);
        relation Edge(c0: i32, c1: i32);
        answer(a, b, x, y) :- Seeds(a, b), Edge(a, x), Edge(b, y);
        relational {
            r0 = rename Seeds {c0 -> a, c1 -> b};
            r1 = rename Edge {c0 -> a, c1 -> x};
            r2 = natural_join r0 with r1;
            r3 = rename Edge {c0 -> b, c1 -> y};
            r4 = natural_join r2 with r3;
            r5 = project r4 keep {a, b, x, y};
            output r5 as answer(a, b, x, y);
        }
        indexes { Edge[0]; }
    }
}

pub(super) fn self_join_index_requirements() -> TokenStream {
    quote! {
        struct SelfJoinProgram;
        relation R(c0: i32, c1: i32);
        path2(x, y, z) :- R(x, y), R(y, z);
        relational {
            r0 = rename R {c0 -> x, c1 -> y};
            r1 = rename R {c0 -> y, c1 -> z};
            r2 = natural_join r0 with r1;
            r3 = project r2 keep {x, y, z};
            output r3 as path2(x, y, z);
        }
        indexes { R[0]; }
    }
}

pub(super) fn multi_column_index_requirements() -> TokenStream {
    quote! {
        struct MultiColumnProgram;
        relation Pair(c0: i32, c1: i32);
        relation Fact(c0: i32, c1: i32, c2: i32, c3: i32);
        answer(a, b, c, d) :- Pair(a, b), Fact(c, b, a, d);
        relational {
            r0 = rename Pair {c0 -> a, c1 -> b};
            r1 = rename Fact {c0 -> c, c1 -> b, c2 -> a, c3 -> d};
            r2 = natural_join r0 with r1;
            r3 = project r2 keep {a, b, c, d};
            output r3 as answer(a, b, c, d);
        }
        indexes { Fact[1, 2]; }
    }
}

pub(super) fn heterogeneous_index_requirements() -> TokenStream {
    quote! {
        pub struct HeterogeneousProgram;
        relation Person(c0: ::std::string::String, c1: u32);
        relation Label(c0: ::std::string::String, c1: ::std::string::String);
        answer(person, label) :- Person(person, age), Label(person, label);
        relational {
            r0 = rename Person {c0 -> person, c1 -> age};
            r1 = rename Label {c0 -> person, c1 -> label};
            r2 = natural_join r0 with r1;
            r3 = project r2 keep {person, label};
            output r3 as answer(person, label);
        }
        indexes { Label[0]; Label[1]; }
    }
}

pub(super) fn projected_set_index_requirements() -> TokenStream {
    quote! {
        pub struct ProjectedSetProgram;
        relation Rows(c0: i32, c1: i32);
        answer(x) :- Rows(x, y);
        relational {
            r0 = rename Rows {c0 -> x, c1 -> y};
            r1 = project r0 keep {x};
            output r1 as answer(x);
        }
        indexes {}
    }
}
