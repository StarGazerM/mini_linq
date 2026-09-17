# Borrowed execution through the existing compiler stages

The `ra-execution` example compiles one `Star` query through the same stage APIs
as the other indexed routes. It selects an empty index policy and a borrowed
scan realization. All query iterator operators are emitted by the shared
Pipeline compiler. Input access and replay storage remain Native operations.

```text
query!                 CQ → RA
choose_indexes!        RA → IX with indexes {}
choose_realization!    IX → Rust containing Access and Native calls
  replay_rows!         Native Build → storage construction around Access
  pull_stream!         Access → Pipeline → stream_pipeline! → support iterator
  pull_exists!         Access → Pipeline → exists_pipeline! → first-witness bool
  pull!                Access → Pipeline → iterator_pipeline! → distinct iterator
```

The indented entries are local macro invocations inside the physical output.
They are not extra whole-query languages. Native Scan expands to an owning
replay cursor; input Type, Scan and Field expand through each relation's provider.

## API names and selected implementations

| Caller-facing binding | Selected implementation | Exact boundary |
| --- | --- | --- |
| `mini_linq::query` | `graph_query` | CQ → planned named RA |
| `mini_linq::choose_indexes` | `no_indexes` | RA → IX, preserving RA and writing no demands |
| `mini_linq::choose_realization` | `borrowed_scans` | IX → staged Rust with Access/Native holes |
| `mini_linq::inputs::Facts`, `Tags` | `tuple_hash_rows` | Native Type/Scan/Field on caller-owned tuple sets |
| `mini_linq::inputs::Allowed` | `tuple_sorted_rows` | Same operations on a caller-owned sorted set |
| `mini_linq::pull_stream` | `pull_stream` | Access → Pipeline, selecting `stream_pipeline` |
| `mini_linq::pull_exists` | `pull_exists` | Access → Pipeline, selecting `exists_pipeline` |
| `mini_linq::pull` | `pull` | Access → Pipeline, selecting `iterator_pipeline` |
| `mini_linq::replay_rows` | `replay_rows` | Native Build/Scan → selected storage traits |
| `mini_linq::storage` | `cq_storage` | `BuildReplay`, `ReplayRead`, `Replay` |

These bindings are ordinary Rust imports. The example and selected feature list
are in [main.rs](../examples/ra-execution/main.rs) and
[the frontend manifest](../crates/mini-linq/Cargo.toml). No helper macro changes a
stage's input language. The planner's `choose_indexes` continuation now invokes
an actual RA-to-IX pass.

The frontend's replay defaults are symbolic paths `mini_linq::storage` and
`mini_linq::pull_stream`. Existing Native administrative `storage`/`iterator`
options can override those paths. Typed compiler clients supply explicit macro
paths in `borrowed_scans::Options`; the implementation never calls another
compiler family directly.

## Input domain and borrowed API

`borrowed_scans` admits a checked IX module with an empty overlay, and RA
Unit, Rename, Project, NaturalJoin and Semijoin. Output arguments must be bound
variables; reordering and repetition are allowed. Select, Extend, computed
outputs, invalid provider bindings and nonempty index requirements are rejected.
It does not erase written demands or silently choose a different representation.

For the example, the generated API has borrowed input parameters and returns
`impl Iterator<Item = (&String, &i32)>` tied to their lifetime. Input providers
must implement Native Type, Scan and Field and expose stable fields in
caller-owned storage. Rust rejects a field reference into temporary traversal
state. Tuple hash/sorted providers satisfy this contract; choosing a container
does not turn the selected nested matching into hash or merge join.

The physical pass constructs internal logical rows as tuples of field
references in sorted heading order. Projection, tuple retention and replay copy
those references. They do not clone payloads. Matching shared fields requires
equality, and the final distinct tuple requires `Eq` and `Hash` for its referenced
values. Intermediate support streams add no payload `Clone`, `Eq` or `Hash`
requirement beyond operations that actually inspect those fields. Input storage
can impose its own separate requirements.

## Operator-to-region rules

The following table describes emitted Access regions; it is not a substitute
for a captured macro expansion.

| RA operator | Access region | Observation |
| --- | --- | --- |
| Unit | Empty body, output `()` | One empty binding |
| Rename | One Native input scan, output borrowed fields in result order | Support stream |
| Project | One predecessor scan, output selected reference coordinates | Support stream |
| NaturalJoin | Two predecessor scans followed by shared-coordinate equality | Support stream of merged reference tuples |
| Semijoin | Left scan and an `if` containing a right-side Access region | `pull_exists` stops at the first compatible right row |
| Output | One result scan and head-order projection | Distinct answer iterator |

Each region is parsed as `cq_ir::rust_access_plan::Plan` and checked before
emission. It then goes through `cq_execution::pull::compile` to the existing
linear `IteratorPipeline`. The shared operator emitter supplies all `map`,
`flat_map`, filtering and projection code. The existence endpoint omits only the
terminal Distinct, retains every preceding operator, and polls once for a witness.
It returns a boolean rather than a stream or collected set.

