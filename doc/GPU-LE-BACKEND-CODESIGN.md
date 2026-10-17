# Co-design note: Lookup&Expand / GPU on the R/E/C back-end model

From the GPU session (branch `gpu` in /data/mini_linq-gpu, 13 commits over df91695, not pushed).
For the codegen session that landed backend.tex / repr / stage / column-level printers (ad9727b).

## Classification of what LE needs (checked against the semantics)

1. Written-order denotation (bottom + Set, relational steps may move across Rust): covers all of
   L&E. A lookup state `R -> S` (Birler et al. VLDB'24) denotes `R ⋉ S` with a witness;
   `e(R -> S) = R ⋈ S`. Evaluation counts are explicitly outside the meaning. No IR semantic change.
2. Plan: VO already expresses lookup-first/expand-late (partial non-introducing subatom = semijoin of
   the prefix; later full subatom = join; relational steps may move later). What was missing was a
   planner: `cq_planners::order(le)` (external, Plan-Le/CK-Le). Done on `gpu`.
3. Keeping the lookup's handle so the expansion does not probe again: this is **R**, data refinement.
   The semi's result representation carries, for the build occurrence `b`, an *unexpanded* witness:
   a RowId range into `b`'s representation (for a clustered run `(begin,count)`; for a hash index a
   bucket cursor). α of that representation fetches `b`'s columns only after expansion. This is the
   factorized state of L&E and the join-index framing backend.tex already cites.
   On `gpu` I recorded it as a separate Structures entry `p via cursor s;` with hidden row fields and
   per-operator threading in the realization — the wrong encoding: it duplicates the carrier
   environment you just built. Proposal: drop it after rebase and express it as a `repr` extension:

       s repr = positions { … } owned [ … ] ranges { b at s };   // or a `witness` keyword
       -- STR-Repr: coverage unchanged (b's columns are NOT in L_s until expanded);
       -- the consumer probe reads the range: op_ρ commutes with α by IndexRead's lookup law.

   Capability premise (Copy/'host-lifetime cursor) is a rustc obligation, as for other carriers.
   `SortedRun` (runtime/cq-storage on `gpu`) provides a Copy range cursor; HashIndex's does not.
4. Expansion with batches: **E**. ST-Stage currently forbids spanning a probe because fan-out makes the
   chunk size variable. With ranges as the probe's output representation the chunk stays fixed:
   a batch of N input rows yields N ranges plus their exclusive scan (total M); the expansion is a
   virtual array addressed by position (owner = upper_bound(offsets, o) − 1, row = begin + o − offset),
   consumed in chunks of N. This is the "lookup-expand descriptor" the GPU kernels use; it is also a
   valid CPU staging rule (ST-Stage relaxation: a probe may be spanned iff its output repr is ranges).
5. Tile kernels, dynamic tile cursor, descriptor pool/stealing: **C**. Block-cooperative unfold of the
   same stream algebra; BK-Dual-style obligation (same R/E, preserve Before, exhaustive = same bag).
   Consequence: the planned `cq-lang-device` shrinks to recording R (device layouts/directories),
   E (tile = batch N, frontier K) and C choices per kernel; no own row model.

## Asks for the codegen side

- Can the column environment carry a range/witness carrier for a build occurrence (not a base
  occurrence) and prune it at the consuming probe? If yes, `via cursor` disappears entirely.
- Is a `stage = batch N` spanning a probe acceptable under rule 4 (ranges repr), or should the
  expansion be its own evaluation-unit kind?
- Rebase of `gpu` onto ad9727b will conflict in cq-print/lib.rs, cq-physical-realization/operators.rs
  and cq-lang-structures (realization of `via cursor`). I intend to drop that realization and redo it
  as above; `order(le)`, `SortedRun`, `sorted_runs`, the formal rules and the GPU workspace carry over.

Evidence on `gpu`: doc/GPU-DESIGN.md (H05 SF10: hand-written GPU 14.6 ms vs srdatalog 583 ms vs DuckDB
24T 154 ms; dimension-first expansion 23.6 ms vs srdatalog 58 s), doc/STATUS.md `order(le)` (SF1:
le_runs 0.78–0.84× default where a cursor is retained on an integral-key build).
