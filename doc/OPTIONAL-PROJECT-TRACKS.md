# Advanced extension menu after the basic checkpoint

Every team completes **one** released advanced extension after the common core.
This document records the planned extension families and how they relate to the
[negation and aggregation option](HW4-OPTIONAL.md). An option becomes assignable
only when staff publishes its `pick-one/<name>` branch with a complete scaffold,
tests, and verified reference implementation.

Teams rank three preferences. With an expected enrollment below 20, the
instructor balances assignments so different teams explore different decisions
when practical. If an option is shared, teams receive different workloads,
claims, or adversarial cases. Students complete one extension deeply; combining
tracks is not required.

The current baseline is a genuine pull path. `compile_pull` first exposes a
named IteratorPipeline with results `iter0`, `iter1`, and so on, and `scan`,
left-deep binary `join`, `filter`, `project`, and `distinct` operators. Each
consumer explicitly names its predecessor. The supplied Rust lowering maps
that plan to one lazy iterator that embodies the entire query execution. Each
client call to `next` pulls one binding through the operator chain; intermediate
tuples are streamed, not collected as relations. Public `Storage::query`
constructs the result iterator without reading a row. Its only `for` loop is in
an eager client that chooses to materialize the root iterator. Rust may inline
the emitted combinators, but operator fusion is not a requirement of this
baseline.

The synchronous push warm-up below is part of the required execution-control
core. Tokio channels and the IVM continuation are separate pick-one options.
Push branches from RustAccessPlan rather than translating the pull-specific
IteratorPipeline. Async tasks and channels are a second push realization, not
a pull variant.

## Route graph and configuration rule

The required baseline is one explicit chain:

```text
CQ
  -- Basic HW1 --> RelationalPlan                         // rN DAG references
  -- Basic HW2 --> IndexRequirements(RelationalPlan)
  -- Basic HW3 --> sequential Rust residual + RustAccessPlan
  -- supplied pull --> IteratorPipeline                  // iterN edges
  -- supplied Rust lowering --> lazy result-set Iterator
```

The negation and aggregation option extends the first four languages rather
than bypassing the new
logical layer:

```text
Extended CQ with ! and agg
  --> Extended RelationalPlan with AntiSemijoin and AggregateApply
  --> Extended IndexRequirements preserving that entire plan
  --> Extended sequential RustAccessPlan
  --> the supplied pull route
```

The general RelationalPlan already supplies the singleton zero-attribute `Unit`
source. HW4 uses it when global negation or aggregation appears before any
positive binding; their empty correlation keys require no index entries. The
narrow Basic-HW2/3 assignment does not require Unit. The implemented default
expression route already supports it; global negation and aggregation still
require their own logical and physical lowering.

The negation and aggregation option is cumulative: students edit the production grammar and the same
typed passes, completed extended queries continue to use `mini_linq!`, and its
`hw4` feature gates only extension-specific test targets. It does not select a backend or
swap macro semantics. A backend alternative such as push, Tokio, or Rayon gets
a separately named route macro and typed target when it is implemented. A
Cargo feature may make that separately named route's dependencies and tests
available; it must not reinterpret an existing macro. Separate checkpoint
branches are the default packaging mechanism when two options change
overlapping decisions.

The extension families are intentionally separate at first:

| Track | Decision it exposes | Correct branch point |
|---|---|---|
| Push and async messages | who drives execution and how intermediate bindings move | `rust_access_plan::Plan` |
| Parallel Rayon execution | work partitioning, parallel builders, and result ownership | IndexRequirements, before sequential physical Rust |
| Multiple queries | global index sharing across independent query bodies | CQ, RelationalPlan, and IndexRequirements |

Do not combine the tracks in their first release. A student who completes one
track should be able to explain exactly which decision changed and which parts
of the basic route remained unchanged. Each is cut from the completed basic
reference as its own `pick-one/<name>` branch; do not make the baseline
production grammar a three-way `cfg` matrix merely to store future choices.

The first-release compatibility policy is:

