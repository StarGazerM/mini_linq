# Semantics of the chain

What each language denotes, what each pass may decide, and the obligations
every printed program (every *dialect*) meets. Grammars are in
[DSL.md](DSL.md); crates in [COMPILER-STRUCTURE.md](COMPILER-STRUCTURE.md); the measured WCOJ claim in [`wcoj.rs`](../facade/mini-linq/tests/wcoj.rs).

## One meaning, many presentations

Every language denotes the same thing: the set of answers of the conjunctive
query, `⟦Q⟧(D) = { h(head) | h : Vars → Dom, h(body) ⊆ D }`. Passes change
how that set is *presented*, never what it is. Correspondence between
adjacent languages is therefore an erasure: CQ-NF is the kernel every plan
erases to (`Kernel`, compared up to renaming of variables), VO reductions
erase to the unreduced plan, RA erases back to the kernel
(`relational_plan::kernel`), IX/Physical/Structures erase by dropping their
section.

| language | presentation | decision recorded |
|---|---|---|
| CQ-NF | atoms over distinct variables, guards, bindings | none (canonical form) |
| VO | the order in which valuations are built; granularity of each step | valuation order, split |
| VO (reduced) | + consistency checks placed before joins | where reductions go |
| RA | straight-line named algebra (SSA without φ) | none (elaboration) |
| IX | the key sets the plan applies results at | none (derived demands) |
| Physical | one operator per result; `pipe`/`let`/`alias` modes | operator form, dedup placement |
| Structures (Γ) | the value answering each keyed read | representation |

A valuation plan's step `i` denotes the partial valuations on the variables
bound so far that satisfy every subatom seen; the last step denotes `⟦Q⟧`.
Consistency checks (semijoins) are identities under set semantics — they
change the size of intermediates, never the answer.

## Keys are curryings

A relation `R ⊆ Tup(X)` read on a key `K ⊆ X` is used at the curried type
`Tup(K) → P(Tup(X∖K))`. IX lists these types (sets, unordered). A value in Γ
may represent a chain of them: `Type [K₁][K₂]` is `Tup(K₁) → Tup(K₂) → …`,
and by the exponential law `(A × B → C) ≅ (A → B → C)` it answers a read on
every prefix `K₁ ∪ … ∪ Kᵢ`. That is a trie; selection covers a result's
demanded keys by chains. `Build` is currying, `lookup` is application,
`α ∘ build = id` is `uncurry ∘ curry = id`; a lazily built level is
call-by-need. Level order is representation and never appears above Γ.

## Modes, grades, dedup

- `pipe` is linear: read exactly once, as a stream; `let` is shared: read
  through a key or more than once. Mode is inferred from use.
- `merge` needs both sides ordered on its key — by a structure, an ordered
  host key, or a pipe sorted by its own merge.
- `canon` marks a result inserted into a canonical index (dedup). The return
  is canonical. `project = canon` dedups every key-dropping projection; set
  semantics is thereby a property of representation, not a separate algebra.

## Obligations of every printed program

- **D1** The distinct result of running the program is `⟦Q⟧`.
- **D2** Orders a `merge` relies on are preserved, or the dialect refuses.
- **D3** A pipe is consumed once — licenses moves, fusion, phases, fan-out.
- **D4** Embedded Rust is pure — licenses evaluation order and count
  (laziness, batching, parallel evaluation). The recognized comparison
  fragment is pure by construction; other expressions are trusted.
- **D5** Hosts are immutable for a run.
- **D6** A column of declared type `T` denotes `&T` in every embedded
  expression; no printer copies a binding. Ownership is created only by a
  `let`, a final computed column, or the endpoint.

Dialects (pull, push, parallel, batched, incremental) are printers and
runtimes that meet D1–D6; they are checked by running them against the
reference printer, not by a proof.

## Worst-case-optimal join, decomposed

Generic join is not a separate algorithm here. It is: `split = Fine` in the
order pass (bind one variable at a time), `project = canon` (enumerate each
prefix as a set), prefix checks as semijoins on the clause relation (so one
curried structure serves them), and `intersect = smallest` (enumerate the
smaller candidate set, test the other). `crates/mini-linq/tests/wcoj.rs`
measures it on the fan instance: binary and first-only plans do Θ(n²) hash
work, the smallest-first plan Θ(n).
