//! # cq-rewrites
//!
//! External checked rewrite passes for MiniLinq. This standalone workspace uses
//! only the public core API. `cq-rewrites` exports the proc macros;
//! `cq-rewrite-passes` exports the corresponding functions for tools and tests.
//! Every macro runs `cq_stage::rewrite`, including the language's source, target,
//! and correspondence checks.
//!
//! `sorted_runs` takes no arguments and retypes only explicitly built
//! `::cq_storage::HashIndex` payload providers with one nonempty key level.
//! Shared membership reads keep using the same exact payload provider. Host
//! fallbacks, membership-only annotations, multi-level tries, and providers
//! used as smallest-intersection candidates are unchanged. The replacement
//! requires Ord + Copy keys and Copy items (Leibniz admissibility is a premise);
//! rustc checks Build, IndexRead and ExactMembership at the existing guards.
//! Only a single stored borrowed i32/i64 column is eligible, recording
//! BorrowedI64TupleCodec for the rank directory. Multi-column, non-integral,
//! and computed/owned keys remain HashIndex rather than selecting NoDense.
//! Eligible rank spans exceeding 1,048,576 bits still fall back to binary
//! search at runtime. No default selection changes.
//! Use `pipeline { structures = [cq_rewrites::sorted_runs]; }`.
//!
//! `retain_lookups` takes no arguments and records a `share` decision and a
//! `repr` witness for each eligible eager semijoin followed by a probe of the
//! same SortedRun build and preserved keys. The cursor is
//! `::cq_storage::RunCursor<'_, _>`; an existing positions representation is
//! preserved. The Structures contract rejects witness escape across retained
//! rows, canonicalization, opaque steps or other unsupported boundaries.
//! HashIndex builds and probes preceding the semijoin are unchanged. Compose
//! explicitly with
//! `pipeline { structures = [cq_rewrites::sorted_runs, cq_rewrites::retain_lookups]; }`.
//! Neither rewrite is enabled by default.
//!
//! ```rust
//! let rows = [(1, 10), (1, 11), (2, 20)];
//! let answer: Vec<_> = mini_linq::compile! {
//!     pipeline {
//!         physical = [cq_rewrites::canon_prefixes];
//!         structures = [cq_rewrites::retype(dedup => ::std::collections::BTreeSet<_>)];
//!     }
//!     struct Project;
//!     relation R(x: i32, y: i32) = (rows);
//!     answer(x) :- R(x, y);
//! }.map(|(x,)| *x).collect();
//! assert_eq!(answer.into_iter().collect::<std::collections::BTreeSet<_>>(),
//!            std::collections::BTreeSet::from([1, 2]));
//! ```
//!
//! The Physical macros accept no arguments, `all`, or comma-separated result
//! names, such as `merge_joins(j0, j2)`. The first two forms visit every admissible
//! occurrence. A named selection reports an error if any result is missing or
//! inadmissible. Reapplying a rewrite is idempotent. A projection already known
//! to preserve a key needs no additional canonical boundary.
//!
//! | Pass | Decision written | Admissibility |
//! |---|---|---|
//! | `merge_joins` | `probe` becomes `merge` | Nonempty Ord key; each side is retained, a reusable host, or a merge stream ordered on that same key. Native ordered host reads still require `OrderedRead` from rustc. |
//! | `canon_prefixes` | Adds `canon` on a projection | Drops columns without retaining an inferred uniqueness key; retained columns satisfy the language's Leibniz checks. Existing canonical boundaries remain. |
//! | `smallest_intersections` | Hoists independent relational definitions and records `probe smallest` | The public Physical `binary_intersection` predicate accepts the probe/check pair, and every interposed definition can move before the probe without moving Rust or breaking dependencies. |
//! | `lazy_semijoins` | `semi eager` becomes `semi lazy` | The right input is a reusable host and all readers sharing its key are lazy membership reads after the rewrite. Named selections must include every eager peer. |
//!
//! These passes preserve the operator graph, declarations, keys, modes, return
//! head, and opaque Rust expressions. `smallest_intersections` moves interposed
//! relational definitions earlier, preserving their dependency order, to make
//! the candidate probe and its check adjacent. This includes reusable host
//! renames and retained relational chains. Bind/guard nodes stay at their original
//! positions with the same payloads. An interposed Rust node or a consumer of the
//! candidate blocks the rewrite; an explicit named selection then reports an
//! inadmissible result. Other passes preserve definition order.
//!
//! The generated generic-fine triangle and the corpus `fine_triangle` both get
//! an active smallest choice. For example:
//!
//! ```rust
//! let r = [(1, 10), (1, 11)];
//! let s = [(10, 100), (11, 101)];
//! let t = [(1, 100)];
//! let answer: Vec<_> = mini_linq::compile! {
//!     pipeline {
//!         plan = cq_planners::order(written, fine);
//!         physical = [cq_rewrites::canon_prefixes, cq_rewrites::smallest_intersections];
//!     }
//!     struct Triangle;
//!     relation r(a:i32,b:i32) = (r);
//!     relation s(b:i32,c:i32) = (s);
//!     relation t(a:i32,c:i32) = (t);
//!     answer(a,b,c) :- r(a,b),s(b,c),t(a,c);
//! }.map(|(a,b,c)| (*a,*b,*c)).collect();
//! assert_eq!(answer, vec![(1,10,100)]);
//! ```
//!
//! `retype(selector => RustType)` replaces only recorded types. It preserves
//! Physical, entry order, reader identities, and index levels. A bare result name
//! selects that result's built indexes, keysets and keyed host fallbacks, including
//! supplemental candidate/membership reads. Scan, point, and range adapters keep
//! their recorded types. `result(name)` disambiguates a result named like a kind.
//!
//! | Kind selector | Selected entries |
//! |---|---|
//! | `payload` | Built and host entries answering a probe |
//! | `membership` | Built and host entries answering a semijoin, plus supplemental membership fallbacks |
//! | `ordered` | Built and host entries answering a merge |
//! | `trie` | Built entries with multiple written key levels |
//! | `dedup` | Canonical-boundary and computed-head dedup state |
//! | `rows` | Recorded retained row collections |
//! | `selection` | Immutable positional selections, preserving base, lineage and summary policy |
//! | `keyset` | Exact key-only builds, also matched by `membership` |
//! | `join_filter` | Conservative filters with a mandatory covering exact probe |
//!
//! `retype(rN join_filter => T)` selects only that scan's join-filter entry.
//! The type must implement `Build` and `ConservativeMembership`; the printer
//! emits a rustc capability guard. Exact membership types qualify automatically.
//! `join_filter(rN)` adds an exact filter when the scan has an admissible covering
//! probe, even without default positional selection. This explicitly requires
//! a positional host. Apply `retype` afterwards to choose another type. The
//! Structures contract rejects missing covers, anti joins and opaque Rust in
//! the intervening spine. Neither advice changes the exact covering probe.
//!
//! Keyset budget changes are explicit type rewrites. A read with an exact
//! interval summary cannot silently lose that trait: rewrite its summary to
//! `row` explicitly or retain a compatible provider. Aliases satisfy the emitted
//! runtime trait guards; no printer substitutes another provider. When a
//! supplementary keyset serves membership, the payload index keeps its separate
//! classification and needs only the traits used by its remaining readers.
//!
//! `kind(name)` explicitly selects a kind. Classification follows recorded reads
//! and levels, so aliases do not change which entries match. An entry serving
//! several kinds matches each one. Shared host fallback types must agree; a
//! partial kind rewrite that violates this constraint is rejected. An unmatched
//! selector is an error. Named Rust types can contain inference parameters.
//! Known incompatible representations fail the Structures check; arbitrary
//! aliases and custom implementations must satisfy the emitted runtime traits.
//!
//! For example, this alias cannot provide the requested payload index:
//!
//! ```compile_fail
//! type NotAnIndex<T> = std::marker::PhantomData<T>;
//! let left = [(1,)];
//! let right = [(1, 2)];
//! let _: Vec<_> = mini_linq::compile! {
//!     pipeline { structures = [cq_rewrites::retype(payload => NotAnIndex<_>)]; }
//!     struct MissingBuild;
//!     relation L(x: i32) = (left);
//!     relation R(x: i32, y: i32) = (right);
//!     answer(x, y) :- L(x), R(x, y);
//! }.collect();
//! ```
//!
//! A caller can start at any supported boundary with an explicit queue:
//!
//! ```rust
//! let input = [(1, 2), (1, 3)];
//! let output: Vec<_> = cq_rewrites::canon_prefixes! {
//!     arguments { projected };
//!     pipeline { mini_linq::passes::select, mini_linq::passes::print_pull };
//!     physical {
//!         relation R(a: i32, b: i32) = (input);
//!         r = pipe rename R@0 { a -> x, b -> y };
//!         projected = pipe project r keep { x };
//!         return projected as answer(x);
//!     }
//! }.map(|(x,)| *x).collect();
//! assert_eq!(output, vec![1]);
//! ```
//!
//! Tests use `cq-testkit`'s embedded corpus, mutation harness, independent Set
//! oracles, and executable written-order refinement. Sixty generated tests enter
//! at corpus IX or Physical and use the default lower/select/print macros. Other
//! tests exercise active merge and smallest choices, custom runtime traits,
//! representation obligations, exact IR fixtures, duplicates, and empty inputs.
//!
//! The authored adjacent triangle's finite 48-wide fan test resets runtime counters after fixture/oracle
//! construction and exhausts a fresh query before each snapshot:
//!
//! | Rewrites | Lookups | Enumerated rows | Cursor steps | Index builds |
//! |---|---:|---:|---:|---:|
//! | Baseline | 112,944 | 117,601 | 230,549 | 3 |
//! | Canonical prefixes | 2,353 | 4,801 | 7,158 | 3 |
//! | Smallest intersections | 9,264 | 9,313 | 11,670 | 4 |
//! | Both | 193 | 4,801 | 4,855 | 4 |
//!
//! All four answers equal the independent CQ oracle. Smallest selection adds a
//! membership build, so some counters increase. This experiment establishes
//! work reduction on this instance, not a general complexity bound.
//! Counts are outside the denotation: rewrites retain Kleisli bottom + Set,
//! written-order refinement, opaque Rust steps, and Leibniz equality.
//!
//! The mandatory `generated` regression uses `cq_planners::order(written, fine)`
//! through the public macro pipeline, captures its selected Physical program,
//! and checks its active smallest intersection, both contracts, and public
//! correspondence. Executed triangle and fan answers, duplicates, and empty inputs
//! agree with cq-testkit's CQ oracle. A generated query with opaque guards and a
//! partial division bind retains the exact Rust definitions and positions after
//! the active hoist, and refines executed written-order answers. The mandatory
//! `growth` regression uses the workload's symmetric fan with linear input size
//! and a planted triangle at scales 24, 48, and 96:
//!
//! | Scale | Binary enumerated rows | Generic-fine + both rewrites |
//! |---|---:|---:|
//! | 24 | 770 | 422 |
//! | 48 | 2,666 | 806 |
//! | 96 | 9,914 | 1,574 |
//!
//! Both plans' answers match the independent CQ oracle at every scale. The test
//! requires at least 3x row growth per doubling for binary and at most 2.5x for
//! generic-fine plus both rewrites. Observed ratios are 3.462/3.719 for binary
//! and 1.910/1.953 for generic-fine. The optimized plan builds five indexes at
//! each scale, versus two for binary. These finite measurements validate the
//! specified growth regression; they do not establish a general complexity bound.
//!
//! Run `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, and
//! `cargo test` here. The workspace defaults include both support crates.
//! From the repository root, `python3 tools/check_external.py` exercises packaged
//! public dependencies outside the checkout.