| Pair | Design relationship | Released together? |
|---|---|---|
| HW4 + push/Tokio | Compatible in principle, but every push target needs explicit anti-semijoin and blocking aggregate behavior | No; separate checkpoints |
| HW4 + Rayon | Compatible in principle, but aggregation changes partition-local state and reduction | No; separate checkpoints |
| HW4 + multiple queries | Compatible in principle, but the extended operators and keys must be checked per query and globally | No; separate checkpoints |
| HW4 + typed baseline | The required `sum` still fixes its amount and result to `i32`; unrelated relation columns retain their declared Rust types | Compatible with that explicit aggregate restriction |
| push/Tokio + Rayon | They choose different execution graphs, ownership boundaries, and hosts | Alternative physical routes, not feature stacking |
| push/Tokio + multiple queries | Query-local push can coexist with shared storage only after a combined host/termination contract is supplied | No; separate checkpoints |
| push/Tokio + typed baseline | Synchronous callbacks preserve the declared types; Tokio additionally needs owned `Send` message types | No combined checkpoint |
| Rayon + multiple queries | Plausible, but it needs globally shared parallel builders and per-query result reduction | No combined checkpoint |
| Rayon + typed baseline | Rayon adds `Send + Sync` to the baseline's ordinary ownership and clone obligations | The parallel checkpoint must state those extra bounds |
| multiple queries + typed baseline | Shared declarations must retain one exact ordered type list across all queries | The multiple-query checker owns this validation |

## What staff must provide for every track

Students should transform programs, not reverse-engineer assignment
infrastructure. Every released track therefore includes:

1. the complete target syntax object when a new IR is justified;
2. its parser, token emission, span-aware local checker, and public macro;
3. the surrounding Rust API/runtime wiring and Cargo dependencies;
4. one complete triangle-like program at every boundary, not an ellipsis or a
   fragment detached from its enclosing `run` function;
5. either a supplied emitter from the last IR to ordinary Rust or, when that
   emitter is the exercise, exact target fixtures plus a post-deadline
   reference implementation;
6. structural, compile, runtime, differential, and error fixtures; and
7. numbered comments at every student-owned function or match arm.

Any added public macro follows the basic typed boundary directly: its `#[proc_macro]`
entry parses the invocation into the typed source and calls the corresponding
typed compiler function. Grammar failures become span-aware `syn::Error`
diagnostics emitted as `compile_error!`. There is no separate public expansion
wrapper. The public typed compiler function calls its source's
`contract::check` before lowering; semantically invalid input returns a
span-aware `syn::Error` in both debug and release builds. The macro entry
forwards this error as `compile_error!` without duplicating the check. Debug
postconditions separately check the specified transformation and may be
disabled without removing source validation. Tests and tools can also call
`contract::check` directly.

The terminology is precise:

- CQ, RelationalPlan, and IndexRequirements are whole-program IRs.
- A complete `syn::File` containing a local macro invocation is a **staged Rust
  program** or **program residual**.
- The syntax object parsed by that local macro is a local IR.
- `pull`, `push`, and `push_tokio` name transformations. They are not names for
  an otherwise unspecified query IR.
- IteratorPipeline is the concrete operator target of `pull`; callback and
  scheduled push transformations have different targets.

## Track A: push execution, Tokio channels, and an IVM continuation

### A1. Synchronous push warm-up

The smallest honest comparison with pull is a callback pipeline. Staff
supplies the following target language and its complete `Program + Storage`
shell:

```text
RustCallbackNest ::= "for_each" RustPat "in" "(" RustExpr ")" ";" RustCallbackNest
                   | "guard" "(" RustExpr ")" ";" RustCallbackNest
                   | "emit" "(" RustExpr ")" ";"
```

Every syntax object has a direct meaning: `for_each` invokes its continuation
for each source item, `guard` invokes it conditionally, and `emit` calls the
method's consumer with one output row. The student implements only:

```rust
fn compile_push(
    source: &rust_access_plan::Plan,
) -> syn::Result<rust_callback_nest::Nest>;
```

For the triangle, the complete local result is:

```rust
::mini_linq::rust_callback_nest! {
    for_each (x, y,) in (self.rows0.iter());
    for_each (z,) in (
        self.index0
            .get(&(::core::clone::Clone::clone(y),))
            .into_iter()
            .flatten()
    );
    guard (self.index1.contains(&(
        ::core::clone::Clone::clone(z),
        ::core::clone::Clone::clone(x),
    )));
    emit ((
        ::core::clone::Clone::clone(x),
        ::core::clone::Clone::clone(y),
        ::core::clone::Clone::clone(z),
    ));
}
```

The supplied emitter places the expansion in a method such as
`produce(&self, mut consume: impl FnMut(Row))` and is structural:

