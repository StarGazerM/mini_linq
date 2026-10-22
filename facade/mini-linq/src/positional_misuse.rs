//! Positional capabilities are explicit and checked by generated Rust.
//!
//! A nonpositional host must use the original selector, not assert positions:
//! ```compile_fail
//! let rows = std::collections::HashSet::from([(1_i64,), (2,)]);
//! let _: Vec<_> = mini_linq::compile! {
//!     pipeline { select = mini_linq::passes::select(positional(R)); }
//!     struct BadHost; relation R(a:i64) = (rows);
//!     answer(a) :- R(a), if *a == 1;
//! }.collect();
//! ```
//!
//! Unknown providers still discharge the interval trait at a named build:
//! ```compile_fail
//! type NoInterval<T> = std::collections::HashSet<T>;
//! let left = [(1_i64,)]; let right = [(1_i64,1_i64)];
//! let _: Vec<_> = mini_linq::passes::sequence! {
//!     pipeline { mini_linq::passes::sequential, mini_linq::passes::translate_pull };
//!     physical {
//!         relation L(a:i64) = (left); relation R(b:i64,c:i64) = (right);
//!         l = pipe rename L@0 {a -> x}; r = pipe rename R@1 {b -> x,c -> z};
//!         s = let point select r if (z = 1);
//!         t = pipe semi eager [x] l,s;
//!         out = pipe canon project t keep {x}; return out as answer(x);
//!     }
//!     structures {
//!         l host scan = ::core::iter::Map<_,_> [];
//!         r host scan = ::core::iter::Map<_,_> [];
//!         r host s = ::core::iter::Map<_,_> [z];
//!         s rows = ::std::vec::Vec<_>;
//!         s keyset = NoInterval<_> [x] owned base R lineage{x -> b,z -> c};
//!         t read = positional base L lineage{x -> a} column a
//!             marker ::cq_load::ColumnAt<0> predicate membership s[x] summary exact;
//!         out canon = ::std::collections::HashSet<_>;
//!     }
//! }.collect();
//! ```
//!
//! An alias does not turn a positional mask into an ordered index:
//! ```compile_fail
//! let host = [(1_i64,)];
//! let selected = cq_storage::Selection::select(&host, cq_load::column::<0>(),
//!     &cq_storage::Point(1_i64), cq_storage::SelectionSummary::None);
//! cq_storage::require::ordered(&selected);
//! ```
