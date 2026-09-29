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

The current baseline is a genuine pull path. `structures_to_rust!` prints
one Rust block expression over the `cq-storage` traits, evaluated in the
caller's scope over caller-owned host values: it builds the chosen structures,
then evaluates to one iterator of answer tuples that embodies the entire
query execution. Each client call to `next` pulls one binding through the
operator chain; intermediate `pipe` results are streamed, not collected as
relations. A program over hosts alone evaluates to the lazy iterator itself;
one that builds structures collects its answers into a `Vec` inside the block,
since the structures live only for the block. Rust may inline the emitted
combinators, but operator fusion is not a requirement of this baseline.

The synchronous push warm-up below is part of the required execution-control
core. Tokio channels and the IVM continuation are separate pick-one options.
Push branches from the bindings Γ (the Structures program) rather than
translating the pull-specific printed program. Async tasks and channels are a
second push realization, not a pull variant.

## Route graph and configuration rule

The required baseline is one explicit chain of complete programs:

```text
CQ
  -- Basic HW1 (normalize, order, elaborate) --> RA     // relational { rN = operator; … output … }
  -- Basic HW2 --> IX                                    // RA + indexes { iN = @rK[cols]; }
  -- Basic HW3 --> Physical                              // standalone: mode and operator per result
  -- supplied selection --> Structures                   // Physical + structures { … } (bindings Γ)
  -- supplied printer --> one Rust block expression
```

The negation and aggregation option extends these languages rather than
bypassing the logical layer:

```text
Extended CQ with ! and agg
  --> Extended RA with antisemijoin and aggregate_apply
  --> Extended IX over that entire RA program
  --> Extended Physical and Structures
  --> the supplied printer
```

RA already supplies the singleton zero-attribute source `r0 = unit;`. HW4 uses
it when global negation or aggregation appears before any positive binding.
An empty correlation is still an explicit exact-result `[]` demand when it
feeds a keyed operator. The narrow Basic-HW2/3 assignment does not require
Unit; global negation and aggregation still require their own logical and
physical lowering.

The negation and aggregation option is cumulative: students edit the production grammar and the same
typed passes, completed extended queries continue to use `mini_linq!`, and the
`mini-linq-course` `hw4` feature gates only extension-specific test targets. It does not select a backend or
swap macro semantics. A backend alternative such as push, Tokio, or Rayon gets
a separately named route macro and typed target when it is implemented. A
Cargo feature may make that separately named route's dependencies and tests
available; it must not reinterpret an existing macro. Separate checkpoint
branches are the default packaging mechanism when two options change
overlapping decisions.

The extension families are intentionally separate at first:

| Track | Decision it exposes | Correct branch point |
|---|---|---|
| Push and async messages | who drives execution and how intermediate bindings move | Γ |
| Parallel Rayon execution | work partitioning, parallel builders, and result ownership | IX, before sequential physical lowering |
| Multiple queries | index sharing across independent query bodies | CQ, RA, and IX |

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
| Rayon + typed baseline | Rayon adds `Send + Sync` to the baseline's operation-local trait obligations | The parallel checkpoint must state those extra bounds |
| multiple queries + typed baseline | Shared declarations must retain one exact ordered type list across all queries | The multiple-query checker owns this validation |

## What staff must provide for every track

Students should transform programs, not reverse-engineer assignment
infrastructure. Every released track therefore includes:

1. the complete target syntax object when a new IR is justified;
2. its parser, token emission, span-aware local checker, and public macro;
3. the surrounding Rust API/runtime wiring and Cargo dependencies;
4. one complete triangle-like program at every boundary, not an ellipsis or a
   fragment detached from its enclosing caller-written host function;
5. either a supplied emitter from the last IR to ordinary Rust or, when that
   emitter is the exercise, exact target fixtures plus a post-deadline
   reference implementation;
6. structural, compile, runtime, differential, and error fixtures; and
7. numbered comments at every student-owned function or match arm.

