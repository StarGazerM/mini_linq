# Next step: schedules for plan control

## Goal

A query controls plan decisions (join order, branches, reductions, build side,
structure types) through a **schedule** that names source entities and is
consumed by the pass that owns each decision. Schedules change cost, never
answers: $[\![\mathrm{Rust}]\!] = [\![\mathrm{CQ}]\!]$ for every schedule.

## Rejected: positional quotes into lower IRs

An earlier draft spliced raw lower-IR text at positional anchors
(`quote vo after <pass> @ 3 4 { … }`). Rejected because it:

- addresses the compiler's expansion by position: one-time, brittle references
  (Exo 2's taxonomy) that break when the query or a pass changes;
- edits a level's output instead of the input of the transformer that decides,
  unlike multi-stage programming and towers of interpreters, where control is
  static input to the generator or a modified interpreter;
- exposes lower-IR syntax to users, which MSP macro systems encapsulate.

## Prior art adopted

| Source | Taken |
|---|---|
| Halide; pg_hint_plan (`Leading(((l s) (o c)))`) | nominal, stable references to names the user wrote; separate schedule |
| Scheduling-language taxonomy (Hall et al. 2024) | separate schedules allow several strategies per algorithm; pragmas allow one |
| MetaML / Scala 3 quotes; hygiene (Flatt, POPL 2016) | references are hygienic bindings; users never see generated code |
| Collapsing towers (Amin, Rompf, POPL 2018) | control a level by parameterizing its transformer; other levels pass through |
| Origin tracking / resugaring (Pombrio, Krishnamurthi) | compiler-maintained mapping from lowered code to source, internal only |
| Exo 2 (ASPLOS 2025) | checked primitives; compiler-forwarded references; later, user-defined schedule functions |

## Design

1. **Names.** Atoms may bind an alias: `l: Lineitem(..)`. Aliases are hygienic
   identifiers scoped to the query; an unaliased relation occurring once is
   named by its relation. Ambiguous or unbound names are errors.
2. **Schedule.** A separate `schedule { … }` beside the query, one per
   configuration. Directives name aliases only:
   - `join(<tree>)`: association, e.g. `join((l s d) (o (c ⋉ d)))`;
   - `reduce(c by d)`: an extra semijoin;
   - `build(o)` / `stream(l)`: build side (replaces `plan(stream(..))`);
   - `type(<name>, Path)`, `join_filter(<name>, Path)`: structure choices
     (replaces `retype`/`custom_indexes` keyed by `r2`).
3. **Consumption.** Each directive is a static argument to its owning pass
   (ordering, reduction, orientation, Structures rewrites). The pass decides
   under the directive and records the decision in its output IR as today.
   A directive no pass consumes fails compilation.
4. **Checks.** The owning pass's contract and `check_rewrite` validate the
   result; an unsatisfiable directive (e.g. `join` grouping atoms sharing no
   column, or moving opaque Rust earlier) is a compile error naming it.
5. **Forwarding.** Source names travel through every IR as checked provenance
   on definitions, so directives consumed below VO resolve the same alias.
   Provenance is internal; users never write IR coordinates.

## Steps

1. **S1 Names.** Aliases in CQ syntax and surface; carried on atoms in VO.
2. **S2 Schedule slot.** Parse `schedule { … }`; split directives into
   per-pass arguments; error on unconsumed directives.
3. **S3 Order and reduce.** Ordering/reduction consume `join` and `reduce`;
   H07-block is the acceptance kernel.
4. **S4 Provenance.** Record alias provenance on RA/IX/Physical/Structures
   definitions.
5. **S5 Build and structures.** Orientation consumes `build`/`stream`;
   Structures rewrites consume `type`/`join_filter`; remove `plan(stream(..))`
   and `r2`-keyed arguments.
6. **S6 Schedule functions** (later): user-defined directives composed from
   checked primitives.

## Acceptance

| Invariant | Evidence |
|---|---|
| No schedule means no change | every IR and the generated Rust byte-identical across the corpus |
| Soundness | every directive-induced result passes its pass's checks; violations fail compilation |
| Stability | renaming generated results or reordering unrelated clauses leaves schedules valid |
| Completeness | an unconsumed or unresolved directive fails compilation |
| Performance | H07-block with `join((l s d) (o (c ⋉ d)))` at or below DuckDB 1 thread; H05 and D85 via `stream`/`build` |
| Migration | `retype`, `merge_joins` and `plan(stream(..))` users moved to schedules |