#![forbid(unsafe_code)]

use proc_macro::TokenStream;

macro_rules! physical_rewrite {
    ($name:ident) => {
        #[proc_macro]
        pub fn $name(input: TokenStream) -> TokenStream {
            cq_stage::rewrite::<cq_lang_physical::Program>(input.into(), cq_rewrite_passes::$name)
                .into()
        }
    };
}

physical_rewrite!(merge_joins);
physical_rewrite!(canon_prefixes);
physical_rewrite!(smallest_intersections);
physical_rewrite!(lazy_semijoins);

/// Change only the representation types matched by `selector => RustType`.
#[proc_macro]
pub fn retype(input: TokenStream) -> TokenStream {
    cq_stage::rewrite::<cq_lang_structures::Program>(input.into(), cq_rewrite_passes::retype).into()
}

/// Record an exact join filter on a named eligible scan occurrence.
#[proc_macro]
pub fn join_filter(input: TokenStream) -> TokenStream {
    cq_stage::rewrite::<cq_lang_structures::Program>(input.into(), cq_rewrite_passes::join_filter)
        .into()
}

/// Retype eligible one-level payload builds to stable sorted runs.
#[proc_macro]
pub fn sorted_runs(input: TokenStream) -> TokenStream {
    cq_stage::rewrite::<cq_lang_structures::Program>(input.into(), cq_rewrite_passes::sorted_runs)
        .into()
}

/// Retain a SortedRun lookup cursor from an eager semijoin until its later probe.
///
/// Records checked `share` and `repr ... witness` decisions only when invoked;
/// ordinary pipelines retain their existing lookup behavior. Compose after
/// `sorted_runs` to make eligible payload builds expose Copy range cursors.
#[proc_macro]
pub fn retain_lookups(input: TokenStream) -> TokenStream {
    cq_stage::rewrite::<cq_lang_structures::Program>(
        input.into(),
        cq_rewrite_passes::retain_lookups,
    )
    .into()
}
