use proc_macro2::TokenStream;
use quote::quote;

pub struct LoweringFixture {
    pub source: TokenStream,
    pub expected: TokenStream,
}

pub struct PhysicalFixture {
    pub source: TokenStream,
    pub program_name: &'static str,
    pub rust_access_plan: TokenStream,
    pub iterator_pipeline: TokenStream,
    pub required_rust: &'static [&'static str],
    pub reused_normalized_inputs: &'static [usize],
    pub runtime_assertions: TokenStream,
}

pub struct InvalidSources {
    pub cq: TokenStream,
    pub index_requirements: TokenStream,
    pub rust_access_plan: TokenStream,
}

pub fn cq_to_relational_plan_and_indexes() -> Vec<LoweringFixture> {
    vec![
        LoweringFixture {
            source: quote! {
                pub struct TriangleProgram;
                relation R(c0: i32, c1: i32);
                relation S(c0: i32, c1: i32);
                relation T(c0: i32, c1: i32);
                triangle(x, y, z) :- R(x, y), S(y, z), T(z, x).
            },
            expected: triangle_index_requirements(),
        },
        LoweringFixture {
            source: quote! {
                struct NonLeadingKey;
                relation Seed(c0: i32);
                relation Edge(c0: i32, c1: i32);
                answer(x, z) :- Seed(x), Edge(z, x).
            },
            expected: quote! {
                struct NonLeadingKey;
                relation Seed(c0: i32);
                relation Edge(c0: i32, c1: i32);
                answer(x, z) :- Seed(x), Edge(z, x).
                relational {
                    r0 = rename Seed {c0 -> x};
                    r1 = rename Edge {c0 -> z, c1 -> x};
                    r2 = natural_join r0 with r1;
                    r3 = project r2 keep {x, z};
                    output r3 as answer(x, z).
                }
                indexes { Edge[1]; }
            },
        },
        LoweringFixture {
            source: quote! {
                struct RawNames;
                relation r#type(c0: i32);
                relation Edge(c0: i32, c1: i32);
                answer(x, z) :- r#type(r#x), Edge(z, x).
            },
            expected: quote! {
                struct RawNames;
                relation r#type(c0: i32);
                relation Edge(c0: i32, c1: i32);
                answer(x, z) :- r#type(r#x), Edge(z, x).
                relational {
                    r0 = rename r#type {c0 -> r#x};
                    r1 = rename Edge {c0 -> z, c1 -> x};
                    r2 = natural_join r0 with r1;
                    r3 = project r2 keep {x, z};
                    output r3 as answer(x, z).
                }
                indexes { Edge[1]; }
            },
        },
        LoweringFixture {
            source: quote! {
                struct SelfJoinProgram;
                relation R(c0: i32, c1: i32);
                path2(x, y, z) :- R(x, y), R(y, z).
            },
            expected: self_join_index_requirements(),
        },
        LoweringFixture {
            source: quote! {
                pub struct ReusedIndexProgram;
                relation Seeds(c0: i32, c1: i32);
                relation Edge(c0: i32, c1: i32);
                answer(a, b, x, y) :- Seeds(a, b), Edge(a, x), Edge(b, y).
            },
            expected: reused_index_requirements(),
        },
        LoweringFixture {
            source: quote! {
                struct MultiColumnProgram;
                relation Pair(c0: i32, c1: i32);
                relation Fact(c0: i32, c1: i32, c2: i32, c3: i32);
                answer(a, b, c, d) :- Pair(a, b), Fact(c, b, a, d).
            },
            expected: multi_column_index_requirements(),
        },
    ]
}

