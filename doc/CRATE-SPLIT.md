# Crate split: Slice D and D2 handoff

No commit was made. All numbered steps reached a passing
`cargo test --workspace --locked` checkpoint. Step 2 was reported as the
integrator checkpoint before section extraction began.

## Per-step record

| Step | Files moved or changed | Workspace gate | Expansion evidence |
|---|---|---|---|
| 1 | `crates/cq-ir/src` split among `languages/cq-syntax`, `cq-lang-cq`, `cq-lang-vo`, `cq-lang-ra`, `cq-lang-ix`, `cq-lang-physical`, `cq-lang-structures`; state protocol moved to `runtime/cq-protocol`; passes, runtime crates and facade moved with `git mv`; manifests, tools, docs and regression paths updated | 183 tests passed | Baseline captured before the moves; compared at step 2 |
| 2 | Adjacent language dependencies and pass endpoint dependencies wired; source imports rewritten; `cq-ir` removed | 183 tests passed | All ten expansions byte-identical; integrator checkpoint reported |
| 3 | Language `Section` types, Env contracts and optional `reference.rs` adapters; shared `Staged` parser; pass outputs changed to sections; facade and consumer fixtures reassemble payloads | 186 tests passed | All ten expansions byte-identical |
| 4 | `entry.rs` core moved to `facade/cq-stage/src/lib.rs`; documented non-proc-macro API; facade retains ABI conversion | 187 tests passed | Covered by final baseline comparison |
| 5 | `access.rs`, `expressions.rs`, shared context, layout and key helpers moved to `passes/cq-print`; pull control flow remains in `cq-physical-realization` | 187 tests passed | Covered by final baseline comparison |
| 6 | `tools/check_dependencies.py`, pass endpoint metadata, standalone language fixtures and frontend/consumer probes | 187 tests passed; dependency command passed with the isolation qualification below | Covered by final baseline comparison |
| 7 | `examples/own-dialect`, `examples/stacked-ir`, regression entries, expansion comparison scripts | 187 tests passed; both scenarios passed | Reference baseline: 10/10 identical; stacked/reference: 10/10 identical |

The move inventory, logs and expansion captures are in the ignored
[`target/slice-d`](../target/slice-d/) directory. The before/after capture
command can be repeated with `tools/check_expansions.py`.

## Interface details and isolation (D2)

- Every later language core owns a section. `contract::check(section, env)`
  checks that section; the default `reference-prefix` feature exposes the
  reference composition as `Module`. `contract::check_module` validates the
  concatenated reference payload. Disabling the feature removes the producer
  language dependency.
- IX and Physical reference Env adapters cache validated headings. This is
  necessary to implement `heading() -> Option<&[String]>` without returning a
  slice into temporary RA analysis. Structures owns its own `PhysicalFacts`,
  `ModeKind` and `UniqueOp` rather than depending on producer types.
- Shared syntax includes `RelationDecl`/`ColumnDecl` (needed by `InputList`)
  and `Atom`, `Filter`, `Let`, `TypeAnnotation` (used by multiple grammars).
  CQ re-exports these nodes. This extends the requested shared syntax list
  without moving a complete CQ program or language contract into syntax.
- Every pass now takes its source `Section` and source language's `Env`,
  followed by options/policy/endpoints, and returns only the target section
  (or `syn::Expr` for the pull printer). CQ is the root: its `Section` aliases
  its complete `Module`, and `()` implements its empty Env. `cq-print`'s
  `Context::new` takes a Structures section, Env and checked analysis.
- Composed-module conveniences are `pass::reference::{compile, check}`,
  behind each pass's default-on `reference` feature. The reference elaboration
  check retains its independent CQ/RA kernel comparison; the standalone check
  requires the exact elaboration. The facade calls core APIs with explicitly
  constructed source Envs. Existing test/driver adapters use `reference`.
- Every pass's production language dependencies disable defaults. Only its
  `reference` feature re-enables endpoint `reference-prefix` features (and
  `cq-print/reference` for the pull printer). Language reference features now
  explicitly propagate through the optional prefix chain.
- **The strict pass-isolation gap is closed.** `check_dependencies.py --isolate`
  selects endpoints directly, without following their dependency closure.
  Each pass is built and tested with `--no-default-features`, with only its
  source/target language directories plus syntax present, and `cq-print` for
  the pull printer. VO -> VO has one distinct endpoint; printers have one
  language endpoint. The checker verifies resolved features contain no
  `reference-prefix` and that each pass actually executes a test. Composed
  tests require `reference`; every pass also has `tests/core.rs` with an
  independent environment.

The isolation copier continues to remove unavailable optional path manifests
(Cargo resolves them even when disabled) and dev-dependencies. Production
non-optional edges are preserved and checked; core tests use only the copied
endpoints. It copies pass integration tests and their `required-features`
metadata, so reference tests are skipped by Cargo, not silently lost.

### Source Env audit

Contract-only producer traits (`CqEnv`, `RaEnv`, `IxEnv`, `PhysicalEnv`) remain
small. Where passes need more facts, the source language exposes an extended
`Env`; it never mentions an upstream language type.

