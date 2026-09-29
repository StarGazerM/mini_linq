# Slice D handoff

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

## Interface details and remaining isolation gap

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
- Each pass's `compile` returns its target section; `check` takes that section.
  The facade and reference consumer fixtures explicitly reassemble prefixes.
  Rust printers target `syn::Expr`, and the VO → VO pass has one distinct
  language dependency.
- **Strict two-language pass isolation remains unmet.** The checker enforces
  exactly the direct source/target language edges, plus `cq-print` for the
  pull printer. It physically isolates every language without its optional
  prefix. Pass isolation currently copies the endpoints' production
  dependency closure, including their default reference prefixes. The
  existing reference pass signatures still consume composed source `Module`
  types. Consequently the passing dependency command is not evidence that
  every pass builds with only two language directories present. Achieving
  that stronger requirement needs reference-independent pass source APIs as
  well as the language Env contracts implemented here.

`own-planner` and `diverged-ra` were not added. The push example supports the
requested `built` and `generic` queries, including their selected host/built
access paths, and diagnoses unsupported operators. The trace example adds
and erases an empty section between Physical and Structures.

## Final gates

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
and oracle checks. They do not establish the stricter pass isolation above
or broaden the supported execution domain.