pub fn index_requirements_to_staged_rust() -> Vec<PhysicalFixture> {
    vec![
        PhysicalFixture {
            source: triangle_index_requirements(),
            program_name: "TriangleProgram",
            rust_access_plan: triangle_rust_access_plan(),
            iterator_pipeline: triangle_iterator_pipeline(),
            required_rust: &["Vec < (i32", "HashMap < (i32", "HashSet < (i32", "entry (("],
            reused_normalized_inputs: &[],
            runtime_assertions: triangle_runtime_assertions(),
        },
        PhysicalFixture {
            source: row_only_index_requirements(),
            program_name: "RowOnlyProgram",
            rust_access_plan: row_only_rust_access_plan(),
            iterator_pipeline: row_only_iterator_pipeline(),
            required_rust: &["Vec < (i32", "sort_unstable"],
            reused_normalized_inputs: &[],
            runtime_assertions: row_only_runtime_assertions(),
        },
        PhysicalFixture {
            source: unused_input_index_requirements(),
            program_name: "UnusedInputProgram",
            rust_access_plan: unused_input_rust_access_plan(),
            iterator_pipeline: unused_input_iterator_pipeline(),
            required_rust: &["Vec < (i32", "sort_unstable"],
            reused_normalized_inputs: &[],
            runtime_assertions: unused_input_runtime_assertions(),
        },
        PhysicalFixture {
            // Direct IndexRequirements entry may include a locally valid extra.
            // Its builder must survive even though the access sequence never uses it.
            source: extra_index_requirements(),
            program_name: "ExtraIndexProgram",
            rust_access_plan: extra_index_rust_access_plan(),
            iterator_pipeline: extra_index_iterator_pipeline(),
            required_rust: &[
                "Vec < (i32",
                "HashMap < (i32",
                "HashSet < (i32",
                "index0",
                "index1",
                "index2",
                "entry ((",
            ],
            reused_normalized_inputs: &[1],
            runtime_assertions: extra_index_runtime_assertions(),
        },
        PhysicalFixture {
            source: scan_proper_full_index_requirements(),
            program_name: "ScanProperFullProgram",
            rust_access_plan: scan_proper_full_rust_access_plan(),
            iterator_pipeline: scan_proper_full_iterator_pipeline(),
            required_rust: &[
                "HashSet < (i32",
                "HashMap < (i32",
                "index0",
                "index1",
                "entry ((",
            ],
            reused_normalized_inputs: &[0],
            runtime_assertions: scan_proper_full_runtime_assertions(),
        },
        PhysicalFixture {
            source: multiple_proper_indexes_index_requirements(),
            program_name: "MultipleProperIndexesProgram",
            rust_access_plan: multiple_proper_indexes_rust_access_plan(),
            iterator_pipeline: multiple_proper_indexes_iterator_pipeline(),
            required_rust: &[
                "Vec < (i32",
                "HashMap < (i32",
                "index0",
                "index1",
                "entry ((",
            ],
            reused_normalized_inputs: &[1],
            runtime_assertions: multiple_proper_indexes_runtime_assertions(),
        },
        PhysicalFixture {
            source: non_leading_membership_index_requirements(),
            program_name: "NonLeadingMembershipProgram",
            rust_access_plan: non_leading_membership_rust_access_plan(),
            iterator_pipeline: non_leading_membership_iterator_pipeline(),
            required_rust: &["Vec < (i32", "HashMap < (i32", "HashSet < (i32", "entry (("],
            reused_normalized_inputs: &[],
            runtime_assertions: non_leading_membership_runtime_assertions(),
        },
        PhysicalFixture {
            source: reused_index_requirements(),
            program_name: "ReusedIndexProgram",
            rust_access_plan: reused_index_rust_access_plan(),
            iterator_pipeline: reused_index_iterator_pipeline(),
            required_rust: &["Vec < (i32", "HashMap < (i32", "entry (("],
            reused_normalized_inputs: &[],
            runtime_assertions: reused_index_runtime_assertions(),
        },
        PhysicalFixture {
            source: self_join_index_requirements(),
            program_name: "SelfJoinProgram",
            rust_access_plan: self_join_rust_access_plan(),
            iterator_pipeline: self_join_iterator_pipeline(),
            required_rust: &["relation0", "Vec < (i32", "HashMap < (i32", "entry (("],
            reused_normalized_inputs: &[0],
            runtime_assertions: self_join_runtime_assertions(),
        },
        PhysicalFixture {
            source: multi_column_index_requirements(),
            program_name: "MultiColumnProgram",
            rust_access_plan: multi_column_rust_access_plan(),
            iterator_pipeline: multi_column_iterator_pipeline(),
            required_rust: &[
                "HashMap < (i32",
                "Vec < (i32",
                "Clone :: clone (b)",
                "entry ((",
            ],
            reused_normalized_inputs: &[],
            runtime_assertions: multi_column_runtime_assertions(),
        },
        PhysicalFixture {
            source: heterogeneous_index_requirements(),
            program_name: "HeterogeneousProgram",
            rust_access_plan: heterogeneous_rust_access_plan(),
            iterator_pipeline: heterogeneous_iterator_pipeline(),
            required_rust: &[
                ":: std :: string :: String",
                "Clone :: clone",
                "relation1 . iter",
            ],
            reused_normalized_inputs: &[1],
            runtime_assertions: heterogeneous_runtime_assertions(),
        },
        PhysicalFixture {
            source: projected_set_index_requirements(),
            program_name: "ProjectedSetProgram",
            rust_access_plan: projected_set_rust_access_plan(),
            iterator_pipeline: projected_set_iterator_pipeline(),
            required_rust: &["Vec < (i32", "sort_unstable"],
            reused_normalized_inputs: &[],
            runtime_assertions: projected_set_runtime_assertions(),
        },
    ]
}

