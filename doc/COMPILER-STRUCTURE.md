# Compiler structure and decision ownership

The compiler has independently usable language edges and smaller native Access
operations. [Access refinement](ACCESS-REFINEMENT.md) explains the physical design;
[the boundary review](IR-STAGE-REVIEW.md) inventories every stage.

| Module | Owns | Does not own |
| --- | --- | --- |
| `cq-ir` | CQ, named RA, index requirements, Access, Pipeline, provider fragment syntax and local validation | Compilation policies and runtime execution |
| `mini-linq` frontends | Typed parsing, options, macro entry points and continuation calls | Private query parsers or hidden capability catalogues |
| `cq_to_relational/canonical.rs` | Checked default CQ entry and canonical written-order lowering | Index selection or physical representation |
| `cq_to_relational.rs` and its other submodules | Checked planning entries, scheduling rules and graph/YA analysis | Physical representation selection |
| `relational_to_indexes.rs` | Checked default/result-index entries and demands on named RA results | Runtime storage or block-filter scheduling |
| `relational_to_indexes/input.rs` | Equality-index demands on declared inputs | Rust storage construction |
| `relational_to_indexes/linear.rs` | Storage-free facts about admitted linear RA and its operands | Hash layouts, builder code or query execution |
| `physical/mod.rs` | Checked indexed-physical entry and selection of its admitted realization | CQ scheduling or index inference |
| `physical/input_indexed.rs`, `lazy.rs`, `materialized.rs` | Input-indexed, lazy result-indexed and materialized Rust realization; materialized checked entries | A universal data-structure interface |
| `physical/rust.rs` | Shared Rust representation helpers | Logical planning |
| `physical/direct/mod.rs` | Checked configured RA-to-Access entry | Canonical index demands |
| `physical/direct/layout.rs` | Admitted RA domain, current result layouts and provider inheritance | Native storage internals |
| `physical/direct/resources.rs` | Versioned key-resource identity, first use and projected Access construction | Candidate scan choice |
| `physical/direct/lower.rs` | RA operators to Access, exact residuals and materialization sites | Hash/zone implementation details |
| `physical/direct/wiring.rs`, `api.rs` | Configured lexical macros, query API and optional observations | Runtime relational operators |
| `physical/direct/check.rs` | Source-to-emitted-code correspondence for the direct edge | Arbitrary macro semantics or Rust adequacy |
| `physical/access/scan.rs` | Current-row or block-pruned candidate Access | Exact semijoin membership |
| `physical/access/keys.rs` | Exact key construction, membership and optional coordinate summary | Relation layout or logical scheduling |
| `physical/access/witnesses.rs` | Hash-bucket or deferred nested witness access | Whole-program join planning |
| `physical/providers` | Native-set and block operation expansion through public traits | New RA operators |
| `pull.rs`, `pipeline.rs` | Access-to-Pipeline and Pipeline-to-Rust endpoints | Storage policy |
| `pass_contracts/{canonical,indexes,ordered,planned_join,physical,pull,pipeline}` | Independent correspondence checks for the respective edges | Constructing the lowering's expected output by calling its emitter |
| `cq-storage` | Immutable payload/metadata, masks, operation traits, summaries and passive counters | Relation APIs, semijoin/join functions, query plans or an interpreter |

Paths in the table are relative to `crates/`; compiler paths without a crate
prefix are inside `crates/cq-compiler/src/`.

Each transformation owns its checked public entry. The crate root re-exports
the `compile_*` API, while `pass_contracts` declares independently implemented
edge checkers. Shared analysis is placed at the semantic boundary that defines
its facts; index selection therefore does not import a physical backend to
recognize RA. Course assignments select existing edges without defining the
compiler's module hierarchy.

Configuration is separate from semantic inputs. Providers are selected per input
and materialized result; candidate scans and key resources per semijoin; witness
access per join. Defaults are ordinary paths, with explicit scoped overrides.
Rename and Semijoin inherit the representation they view. Configuration errors
are diagnosed where their operation domain is known.

The direct physical pass carries Γ in Rust data structures and emits
`Rust[Access]`. Provider/access macro arguments contain only an operation's actual
semantic inputs. An implementation belongs at the smallest existing boundary that owns
its changed construction, access and lifetime. A new IR requires a meaning that
these boundaries cannot already express, together with grammar, judgment and
lowering rules; a desire to expose another macro alone does not establish that need.

A macro alias changes a continuation only where generated paths refer to it.
Lexical wrappers bind selected paths and services explicitly. Compatibility has
two parts: expanded Rust must type check, and the implementation must satisfy its
observation and ownership laws. Source/template audits and bounded executions
supply evidence for the supplied implementations, not a proof for arbitrary aliases.
