# Compiler structure

The unit of replacement is an implementation of a semantic macro boundary.
Crates group related implementations of that boundary; modules own individual
algorithms, their admitted domains, emitters, correspondence checks and tests.
A feature includes an implementation. A caller's Rust macro binding selects it.

The [boundary specification](IR-STAGE-REVIEW.md) defines the language graph,
public macro families and replacement laws. The [formal supplement](formal/README.md)
links inference rules to implementation markers.

## Package responsibilities

| Package | Modules / responsibility | Production workspace dependencies |
| --- | --- | --- |
| `cq-ir` | CQ, RA, equality demands, Access, Pipeline and Native syntax; local judgments and factual heading queries | None |
| `cq-storage` | Data structures and minimal operation traits; block selections, summaries and passive counters | None |
| `cq-plan` | `canonical`, `prefix`, `graph`; logical scheduling and derivations | `cq-ir` |
| `cq-index` | `input`, `result`; equality-demand policies | `cq-ir` |
| `cq-realize` | `hash_inputs`, `hash_prefix`, `hash_materialized`, `relations`, exploratory `nested_push` / `nested_pull`; whole-query realization and hosts | `cq-ir` |
| `cq-native` | `rows`, `candidates`, `keys`, `witnesses`, `pairs`; local Native operation implementations | `cq-ir` |
| `cq-execution` | `pull`, `push`; Access execution alternatives | `cq-ir` |
| `cq-pipeline` | `iterator`, `collect`, `stream`; Pipeline endpoints | `cq-ir` |
| `cq-compose` | Typed named compositions for tools and AST clients | Selected compiler families, `cq-ir` |
| `mini-linq` | Procedural-macro entries, invocation-scoped continuations and administrative configuration | Selected families and optional composition |

Implementation libraries are ordinary Rust crates because typed clients use
`compile`, `check`, `explain` and `replay`. The proc-macro crate holds exported
entries and calls those libraries. It owns no additional query language.
Runtime storage never invokes a compiler and contains no semijoin interpreter.
Generated Rust may use caller-selected storage paths without the emitting
compiler library linking that storage crate.

Combining row providers and access algorithms in `cq-native` reflects their
shared local-operation dialect. Their modules remain independent alternatives;
block rows do not imply pruning, zone keys or pull. Combining pull and push in
`cq-execution` makes their common Access input explicit without pretending their
results have identical demand contracts. Renaming the whole-query family to
`cq-realize` avoids implying that pull is somehow outside physical execution.

## Module ownership and features

```text
cq-realize/src/                    cq-native/src/
  hash_inputs/                      rows/{hash,sorted,block,tuple_hash,tuple_sorted}.rs
  hash_prefix/                      candidates/{scan,prune}.rs
  hash_materialized/                keys/{exact,zone}.rs
  nested_push.rs                    (independent whole-RA push experiment)
  nested_pull.rs                    (independent whole-RA pull experiment)
  nested_support.rs                 (domain checks and host syntax only)
  relations/                        witnesses/{hash,nested}.rs
    layout.rs                       pairs/{hash,nested,merge}.rs
    expressions.rs
    resources.rs                  cq-execution/src/
    lower.rs                        pull/{mod,check}.rs
    check.rs                        push.rs
    hosts/{plain,observed}.rs
```

Shared helpers express a common mechanism, not a global backend selector.
The nested experiments share validation and parameter/type spelling. Their
recursive execution lowering stays separate; neither calls the other's emitter.
For example, native set emission takes type and row representation syntax from
its owning implementation. It does not inspect a provider name and choose a
backend. Each realization checks its own index coverage; the IR describes facts
and validity, not preferred implementation decisions.

Production dependencies cannot connect implementation families or point back
at composition. Tests may compose families through development dependencies.
Cargo features are independent additive implementation switches. Frontend
`prune-rows` includes candidate pruning alone; its caller supplies row, key and
iterator implementations separately. A selected macro retains its meaning when
another implementation is enabled.

Cargo isolates crates, not sibling modules. The dependency guard therefore also
builds source closures with unused siblings physically absent. This establishes
buildability without turning every helper or algorithm into another package.
A module can become a package later when dependencies or deployment justify it;
its macro protocol need not change.

## Composition and decision ownership