pub fn triangle_rust_access_plan() -> TokenStream {
    quote! {
        triangle(x, y, z) => ((
            ::core::clone::Clone::clone(x),
            ::core::clone::Clone::clone(y),
            ::core::clone::Clone::clone(z),
        )) :-
            R(x, y) => for (x, y,) in (self.rows0.iter()),
            S(y, z) => for (z,) in (
                self.index0
                    .get(&(::core::clone::Clone::clone(y),))
                    .into_iter()
                    .flatten()
            ),
            T(z, x) => if (self.index1.contains(&(
                ::core::clone::Clone::clone(z),
                ::core::clone::Clone::clone(x),
            ))).
    }
}

fn row_only_rust_access_plan() -> TokenStream {
    quote! {
        answer(z, x, y) => ((
            ::core::clone::Clone::clone(z),
            ::core::clone::Clone::clone(x),
            ::core::clone::Clone::clone(y),
        )) :-
            Rows(x, y, z) => for (x, y, z,) in (self.rows0.iter()).
    }
}

fn unused_input_rust_access_plan() -> TokenStream {
    quote! {
        answer(x) => ((::core::clone::Clone::clone(x),)) :-
            Rows(x) => for (x,) in (self.rows0.iter()).
    }
}

fn extra_index_rust_access_plan() -> TokenStream {
    quote! {
        answer(x) => ((::core::clone::Clone::clone(x),)) :-
            Rows(x) => for (x,) in (self.rows0.iter()).
    }
}

fn scan_proper_full_rust_access_plan() -> TokenStream {
    quote! {
        answer(x, y, z) => ((
            ::core::clone::Clone::clone(x),
            ::core::clone::Clone::clone(y),
            ::core::clone::Clone::clone(z),
        )) :-
            R(x, y) => for (x, y,) in (self.index0.iter()),
            R(y, z) => for (z,) in (
                self.index1
                    .get(&(::core::clone::Clone::clone(y),))
                    .into_iter()
                    .flatten()
            ),
            R(x, y) => if (self.index0.contains(&(
                ::core::clone::Clone::clone(x),
                ::core::clone::Clone::clone(y),
            ))).
    }
}

fn multiple_proper_indexes_rust_access_plan() -> TokenStream {
    quote! {
        answer(x) => ((::core::clone::Clone::clone(x),)) :-
            Seed(x) => for (x,) in (self.rows0.iter()).
    }
}

fn non_leading_membership_rust_access_plan() -> TokenStream {
    quote! {
        answer(x, z) => ((
            ::core::clone::Clone::clone(x),
            ::core::clone::Clone::clone(z),
        )) :-
            Seed(x) => for (x,) in (self.rows0.iter()),
            Edge(z, x) => for (z,) in (
                self.index0
                    .get(&(::core::clone::Clone::clone(x),))
                    .into_iter()
                    .flatten()
            ),
            Keep(z, x) => if (self.index1.contains(&(
                ::core::clone::Clone::clone(z),
                ::core::clone::Clone::clone(x),
            ))).
    }
}

fn reused_index_rust_access_plan() -> TokenStream {
    quote! {
        answer(a, b, x, y) => ((
            ::core::clone::Clone::clone(a),
            ::core::clone::Clone::clone(b),
            ::core::clone::Clone::clone(x),
            ::core::clone::Clone::clone(y),
        )) :-
            Seeds(a, b) => for (a, b,) in (self.rows0.iter()),
            Edge(a, x) => for (x,) in (
                self.index0
                    .get(&(::core::clone::Clone::clone(a),))
                    .into_iter()
                    .flatten()
            ),
            Edge(b, y) => for (y,) in (
                self.index0
                    .get(&(::core::clone::Clone::clone(b),))
                    .into_iter()
                    .flatten()
            ).
    }
}

fn self_join_rust_access_plan() -> TokenStream {
    quote! {
        path2(x, y, z) => ((
            ::core::clone::Clone::clone(x),
            ::core::clone::Clone::clone(y),
            ::core::clone::Clone::clone(z),
        )) :-
            R(x, y) => for (x, y,) in (self.rows0.iter()),
            R(y, z) => for (z,) in (
                self.index0
                    .get(&(::core::clone::Clone::clone(y),))
                    .into_iter()
                    .flatten()
            ).
    }
}

fn multi_column_rust_access_plan() -> TokenStream {
    quote! {
        answer(a, b, c, d) => ((
            ::core::clone::Clone::clone(a),
            ::core::clone::Clone::clone(b),
            ::core::clone::Clone::clone(c),
            ::core::clone::Clone::clone(d),
        )) :-
            Pair(a, b) => for (a, b,) in (self.rows0.iter()),
            Fact(c, b, a, d) => for (c, d,) in (
                self.index0
                    .get(&(
                        ::core::clone::Clone::clone(b),
                        ::core::clone::Clone::clone(a),
                    ))
                    .into_iter()
                    .flatten()
            ).
    }
}

