# GPU execution design (exploration branch `gpu`)

Status: design settled in discussion 2026-10-15/16; de-risk phase in
`features/gpu`. Nothing here is wired into the compiler pipeline yet.

## Decisions

1. **Attach point.** A new checked IR `cq-lang-device` between Structures and a
   terminal printer. Every GPU decision (layout, build kind, probe mode, tile
   shape, compaction thresholds, segment boundaries, skew policy) is recorded
   as IR text. Not emitter flags.
2. **Toolchain.** Kernels are ordinary Rust compiled by the cuda-oxide fork
   (`https://github.com/StarGazerM/cuda-oxide.git`, rev
   `5f7c34fddadcf250f2e45d0cffe83a3d27f2bea3`, nightly-2026-08-28); host
   algorithms come from RCCL (`/data/rccl`: radix sort, scan, gather, merge,
   select/unique, Bloom, sorted search). No CUDA C, no NVRTC, no OptiX.
3. **First workload.** TPC-H retail kernels (features/workloads), SF1 exact,
   SF10 timed, compared with srdatalog-python on the same projected inputs.

## Execution model: block-cooperative resumable tile iterators

An iterator whose `next()` is executed by one thread block and yields a
*tile* of at most `T = NT × IPT` rows (256 × 4 = 1024), columns in registers,
count in shared memory. Operators are block-wide functions `Tile → Tile*` with
explicit resumable state. One fused kernel per Physical stream.

| Operator | Mechanism | Notes |
|---|---|---|
| `TileScan` | vectorised read-only loads; tile cursor from a global atomic | dynamic scheduling = tail balancing |
| `TileFilter` | bitmap (dense keys) / Bloom / hash set / total guard → mask | mini_linq join filters and semijoins land here, before expansion |
| `Compact(θ)` | block scan + smem shuffle when activity < θ | Pyper block-level lane refill |
| `TileLookup` | point probe, `expect 1` | rank directory or fences |
| `TileExpand` | lookup → `(begin,count)`; block scan → M; output-parallel owner search over smem offsets; loops ⌈M/T⌉ output tiles | in-tile skew balanced |
| `TileMerge` | tile keys sorted in smem; fences searched; index blocks streamed through smem; merge-path join | for probe streams clustered on the build key |
| `Segment` | materialise tile stream to global, columnar | when live set × IPT exceeds the register budget |
| `TileStore` | one atomic per tile, coalesced columnar writes; overflow → save `(tile, cursor)` and relaunch unfinished tiles | no count/replay pass |
| `canon` / folds | device hash set with full key compare, or sort+unique; hash aggregate; block reduce | sound dedup |

Columns not needed by later probes are carried as `u32` row ids and gathered
at emit (`carry` vs `gather` is recorded per column).

## Lookup-Expand as lazy virtual expansion (multiway joins)

A lookup over a tile produces a *descriptor*: per item `(parent_slot, begin,
count)` and the exclusive scan of `count` (total M). The expansion is a
virtual array of length M addressable by position: `p = upper_bound(offsets,
o) − 1`, `row = begin[p] + o − offsets[p]`. For intersections (cyclic
queries) the virtual array is the merge-path work space of the ranges and is
splittable at any rank in O(log).

- Per block: depth-first over stages; each stage holds one input tile
  (factorised as `(parent_slot u16, row u32)`) and one descriptor in smem.
  Memory per block is fixed by depth × T.
- Cross-block balance: when a stage's M exceeds K·T the block publishes its
  14 KB descriptor to a bounded global pool with a `(head, tail)` cursor pair;
  the owner consumes T-chunks from the head, idle blocks steal from the tail.
  A pathological key is split across the GPU without materialising its
  expansion. Pool full → owner continues depth-first (imbalance, never OOM).
- Under-full tiles: a stage accumulates its input across upstream tiles until
  full or upstream is exhausted.

## Build representation: sorted, blocked, clustered runs

A `let` build is one or more runs: keys sorted (RCCL radix sort), payload
columns physically clustered with the key (GFTR), 2048-row blocks with a
`min key` fence array, `offsets[unique+1]` for CSR expansion, and a stats
word `{max_count, key span, bytes}`.