The default route expands one caller-bound decision at a time:

```text
mini_linq → canonical_query or prefix_query
          → choose_indexes → input_indexes or result_indexes
          → choose_realization → hash_inputs, hash_prefix or hash_materialized
          → pull / pull_set → selected Pipeline endpoint
```

`optimized_query` chooses graph or prefix planning. It shares the same subsequent
bindings. The graph planner's semijoin RA reaches a compatible realization under
default wiring. Workload compilation enters this same macro route.

Each dispatcher examines its actual input. IX contains no hidden route tag.
Changing an index policy can therefore change which realization accepts the
result. Explicit implementation entries do not invoke default selection. A
caller can replace the entire RA-to-Rust edge with configured relation realization
or bind a smaller native operation inside that realization.

Typed `cq-compose` clients intentionally run a fixed Rust function composition;
they are a separate API from invocation-scoped macro composition. Its index and
realization selection functions expose the same domain decisions, while the
frontend emits calls rather than consuming later macro choices internally.

An eager recipe is a Rust facade:

```rust
mod eager {
    pub use ::mini_linq::{
        optimized_query, graph_query, prefix_query,
        result_indexes as choose_indexes,
        hash_materialized as choose_realization,
        pull_set, collect_pipeline,
    };
}
```

Use `eager as mini_linq` in the scope where that recipe is desired. A facade is
not a new IR or compiler pass. Mixed lazy/eager tests use separate scopes because
host-expression effects can differ between the two schedules.

## Configuration and rejection

Relation bindings select providers per input and producing RA result. Rename,
Select and Semijoin inherit their source representation. Separate bindings select
candidate traversal, key resources and complete pair algorithms. Scoped defaults
are optional when all used operations have explicit bindings. Unused operations
do not require configuration.

Native options are checked for the selected operation. Scan needs none; pruning
needs its iterator continuation; exact/zone construction needs its collection and
iterator; block construction needs packing size. A direct membership test needs
no construction iterator. Observers add an explicit counter expression only to
operations that can use it.

Leaf operations are declared once in `cq-ir::native` using derived parsing. The
frontend parses only administrative macro paths and expressions around them.
Unsupported combinations fail at selected expansion or Rust type checking:
there is no universal storage wrapper, silently substituted representation or
runtime capability registry.

Provider traits expose only the required storage operations. `BuildBlocks`
accepts any iterator of unique rows; a HashSet is not part of its public carrier
contract. Materialization owns uniqueness before block construction. Private
storage layout stays behind qualified trait calls in generated code.

Ownership is divided at the same boundaries. Relation lowering supplies borrowed
field expressions to key operations and pair algorithms; exact key providers
retain references, while zone providers copy referenced integer coordinates.
Set selection stores row references, so the enclosing realization keeps earlier
views and owners alive for repeated selection and borrowed resources. Expression
aliases also borrow computed fields. Consuming expressions must request ownership
explicitly; construction of owned records and answers may still clone fields.
These are operation and lifetime contracts within the existing Native dialect,
not another ownership IR or a universal storage wrapper.

The observed relation host forwards its counter through selected native paths;
it does not choose storage. Item and callable hosts share definition lowering.
The callable captures packing expressions in ordinary Rust scope. Internal
handles cannot escape the borrowed inputs; returned answers are owned.

## Validation and extraction

```sh
cargo test --workspace
python3 tools/check_dependencies.py --isolate
python3 tools/check_features.py
python3 tools/export_ya.py --help
make -C doc/formal
```

The dependency tool distinguishes a macro-resolution probe from an executable
query consumer. Feature tests run real package targets, including independently
selected push execution, tuple providers and pair algorithms. The matrix is
derived from Cargo metadata and target prerequisites; it does not enumerate all
possible feature subsets.

[YA extraction](../teaching/ya/README.md) physically selects its graph, result-index,
eager hash, pull and collection implementations. Students receive the required
syntax and foundation without unrelated native storage or execution alternatives.
Its reference overlay and restored exercise stubs are verified independently.

Execution tests and correspondence tests answer different questions. Source
isolation alone proves neither relational equivalence nor legal ownership in
emitted code. The [snapshot](formal/snapshot.txt) records the commands actually
completed and source identity; this document does not assert unrun coverage.