fn projected_set_rust_access_plan() -> TokenStream {
    quote! {
        answer(x) => ((::core::clone::Clone::clone(x),)) :-
            Rows(x, y) => for (x, y,) in (self.rows0.iter()).
    }
}

fn heterogeneous_rust_access_plan() -> TokenStream {
    quote! {
        answer(person, label) => ((
            ::core::clone::Clone::clone(person),
            ::core::clone::Clone::clone(label),
        )) :-
            Person(person, age) => for (person, age,) in (self.rows0.iter()),
            Label(person, label) => for (label,) in (
                self.index0
                    .get(&(::core::clone::Clone::clone(person),))
                    .into_iter()
                    .flatten()
            ).
    }
}

fn triangle_iterator_pipeline() -> TokenStream {
    quote! {
        iter0 = scan (x, y,) in (self.rows0.iter()) yield (x, y,);
        iter1 = join iter0 as (x, y,) with (z,) in (
            self.index0
                .get(&(::core::clone::Clone::clone(y),))
                .into_iter()
                .flatten()
        ) yield (x, y, z,);
        iter2 = filter iter1 as (x, y, z,) if (
            self.index1.contains(&(
                ::core::clone::Clone::clone(z),
                ::core::clone::Clone::clone(x),
            ))
        );
        iter3 = project iter2 as (x, y, z,) yield ((
            ::core::clone::Clone::clone(x),
            ::core::clone::Clone::clone(y),
            ::core::clone::Clone::clone(z),
        ));
        iter4 = distinct iter3;
        return iter4.
    }
}

fn row_only_iterator_pipeline() -> TokenStream {
    quote! {
        iter0 = scan (x, y, z,) in (self.rows0.iter()) yield (x, y, z,);
        iter1 = project iter0 as (x, y, z,) yield ((
            ::core::clone::Clone::clone(z),
            ::core::clone::Clone::clone(x),
            ::core::clone::Clone::clone(y),
        ));
        iter2 = distinct iter1;
        return iter2.
    }
}

fn unused_input_iterator_pipeline() -> TokenStream {
    quote! {
        iter0 = scan (x,) in (self.rows0.iter()) yield (x,);
        iter1 = project iter0 as (x,) yield ((::core::clone::Clone::clone(x),));
        iter2 = distinct iter1;
        return iter2.
    }
}

fn extra_index_iterator_pipeline() -> TokenStream {
    quote! {
        iter0 = scan (x,) in (self.rows0.iter()) yield (x,);
        iter1 = project iter0 as (x,) yield ((::core::clone::Clone::clone(x),));
        iter2 = distinct iter1;
        return iter2.
    }
}

fn scan_proper_full_iterator_pipeline() -> TokenStream {
    quote! {
        iter0 = scan (x, y,) in (self.index0.iter()) yield (x, y,);
        iter1 = join iter0 as (x, y,) with (z,) in (
            self.index1
                .get(&(::core::clone::Clone::clone(y),))
                .into_iter()
                .flatten()
        ) yield (x, y, z,);
        iter2 = filter iter1 as (x, y, z,) if (
            self.index0.contains(&(
                ::core::clone::Clone::clone(x),
                ::core::clone::Clone::clone(y),
            ))
        );
        iter3 = project iter2 as (x, y, z,) yield ((
            ::core::clone::Clone::clone(x),
            ::core::clone::Clone::clone(y),
            ::core::clone::Clone::clone(z),
        ));
        iter4 = distinct iter3;
        return iter4.
    }
}

fn multiple_proper_indexes_iterator_pipeline() -> TokenStream {
    quote! {
        iter0 = scan (x,) in (self.rows0.iter()) yield (x,);
        iter1 = project iter0 as (x,) yield ((::core::clone::Clone::clone(x),));
        iter2 = distinct iter1;
        return iter2.
    }
}

fn non_leading_membership_iterator_pipeline() -> TokenStream {
    quote! {
        iter0 = scan (x,) in (self.rows0.iter()) yield (x,);
        iter1 = join iter0 as (x,) with (z,) in (
            self.index0
                .get(&(::core::clone::Clone::clone(x),))
                .into_iter()
                .flatten()
        ) yield (x, z,);
        iter2 = filter iter1 as (x, z,) if (
            self.index1.contains(&(
                ::core::clone::Clone::clone(z),
                ::core::clone::Clone::clone(x),
            ))
        );
        iter3 = project iter2 as (x, z,) yield ((
            ::core::clone::Clone::clone(x),
            ::core::clone::Clone::clone(z),
        ));
        iter4 = distinct iter3;
        return iter4.
    }
}