Any added public macro follows the basic typed boundary directly: its `#[proc_macro]`
entry parses the invocation into the typed source and calls the corresponding
typed `compile` function. Grammar failures become span-aware `syn::Error`
diagnostics emitted as `compile_error!`. There is no separate public expansion
wrapper. `compile` checks its source's `contract::check` before lowering and
the target's contract before returning; semantically invalid input returns a
span-aware `syn::Error` in both debug and release builds. The macro entry
forwards this error as `compile_error!` without duplicating the check. Each
pass also exposes `check(&source, &target, options)`, an independent
correspondence checker that callers and tests invoke explicitly; `compile`
does not run it. The printer has no `check`; its evidence is oracle tests,
`cq_storage::require` trait guards, and expansion fixtures. Tests and tools
can also call `contract::check` directly.

The terminology is precise:

- CQ, VO, RA, IX, Physical, and Structures name IR languages
  (`cq::Program`, …, `structures::Program`). Each is one complete program;
  the `structures { … }` block of a Structures program records the bindings Γ.
- The Rust block printed by `structures_to_rust!` is the **printed pull
  program**.
- `structures_to_rust`, `push`, and `push_tokio` name transformations. They are not names for
  an otherwise unspecified query IR.
- The printed pull program is the concrete target of `structures_to_rust`; callback and
  scheduled push transformations have different targets.

## Track A: push execution, Tokio channels, and an IVM continuation

### A1. Synchronous push warm-up

The smallest honest comparison with pull is a callback pipeline. Staff
supplies the following target language and its complete host shell:

```text
RustCallbackNest ::= "for_each" RustPat "in" "(" RustExpr ")" ";" RustCallbackNest
                   | "guard" "(" RustExpr ")" ";" RustCallbackNest
                   | "emit" "(" RustExpr ")" ";"
```

Every syntax object has a direct meaning: `for_each` invokes its continuation
for each source item, `guard` invokes it conditionally, and `emit` calls the
host's consumer with one output row. The student implements only:

```rust
fn compile_push(
    source: &structures::Program,
) -> syn::Result<rust_callback_nest::Nest>;
```

For the triangle (`answer(a, b, c) :- R(a, b), S(b, c), T(c, a)`, with the
baseline selection binding `r1 = HashIndex [b]` and `r3 = HashIndex [a, c]`),
the complete local result is:

```rust
::mini_linq::rust_callback_nest! {
    for_each (a, b,) in (R.iter());
    for_each (c,) in (::cq_storage::IndexRead::lookup(&r1, &(b,)));
    guard (::cq_storage::ExactMembership::contains(&r3, &(a, c,)));
    emit ((a, b, c,));
}
```

The supplied emitter builds the selected structures exactly as the pull
printer does, then places the nest in a caller-written host function that
receives `mut consume: impl FnMut(Row)`. It is structural:

```rust
::core::iter::Iterator::for_each(R.iter(), |(a, b,)| {
    ::core::iter::Iterator::for_each(
        ::cq_storage::IndexRead::lookup(&r1, &(b,)),
        |(c,)| {
            if ::cq_storage::ExactMembership::contains(&r3, &(a, c,)) {
                consume((a, b, c,));
            }
        },
    );
});
```

This is fused synchronous push through callbacks. Unlike the baseline
printed pull program, its final representation deliberately erases the operator
stream boundary into nested producer/consumer callbacks. It has no queue,
backpressure, lazily returned result iterator, or parallelism. At the
query-operator boundary each producer invokes its continuation; a leaf source
may still use a Rust iterator internally. A materializer passes a closure that
inserts emitted rows; the pipeline itself never exposes `next`.
Both callbacks borrow their tuples, so bound fields stay `&T` as in the
baseline; the traversal clones nothing.
Because closure lowering changes the meaning of nonlocal control, the supplied
checker rejects unsupported nonlocal control in embedded expressions.