| Consumer | Source facts read beyond its section |
|---|---|
| CQ order | None: root section owns schemas and query; pass supplies VO Env |
| VO reduction | Clauses for validation; source GYO join tree for tree reduction |
| VO elaboration | Clauses, input schemas and output head |
| RA index policy | Input schemas and expected head; derives headings from RA |
| IX Physical lowering | Sorted headings, join demands, host schemas, ordered semantic operations and output; binding/expression facts accompany the target Env |
| Physical structure selection | Logical/heading/host/demand facts already used by the contract, plus host schemas and index advice; expressions and binding provenance accompany the target Env |
| Structures printers | Physical reads/modes/canon/scans, host layouts, execution operations, expression bindings, computed-column provenance and output head |

IX owns the token-free `Operation` vocabulary used by lowering and row-fact
analysis. Structures owns its printer execution facts. No RA or Physical
parser types were moved into shared syntax. The passes expose `Env::new`
adapters for their target contracts, so consumers can carry these facts
through the unchanged pipeline. Reference adapters cache checked headings
and row facts rather than repeatedly analyzing composed prefixes.

### Independent surface example

[`examples/diverged-ra`](../examples/diverged-ra/README.md) is its own workspace.
The local `ra-theta` crate defines `theta_join` sugar, desugars it to the IX
semantic Join + Select vocabulary, derives its own IX demands, and implements
IX's Env. This is the allowed alternative to a new `union` operator: unchanged
Physical cannot represent union. The example does not use `cq-index-policy`,
which consumes the reference RA section.

`build.rs` prints two real programs through the unchanged downstream passes;
the library includes and executes that Rust under probe and merge policies.
Tests compare both with a nested-loop oracle, including duplicates, empty
inputs, and 256 small input pairs per route. `check.py`, registered in the
regression inventory consumed by `tools/test_regressions.sh`, additionally
checks lockfile package names, resolved features, and original manifest paths.
The internal dependency list is `ra-theta`, `cq-lang-ix`, `cq-lang-physical`,
`cq-physical-lowering`, `cq-lang-structures`, `cq-structure-selection`,
`cq-physical-realization`, `cq-print`, `cq-syntax`, and `cq-storage`. There are
no reference CQ/VO/RA crates, copied downstream crates, or facade dependencies.

`own-planner` remains outside these slices. The push example still supports
its `built` and `generic` queries and now consumes the new printer context;
the stacked trace example still adds and erases an empty section.

## Slice D gates (historical)

| Command / evidence | Result |
|---|---|
| `cargo fmt --all` | Passed |
| `cargo clippy --workspace --all-targets --locked -- -D warnings` | Passed |
| `cargo test --workspace --locked` | 187 passed |
| `python3 tools/check_dependencies.py --isolate --frontends --consumers` | Passed with pass-closure qualification above |
| `bash tools/test_regressions.sh` | 30 passed, 0 failed |
| `python3 tools/check_features.py` | 11 feature cases passed |
| `python3 doc/formal/check.py` | 82 rule locations checked; paths re-pinned |
| `cargo test --manifest-path xtask/Cargo.toml --locked` | 8 passed |
| `cargo run --manifest-path examples/composition/Cargo.toml --locked` | Passed |
| `cargo test` and `cargo run --manifest-path doc/formal/example/Cargo.toml --locked` | Passed |
| Final `physical_query` expansions compared with pre-step-1 captures | 10/10 byte-identical |
| Stacked-IR execution and expansion comparison | 9 tests on each route; 10/10 byte-identical expansions |

These gates establish the listed structural, parser/contract, generated-Rust
and oracle checks. They did not establish strict pass isolation; D2 adds the evidence below.
They do not broaden the supported execution domain.

## Slice D2 gates

No commit was made. Logs and before/after expansion captures are under
[`target/slice-d2`](../target/slice-d2/).

| Command / evidence | Result |
|---|---|
| `cargo fmt --all -- --check` | Passed; updated independent examples formatted too |
| `cargo clippy --workspace --all-targets --locked -- -D warnings` | Passed; diverged-ra workspace/all-targets also passed |
| `cargo test --workspace --locked` | 195 tests passed |
| `python3 tools/check_dependencies.py --isolate --frontends --consumers` | Passed: every pass tested with only its endpoints and no reference prefix; six frontend and two consumer probes passed |
| `bash tools/test_regressions.sh` | 31 passed, 0 failed |
| `python3 tools/check_features.py` | 11 feature cases passed |
| `python3 doc/formal/check.py` | 82 rule locations; source markers updated |
| `cargo test --manifest-path xtask/Cargo.toml --locked` | 8 tests passed |
| Root examples, composition, own-dialect, stacked-ir, diverged-ra, formal example (including constraint planning), and the three Rust note example workspaces | Passed |
| Final `physical_query` expansion comparison | 10/10 byte-identical |

The formal gate checks source-location inventory, not mathematical proofs.
Isolation, generated Rust execution and expansion equality are separate gates.

D2 file ownership: workspace/dependency manifests; language `lib.rs`, optional
`reference.rs`, IX `env.rs` and Structures `execution.rs`; all eight pass
crates and their `tests/core.rs`; the mini-linq facade; reference test/driver
callers in xtask and the formal example; the own-dialect printer; the complete
new diverged-ra workspace; `tools/check_dependencies.py`,
`tools/regression-targets.tsv`, `doc/formal/rules.tsv`, and this handoff.
The optional, unregistered HW4 negation/aggregation exercise is not an active
example gate and was not extended by D2.