fn reused_index_iterator_pipeline() -> TokenStream {
    quote! {
        iter0 = scan (a, b,) in (self.rows0.iter()) yield (a, b,);
        iter1 = join iter0 as (a, b,) with (x,) in (
            self.index0
                .get(&(::core::clone::Clone::clone(a),))
                .into_iter()
                .flatten()
        ) yield (a, b, x,);
        iter2 = join iter1 as (a, b, x,) with (y,) in (
            self.index0
                .get(&(::core::clone::Clone::clone(b),))
                .into_iter()
                .flatten()
        ) yield (a, b, x, y,);
        iter3 = project iter2 as (a, b, x, y,) yield ((
            ::core::clone::Clone::clone(a),
            ::core::clone::Clone::clone(b),
            ::core::clone::Clone::clone(x),
            ::core::clone::Clone::clone(y),
        ));
        iter4 = distinct iter3;
        return iter4.
    }
}

fn self_join_iterator_pipeline() -> TokenStream {
    quote! {
        iter0 = scan (x, y,) in (self.rows0.iter()) yield (x, y,);
        iter1 = join iter0 as (x, y,) with (z,) in (
            self.index0
                .get(&(::core::clone::Clone::clone(y),))
                .into_iter()
                .flatten()
        ) yield (x, y, z,);
        iter2 = project iter1 as (x, y, z,) yield ((
            ::core::clone::Clone::clone(x),
            ::core::clone::Clone::clone(y),
            ::core::clone::Clone::clone(z),
        ));
        iter3 = distinct iter2;
        return iter3.
    }
}

fn multi_column_iterator_pipeline() -> TokenStream {
    quote! {
        iter0 = scan (a, b,) in (self.rows0.iter()) yield (a, b,);
        iter1 = join iter0 as (a, b,) with (c, d,) in (
            self.index0
                .get(&(
                    ::core::clone::Clone::clone(b),
                    ::core::clone::Clone::clone(a),
                ))
                .into_iter()
                .flatten()
        ) yield (a, b, c, d,);
        iter2 = project iter1 as (a, b, c, d,) yield ((
            ::core::clone::Clone::clone(a),
            ::core::clone::Clone::clone(b),
            ::core::clone::Clone::clone(c),
            ::core::clone::Clone::clone(d),
        ));
        iter3 = distinct iter2;
        return iter3.
    }
}

fn projected_set_iterator_pipeline() -> TokenStream {
    quote! {
        iter0 = scan (x, y,) in (self.rows0.iter()) yield (x, y,);
        iter1 = project iter0 as (x, y,) yield ((::core::clone::Clone::clone(x),));
        iter2 = distinct iter1;
        return iter2.
    }
}

fn heterogeneous_iterator_pipeline() -> TokenStream {
    quote! {
        iter0 = scan (person, age,) in (self.rows0.iter()) yield (person, age,);
        iter1 = join iter0 as (person, age,) with (label,) in (
            self.index0
                .get(&(::core::clone::Clone::clone(person),))
                .into_iter()
                .flatten()
        ) yield (person, age, label,);
        iter2 = project iter1 as (person, age, label,) yield ((
            ::core::clone::Clone::clone(person),
            ::core::clone::Clone::clone(label),
        ));
        iter3 = distinct iter2;
        return iter3.
    }
}

fn triangle_runtime_assertions() -> TokenStream {
    quote! {
        let storage = TriangleProgram::build(
            [(1, 2,), (1, 2,), (2, 3,)],
            [(2, 3,), (3, 4,)],
            [(3, 1,), (4, 2,)],
        );

        // Calling `query` constructs the whole-result cursor but requests no
        // rows. Taking one row and dropping the cursor leaves storage reusable.
        let rows_requested = ::std::cell::Cell::new(0usize);
        let mut prefix = storage.query().inspect(|_| {
            rows_requested.set(rows_requested.get() + 1);
        }).take(1);
        assert_eq!(rows_requested.get(), 0);
        assert!(prefix.next().is_some());
        assert_eq!(rows_requested.get(), 1);
        assert!(prefix.next().is_none());
        drop(prefix);

        let lazy_rows = storage.query().collect::<::std::vec::Vec<_>>();
        let lazy_set = lazy_rows
            .iter()
            .cloned()
            .collect::<::std::collections::HashSet<_>>();
        assert_eq!(lazy_rows.len(), lazy_set.len());
        assert_eq!(
            lazy_set,
            ::std::collections::HashSet::from([(1, 2, 3,), (2, 3, 4,)]),
        );
        assert_eq!(storage.materialize(), vec![(1, 2, 3,), (2, 3, 4,)]);

        assert_eq!(
            TriangleProgram::run(
                [(1, 2,), (1, 2,), (2, 3,)],
                [(2, 3,), (3, 4,)],
                [(3, 1,), (4, 2,)],
            ),
            vec![(1, 2, 3,), (2, 3, 4,)],
        );
    }
}