Directory: `rank` (bitmap over a dense integer key domain plus per-word
popcount prefix: key → position among unique keys in two loads) when the
domain is dense (TPC-H keys, dictionary ids, `AdaptiveKeySet` columns);
otherwise fences + in-block binary search. A hash directory is an optional
later annotation, never a prerequisite (RCCL's static map is u32-key only).

Recursion: delta = a new run; compaction = RCCL merge of sorted runs.

## Why (evidence)

- Crystal (SIGMOD'20): tiles in registers, one atomic per tile, coalesced
  stores reach bandwidth; joins are L2-bound; no expansion support.
- DogQC (VLDB'20): filter vs expansion divergence; warp-level push-down and
  lane refill.
- Pyper (VLDB'21): monolithic fused kernels lose >50 % to divergence and
  register/smem contention; block-level Shuffle and Segment.
- Wu/Koutsoukos/Alonso (VLDB'24): materialisation up to 75 % of GPU join
  time; clustered gathers 8.5× faster → payloads sorted with keys.
- GPUlog/HISA (ASPLOS'25): sorted array + hash on unique keys; delta merge by
  concatenation; srdatalog's lineage.
- srdatalog: count→scan→host sync→replay, root-only histogram balancing,
  whole-sort maintenance; v8 proposed but did not implement bounded binding
  batches. The descriptor is that saved cursor, made stealable.
- RTIndeX/cgRX: BVH lookups do not beat hash/CSR for equi-joins; OptiX is
  unreachable from cuda-oxide. Not used.

## De-risk phase (features/gpu)

Hand-written kernels before any IR work:

1. `tiles` device crate: block load/scan/compact/sort, owner search,
   merge-path partition, with CPU-oracle tests.
2. `blocked-run`: build from columnar buffers via RCCL; fences; rank
   directory; stats.
3. `h05`: retail_h05 in two plans — fact-first (lineitem scan, point probes)
   and dimension-first (customer scan, expand orders, expand lineitem) — SF1
   exact against a CPU reference and DuckDB, SF10 timed, compared with
   srdatalog-python on identical projected inputs.
4. Resource audit: `ptxas -v` registers/frame/spills per kernel.

Acceptance: exact answers; H05 SF10 fact-first stream ≤ 10 ms; dimension-first
within 2× of fact-first with flat device memory; no spills at ≤ 64 registers
or a documented Segment split.

## De-risk results (2026-10-16, RTX 6000 Ada, retail_h05)

Hand-written kernels in `features/gpu` (cuda-oxide, no RCCL in the device
path — see the compiler bug below). Exact against the DuckDB answer at SF1
(7,242 rows) and SF10 (72,983 rows) for both plans. Times are medians of five
runs after one warmup; GPU numbers exclude the one-time column upload, include
run builds, the fused kernel and exact dedup. srdatalog-python times are
`srdatalog_run` after `prepare` (its index builds included, H2D excluded).
DuckDB is the full `SELECT DISTINCT` on the dbgen tables.

| System (H05) | SF1 ms | SF10 ms |
|---|---:|---:|
| GPU Plan A, fact-first (build / kernel / dedup = 12.3 / 0.91 / 1.38 at SF10) | 3.3 | 14.6 |
| GPU Plan B, dimension-first expand (22.3 / 0.30 / 0.93) | 5.3 | 23.6 |
| srdatalog-python, fact-first order | 38.1 | 582.6 |
| srdatalog-python, default plan | 38.3 | 633.5 |
| srdatalog-python, dimension-first order | 2,872 | 58,051 |
| DuckDB 1 thread | 55 | 618 |
| DuckDB 24 threads | 21 | 154 |
| CPU MiniLinq default (doc/STATUS.md, other machine) | 33.7 | — |

Findings:

- Plan A's fused tile kernel runs at the bandwidth model: 60 M lineitem rows
  × 16 B in 0.91 ms ≈ 1.06 TB/s nominal (supplier column reads are
  predicate-gated; the obligatory order-key column alone is 530 GB/s).
  32 registers, 9 KB dynamic smem, no spills; Plan B 34 registers, 33 KB smem.
- The in-tile expansion loop (Plan B) handled the maximum stage expansion of
  1,643 rows (M/T = 1.6) with no frontier spill and flat memory; the same
  order in srdatalog costs 58 s. Plan B's kernel is 3× faster than Plan A
  because ASIA customers prune 4/5 of lineitem before it is read.
- Build dominates: the dense counting-sort builder is ~10× off bandwidth
  (15 M orders + 1.5 M customers in 12 ms; lineitem by order key in 22 ms
  although dbgen already emits it sorted). Next targets: a sortedness check
  that makes clustered inputs free, and a bandwidth-bound counting sort.
- Dedup via a packed-u64 CAS set costs ~1 ms, dominated by readback and
  allocation boundaries, not hashing.

Toolchain limitations hit (details in `features/gpu/README.md` and
`features/gpu/compare/rccl-ptx-collision.txt`):

1. cuda-oxide names static shared globals `__shared_mem_{n}` per module and
   the host embedder concatenates PTX without namespacing; loading RCCL's
   radix sort (monomorphised into our crate) together with RCCL's own scan
   module fails `ptxas` with a duplicate `__shared_mem_0`. Needs a compiler
   fix (module-unique names) before RCCL device algorithms can be combined
   with user kernels.
2. Cross-crate `#[device]` helpers taking shared-memory pointers must receive
   `DynamicSharedArray::<u8>::get_raw()` casts; `SharedArray` arguments fail
   PTX address-space verification.
3. The non-generic `load()` binds the first package-named bundle, so a
   library and its oracle binary cannot live in the same package.

