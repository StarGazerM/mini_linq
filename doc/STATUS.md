# Status

## Where we are

- Semantics, IRs, checks and the pass protocol are complete. Every decision is
  recorded in IR text, and all features build outside the repository.
- Semijoins no longer copy rows: reduced relations are selections over their
  host, filters run inside the scan, and zone-mapped hosts skip blocks.
- Hashing uses a recorded fast hasher and a compact index layout.

## Gap to DuckDB (SF 1, single thread, query time)

| Kernel, plan | MiniLinq | DuckDB |
|---|---:|---:|
| D85 hypergraph | 73 ms | ~40 ms |
| H03 hypergraph | 76 ms | ~36 ms |
| H03 default | ~350 ms | ~36 ms |
| D85 default | 1,512 ms | ~40 ms |
| H09 default | 2,020 ms | ~160 ms |
| Q19 default | 145 ms | ~155 ms |

Yannakakis plans are within about 2× of DuckDB. Default plans are 10–40× slower.
Their hottest symbol is now `HashIndex::lookup`: one probe per row.

## Left work

1. **Measure batching.** Profile whether batched lookups and row-at-a-time
   iteration explain the default-plan gap before changing the runtime.
2. **Plan shape.** Implied selections from guards, e.g. H07's
   `n_name ∈ {FRANCE, GERMANY}`, are not derived. Opaque Rust guards (H09's
   `contains`) stay where written by design.
3. **Positional default.** Selections need the caller's `positional(..)` advice,
   because the compiler cannot see host Rust types. Defaults copy.
4. **Approximate filters** (Bloom, coarse bitmaps) need a covering exact check
   in the IR; not started.
5. **Known limits.** No keys or constraints on data; no query minimization;
   no parallel execution; block fix can form Cartesian intermediates in rare
   cases; D27 hypergraph 4% slower after the hashing change.

## Next step

Profile H09 and D85 default at SF 1 with batched-lookup experiments, then decide
whether a batch interface in `cq-storage` (recorded in Structures) is justified.