fn row_only_runtime_assertions() -> TokenStream {
    quote! {
        assert_eq!(
            RowOnlyProgram::run([(1, 2, 3,), (1, 2, 3,), (4, 5, 6,)]),
            vec![(3, 1, 2,), (6, 4, 5,)],
        );
    }
}

fn unused_input_runtime_assertions() -> TokenStream {
    quote! {
        let storage = UnusedInputProgram::build([(2,), (1,), (2,)], [(9, 10,), (11, 12,)]);
        assert_eq!(storage.query().collect::<::std::vec::Vec<_>>().len(), 2);
        assert_eq!(storage.materialize(), vec![(1,), (2,)]);
        assert_eq!(
            UnusedInputProgram::run([(2,), (1,), (2,)], [(9, 10,), (11, 12,)]),
            vec![(1,), (2,)],
        );
    }
}

fn extra_index_runtime_assertions() -> TokenStream {
    quote! {
        assert_eq!(
            ExtraIndexProgram::run(
                [(2,), (1,), (2,)],
                [(1, 10,), (1, 10,), (2, 20,)],
            ),
            vec![(1,), (2,)],
        );
    }
}

fn scan_proper_full_runtime_assertions() -> TokenStream {
    quote! {
        let storage = ScanProperFullProgram::build([
            (1, 2,),
            (1, 2,),
            (2, 3,),
            (2, 4,),
        ]);
        assert_eq!(
            storage.query().collect::<::std::collections::HashSet<_>>(),
            ::std::collections::HashSet::from([(1, 2, 3,), (1, 2, 4,)]),
        );
        assert_eq!(
            storage.materialize(),
            vec![(1, 2, 3,), (1, 2, 4,)],
        );
    }
}

fn multiple_proper_indexes_runtime_assertions() -> TokenStream {
    quote! {
        let storage = MultipleProperIndexesProgram::build(
            [(2,), (1,), (2,)],
            [(1, 10,), (1, 11,), (2, 10,)],
        );
        assert_eq!(storage.query().collect::<::std::vec::Vec<_>>().len(), 2);
        assert_eq!(storage.materialize(), vec![(1,), (2,)]);
        assert_eq!(
            MultipleProperIndexesProgram::run(
                [(2,), (1,), (2,)],
                [(1, 10,), (1, 11,), (2, 10,)],
            ),
            vec![(1,), (2,)],
        );
    }
}

fn non_leading_membership_runtime_assertions() -> TokenStream {
    quote! {
        assert_eq!(
            NonLeadingMembershipProgram::run(
                [(1,), (2,)],
                [(3, 1,), (4, 1,), (5, 2,)],
                [(3, 1,), (5, 2,), (9, 9,)],
            ),
            vec![(1, 3,), (2, 5,)],
        );
    }
}

fn reused_index_runtime_assertions() -> TokenStream {
    quote! {
        assert_eq!(
            ReusedIndexProgram::run(
                [(1, 2,), (1, 2,)],
                [(1, 10,), (1, 11,), (2, 20,), (2, 21,)],
            ),
            vec![
                (1, 2, 10, 20,),
                (1, 2, 10, 21,),
                (1, 2, 11, 20,),
                (1, 2, 11, 21,),
            ],
        );
    }
}

fn self_join_runtime_assertions() -> TokenStream {
    quote! {
        assert_eq!(
            SelfJoinProgram::run([
                (1, 2,),
                (1, 2,),
                (2, 3,),
                (2, 4,),
                (3, 5,),
            ]),
            vec![(1, 2, 3,), (1, 2, 4,), (2, 3, 5,)],
        );
    }
}

fn multi_column_runtime_assertions() -> TokenStream {
    quote! {
        assert_eq!(
            MultiColumnProgram::run(
                [(1, 2,), (5, 6,), (1, 2,)],
                [
                    (9, 2, 1, 8,),
                    (7, 2, 1, 6,),
                    (4, 6, 5, 3,),
                    (0, 6, 1, 0,),
                ],
            ),
            vec![(1, 2, 7, 6,), (1, 2, 9, 8,), (5, 6, 4, 3,)],
        );
    }
}

fn projected_set_runtime_assertions() -> TokenStream {
    quote! {
        let storage = ProjectedSetProgram::build([
            (2, 30,),
            (1, 20,),
            (1, 10,),
            (1, 10,),
        ]);
        let lazy_rows = storage.query().collect::<::std::vec::Vec<_>>();
        let lazy_set = lazy_rows
            .iter()
            .cloned()
            .collect::<::std::collections::HashSet<_>>();

        // Distinct belongs to the public result-set iterator, not only to the
        // eager convenience method: two derivations of `(1,)` yield one row.
        assert_eq!(lazy_rows.len(), 2);
        assert_eq!(
            lazy_set,
            ::std::collections::HashSet::from([(1,), (2,)]),
        );
        assert_eq!(storage.materialize(), vec![(1,), (2,)]);
        assert_eq!(
            ProjectedSetProgram::run([
                (2, 30,),
                (1, 20,),
                (1, 10,),
                (1, 10,),
            ]),
            vec![(1,), (2,)],
        );
    }
}

