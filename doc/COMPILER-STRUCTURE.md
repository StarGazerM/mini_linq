# Compiler structure and decision ownership

The compiler has independently usable language edges and smaller native Access
operations. [Access refinement](ACCESS-REFINEMENT.md) explains the physical design;
[the boundary review](IR-STAGE-REVIEW.md) inventories every stage.

| Module | Owns | Does not own |
| --- | --- | --- |
| `cq-ir` | CQ, named RA, index requirements, Access, Pipeline, provider fragment syntax and local validation | Compilation policies and runtime execution |
| `mini-linq` frontends | Typed parsing, options, macro entry points and continuation calls | Private query parsers or hidden capability catalogues |
| `cq-compiler/cq_to_relational` | Scheduling rules, graph/YA analysis and source correspondence | Physical representation selection |
| `cq-compiler/relational_to_indexes` | Named equality-index demands for the canonical route | Block-filter scheduling |
| `physical/materialized.rs`, `lazy.rs`, `rust.rs` | Their admitted canonical physical realization | A universal data-structure interface |
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
| `cq-storage` | Immutable payload/metadata, masks, operation traits, summaries and passive counters | Relation APIs, semijoin/join functions, query plans or an interpreter |

Paths in the table are relative to `crates/`; compiler paths without a crate
prefix are inside `crates/cq-compiler/src/`.

Configuration is separate from semantic inputs. Providers are selected per input
and materialized result; candidate scans and key resources per semijoin; witness
access per join. Defaults are ordinary paths, with explicit scoped overrides.
Rename and Semijoin inherit the representation they view. Configuration errors
are diagnosed where their operation domain is known.

The direct physical pass carries Γ in Rust data structures and emits
`Rust[Access]`. Provider/access macro arguments contain only an operation's actual
semantic inputs. No serialized physical context or empty index pass mediates that
edge. A new implementation belongs at the smallest existing boundary that owns
its changed construction, access and lifetime. A new IR requires a meaning that
these boundaries cannot already express, together with grammar, judgment and
lowering rules; a desire to expose another macro alone does not establish that need.

A macro alias changes a continuation only where generated paths refer to it.
Lexical wrappers bind selected paths and services explicitly. Compatibility has
two parts: expanded Rust must type check, and the implementation must satisfy its
observation and ownership laws. Source/template audits and bounded executions
supply evidence for the supplied implementations, not a proof for arbitrary aliases.