```rust
::core::iter::Iterator::for_each(
    ::core::iter::IntoIterator::into_iter(self.rows0.iter()),
    |(x, y,)| {
        ::core::iter::Iterator::for_each(
            ::core::iter::IntoIterator::into_iter(
                self.index0
                    .get(&(::core::clone::Clone::clone(y),))
                    .into_iter()
                    .flatten(),
            ),
            |(z,)| {
                if self.index1.contains(&(
                    ::core::clone::Clone::clone(z),
                    ::core::clone::Clone::clone(x),
                )) {
                    consume((
                        ::core::clone::Clone::clone(x),
                        ::core::clone::Clone::clone(y),
                        ::core::clone::Clone::clone(z),
                    ));
                }
            },
        );
    },
);
```

This is fused synchronous push through callbacks. Unlike the baseline
IteratorPipeline, its final representation deliberately erases the operator
stream boundary into nested producer/consumer callbacks. It has no queue,
backpressure, lazily returned result iterator, or parallelism. At the
query-operator boundary each producer invokes its continuation; a leaf source
may still use a Rust iterator internally. A materializer passes a closure that
inserts emitted rows; the pipeline itself never exposes `next`.
Both callbacks borrow their physical tuples; the only owned row constructed
by this traversal is the value passed to `consume`.
Because closure lowering changes the meaning of nonlocal control, the supplied
checker rejects unsupported nonlocal control in embedded expressions.