fn heterogeneous_runtime_assertions() -> TokenStream {
    quote! {
        assert_eq!(
            HeterogeneousProgram::run(
                [
                    (::std::string::String::from("Ada"), 1u32,),
                    (::std::string::String::from("Grace"), 2u32,),
                ],
                [
                    (
                        ::std::string::String::from("Ada"),
                        ::std::string::String::from("Compiler"),
                    ),
                    (
                        ::std::string::String::from("Grace"),
                        ::std::string::String::from("COBOL"),
                    ),
                    (
                        ::std::string::String::from("Other"),
                        ::std::string::String::from("Unused"),
                    ),
                ],
            ),
            vec![
                (
                    ::std::string::String::from("Ada"),
                    ::std::string::String::from("Compiler"),
                ),
                (
                    ::std::string::String::from("Grace"),
                    ::std::string::String::from("COBOL"),
                ),
            ],
        );
    }
}

pub fn invalid_sources() -> InvalidSources {
    InvalidSources {
        cq: quote! {
            struct BadCq;
            relation R(c0: i32);
            answer(x) :- R(y).
        },
        index_requirements: quote! {
            struct BadIndexes;
            relation R(c0: i32);
            answer(x) :- R(x).
            relational {
                r0 = rename R {c0 -> x};
                r1 = project r0 keep {x};
                output r1 as answer(x).
            }
            indexes { R[0]; R[0]; }
        },
        rust_access_plan: quote! {
            answer(x) => ([x]) :-
                R(x) => if (rows.contains(&[x])).
        },
    }
}

pub fn missing_required_index() -> TokenStream {
    quote! {
        pub struct MissingRequiredIndex;
        relation Seed(c0: i32);
        relation Edge(c0: i32, c1: i32);
        answer(x, z) :- Seed(x), Edge(x, z).
        relational {
            r0 = rename Seed {c0 -> x};
            r1 = rename Edge {c0 -> x, c1 -> z};
            r2 = natural_join r0 with r1;
            r3 = project r2 keep {x, z};
            output r3 as answer(x, z).
        }
        indexes {}
    }
}

fn triangle_index_requirements() -> TokenStream {
    quote! {
        pub struct TriangleProgram;
        relation R(c0: i32, c1: i32);
        relation S(c0: i32, c1: i32);
        relation T(c0: i32, c1: i32);
        triangle(x, y, z) :- R(x, y), S(y, z), T(z, x).
        relational {
            r0 = rename R {c0 -> x, c1 -> y};
            r1 = rename S {c0 -> y, c1 -> z};
            r2 = natural_join r0 with r1;
            r3 = rename T {c0 -> z, c1 -> x};
            r4 = natural_join r2 with r3;
            r5 = project r4 keep {x, y, z};
            output r5 as triangle(x, y, z).
        }
        indexes { S[0]; T[0, 1]; }
    }
}

fn row_only_index_requirements() -> TokenStream {
    quote! {
        pub struct RowOnlyProgram;
        relation Rows(c0: i32, c1: i32, c2: i32);
        answer(z, x, y) :- Rows(x, y, z).
        relational {
            r0 = rename Rows {c0 -> x, c1 -> y, c2 -> z};
            r1 = project r0 keep {z, x, y};
            output r1 as answer(z, x, y).
        }
        indexes {}
    }
}

fn unused_input_index_requirements() -> TokenStream {
    quote! {
        pub struct UnusedInputProgram;
        relation Rows(c0: i32);
        relation Unused(c0: i32, c1: i32);
        answer(x) :- Rows(x).
        relational {
            r0 = rename Rows {c0 -> x};
            r1 = project r0 keep {x};
            output r1 as answer(x).
        }
        indexes {}
    }
}

fn extra_index_requirements() -> TokenStream {
    quote! {
        pub struct ExtraIndexProgram;
        relation Rows(c0: i32);
        relation Edge(c0: i32, c1: i32);
        answer(x) :- Rows(x).
        relational {
            r0 = rename Rows {c0 -> x};
            r1 = project r0 keep {x};
            output r1 as answer(x).
        }
        // Written full-key first on purpose: proper-key builders still have to
        // borrow relation1 before index0 takes ownership of that HashSet.
        indexes { Edge[0, 1]; Edge[0]; Edge[1]; }
    }
}

