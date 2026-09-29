# What is established

Each item is a design decision the code now enforces, with the place that
enforces or demonstrates it. Anything not listed here is open; the open items
are at the end.

## Meaning

1. **One meaning across presentations.** CQ, VO, RA, IX, Physical and
   Structures all denote the same finite set `⟦Q⟧` of answer tuples (set
   semantics). A pass may change presentation, never meaning.
   [SEMANTICS.md](SEMANTICS.md)
2. **Storage adds no meaning.** A built structure abstracts back to the rows it
   was built from (`α ∘ build = id`); the runtime laws are tested in
   [representation_laws.rs](../runtime/cq-storage/tests/representation_laws.rs).
3. **Deduplication is a canonical index**, not a separate operator: a result
   marked `canon` is inserted into a set, and the returned result must be
   canonical by that flag or by key inference.
   [cq-lang-physical](../languages/cq-lang-physical/src/lib.rs)
4. **Keys are curryings.** An IX demand is an unordered key set on an exact RA
   result. A multi-level Structures entry (`Type [a][c]`) is one curried index
   that answers every prefix of its levels; level order is representation.
   [cq-lang-structures](../languages/cq-lang-structures/src/lib.rs),
   `cq_storage::Trie`.
5. **Modes are binding time.** `pipe` is read once as a stream, `let` is a
   value read through keys or more than once, `alias` is a host read in place.

## Program shape

6. **Every IR is a complete program.** VO extends CQ, IX extends RA and
   Structures extends Physical; RA and Physical depend only on `cq-syntax`. No
   IR carries its producer's program, and there are no environment traits.
   [COMPILER-STRUCTURE.md](COMPILER-STRUCTURE.md)
7. **Languages make no capability claims.** Whether a type supports the reads
   a plan performs is rustc's check, through the `cq_storage::require::*`
   guards the printer emits beside each value.
8. **No fallback hides an unmet premise.** A missing demand, an unserved read
   or a non-canonical return is a diagnostic. Defaults exist only as named
   policy rules (e.g. `join = probe`, `HashIndex` for an unadvised probe).
9. **The printer inserts no binding clones.** Rows borrow host and built
   fields; the only clone constructs an owned host lookup key from bound
   columns. [cq-physical-realization](../passes/cq-physical-realization/src/lib.rs)

## Contracts

10. **A language contract is local.** `contract::check(&Program)` sees one
    program (and, for an extension, its base) and returns the analysis
    consumers use. Every `compile` ends by checking its target contract.
11. **A pass check is an independent correspondence.** `check(source, target,
    options)` checks the source domain, the target contract, an erasure or
    kernel relation, and the obligations of every explicit choice; it never
    calls `compile` (`tools/check_dependencies.py` rejects one that does) and
    accepts any admissible alternative where the options leave a choice free.
    [pass_contracts.tex](formal/pass_contracts.tex)
12. **The printer has no check.** Its evidence is oracle agreement of the
    executed expansion and rustc's trait guards.
13. **Four kinds of evidence stay separate:** language contracts, pass
    correspondences, fixtures (which pin the choices checks leave free) and
    oracles. Each pass test pins `compile(stage)` to the next stage of the
    30-scenario corpus in
    [test-support/relational/scenarios](../test-support/relational/scenarios),
    checks it, rejects targeted mutations and compares answers.
14. **Refactors preserve expansions byte for byte.**
    `tools/check_expansions.py` compares the generated Rust of every
    `physical_query` module with a saved baseline.

## Planning

15. **Planning is analysis, then transformation:** normalize (CQ→CQ-NF),
    order (→VO), reduce (VO→VO), elaborate (VO→RA). The hypergraph (GYO,
    join tree, width) is an analysis the passes read, not an IR.
16. **Reductions erase to the identity.** A guard pushed onto an atom or a
    Yannakakis semijoin changes no answer; the reduce check verifies the
    erasure and that semijoins form a full reducer of a join tree.
17. **Policy is query-independent and caller-supplied, and no pass reselects**
    a choice an earlier pass recorded (selection preserves the Physical
    program unchanged; lowering respects `join`, `project`, host facts).
18. **Worst-case-optimal join is not a new IR:** `split = Fine`,
    `project = canon`, checks read through one trie, and
    `intersect = smallest`. On the fan instance the hash count of that
    configuration grows ≤2.5× per doubling of `n` while binary and
    first-candidate plans grow ≥3.5×
    ([wcoj.rs](../facade/mini-linq/tests/wcoj.rs)). This is measured, not a
    proven complexity bound.

## Modularity

19. **Dependencies follow folders.** A language depends on `cq-syntax` and at
    most its base; a pass on exactly its source and target languages;
    `--isolate` builds each alone.
20. **Stages can be swapped, stacked and diverged by crate choice:**
    [own-dialect](../examples/own-dialect) replaces the printer,
    [stacked-ir](../examples/stacked-ir) inserts a stage with byte-identical
    output, [diverged-ra](../examples/diverged-ra) builds IX from its own
    surface without CQ, VO or the planning passes.
21. **Dialects are printers** (with their runtime) that must meet the
    obligations D1–D6 of [SEMANTICS.md](SEMANTICS.md), checked against
    oracles.

## Open

- `union` (and any operator beyond the positive conjunctive fragment).
- Bag or annotated (K-relation) semantics.
- A proven WCOJ complexity bound.
- Host-declared tries (`keyed [a][b]` on a host's own nested maps).
- Negation and aggregation beyond the optional HW4 specification.
