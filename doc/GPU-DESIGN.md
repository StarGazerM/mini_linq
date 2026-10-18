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
| GPU Plan A, fact-first (build / kernel / dedup = 4.9 / 0.94 / 0.35 at SF10, rccl-memory pool, bitmap-word scans) | 1.9 | 6.2 |
| GPU Plan B, dimension-first expand (9.5 / 0.30 / 0.35) | 2.6 | 10.1 |
| srdatalog-python, **best of 14 orders** (`c,n,o,d,s`, clause order 3,4,1,2,0,5, Region/Nation as derived unary filters) | 5.2 | 176.8 |
| srdatalog-python, fact-first order (`o,s,c,d,n,r`, first baseline) | 38.1 | 574–583 |
| srdatalog-python, default plan | 38.3 | 633.5 |
| srdatalog-python, dimension-first order (`r,n,c,o,d,s`) | 2,872 | 58,051 |
| DuckDB 1 thread | 55 | 618 |
| DuckDB 24 threads | 21 | 154 |
| CPU MiniLinq default (doc/STATUS.md, other machine) | 33.7 | — |

srdatalog's sensitivity to variable order is itself a finding: 14 orders at
SF1 span 5.2 ms to 2.9 s, and four accepted orders (`o,d,c,n,r,s`,
`o,d,s,c,n,r`, `s,n,r,o,d,c`, `d,o,c,n,r,s`) compile but return wrong counts
(125 or 0 instead of 7,242) — recorded as rejected in
`features/gpu/compare/srdatalog_h05.sweep.json`, not used.

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


## Compiler divergence status (2026-10-16)

Landed on `gpu` through the public pipeline, each a recorded decision with an
independent check and no RA′/VO/Physical grammar change:

| Piece | Home | Formal rule |
|---|---|---|
| `cq_planners::order(le)`: lookups as non-introducing partial subatoms in GYO ear order, coarse expansions delayed to first use / Rust Before boundary / scope exit | features/planners | Plan-Le, CK-Le |
| `SortedRun<K,V,C>`: stable clustered payloads, CSR offsets, Copy `RunCursor`, bounded rank directory | runtime/cq-storage | STR-SortedRun |
| `cq_rewrites::sorted_runs`: retypes one-level builds with an integral codec | features/rewrites | EXT-SortedRuns |
| `p via cursor s;`: a probe expands the earlier semi's cursor, residual key columns filtered on payload; opt-in via `cq_rewrites::retained_lookups` | cq-lang-structures, realization | STR-RetainedCursor |

SF1 CPU evidence is in `doc/STATUS.md` (`order(le)`): 16–22 % faster where a
cursor is retained on an integral-key build (H03, H07, H07-block, H08), slower
where it cannot be (host-fallback keysets, composite `HashIndex` whose cursor
holds an `Rc`). The next CPU lever is a Copy cursor for `HashIndex`.

## `cq-lang-device`: grammar and contract (draft for review)

Shape: an external stacked language over the complete Structures program,
exactly like `features/stacked-ir`'s Trace over Physical: text is
`physical { … } structures { … } device { … }`; the local contract checks
Structures and the annotation; `Structures → Device` is a `cq_stage::pass`
with its own correspondence check; the GPU printer is the terminal
`Device → syn::Expr`. The language crate is plain Rust (syn) and lives in
`features/gpu`; only the printer's generated code needs the nightly pin. No
core crate depends on cuda-oxide.

