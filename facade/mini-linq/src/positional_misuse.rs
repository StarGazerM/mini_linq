//! Positional capabilities are explicit and checked by generated Rust.
//!
//! A nonpositional host must use the original selector, not assert positions:
//! ```compile_fail
//! let rows = std::collections::HashSet::from([(1_i64,), (2,)]);
//! let _: Vec<_> = mini_linq::compile! {
//!     pipeline { select = mini_linq::passes::select(positional(R)); }
//!     struct BadHost; relation R(a:i64) = (rows);
//!     answer(a) :- R(a), where a == 1;
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
//!         l = pipe rename L {a -> x}; r = pipe rename R {b -> x,c -> z};
//!         s = let point select r if (z = 1);
//!         t = pipe semi eager [x] l,s;
//!         out = let canon project t keep {x}; return out as answer(x);
//!     }
//!     structures { providers { host_scan = ::core::iter::Map<_, _> { reads [], build typed }; vec = ::std::vec::Vec<_> { reads [], build typed, accounting rows }; no_interval = NoInterval<_> { reads [exact, range, membership], build typed }; sort = ::cq_storage::dedup::Sort { reads [distinct], build inferred, shared }; }
//!         l host scan = @host_scan as ::core::iter::Map<_,_> [];
//!         r host scan = @host_scan as ::core::iter::Map<_,_> [];
//!         r host result(s) = @host_scan as ::core::iter::Map<_,_> [z];
//!         s rows = @vec as ::std::vec::Vec<_>;
//!         s keyset = @no_interval as NoInterval<_> [x] owned base R lineage{x -> b,z -> c};
//!         t read = positional base L lineage{x -> a} column a
//!             marker ::cq_load::ColumnAt<0> predicate membership s[x] summary exact;
//!         out rows = @sort as ::cq_storage::dedup::Sort; out canon = rows;
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
