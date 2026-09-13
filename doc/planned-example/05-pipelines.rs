pub struct PlannedExample;
pub struct PlannedExampleStorage {
    answer: ::std::collections::HashSet<(i32,)>,
    stats: PlannedExampleExecutionStats,
}
#[derive(Debug, Default)]
pub struct PlannedExampleExecutionStats {
    pub results: ::std::vec::Vec<(&'static str, usize, usize)>,
    pub index_rows: usize,
    pub join_pairs: usize,
    pub semijoin_probes: usize,
}
impl PlannedExample {
    pub fn build(
        input0: impl ::core::iter::IntoIterator<Item = (i32, i32)>,
        input1: impl ::core::iter::IntoIterator<Item = (i32, i32)>,
    ) -> PlannedExampleStorage {
        let mut __stats = PlannedExampleExecutionStats::default();
        let __pairs = ::core::cell::Cell::new(0usize);
        let __probes = ::core::cell::Cell::new(0usize);
        let input0: ::std::collections::HashSet<(i32, i32)> = input0
            .into_iter()
            .collect();
        let input1: ::std::collections::HashSet<(i32, i32)> = input1
            .into_iter()
            .collect();
        let relation0: ::std::collections::HashSet<(i32, i32)> = {
            let __left = &input0;
            ::mini_linq::collect_pipeline! {
                iter0 = scan(v0, v1,) in (__left.iter()) yield (v0, v1,); iter1 = project
                iter0 as (v0, v1,) yield ((::core::clone::Clone::clone(v0),
                ::core::clone::Clone::clone(v1),)); iter2 = distinct iter1; return iter2;
            }
        };
        __stats.results.push(("r0", 2usize, relation0.len()));
        let index0: ::std::collections::HashSet<_> = relation0
            .iter()
            .map(|__row| (::core::clone::Clone::clone(&__row.1),))
            .collect();
        __stats.index_rows += relation0.len();
        let relation1: ::std::collections::HashSet<(i32, i32)> = {
            let __left = &input1;
            ::mini_linq::collect_pipeline! {
                iter0 = scan(v1, v2,) in (__left.iter()) yield (v1, v2,); iter1 = project
                iter0 as (v1, v2,) yield ((::core::clone::Clone::clone(v1),
                ::core::clone::Clone::clone(v2),)); iter2 = distinct iter1; return iter2;
            }
        };
        __stats.results.push(("r1", 2usize, relation1.len()));
        let relation2: ::std::collections::HashSet<(i32, i32)> = {
            let __left = &relation1;
            ::mini_linq::collect_pipeline! {
                iter0 = scan(v1, v2,) in (__left.iter()) yield (v1, v2,); iter1 = filter
                iter0 as (v1, v2,) if ({ let value : ::core::primitive::i32 = * v2; value
                } > 0i32); iter2 = project iter1 as (v1, v2,) yield
                ((::core::clone::Clone::clone(v1), ::core::clone::Clone::clone(v2),));
                iter3 = distinct iter2; return iter3;
            }
        };
        __stats.results.push(("r2", 2usize, relation2.len()));
        let relation3: ::std::collections::HashSet<(i32, i32)> = {
            let __left = &relation2;
            let __index = &index0;
            let __probes_ref = &__probes;
            ::mini_linq::collect_pipeline! {
                iter0 = scan(v1, v2,) in (__left.iter()) yield (v1, v2,); iter1 = filter
                iter0 as (v1, v2,) if ({ __probes_ref.set(__probes_ref.get() + 1);
                __index.contains(& (::core::clone::Clone::clone(v1),)) }); iter2 =
                project iter1 as (v1, v2,) yield ((::core::clone::Clone::clone(v1),
                ::core::clone::Clone::clone(v2),)); iter3 = distinct iter2; return iter3;
            }
        };
        __stats.results.push(("r3", 2usize, relation3.len()));
        let index1: ::std::collections::HashSet<_> = relation3
            .iter()
            .map(|__row| (::core::clone::Clone::clone(&__row.0),))
            .collect();
        __stats.index_rows += relation3.len();
        let relation4: ::std::collections::HashSet<(i32, i32)> = {
            let __left = &relation0;
            let __index = &index1;
            let __probes_ref = &__probes;
            ::mini_linq::collect_pipeline! {
                iter0 = scan(v0, v1,) in (__left.iter()) yield (v0, v1,); iter1 = filter
                iter0 as (v0, v1,) if ({ __probes_ref.set(__probes_ref.get() + 1);
                __index.contains(& (::core::clone::Clone::clone(v1),)) }); iter2 =
                project iter1 as (v0, v1,) yield ((::core::clone::Clone::clone(v0),
                ::core::clone::Clone::clone(v1),)); iter3 = distinct iter2; return iter3;
            }
        };
        __stats.results.push(("r4", 2usize, relation4.len()));
        let mut index2 = ::std::collections::HashMap::<_, ::std::vec::Vec<_>>::new();
        for __row in &relation4 {
            index2
                .entry((::core::clone::Clone::clone(&__row.1),))
                .or_default()
                .push((::core::clone::Clone::clone(&__row.0),));
        }
        __stats.index_rows += relation4.len();
        let relation5: ::std::collections::HashSet<(i32,)> = {
            let __left = &relation3;
            ::mini_linq::collect_pipeline! {
                iter0 = scan(v1, v2,) in (__left.iter()) yield (v1, v2,); iter1 = project
                iter0 as (v1, v2,) yield ((::core::clone::Clone::clone(v1),)); iter2 =
                distinct iter1; return iter2;
            }
        };
        __stats.results.push(("r5", 1usize, relation5.len()));
        let relation6: ::std::collections::HashSet<(i32, i32)> = {
            let __left = &relation5;
            let __index = &index2;
            let __pairs_ref = &__pairs;
            ::mini_linq::collect_pipeline! {
                iter0 = scan(v1,) in (__left.iter()) yield (v1,); iter1 = join iter0 as
                (v1,) with(v0,) in (__index.get(& (::core::clone::Clone::clone(v1),))
                .into_iter().flatten()) yield (v1, v0,); iter2 = project iter1 as (v1,
                v0,) yield ({ __pairs_ref.set(__pairs_ref.get() + 1);
                (::core::clone::Clone::clone(v0), ::core::clone::Clone::clone(v1),) });
                iter3 = distinct iter2; return iter3;
            }
        };
        __stats.results.push(("r6", 2usize, relation6.len()));
        let relation7: ::std::collections::HashSet<(i32,)> = {
            let __left = &relation6;
            ::mini_linq::collect_pipeline! {
                iter0 = scan(v0, v1,) in (__left.iter()) yield (v0, v1,); iter1 = project
                iter0 as (v0, v1,) yield ((::core::clone::Clone::clone(v0),)); iter2 =
                distinct iter1; return iter2;
            }
        };
        __stats.results.push(("r7", 1usize, relation7.len()));
        __stats.join_pairs = __pairs.get();
        __stats.semijoin_probes = __probes.get();
        let answer = relation7
            .iter()
            .map(|__row| { (::core::clone::Clone::clone(&__row.0),) })
            .collect();
        PlannedExampleStorage {
            answer,
            stats: __stats,
        }
    }
    pub fn run(
        input0: impl ::core::iter::IntoIterator<Item = (i32, i32)>,
        input1: impl ::core::iter::IntoIterator<Item = (i32, i32)>,
    ) -> ::std::vec::Vec<(i32,)> {
        Self::build(input0, input1).materialize()
    }
    pub fn run_with_stats(
        input0: impl ::core::iter::IntoIterator<Item = (i32, i32)>,
        input1: impl ::core::iter::IntoIterator<Item = (i32, i32)>,
    ) -> (::std::vec::Vec<(i32,)>, PlannedExampleExecutionStats) {
        let storage = Self::build(input0, input1);
        (storage.materialize(), storage.stats)
    }
}
impl PlannedExampleStorage {
    pub fn query(&self) -> impl ::core::iter::Iterator<Item = (i32,)> + '_ {
        self.answer.iter().cloned()
    }
    pub fn materialize(&self) -> ::std::vec::Vec<(i32,)> {
        let mut rows: ::std::vec::Vec<_> = self.query().collect();
        rows.sort_unstable();
        rows
    }
    pub fn stats(&self) -> &PlannedExampleExecutionStats {
        &self.stats
    }
}