This fused callback path is the closest course analogue of HyPer's
[data-centric produce/consume
compilation](https://www.vldb.org/pvldb/vol4/p539-neumann.pdf). The callback
forms are compile-time interfaces used to create one fused pipeline; they are
not runtime channels.

### A2. Advanced Tokio channel lowering

Async message passing is a distinct, harder push target. It also branches from
Γ; the physical operators and their heading columns make it possible
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
    for (a, b,) in (R.iter());
    send (::core::clone::Clone::clone(a), ::core::clone::Clone::clone(b),);
}

stage probe_s rx0 -> tx1 {
    recv (a, b,);
    for (c,) in (::cq_storage::IndexRead::lookup(&r1, &(&b,)));
    send (a, b, ::core::clone::Clone::clone(c),);
}

guard_stage check_t rx1 -> tx2 {
    recv (a, b, c,);
    if (::cq_storage::ExactMembership::contains(&r3, &(&a, &c,)));
    send (a, b, c,);
}

sink collect <- rx2 {
    recv (a, b, c,);
    { result.push((a, b, c,)); }
}

schedule try_join [scan_r, probe_s, check_t, collect];
```

This target already decides the stage graph, message schemas, channel
capacity, concrete Rust operations, and schedule. The student does not design
Tokio plumbing. The only student transformation is:

```rust
fn compile_push_tokio(
    source: &structures::Program,
) -> syn::Result<tokio_channel_plan::Plan>;
```

The scan and lookup still borrow their stored tuples. A `send`, however, is an
explicit ownership boundary: it constructs an owned channel message by
cloning the borrowed fields. Later `recv` bindings are owned; lookup and
membership keys borrow them. This channel cost is part of the chosen push
protocol, not an implicit `.cloned()` traversal of storage.

The supplied host shell is an `async fn` over caller-owned hosts returning
`Result<Vec<Row>, ChannelError>`. It uses bounded `tokio::sync::mpsc::channel(capacity)`,
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
the host future cancels the query. No partial-result guarantee is made after
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
this branch starts at IX, before sequential physical lowering chooses modes,
operators, and sequential builders. The baseline distinct-state provider
remains caller-bound; the parallel route defines its own reduction instead:

```text
RA -> IX
       |-- sequential -> Physical -> Structures (Γ)
       |                              |-- pull -> printed Rust block
       |                              `-- push -> RustCallbackNest -> fused callback Rust
       `-- parallel   -> RayonFoldPlan -> Rayon Rust
```

Staff supplies a separate route, the Rayon dependency and Cargo harness, and
the following local IR:

```text
Plan ::= "accumulator" Ident "=" "(" RustExpr ")" ";"
         "merge" "=" "(" RustExpr ")" ";"
         Sink ":-" Driver ("," Clause)* ";"

Driver ::= BodyItem "=>" "fold" RustPat "in" "(" RustExpr ")"
Clause ::= BodyItem "=>" "bind" RustPat "<-" "(" RustExpr ")"
         | BodyItem "=>" "if" "(" RustExpr ")"