```text
Device    ::= "device" "{" Tile? Column* Run* Filter* Kernel* Store* "}"
Tile      ::= "tile" Int "x" Int ";"                        // threads × items per thread (default 256 x 4)
Column    ::= "column" Ident "." Ident "=" Layout ";"       // host column on device
Layout    ::= "i32" | "i64" | "dict" Ident                  // strings are dictionary ids, dictionary named
Run       ::= "run" Ident "=" "Run" "[" Ident,* "]" Directory Blocks? Build ";"
Directory ::= "directory" ("rank" | "fences" | "none")
Blocks    ::= "blocks" Int
Build     ::= "build" ("counting" | "sort")                 // counting requires an integral codec in Structures
Filter    ::= "filter" Ident "=" ("Bitmap" | "Bloom" | "DeviceSet") "[" Ident,* "]" ";"
Kernel    ::= "kernel" Ident "=" "stream" Ident "{" Step* "}" Policy* ";"
Step      ::= "scan" Ident
            | "filter" Ident                                 // a Structures membership / join-filter read
            | "guard" Ident                                  // a total Rust guard, lowered to a device expression
            | "compact" Float                                // activity threshold θ
            | "lookup" Ident "[" Ident,* "]" "expect" "1"    // Physical probe first / semi on a run
            | "expand" Ident "[" Ident,* "]" ("from" Ident)? // Physical probe; `from` names the lookup whose descriptor it reuses (= via cursor)
            | "merge" Ident "[" Ident,* "]"                  // ordered block merge on a clustered stream
            | "intersect" Ident "," Ident "[" Ident "]"      // Physical probe smallest: merge-path on two ranges
            | "segment"                                      // materialise the tile stream (register budget)
            | "store" Ident                                  // the kernel's Physical result
Policy    ::= "carry" "[" Ident,* "]" | "gather" "[" Ident,* "]"
            | "frontier" Int                                 // K: spill descriptor when M > K·T; 0 = never
            | "registers" Int                                // budget the check enforces per kernel
Store     ::= "store" Ident "=" Capacity Dedup? ";"
Capacity  ::= "capacity" ("bound" Ident | "grow")           // |scan| × Π expect-1 is a bound; grow = relaunch unfinished tiles
Dedup     ::= "canon" "DeviceSet"                           // full-key compare; the Structures canon type is retyped
```

Contract (facts the check derives from Structures + annotation; each is a
rejection, not a default):

1. **Coverage.** Every Structures `let` build read by a kernel has a `run`;
   every `Keyed` key level of that build is the run's key; `directory rank`
   requires the build's recorded integral codec and `build counting` requires
   `rank`; every membership / join-filter read has a `filter`; every host
   relation scanned has `column` entries for each column the stream reads.
2. **Kernel ↔ pipe.** `kernel K = stream l` names a Physical `pipe` scan; the
   kernel's steps are the consumer chain of `l` in definition order, one step
   per Physical definition: `select`/`guard` → `filter`/`guard`, `semi` →
   `filter` or `lookup expect 1`, `probe first` → `lookup`, other `probe` →
   `expand`, `probe smallest` → `intersect`, `merge` → `merge`, `project` →
   `carry`/`gather`, the pipe's end → `store`. A Structures `p via cursor s`
   must appear as `expand … from <lookup of s>`; an `expand from` without the
   Structures entry is rejected (the decision is recorded once, in Structures).
3. **Budget.** The live set of a kernel at any step is the carried columns
   plus one `u32` per gathered column plus one descriptor per open expansion;
   `Σ bytes × ipt / 4 ≤ registers` or a `segment` precedes the overflow.
   `expand` and `intersect` require `directory ≠ none`; `merge` requires
   `blocks`; `compact` only after a filtering step.
4. **Output.** `capacity bound l` is admissible only when every expansion in
   the kernel is `expect 1` or reads a run whose Structures build is a
   projection of a functional (PK) key; otherwise `grow`. `canon DeviceSet`
   is admissible only where Structures records a canon boundary.
5. **Blocks, folds, recursion.** Union/fallback arms are separate kernels
   with an `anti` filter per fallback step; aggregate and recursion contexts
   stay external surface contexts (unchanged) — deferred, rejected for now.

Correspondence (`Structures → Device` pass check): erasing the `device`
block yields the source Structures program unchanged; every kernel's step
list erases to its pipe's definition chain; every run/filter erases to its
build/membership entry. Refinement: a kernel's stored set equals the Physical
pipe's set under the core premises; the printer's obligations (`Copy`
cursors, codec totality, dictionary completeness) are emitted as Rust trait
obligations like `cq-print` does, so an ineligible program fails rustc.

Authored fixture, retail_h05 Plan A (fact-first; Structures from
`order(le)` + `sorted_runs` + `retained_lookups`), abbreviated:

