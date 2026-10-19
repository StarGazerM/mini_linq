# Lookup&Expand on the R/E/C back-end model

The co-design is implemented through existing Structures contracts, not a
separate GPU language. `cq_gpu::print` consumes checked Structures and emits
`syn::Expr`. The old `p via cursor s;` hidden-field implementation and proposed
`cq-lang-device` layer have been removed from the design.

## Current responsibilities

1. **Plan:** `cq_planners::order(le)` records non-introducing partial subatoms
   for early lookup and coarse full subatoms for later expansion. Plan-Le and
   CK-Le preserve written column ownership and Rust Before obligations.
   A lookup can be understood relationally as a semijoin with a witness, and
   expansion as the corresponding join. That intuition is not permission to
   reorder arbitrary partial Rust: bottom + Set does not make partial
   computations freely commutative. Totality, dependency, scope and Before
   premises remain necessary; evaluation counts alone are not the denotation.
2. **E — sharing:** `r share = lookup b [K] first s force p;` (ST-Share)
   records one immutable keyed read evaluated at the eager semi and reused at
   the forcing probe. The coupling invariant is `memo = lookup b k`.
3. **R — witness:** `repr … witness { b [K] at s as Type }` (STR-Witness)
   carries that lookup result as a non-column carrier. It cannot escape into
   the head, canon, retained builds or unrelated control paths, and is pruned
   at the forcing probe. `cq_rewrites::retain_lookups` records eligible pairs;
   `SortedRun` supplies a Copy range cursor. Provider capability is an explicit
   obligation, not a promise that every index cursor can be retained.
4. **E — expansion:** `r stage = batch N expand;` (ST-Expand) permits the
   checked segment-represented expansion. N input rows hold N ranges with
   lengths and an exclusive scan; their total M is traversed by segmented
   flatten. Finite N does not bound M by N, guarantee that M fits an index,
   bound total output memory, or eliminate synchronization. The CPU pull
   printer uses `cq_storage::stage::segmented`; the GPU uses its recorded
   cooperative stage and bounded frontier. The push printer rejects sharing
   and expand staging rather than silently ignoring them.
5. **C — control:** pull, push and GPU execution are realizations of the same
   recorded stream obligations, not new relational meanings. The GPU printer
   accepts explicit tile/frontier/directory arguments and rejects unsupported
   programs. `probe first` selects traversal, not uniqueness or PK evidence;
   a run lookup may contain multiple matching payloads.

The witness exposes payload columns only when the probe expands them. It is
not an extra relational column or a host-domain density assumption. GPU
output capacity and integer-domain limits are independent checked runtime
contracts. The host inputs remain immutable during a query.

## Sources of truth

- [Back-end rules](formal/backend.tex): ST-Share, STR-Witness, ST-Expand and
  the R/E/C refinement obligations.
- [Lookup-expand status and CPU measurements](STATUS.md#lookup-expand-route-cq_plannersorderle).
- [GPU design and evidence boundaries](GPU-DESIGN.md).
- [Implemented GPU pipeline, restrictions and measurements](../experiments/gpu/README.md#structures-gpu-terminal-printer).

Historical timing anecdotes from the pre-integration proposal are superseded
by those linked records; they are not evidence that sharing alone caused a
speedup or that the hand-written and generated kernels have equal performance.
This note summarizes the implementation and does not change the mathematical
core specification.