Sink ::= Atom "=>" RustBlock
```

The triangle residual makes the parallel ownership decision visible (`r1` and
`r3` are the parallel-built indexes keyed as in the baseline):

```rust
::mini_linq::rayon_fold_plan! {
    accumulator local_result = (|| ::std::vec::Vec::new());
    merge = (|mut left, mut right| { left.append(&mut right); left });

    answer(a, b, c) => { local_result.push((a, b, c,)); } :-
        R(a, b) => fold (a, b,) in (R.par_iter()),
        S(b, c) => bind (c,) <- (::cq_storage::IndexRead::lookup(&r1, &(b,))),
        T(c, a) => if (::cq_storage::ExactMembership::contains(&r3, &(a, c,)));
}
```

The target lowers to an outer Rayon `fold` with serial inner probes and a
Rayon `reduce`. Each task owns a private output `Vec`; no shared
`Mutex<_>` state is allowed. The merged candidate vector is
`par_sort_unstable()` and `dedup()` before return, so output order and set
semantics do not depend on Rayon scheduling. The accumulator initializer is an
identity and the merge must be associative; Rayon does not promise a fixed
reduction order.

Rayon's driver and every inner probe borrow the host values and the shared
completed structures. As in the baseline, bound fields are `&T`; nothing is
cloned to build a result row.

The parallel route must also emit different builders. A keyed index is built
by folding task-local `HashMap<Key, Vec<Payload>>` values and reducing them by
merging equal-key buckets. Directly collecting repeated `(key, value)` pairs
into a `HashMap` is wrong because later equal keys replace earlier values.
Query workers share all completed structures read-only.

Student-owned functions are expected to be:

```rust
fn compile_parallel(
    source: &ix::Program,
) -> syn::Result<rayon_fold_plan::Plan>;