```text
device {
    tile 256 x 4;
    column Lineitem.l_orderkey = i64; column Lineitem.l_suppkey = i64;
    column Orders.o_orderkey = i64; column Orders.o_custkey = i64; column Orders.o_orderdate = i32;
    column Region.r_name = dict r_name;
    run orders_idx   = Run [order_key]    directory rank build counting;
    run customer_idx = Run [customer_key] directory rank build counting;
    run supplier_idx = Run [supplier_key] directory rank build counting;
    filter region_asia = Bitmap [region_key];
    kernel h05 = stream lineitem {
        scan lineitem;
        lookup orders_idx [order_key] expect 1;      guard order_date_window;
        lookup customer_idx [customer_key] expect 1;
        lookup nation_idx [nation_key] expect 1;      filter region_asia;
        lookup supplier_idx [supplier_key] expect 1;  guard supplier_nation_eq;
        store answer;
    } carry [order_key, supplier_key, nation_key] registers 64 frontier 0;
    store answer = capacity bound lineitem canon DeviceSet;
}
```

Plan B differs only in Structures (customer stream under `order(le)` with
`Orders`/`Lineitem` as expansions) and therefore in the kernel:
`scan customer; filter region_asia; expand orders_by_cust [customer_key];
guard …; expand lineitem_by_order [order_key]; lookup supplier_idx …` with
`run orders_by_cust = Run [customer_key] directory rank blocks 2048 build
counting;`, `frontier 4`, `capacity grow`.

Open points to settle before code: (a) whether `guard` lowering of total
Rust needs its own allowlist text in Device or reuses `cq-lang-cq`'s total
recognition verbatim (reuse); (b) whether `frontier`/`registers` are per
kernel or per program (per kernel); (c) whether `column … = dict` belongs in
Device or in a load-time dictionary annotation shared with CPU positional
hosts (shared, if `cq-load` gains dictionary columns).

## Later: the spine (Differential Dataflow) and its effect on joins

Reference structure: Differential Dataflow's `Spine` — an arrangement's trace as
a list of immutable, key-sorted batches of geometrically growing size, read
through a `CursorList` that merges per-batch cursors in key order, physically
compacted by merging adjacent similar-size batches, logically compacted only
up to the frontier readers have released. srdatalog's FULL/HEAD is a naive
two-batch approximation (compact at 10 %, re-sort, no directory). Upstream's
`Runs⟨S⟩` (incremental.tex, STR-Maintain) already has the spine's shape:
immutable runs, exact-once insertion, merges never crossing a protected cut
(= DD's logical compaction frontier), `cut_u` (= a cursor as of time u).

Consequences for joins, to be designed with the recursion experiment:

1. Joins are cursor merges over spines: DD's `join_core` walks two
   `CursorList`s in key order; a lookup is `seek_key` across all batches. The
   sorted-run representation is the one that matches; a per-batch hash/rank
   directory is an accelerator, not the structure. A key's witness is the
   `CursorList` state — one range per batch — so the `witness { b [K] at s as
   Type }` entry must admit the trace's Copy cursor (bounded array of ranges).
2. Semi-naive is batch × trace: the new batch of R against the trace of S, and
   the new batch of S against the trace of R as of before this batch (time
   order prevents double counting). The device structure must expose a trace
   cursor as of a boundary without copying.
3. Compaction is explicit and twofold: physical (merge adjacent similar-size
   batches; recorded threshold) and logical (only past the released frontier =
   protected cuts). Both are merge-path over sorted batches; neither re-sorts.
   A merged membership bitmap (OR of per-batch bitmaps, prefix recomputed)
   keeps `seek_key` misses at one load — the dedup-against-trace case.
4. Diffs: DD batches carry `(time, diff)`; under set semantics with
   insert-only increments (incremental.tex Option A) diff is always +1 and
   dedup is a miss test.

Unchanged: RA′/Physical (one keyed read per build), R/E/C placement (the spine
is an R fact of the build; multi-range witnesses are the same E sharing), and
the tile kernels (segmented flatten handles variable segment counts).