Two small changes belong to the existing Access contract:

- The body can be empty: `out() => (()) :- ;` denotes one empty binding.
- When a positive occurrence introduces exactly one fresh variable, `for row in
  (...)` may use that identifier. Tuple patterns still bind exactly the fresh
  variables in occurrence order. This avoids an artificial row-wrapping `map`.

Both execution implementations and Pipeline already support the corresponding
Unit and identifier-binding forms. The parser, validity rules, fixtures and
teaching grammar now agree on them.

## Sharing and storage

Let `uses(r)` count direct operand occurrences plus the output occurrence. The
physical implementation retains `r` when `uses(r) > 1` or `r` is a join/semijoin
right operand. The latter can need many cursors even with one written use.
Definitions are emitted once in checked topological order. Other streams move
to their sole consumer.

A retained definition emits Native `build { AccessPlan }`. `replay_rows` preserves
the Access quotation and constructs storage through `BuildReplay::replay` after
the selected Access/Pipeline stages have supplied its lazy iterator. Native
`scan(value)` calls `ReplayRead::cursor(&value)`. This uses existing Native
Build/Scan meanings; it does not repurpose Materialize or add an operation.

The storage interface is deliberately small:

```rust
trait ReplayRead: Clone {
    type Item: Copy;
    type Cursor: Iterator<Item = Self::Item>;
    fn cursor(&self) -> Self::Cursor;
}
trait BuildReplay<I>: ReplayRead<Item = I::Item>
where
    I: Iterator,
    I::Item: Copy,
{
    fn replay(source: I) -> Self;
}
```

The concrete data structure privately retains an optional source iterator and
a produced prefix in shared `Rc<RefCell<_>>` state. Each cursor owns a share and
an independent position. Handles clone the `Rc`, without cloning the source or
payload. A cache hit returns a copied reference tuple; a frontier request polls
the source once and appends its next item. First exhaustion drops the source
and permanently stops its polling. Cached occurrences remain available.

The invariant is: the cache is exactly the prefix produced by the one upstream
enumeration, and each cursor has observed the prefix up to its own position.
Hit, append and exhaustion preserve it. With stable finite input values and
lawful comparisons, topological induction over the local Access regions gives
the same relational support; final distinctness yields the RA answer set.
This does not assert equality of scan counters or arbitrary side-effect traces.

Dependencies point to earlier RA definitions. Owning handles therefore form no
cycle, and a source poll cannot recursively borrow its own replay state. A
returned item borrows caller storage, not the mutable cache. It remains valid
after the cursor/cache is dropped while its input borrow remains valid.

## Demand, costs and limits

Constructing the query can allocate adapters and replay state, but evaluates no
provider Scan/Field expression. The support and iterator endpoints defer their
Access sources until a poll. `next`, `any` and `fold` execute query work without
collecting final answers. Semijoin existence stops after its first witness;
dropping a query schedules no further polls, although ordinary Rust destructors
can run.

Replay is incremental intermediate materialization. Prefixes can grow to full
enumerations, preserve duplicate occurrences and remain retained for later
cursors. The implementation performs no eviction and uses non-`Send` `Rc` state.
Nested matching still repeats comparisons over cached tuples. These are explicit
costs, not a claim of optimal YA execution, constant space or measured speedup.

The configured relation route still provides block pruning, key resources,
Select/Extend and complete pair algorithms. The hash routes still implement
their admitted index demands. The borrowed scan implementation does not replace
those distinct domains. Eager `push_set` remains an alternative at a compatible
set-valued Access hole; it is not a replacement for a lazy support contract.

## Reproduction and expansion review

```sh
cargo run -p mini-linq --no-default-features \
  --features graph,no-indexes,borrowed-scans,replay-rows,pull,stream-pipeline,exists-pipeline,iterator-pipeline,tuple-hash-rows,tuple-sorted-rows \
  --example ra-execution --locked

python3 tools/trace_macros.py --example ra-execution \
  --features graph,no-indexes,borrowed-scans,replay-rows,pull,stream-pipeline,exists-pipeline,iterator-pipeline,tuple-hash-rows,tuple-sorted-rows \
  --out target/staged-execution-review --declarative
```

The capture records each actual macro input and immediate output, preserving
Access and Pipeline invocations for inspection. Source fingerprints identify
the compiled workspace. Expansion counts are static sites, not runtime visits.
The generated local [annotated review](../target/staged-execution-review/review.md)
explains every captured call and its verified parent; [steps.md](../target/staged-execution-review/steps.md)
contains all complete inputs and immediate outputs for the single Star query.
The [validation snapshot](formal/snapshot.txt) records the completed gates;
the [formal rules](formal/staging.tex) describe replay and endpoint obligations.