This fused callback path is the closest course analogue of HyPer's
[data-centric produce/consume
compilation](https://www.vldb.org/pvldb/vol4/p539-neumann.pdf). The callback
forms are compile-time interfaces used to create one fused pipeline; they are
not runtime channels.

### A2. Advanced Tokio channel lowering

Async message passing is a distinct, harder push target. It also branches from
`rust_access_plan::Plan`; the annotated logical occurrences make it possible
to derive and check which prefix variables each message must carry.

Staff supplies `tokio_channel_plan::Plan` and its checker/emitter. Its fields
use real Rust patterns, expressions, blocks, and message types; it does not
reintroduce storage kinds or logical lookup descriptors.

```text
Plan       ::= "capacity" Integer ";" Channel+ Stage+ Schedule
Channel    ::= "channel" Ident "(" Ident "," Ident ")" ":" RustType ";"
Stage      ::= SourceStage | ForEachStage | GuardStage | SinkStage
SourceStage::= "source" Ident "->" Ident "{"
                 "for" RustPat "in" "(" RustExpr ")" ";"
                 "send" RustExpr ";"
               "}"
ForEachStage
            ::= "stage" Ident Ident "->" Ident "{"
                 "recv" RustPat ";"
                 "for" RustPat "in" "(" RustExpr ")" ";"
                 "send" RustExpr ";"
               "}"
GuardStage ::= "guard_stage" Ident Ident "->" Ident "{"
                 "recv" RustPat ";"
                 "if" "(" RustExpr ")" ";"
                 "send" RustExpr ";"
               "}"
SinkStage  ::= "sink" Ident "<-" Ident "{"
                 "recv" RustPat ";" RustBlock
               "}"
Schedule   ::= "schedule" "try_join" "[" Ident ("," Ident)* "]" ";"
```

The triangle plan makes every binding message and edge explicit:

```text
capacity 64;
channel edge0(tx0, rx0): (::std::primitive::i32, ::std::primitive::i32,);
channel edge1(tx1, rx1): (::std::primitive::i32, ::std::primitive::i32, ::std::primitive::i32,);
channel edge2(tx2, rx2): (::std::primitive::i32, ::std::primitive::i32, ::std::primitive::i32,);

source scan_r -> tx0 {
    for (x, y,) in (self.rows0.iter());
    send (
        ::core::clone::Clone::clone(x),
        ::core::clone::Clone::clone(y),
    );
}

stage probe_s rx0 -> tx1 {
    recv (x, y,);
    for (z,) in (self.index0
        .get(&(::core::clone::Clone::clone(&y),))
        .into_iter()
        .flatten());
    send (x, y, ::core::clone::Clone::clone(z),);
}

guard_stage check_t rx1 -> tx2 {
    recv (x, y, z,);
    if (self.index1.contains(&(
        ::core::clone::Clone::clone(&z),
        ::core::clone::Clone::clone(&x),
    )));
    send (x, y, z,);
}

sink collect <- rx2 {
    recv (x, y, z,);
    { result.insert((x, y, z,)); }
}

schedule try_join [scan_r, probe_s, check_t, collect];
```

This target already decides the stage graph, message schemas, channel
capacity, concrete Rust operations, and schedule. The student does not design
Tokio plumbing. The only student transformation is:

```rust
fn compile_push_tokio(
    source: &rust_access_plan::Plan,
) -> syn::Result<tokio_channel_plan::Plan>;
```

The scan and lookup still borrow their stored tuples. A `send`, however, is an
explicit ownership boundary: it constructs an owned channel message by
cloning the borrowed fields. Later `recv` bindings are owned; constructing a
lookup or membership tuple clones only fields that the message must retain for
the following send. This channel cost is part of the
chosen push protocol, not an implicit `.cloned()` traversal of storage.

The supplied host is an `async fn run_async(...) -> Result<Vec<Row>,
ChannelError>`. It uses bounded `tokio::sync::mpsc::channel(capacity)`,
`send().await`, and `tokio::try_join!`. It does not call `block_on` or create a
nested runtime. The reference path uses local futures rather than
`tokio::spawn`, so this checkpoint teaches concurrency and backpressure
without also requiring `Send + 'static`, `Arc`, and thread parallelism.
Staff places `ChannelError` and any public async support in a normal support
crate or facade; a proc-macro crate cannot serve as the public runtime library,
and students do not configure Tokio dependencies themselves.

The checker enforces:

- positive capacity;
- an acyclic linear single-producer/single-consumer graph;
- exactly one owner for every sender and receiver, with no hidden clones;
- one message field per currently bound variable, in first-binding order;
- each stage scheduled exactly once; and
- a final sink whose variables agree with the query head.

Each stage moves and drops its sender when its source is exhausted. Downstream
receivers drain buffered messages and then observe `None`; this is the normal
termination protocol. Sending to a closed receiver is a typed execution error.
The first stage error cancels the remaining joined futures, and dropping
`run_async` cancels the query. No partial-result guarantee is made after
cancellation.

Required tests compare pull, callback push, and channel push on the same basic
fixtures. They also cover capacity one with more than one message, empty-input
closure, a deliberately closed receiver, cancellation, exact evaluation order
for the synchronous chain, and current-thread Tokio execution. A bounded Tokio
channel provides backpressure; it does not by itself make the query parallel.
The stage-per-occurrence channel pipeline is intentionally a scheduling and
message-semantics exercise; it should not be presented as the default design
of a compiled in-memory join engine.

### A3. Continuing from push to incremental view maintenance

Merely using async channels is not IVM and does not demonstrate semi-naive or
incremental evaluation. A serious continuation uses persistent indexed state
and differential messages. Staff should supply a further target such as
`TokioWorklistActor`:

```rust
struct Change<Row> {
    row: Row,
    diff: i64,
}

enum TriangleDelta {
    R(Change<(i32, i32,)>),
    S(Change<(i32, i32,)>),
    T(Change<(i32, i32,)>),
}

enum Command<Delta> {
    Apply { epoch: u64, batch: Box<[Delta]> },
    Seal { epoch: u64 },
    Stop,
}

enum Event<Row> {
    Changes { epoch: u64, batch: Box<[Change<Row>]> },
    EpochComplete { epoch: u64 },
}
```

One bounded command inbox feeds an actor that owns all persistent indexes and
a local `VecDeque<Work>`. It drains the local worklist to quiescence before
replying to a command. A cyclic graph of bounded channels is deliberately not
the baseline: an empty channel does not prove that no task has an in-flight
message.

A real multiworker cyclic dataflow would need explicit progress tracking, as
in Timely Dataflow, rather than this single-owner quiescence rule. DBSP is a
useful reference for the separate algebraic question of deriving incremental
operators; Tokio only supplies scheduling and communication.

`Seal(e)` is the barrier: the actor accepts no later epoch until its worklist
for `e` is empty, all prior `Changes` are sent, and `EpochComplete { epoch: e }`
has been emitted. Channel closure alone is never reported as fixed-point
completion.

An IVM continuation must demonstrate monotone epochs, duplicate suppression or
signed multiplicities, persistent state, a work-budget error, deletion/update
behavior, and replay equivalence against recomputation from scratch. Whether
that work can substitute for a later Datalog/semi-naive stage is a grading
decision to make later; completion of the async channel lowering alone is not
enough.

## Track B: parallel physical lowering with Rayon

Parallelism is not another spelling of pull or push. It changes work
partitioning, storage construction, sharing, and result ownership. Therefore
this branch starts at IndexRequirements, before the sequential physical pass
has selected a single mutable `BTreeSet` and sequential builders:

```text
RelationalPlan -> IndexRequirements(RelationalPlan)
                    |-- sequential -> staged Rust with RustAccessPlan
                    |                  |-- pull -> IteratorPipeline -> Rust Iterator
                    |                  `-- push -> RustCallbackNest -> fused callback Rust
                    `-- parallel   -> RayonFoldPlan -> Rayon Rust
```

Staff supplies a separate route, the Rayon dependency and Cargo harness, and
the following local IR:

```text
Plan ::= "accumulator" Ident "=" "(" RustExpr ")" ";"
         "merge" "=" "(" RustExpr ")" ";"
         Sink ":-" Driver ("," Clause)* ";"

Driver ::= BodyItem "=>" "fold" RustPat "in" "(" RustExpr ")"
Clause ::= BodyItem "=>" "for" RustPat "in" "(" RustExpr ")"
         | BodyItem "=>" "if" "(" RustExpr ")"
Sink ::= Atom "=>" RustBlock
```

The triangle residual makes the parallel ownership decision visible:

```rust
::mini_linq::rayon_fold_plan! {
    accumulator local_result =
        (|| ::std::vec::Vec::<(
            ::std::primitive::i32,
            ::std::primitive::i32,
            ::std::primitive::i32,
        )>::new());
    merge =
        (|mut left, mut right| { left.append(&mut right); left });

    triangle(x, y, z) => { local_result.push((
        ::core::clone::Clone::clone(x),
        ::core::clone::Clone::clone(y),
        ::core::clone::Clone::clone(z),
    )); } :-
        R(x, y) => fold (x, y,) in (rows0.par_iter()),
        S(y, z) => for (z,) in (
            index0
                .get(&(::core::clone::Clone::clone(y),))
                .into_iter()
                .flatten()
        ),
        T(z, x) => if (index1.contains(&(
            ::core::clone::Clone::clone(z),
            ::core::clone::Clone::clone(x),
        )));
}
```

The target lowers to an outer Rayon `fold` with serial inner probes and a
Rayon `reduce`. Each task owns a private output `Vec`; no shared
`Mutex<BTreeSet<_>>` is allowed. The merged candidate vector is
`par_sort_unstable()` and `dedup()` before return, so output order and set
semantics do not depend on Rayon scheduling. The accumulator initializer is an
identity and the merge must be associative; Rayon does not promise a fixed
reduction order.

Rayon's driver and every inner probe borrow the shared completed storage. Each
task constructs owned rows only when pushing successful heads into its private
accumulator; neither `par_iter` nor the serial bucket traversal clones a stored
tuple wholesale.

The physical pass must also emit different builders. A normalized relation
needed by any scan or proper-key builder is materialized once as a contiguous
`Vec<Row>` and reused by self-joins and multiple indexes. A relation used only
for full-row membership can stay in its normalized `HashSet`. A proper-key
index is built by folding task-local `HashMap<Key, Vec<Complement>>` values and
reducing them by merging equal-key buckets. Directly collecting repeated
`(key, value)` pairs into a `HashMap` is wrong because later equal keys replace
earlier values. Query workers share all completed indexes read-only.

Student-owned functions are expected to be:

```rust
fn compile_index_requirements_parallel(
    source: &index_requirements::Module,
) -> syn::Result<syn::File>;

fn compile_rayon_fold_plan(
    source: &rayon_fold_plan::Plan,
) -> syn::Result<syn::Expr>;
```

Staff supplies the explicit route macro, IR/checker, contracts, complete residual
fixtures, sequential differential oracle, custom Rayon pools, and benchmark
shell. Tests compare exact results at one, two, and four threads and cover
duplicates, projection, misses, self-joins, shared and extra indexes,
non-leading and multi-column keys, skew, and repeated runs. Pool construction
and warm-up stay outside the timed `Program::run` call.

This path is naturally expressed with map/fold/reduce, but that does not make
it the same assignment as async push. The first teaches partition-local state
and associative merging; the second teaches producer/consumer protocol,
backpressure, and termination.

## Track C: several queries with one exact global index set

This option extends the existing CQ program rather than creating a batch-query
wrapper. It supports several independent named CQs over the same declared
inputs:

```text
pub struct SharedGraph;
relation Seed(c0: i32);
relation Edge(c0: i32, c1: i32);

query outgoing(x, z) :- Seed(x), Edge(x, z);
query incoming(x, z) :- Seed(x), Edge(z, x);
query two_hop(x, y, z) :- Edge(x, y), Edge(y, z);
```

The explicit `query` keyword makes the repeated grammar unambiguous. Query
head names must be distinct after raw-identifier normalization. Each query has
its own initially empty variable-binding environment, and a query head cannot
be consumed by another query in this option. This is index sharing across
independent CQs, not recursion, rule union, or Datalog evaluation.

Basic HW1 produces one named RelationalPlan per query. DAG references such as
`r0` are local to that query plan and may be reused in the next plan; they do
not identify shared physical storage or denote algebra operators. Index sharing is derived only in
the next whole-program stage.

Canonical requirements are a stable global first-use union:

```text
seen = {}
for query in source order:
    visit its RelationalPlan definitions in dependency order
    for each supported NaturalJoin with a right-hand Rename:
        key = heading intersection inverted through that Rename
        if key is nonempty and (relation, key) is globally new:
            append (relation, key)
```

The example requires exactly:

```text
indexes {
    Edge[0];
    Edge[1];
}
```

`Edge[0]` used again by `two_hop` is not repeated. The contract is strict:
missing, extra, duplicate, or reordered requirements fail. “Redundant” means
the same normalized relation and the same ordered key columns. `Edge[0]` does
not subsume `Edge[0, 1]`; those are different physical requirements.
This is exact shared-index reuse, in the same broad family as shared dataflow
[arrangements](https://www.vldb.org/pvldb/vol13/p1793-mcsherry.pdf), not a claim
that one arbitrary access path can answer another.

The physical target normalizes every globally used input once, builds one rows
object for each globally enumerated relation and exactly one index for each
global **proper** key, then reuses those Rust locals in every query. A full-row
membership key uses the already-normalized `relationN: HashSet<Row>` directly;
it must not build a second identical `HashSet`. Exactly one final physical
consumer moves each normalized relation, while any earlier genuinely distinct
builder borrows it; a declared input unused by every query and written extra
requirement is not iterated. It emits one query-local
`pull! { rust_access_plan::Plan }` per query and returns a source-order result
tuple from a single convenience `run` call:

```rust
pub trait SharedGraphApi {
    fn run(
        input0: impl ::std::iter::IntoIterator<Item = (::std::primitive::i32,)>,
        input1: impl ::std::iter::IntoIterator<
            Item = (::std::primitive::i32, ::std::primitive::i32,),
        >,
    ) -> (
        ::std::vec::Vec<(::std::primitive::i32, ::std::primitive::i32,)>,
        ::std::vec::Vec<(::std::primitive::i32, ::std::primitive::i32,)>,
        ::std::vec::Vec<(
            ::std::primitive::i32,
            ::std::primitive::i32,
            ::std::primitive::i32,
        )>,
    );
}
```

There is no `BatchPlan`: each named RelationalPlan and each
occurrence-annotated RustAccessPlan remains local to one query, and each
physical expansion is enclosed in its own Rust block. The global sharing facts
live in the extended Program, the collection of query-local relational plans,
canonical IndexRequirements, and the complete Rust file.

Staff supplies the extended syntax/checker, explicit route macro, exact global
contract, and shared-builder harness. Students implement query-local logical
lowering, the global requirement walk, and physical lowering. Tests cover
first-use order across queries, query-local `rN` namespaces, raw identifiers,
reuse within and across queries,
different legal keys for one relation, missing/extra/reordered rejection,
exactly one normalizer and builder per global signature, self-joins, different
output arities, full-row reuse without a redundant set, and generated-name
collisions.

## Baseline typed relation contract

Concrete Rust column types are now part of the required baseline, not an
optional project track. They add no new IR. The existing relation declaration
carries actual `syn::Type` values:

```text
pub struct PurchaseQuery;
relation Person(c0: UserId, c1: String);
relation Purchase(c0: UserId, c1: u64);

purchase_name(person, name, amount) :-
    Person(person, name), Purchase(person, amount);
```

The syntax object keeps a parenthesized `CommaList<ColumnDecl>` whose entries
pair a column name with a `syn::Type`; arity is its length. Atom syntax and
numeric IndexRequirements do not change. The
caller-authored type syntax and spans flow through Program and into generated
Rust. Paths such as `UserId` or `String` resolve at the invocation site. There
is no `TypedProgram`, `ColumnType`, or row-layout descriptor.

Heterogeneous rows, keys, complements, patterns, and outputs use
always-trailing Rust tuples:

```rust
pub fn run(
    input0: impl ::std::iter::IntoIterator<Item = (UserId, String,)>,
    input1: impl ::std::iter::IntoIterator<Item = (UserId, ::std::primitive::u64,)>,
) -> ::std::vec::Vec<(UserId, String, ::std::primitive::u64,)>
```

Physical leaves use tuple patterns over borrowed `.iter()` scans and borrowed
`.flatten()` lookup buckets; they do not apply `.cloned()` to the candidate
stream. Bound positive fields are references. Lookup and membership keys clone
only fields needed to construct an owned key, and the head clones projected
fields to construct its owned result. Key and complement fields remain in
relation-column order. If lazy distinctness returns an owned result while also
retaining it in `seen`, that result must be cloned once at this explicit
result-ownership boundary.

Trait obligations come from the actual generated operations:

| Operation | Rust obligation |
|---|---|
| normalize a row in `HashSet` | row fields satisfy lawful `Eq + Hash` |
| derive an additional rows object or index before the moving final consumer | the cloned fields satisfy `Clone` |
| construct lookup keys and owned head rows from borrowed bindings | the selected fields satisfy `Clone` |
| retain an owned result for lazy distinctness | head fields satisfy `Clone + Eq + Hash` |
| insert the deterministic result in `BTreeSet` | head fields satisfy `Ord` |
| compare a bound value with an indexed column | the Rust types are equal |

The proc macro does not compare `syn::Type` syntax to decide semantic type
equality: aliases and differently qualified paths may denote the same type.
Generated Rust expressions let rustc perform name resolution, trait checking,
and join-type checking.

The baseline scope is owned, concrete Rust types available at the macro call
site, with relation arity from one through twelve. Generic schema binders,
unbound lifetimes, unsized types, implicit coercion rules, custom collations,
and floats without a lawful equality/hash wrapper are out of scope. An async
or parallel track preserves these real column types but adds its own `Send` or
`Sync` obligations. Generated standard-library paths stay absolute, caller
type tokens preserve their spans, and internal identifiers use mixed-site
spans because procedural macro output is otherwise unhygienic.

The staff checkpoint supplies the grammar/checker, compile-pass and compile-fail
harnesses, and concrete examples. Required regression tests include `String`,
`u64`, a derived user type,
one-column tuples, a multi-column key, type aliases with different spellings,
non-`Copy` data, duplicate input set semantics, arity twelve success, arity
thirteen rejection, and rustc failures for incompatible join types or missing
`Eq`, `Hash`, `Clone`, or output `Ord`.

## Evidence and release checklist

Before any track is offered, its staff checkpoint must include a completed
reference solution and prove that the student work is satisfiable. The
student-facing documentation must show, for one complete query program:

1. the source CQ program;
2. the exact predecessor IR;
3. the complete staged Rust file containing the new local IR;
4. the complete next residual after the student's pass; and
5. final compiling Rust.

The tests must distinguish local well-formedness from cross-stage contracts,
compile the generated Rust with its real Cargo dependencies, execute semantic
fixtures, and include at least one adversarial hygiene case. Benchmarks measure
whole public query calls externally unless a track explicitly changes the
baseline's visible `build`/`query`/materialization boundary or adds an
instrumentation pass.

Useful implementation references include Tokio's bounded
[`mpsc`](https://docs.rs/tokio/latest/tokio/sync/mpsc/index.html), Rayon’s
[`ParallelIterator`](https://docs.rs/rayon/latest/rayon/iter/trait.ParallelIterator.html),
Rust’s [procedural-macro hygiene
rules](https://doc.rust-lang.org/stable/reference/procedural-macros.html#procedural-macro-hygiene),
the standard [tuple trait
limits](https://doc.rust-lang.org/std/primitive.tuple.html), HyPer’s
[produce/consume compilation
model](https://www.vldb.org/pvldb/vol4/p539-neumann.pdf), and shared dataflow
[arrangements](https://www.vldb.org/pvldb/vol13/p1793-mcsherry.pdf). The IVM
continuation should also be compared with Timely's [progress
tracking](https://timelydataflow.github.io/timely-dataflow/chapter_5/chapter_5_2.html)
and the [DBSP paper](https://arxiv.org/abs/2203.16684).