fn compile_rayon_fold_plan(
    source: &rayon_fold_plan::Plan,
) -> syn::Result<syn::Expr>;
```

Staff supplies the explicit route macro, IR/checker, contracts, complete residual
fixtures, sequential differential oracle, custom Rayon pools, and benchmark
shell. Tests compare exact results at one, two, and four threads and cover
duplicates, projection, misses, self-joins, shared and extra indexes,
non-leading and multi-column keys, skew, and repeated runs. Pool construction
and warm-up stay outside the timed query call.

This path is naturally expressed with map/fold/reduce, but that does not make
it the same assignment as async push. The first teaches partition-local state
and associative merging; the second teaches producer/consumer protocol,
backpressure, and termination.

## Track C: several queries sharing exact index demands

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

Basic HW1 produces one complete RA program per query. DAG references such as
`r0` are local to that query and may be reused in the next one; they do not
identify shared physical storage. Basic HW2 then runs unchanged per query, so
every demand keeps its baseline identity: an exact query-local RA result plus
sorted-heading column ordinals, qualified by its query, with `[]` recorded
when the correlation is empty. For the example (written join order):

```text
outgoing: indexes { i0 = @r1[0]; }   // r1 = rename Edge { c0 -> x, c1 -> z }; key x = Edge.c0
incoming: indexes { i0 = @r1[0]; }   // r1 = rename Edge { c0 -> z, c1 -> x }; key x = Edge.c1
two_hop:  indexes { i0 = @r1[0]; }   // r1 = rename Edge { c0 -> y, c1 -> z }; key y = Edge.c0
```

Cross-query sharing is not obtained by replacing these exact results with
source-relation names such as `Edge[0]`. The proposed design adds a separate
whole-program sharing stage: it partitions the query-qualified demands into
classes whose results are provably the same keyed relation up to attribute
renaming (here `outgoing.i0` and `two_hop.i0`, but not `incoming.i0`), in
stable first-use order. Its checker rejects missing, extra, duplicate, or
reordered classes and any class whose members are not equivalent. An index on
one key does not subsume another; these classes permit exact index reuse, in
the same broad family as shared dataflow
[arrangements](https://www.vldb.org/pvldb/vol13/p1793-mcsherry.pdf), not a
claim that one arbitrary access path can answer another.

Structure selection then builds exactly one structure per sharing class and
prints one query-local Rust block per query that reads the shared locals. As
in the baseline, the caller writes the host function: its signature, the
borrowed host values, and how the per-query iterators are returned. There is
no `BatchPlan` and no generated `run` API; each query's RA, IX, Physical, and
Structures programs stay local, and the global facts live in the extended CQ
program and the sharing stage.

Staff supplies the extended syntax/checker, explicit route macro, sharing
contract, and shared-builder harness. Students implement per-query lowering,
the sharing walk, and physical lowering. Tests cover first-use order across
queries, query-local `rN` namespaces, raw identifiers, reuse within and across
queries, different keys for one relation, missing/extra/reordered rejection,
exactly one builder per sharing class, self-joins, different output arities,
and generated-name collisions.

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
IX column ordinals do not change. The
caller-authored type syntax and spans flow through Program and into generated
Rust. Paths such as `UserId` or `String` resolve at the invocation site. There
is no `TypedProgram`, `ColumnType`, or row-layout descriptor.

Heterogeneous rows, keys, payloads, patterns, and outputs use always-trailing
Rust tuples. The caller writes the host function and its signature; the
printed block is evaluated inside it over the caller's borrowed hosts:

```rust
#[allow(non_snake_case)]
fn purchase_names<'a>(
    Person: &'a [(UserId, String)],
    Purchase: &'a [(UserId, u64)],
) -> impl Iterator<Item = (&'a UserId, &'a String, &'a u64)> + 'a {
    /* the route macro invocation over this CQ program */
}
```

Host scans and lookup buckets are borrowed; the printer never applies
`.cloned()` to a candidate stream. Host columns flow as `&T`, and bare head
variables stay borrowed. Authored computed head expressions decide their own
ownership; the printer adds no implicit clone. The only printer-introduced
clone is of bound columns needed to probe a host map whose key type is owned.

Trait obligations are operation-local; each comes from an operation the
selected program actually performs:

| Operation | Rust obligation |
|---|---|
| scan or probe a host | the host supports the read its declaration states (`.iter()`, or `cq_storage::IndexRead` for a keyed host map) |
| probe a host map with owned keys | the bound key fields satisfy `Clone` |
| build and read a selected structure | its type implements `cq_storage::Build` and the access traits used; `cq_storage::require` guards name the structure on failure |
| `canon` distinctness | whatever the caller-bound `distinct_state!` provider needs (e.g. `hash_distinct`: `Eq + Hash` rows; `ordered_distinct`: `Ord`) |
| compute a head expression | whatever that authored expression requires |
| compare a bound value with an indexed column | the Rust types are equal |

The proc macro does not compare `syn::Type` syntax to decide semantic type
equality: aliases and differently qualified paths may denote the same type.
Generated Rust expressions let rustc perform name resolution, trait checking,
and join-type checking.

The baseline scope is concrete Rust types available at the macro call site.
The language requires positive arity and unique column names, with no upper
bound; the generated Rust and the trait support of the types involved decide
which tuples are usable. Generic schema binders, unbound lifetimes, unsized
types, implicit coercion rules, custom collations, and floats without a lawful
equality/hash wrapper are out of scope. An async or parallel track preserves
these real column types but adds its own `Send` or `Sync` obligations.
Generated standard-library paths stay absolute, caller type tokens preserve
their spans, and internal identifiers use mixed-site spans because procedural
macro output is otherwise unhygienic.

The staff checkpoint supplies the grammar/checker, compile-pass and compile-fail
harnesses, and concrete examples. Required regression tests include `String`,
`u64`, a derived user type, one-column tuples, a multi-column key, type
aliases with different spellings, non-`Copy` data, duplicate host rows, and
rustc failures for incompatible join types or for a trait missing from an
operation the program actually performs.

## Evidence and release checklist

Before any track is offered, its staff checkpoint must include a completed
reference solution and prove that the student work is satisfiable. The
student-facing documentation must show, for one complete query program:

1. the source CQ program;
2. the exact predecessor IR;
3. the complete staged Rust file containing the new local IR;
4. the complete next residual after the student's pass; and
5. final compiling Rust.

The tests must distinguish language contracts (`contract::check`) from pass
correspondence (`check(&source, &target, options)`), compile the generated
Rust with its real Cargo dependencies, execute semantic fixtures, and include
at least one adversarial hygiene case. Benchmarks measure whole caller-written
host calls externally unless a track explicitly changes that boundary or adds
an instrumentation pass.

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