fn scan_proper_full_index_requirements() -> TokenStream {
    quote! {
        pub struct ScanProperFullProgram;
        relation R(c0: i32, c1: i32);
        answer(x, y, z) :- R(x, y), R(y, z), R(x, y).
        relational {
            r0 = rename R {c0 -> x, c1 -> y};
            r1 = rename R {c0 -> y, c1 -> z};
            r2 = natural_join r0 with r1;
            r3 = rename R {c0 -> x, c1 -> y};
            r4 = natural_join r2 with r3;
            r5 = project r4 keep {x, y, z};
            output r5 as answer(x, y, z).
        }
        // Full-key is written first on purpose. The proper-key map must borrow
        // relation0 before index0 takes ownership, and index0 also supplies the scan.
        indexes { R[0, 1]; R[0]; }
    }
}

fn multiple_proper_indexes_index_requirements() -> TokenStream {
    quote! {
        pub struct MultipleProperIndexesProgram;
        relation Seed(c0: i32);
        relation Edge(c0: i32, c1: i32);
        answer(x) :- Seed(x).
        relational {
            r0 = rename Seed {c0 -> x};
            r1 = project r0 keep {x};
            output r1 as answer(x).
        }
        indexes { Edge[0]; Edge[1]; }
    }
}

fn non_leading_membership_index_requirements() -> TokenStream {
    quote! {
        pub struct NonLeadingMembershipProgram;
        relation Seed(c0: i32);
        relation Edge(c0: i32, c1: i32);
        relation Keep(c0: i32, c1: i32);
        answer(x, z) :- Seed(x), Edge(z, x), Keep(z, x).
        relational {
            r0 = rename Seed {c0 -> x};
            r1 = rename Edge {c0 -> z, c1 -> x};
            r2 = natural_join r0 with r1;
            r3 = rename Keep {c0 -> z, c1 -> x};
            r4 = natural_join r2 with r3;
            r5 = project r4 keep {x, z};
            output r5 as answer(x, z).
        }
        indexes { Edge[1]; Keep[0, 1]; }
    }
}

fn reused_index_requirements() -> TokenStream {
    quote! {
        pub struct ReusedIndexProgram;
        relation Seeds(c0: i32, c1: i32);
        relation Edge(c0: i32, c1: i32);
        answer(a, b, x, y) :- Seeds(a, b), Edge(a, x), Edge(b, y).
        relational {
            r0 = rename Seeds {c0 -> a, c1 -> b};
            r1 = rename Edge {c0 -> a, c1 -> x};
            r2 = natural_join r0 with r1;
            r3 = rename Edge {c0 -> b, c1 -> y};
            r4 = natural_join r2 with r3;
            r5 = project r4 keep {a, b, x, y};
            output r5 as answer(a, b, x, y).
        }
        indexes { Edge[0]; }
    }
}

fn self_join_index_requirements() -> TokenStream {
    quote! {
        struct SelfJoinProgram;
        relation R(c0: i32, c1: i32);
        path2(x, y, z) :- R(x, y), R(y, z).
        relational {
            r0 = rename R {c0 -> x, c1 -> y};
            r1 = rename R {c0 -> y, c1 -> z};
            r2 = natural_join r0 with r1;
            r3 = project r2 keep {x, y, z};
            output r3 as path2(x, y, z).
        }
        indexes { R[0]; }
    }
}

fn multi_column_index_requirements() -> TokenStream {
    quote! {
        struct MultiColumnProgram;
        relation Pair(c0: i32, c1: i32);
        relation Fact(c0: i32, c1: i32, c2: i32, c3: i32);
        answer(a, b, c, d) :- Pair(a, b), Fact(c, b, a, d).
        relational {
            r0 = rename Pair {c0 -> a, c1 -> b};
            r1 = rename Fact {c0 -> c, c1 -> b, c2 -> a, c3 -> d};
            r2 = natural_join r0 with r1;
            r3 = project r2 keep {a, b, c, d};
            output r3 as answer(a, b, c, d).
        }
        indexes { Fact[1, 2]; }
    }
}

fn heterogeneous_index_requirements() -> TokenStream {
    quote! {
        pub struct HeterogeneousProgram;
        relation Person(c0: ::std::string::String, c1: u32);
        relation Label(c0: ::std::string::String, c1: ::std::string::String);
        answer(person, label) :- Person(person, age), Label(person, label).
        relational {
            r0 = rename Person {c0 -> person, c1 -> age};
            r1 = rename Label {c0 -> person, c1 -> label};
            r2 = natural_join r0 with r1;
            r3 = project r2 keep {person, label};
            output r3 as answer(person, label).
        }
        indexes { Label[0]; Label[1]; }
    }
}

fn projected_set_index_requirements() -> TokenStream {
    quote! {
        pub struct ProjectedSetProgram;
        relation Rows(c0: i32, c1: i32);
        answer(x) :- Rows(x, y).
        relational {
            r0 = rename Rows {c0 -> x, c1 -> y};
            r1 = project r0 keep {x};
            output r1 as answer(x).
        }
        indexes {}
    }
}
