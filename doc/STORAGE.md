# Storage: compressed blocks, summaries, scans and merges (contract 8)

Status: design, 2026-10-28. Governs `runtime/cq-load`, `runtime/cq-storage`,
`experiments/gpu/{compress,device-columns,datalog-runtime,sequence}` and the
compiler passes that plan over them. Contract text: `doc/PLANNING-REDESIGN.md`
contract 8. Sources (read for this design; cite them, do not paraphrase from
memory):

| Id | Source | Used for |
|---|---|---|
| D | Lang et al., *Data Blocks*, SIGMOD 2016, §§3.1–3.4, 4.1 | block = codec + SMA + PSMA + payload; prune → predicate on codes → unpack matches; hot/cold freezing |
| M | Moerkotte, *Small Materialized Aggregates*, VLDB 1998, §§2–4 | summaries per physical bucket; qualifying/disqualifying/ambivalent grading |
| B | Kuschewski et al., *BtrBlocks*, SIGMOD 2023, §§2–3; artifact `SchemePicker.hpp` | per-block codec choice by sampling; cascades; statistics are orthogonal to compression |
| F | Afroozeh & Boncz, *FastLanes*, PVLDB 16, §§1–2.4 | 1024-value vectors; decode to the smallest processable type; fused unpack+decode |
| Z | Zukowski et al., ICDE 2006, §3.1 | PFOR/PDICT; decode per vector into a reused cache-resident buffer |
| G | Shanbhag et al., SIGMOD 2022; artifact `binpack_kernel.cuh`, `q11r.cu:70-137` | GPU tile decode fused into the query kernel |
| K | DuckDB `/data/codex/duckdb-src`: `column_data_checkpointer.cpp:192-352`, `row_group.cpp:775-955`, `compression/{bitpacking,rle,dict_fsst}.cpp`, `physical_hash_join.cpp:1837-1966` | analyze→choose→compress; CheckZonemap before scan; filters on runs/dictionary entries; join min/max/IN/Bloom into probe zone maps |

Existing formal anchors this design extends: `backend.tex` R (STR-Repr, data
refinement with abstraction α), `backend_ir.tex` BE-Alpha, BE-Code,
BE-RustRepresentation, GPU-Representation, `incremental.tex` STR-RunRead,
STR-RunSnapshot, STR-RunCompact, DL-DeviceRatio.

## 1. Does this change semantics?

**No new observable, and nothing above Structures changes.** Every level through
Structures denotes `T X = ⊥ + Set_fin(X)` (`backend_ir.tex` §"Why one level").
Compression, summaries, pruning, encoded predicates, late materialization and
merges are all *representation*: they change ρ and how α is computed, never
`supp ∘ α`. Each mechanism is correct only under a law. Those laws, and the
question of which predicates may be evaluated on codes, are where semantics
actually sits:

| Mechanism | Law (premise) | Where it sits | Who discharges it |
|---|---|---|---|
| Block payload | `decode ∘ encode = id` on the column's values | Structures R: a new ρ = `blocks` with α = concatenation of decoded blocks (STR-Repr extension) | codec implementation; tested per codec (round-trip property) |
| Summary prune | summary of a block **encloses** every value in it (min ≤ v ≤ max under the column's Eq/Ord; strings: truncated prefixes rounded conservatively, as DuckDB `string_stats.hpp:26-65`) | Sequence: pruning is `compact` over blocks; BE-Alpha extension | built in the same pass that writes the payload (§2); never from source-file statistics |
| Predicate on codes | for `where x op c`: `op(dec a, c) ⇔ op'(a, enc' c)`. Equality needs `enc` injective. Order needs `enc` monotone. | contract 7 `where` denotation "selection by the representation's Eq/Ord" made concrete: the codec declares which ops it preserves | codec declares `preserves = {eq}` or `{eq, ord}`; compiler emits code-domain evaluation only when the op is in the set, else decodes first |
| Constant translation | `enc'(c)` for a constant absent from a dictionary: equality → empty match; order → rank via lower bound (ordered dictionaries only) | same as above | host, once per invocation (closed operand, contract 7) |
| Narrow carrier | decoding to the smallest type that holds the column's codes is lossless | Sequence/target representation (BE-Code + layout) | representation width fact; never type spelling |
| Late materialization | positions are witnesses, not logical columns (`ir_structures.tex:215-219`) | existing STR-Repr positions | already law |
| Merge/compaction | output blocks represent the same multiset (STR-RunCompact); dedup only where set semantics is recorded (canon) | Storage/Round maintenance (incremental.tex) | merge kernel; summaries recomputed per output block |
| Join filter on summaries | exact keyset `any_in(lo,hi)` is exact; min/max or Bloom only *admit* (false positives), the exact probe stays | existing STR-JoinFilter | unchanged |

The **only** genuinely semantic decision is the codec's preserved-op set.
`where x < c` on a column whose codec preserves only equality (a first-intern
symbol dictionary; `device-columns/src/encoding.rs:4-24` says IDs preserve
equality, not order) must decode before comparing. Comparing codes there
would silently change answers. This is recorded per column representation and
checked; it is not inferred from a type.

## 2. Runtime data structures

One **logical layout and byte layout** shared by every reader (CPU, CUDA,
Metal); **memory placement and ownership are per target** (§11). Arrays:
```
Table   = { rows: u64, block_rows: B, columns: [Column] }      // columnar, never row tuples
Column  = { codec_set: CodecSet, blocks: [Block], summary: Summary /*whole column*/ }
Block   = { rows: u32, codec: CodecId, payload: Bytes,
            summary: Summary,                 // SMA: min, max (+count); exact for ints/dates/decimals,
                                              // conservative prefix for strings
            psma: Option<Psma>,               // D §3.2, built only if requested by advice
            dict: Option<DictRef> }           // per-block or shared, see §5
Vector  = 1024 values inside a block (F); tile = the decode unit a GPU block owns (G)
```

- `B` (block rows) is a multiple of the vector size so tiles never straddle
  blocks. Pruning granularity = block; decode granularity = vector/tile
  (D uses ≤2¹⁶-row blocks with 8192-row scan vectors; these are distinct).
- Codec trait (runtime, generic): `decode_vector(&Block, v, &mut [Code])`,
  `decode_tile` (GPU, cooperative), `encode_block(&[Value]) -> Block` that
  also produces the summary, `preserves() -> {Eq, Ord}`,
  `translate(op, c) -> CodeConstraint | Empty | All`, `size_estimate`.
  Codecs: plain, constant, FOR (bit-packed, per-vector base/width), delta-FOR,
  RLE, dictionary (ordered or equality-only), with BtrBlocks-style cascading
  later.
- `Code` is the narrow carrier type (u8/u16/u32/u64) chosen by the codec,
  not the logical type.
- **Build** (`encode_block`) reads a source slice/batch once and writes
  payload and summary together (K builds stats during compression,
  `bitpacking.cpp:539-550`). Loader stores columnar blocks; rows are never
  decoded into owned tuples (`cq-load/src/zone.rs` today does exactly that
  and is replaced).
- **Copy vs read** (contract 8): scans and index builds borrow blocks. The
  `Build` protocol gains a re-readable source form (`&Table`, column + block
  range); the one-shot iterator form stays only for recorded copy reasons:
  source not retained; mutable between rounds; computed key; reorganization
  (sort/group); and the target's memory reason (§11: crossing PCIe on CUDA;
  a source not allocated as a Metal buffer on Mac).

## 3. Scan pipeline (fixed order)

1. **Prune**: for each block, test the conjunction of `where` predicates
   (and join-filter intervals) against block summaries; three outcomes as in
   M §3: disqualified (skip), qualified (all rows match: no per-row test),
   ambivalent (test rows).
2. **Encoded predicate** (ambivalent blocks): per codec, either evaluate the
   translated predicate on codes (FOR: residual compare against `c − base`;
   dictionary: test each entry once, then codes; RLE: test each run once, as
   K `rle.cpp:561-625`), or decode the vector/tile and test (fused).
3. **Positions** of matching rows (a selection vector per vector/tile).
4. **Late materialization**: decode only the columns still needed, only at
   the selected positions.

Opaque Rust `if` (contract 7) runs after step 4 on decoded values. On GPU it
runs on device only for identity-represented columns (BE-RustRepresentation).

CPU positional scans retain this order when a static predicate and a join
filter share a read. Stable identity facts are queried once at construction;
the iterator selects an identity mode outside its row loop. Each selected
loop omits identity key extraction and tests the static predicate before
extracting a surviving row's join-filter key.

## 4. Compiler support (per level)

| Level | Addition | Recorded fact | Consumer |
|---|---|---|---|
| Structures | host column representation `blocks { codec_set, block_rows, summary, psma? }` per host relation column | ρ = blocks, α = decode-concat (STR-Repr extension: STR-Blocks) | selection; Sequence lowering |
| Structures selection | for each `where` on a blocked host column: `prune(summary)`, `encoded(op)` if `op ∈ preserves`, else `decode-then-test` | per-predicate access decision | printers |
| Sequence | blocked sources: `rows R` over a blocked host lowers to `flatten (map decode_v (compact prune_θ (blocks R)))`; tiles align to vectors | BE-Alpha extension (BE-Blocks): α(blocks R) = α(R); α(compact prune_θ b) ⊇ blocks with matches (soundness by enclosure) | grading, GPU emitter, pull printer |
| Sequence representation | width of each column's code carrier; codec preserved ops | `representation { identity …; encoded …; width …; preserves … }` | layout/resource accounting, BE-RustRepresentation, encoded predicates |
| GPU emitter | tile-cooperative decode (existing `decode_stripe`) fused with the predicate; summary skip per vector; narrow lanes from recorded widths | consumes the above; decides nothing (T-Gpu) | — |
| Datalog Storage/Round | runs as sequences of blocks; maintenance merges blocks (§5) | layout protocol + codec set per run | round printers |

No type inspection anywhere: codec sets, widths and preserved-op sets are
representation facts produced by the loader/binder catalog and recorded in
IR (contract 7).

## 5. Merging compressed indexes

Two data regimes need different maintenance; both keep α exact.

**Static tables (TPC-H catalog).** Built once; no merge. Codec per block is
chosen at load by measured size (K analyze, B sampling).

**Growing relations (Datalog full/delta, leveled runs; incremental.tex
STR-Run*, DL-DeviceRatio).**

- *Tail + frozen blocks* (D §3 hot/cold; K transient vs persistent
  segments): a round's new rows accumulate in an uncompressed tail with a
  running summary. When the tail reaches `B` rows it freezes into a
  compressed block (codec chosen then). Readers of round t see frozen blocks
  plus the tail snapshot (STR-RunSnapshot unchanged).
- *Run merge* for **scan-column** runs (index runs stay plain and merge by
  merge-path, §8) (DL-DeviceRatio threshold unchanged): a k-way merge streams
  the input runs **tile by tile**: decode a tile of each input into
  shared memory/cache, merge (GPU: merge-path partitions, as `runs.rs:351`
  already does on plain words), dedup equal rows only where the run is a set
  (canon recorded), and feed an **output encoder** that buffers one block,
  chooses its codec by size, writes payload + summary, and emits it. Inputs
  are never fully decoded; output summaries are recomputed per block, never
  merged from input summaries (input blocks don't align with output blocks).
- *Sorted runs give summaries for free*: for the sort key, each block's
  summary is its first and last key (LSM fence keys). A lookup binary-searches
  fences, then decodes one block. Non-key columns keep min/max summaries.
- *Dictionaries under growth*: not a codec-local choice. Code assignment is a
  write to shared state that every parallel worker, partition and run
  depends on; see §10.
- *Delta-encoded columns*: merges re-encode from decoded tiles; delta state
  never crosses block boundaries (restart per block/lane, as F §2.3).

Merge correctness is STR-RunCompact (same multiset; set only under canon)
plus per-block encode laws (§1). Merges never change admission, multiplicity
or snapshot publication.

## 6. What changes in the current code

| Today | Contract 8 |
|---|---|
| `cq-load::ZoneRows` owns decoded row tuples with per-block min/max (`zone.rs:9-15`) | columnar blocks; summaries over the stored payload |
| GPU catalog fixes FOR for every column (`compress/src/lib.rs:186-193`); `detect_integer_codec` unused | per-block codec choice by size; codec set recorded |
| GPU vector headers compute min/max then drop max (`device-columns/src/kernels.rs:33-79`) | keep per-vector summary; use it to skip |
| per-row `decode(view,row)` into an `i64` carrier (`sequence/core/src/rows.rs:1030-1055`) | tile-cooperative decode fused with predicates; narrow carriers |
| `AdaptiveKeySet` copies keys (`Vec<K>` staging, `Arc<K>` fallback) | read-in-place builds over blocks; inline keys when a copy is required |
| zones only under `hypergraph_zone` advice (`STATUS.md:350-352`) | summaries are part of every blocked column; pruning is the default scan step |
| run merges on plain words | index runs: merge-path over plain arrays + directory rebuild (§8); scan columns: tile-streamed re-encoding (§5) |


## 7. Implementation slices (each measured against current TPC-H/DOOP)

1. Runtime block format + codec trait + summary build (CPU), round-trip and
   enclosure property tests; loader writes columnar blocks.
2. Structures/Sequence representation facts (STR-Blocks, BE-Blocks,
   preserved ops, widths) + selection of prune/encoded/decode per `where`.
3. CPU scan pipeline (prune → encoded predicate → positions → late
   materialization) in the pull and task printers.
4. GPU: device blocks with per-vector summaries, tile decode fused with
   predicates, narrow lanes, summary skip.
5. Join filters through summaries on both targets.
6. Datalog: tail+frozen blocks, tile-streamed merges, dictionary policy.
7. Index runs (§8): adopt RadixRun as the index-role provider on all
   targets after merging the remote branch; retire AdaptiveKeySet staging/Arc.

## 8. Index runs vs scan columns (RadixRun evidence, 2026-10-28)

Source: remote `origin/codex/all-answers` (not yet merged locally):
`runtime/cq-storage/src/{radix_run,radix_spline}.rs`,
`experiments/mac/mccl/src/radix.rs`, `doc/MAC-IMPLEMENTATION.md` §§ around
lines 1530–1700 (M3 Max measurements). RadixSpline: Kipf, van Renen, Stoian
et al. (aiDM 2020), native port of revision `ab96aa59` (radix12/error32).

`RadixRun<K,V>` = sorted **grouped** keys (`keys: [K]`, one per distinct key),
`offsets: [usize]` (group starts), `items: [V]` (payload column), a
**RadixSpline directory** (radix-12 table + spline points, max error 32), and
a lazily built dense presence bitmap (only when span ≤ 2²⁰ bits and ≤ about one
byte per key). Keys are stored at their own width `K`; `RadixKey` is an
injective, order-preserving encoding into `u64` used only to compute.

Measured (M3 Max): lookup + expansion, 262,144 distinct keys, 65,536 probes,
~50% hits, ms: native RadixRun 2.0–3.4 vs a *packed-occurrence* run with a
coarse `key >> 20` bucket directory 3.9–18.5 (up to ~7× slower). Dense
membership 0.056 vs 3.59 (packed) vs 0.52 (hash). GPU build 1.7–11 ms vs CPU
18–987 ms; GPU merge of a 10% delta 0.9–10 ms (1M–16M rows).

**How this fits contract 8, and what it corrects:**

| Contract 8 principle | RadixRun | Verdict |
|---|---|---|
| payload + summaries built together, never from a decoded copy | directory, min/max, bitmap built with the run; readers borrow slices (`RadixRunView` is allocation-free) | follows |
| summaries enclose / locate values | the directory is a *positional* summary: a PSMA generalization (D §3.2) with a bounded-error law (every present key lies within ±32 of the predicted rank; an exact search inside the window) | follows; adds a new law, bounded error, which construction must guarantee and tests must check |
| order-preserving encodings as a recorded law, no type inspection | `RadixKey` trait: injective, monotone; wider/composite domains need another provider, never truncation | follows (trait-provided, contract 7) |
| merge recomputes summaries per output | partitioned merge of sorted arrays, then rebuild grouping and directory; old cursors stay valid (STR-RunSnapshot) | follows |
| **codec per block chosen by measured size** | grouping (RLE of the sorted key column) is the only compression; payload stays plain, byte-addressable | **contradicts §2 for index runs** |

The measurements say what Data Blocks already says ("only byte-addressable
compression formats … to allow for efficient point accesses", D §3 p.3): a
packed, scan-oriented representation loses badly on lookup + expansion and
on merge. So contract 8 distinguishes **two storage roles**, recorded per
structure:

1. **Scan column** (base relations scanned with predicates): blocked,
   bit-packed FOR/delta/dictionary/RLE chosen per block by size, min/max per
   block, prune → code-domain predicate → late materialization (§§2–3).
2. **Index run** (lookup/probe/expand targets, Datalog runs): sorted and
   grouped (key RLE), payload byte-addressable (plain, or byte-width
   truncation per D §3.3 when a measurement shows it pays), positional
   directory (RadixSpline, or the GPU exact radix-prefix directory;
   `MAC-IMPLEMENTATION.md` says to choose these by matched build + read
   measurements), lazy bounded dense membership. Merge = merge-path over
   plain arrays + directory rebuild. Never bit-packed.

The role is a representation fact chosen by selection from the plan (scan
vs lookup/expand demand), not by the loader. The same relation may hold both
(base scan column + index run built from it, reading it in place). This also
supersedes `AdaptiveKeySet`/`AdaptiveMembership` as the default exact
membership/interval provider for index roles: the runtime's eager 8 MiB bitmap,
`Vec<K>` staging and `Arc<K>` fallback become RadixRun's lazy bounded bitmap
over a grouped run that already exists.

Merging the remote branch is a prerequisite for implementation. It contains a
provider-contract refactor that also removes RadixRun name-specific checks,
which overlaps contract 7's type-blind provider selection. Merge after the
current work is committed, reconciling on the type-blind design.

## 9. Grouped runs in the compiler's semantics

**A RadixRun is compressed, by factorization rather than by a codec.**
Grouping stores the relation `R(k, v)` as `⋃_k {k} × V_k`: each distinct key
once, its payloads contiguous. α is the expansion
`α(keys, offsets, items) = {(keys[g], items[j]) | offsets[g] ≤ j < offsets[g+1]}`
(a factorized representation in the d-representation sense; L&E §3.3 notes
hash tables already factorize the build side on the join key). The directory
is an access summary on top of it. The current layout still spends
uncompressed width where a byte-addressable encoding would not cost point
access: `RunRows.offsets` is `Vec<usize>` (8 bytes per group, origin
`sorted_run.rs:19-23`), and `items: [V]` stores the whole payload row-wise.

**It is already in the IR's vocabulary.** Sequence's `lookup P [k]` yields a
match range and `flatten` expands it (BE-Bind). BE-Offsets' offset array
`o = scan(+,0)(|t_0|,…)` is exactly `RunRows.offsets`. So a grouped run is the
*stored* form of "group by key, keep the flattened offsets": a published
range provider whose offsets are precomputed rather than scanned (compare
the GPU deferred-range dispatch, which computes them with a scan at query
time). Semantics stay at Structures (ρ = grouped run, α = expansion;
STR-Repr extension) and Sequence (BE-Bind, BE-Offsets). Nothing above
changes.

**Laws:** (1) factorization α above (keys strictly increasing under the
key's order law; offsets nondecreasing, last = |items|); (2) directory
bounded error: `|predict(k) − rank(k)| ≤ ε` for every present key, with an
exact search in the window; (3) membership bitmap exact over `[min, max]`;
(4) merge preserves the multiset (STR-RunCompact), dedup only under canon.
Building a run requires reorganization (sort/group), which is a legitimate
reason to write a new layout (a fifth copy reason, alongside §2's four).

**What DuckDB and the papers add, applied to grouped runs:**

| Lesson (source) | Applied to a grouped run | Compiler fact |
|---|---|---|
| Evaluate a predicate once per run/dictionary entry, not per row (K `rle.cpp:561-625`, `dict_fsst.cpp:177-235`) | `where k op c` on the key becomes a contiguous **group range** via two directory lookups; no per-row test. A predicate on `k` tested per distinct key, not per payload | access decision `key-range` on the run (like today's Range read) |
| Expand late / emit dictionary-constant vectors instead of expanding (K dictionary vectors; L&E) | consumers that need only `k`, membership or a count use `(group, offsets)` without touching `items` (semijoin, exists, count = offset difference); expansion only where payload columns are read | Sequence keeps the range unexpanded (`lookup` without `flatten`, or `fold count` over offsets) |
| Byte-addressable narrowing, not bit-packing, for point access (D §3, §3.3) | offsets as u32 when rows < 2³², keys/items truncated to 1/2/4 bytes relative to the run minimum when measured to pay; never sub-byte | per-component width facts in representation (contract 7: widths from representation, not types) |
| Late materialization by column (K `row_group.cpp:1035-1088`; D §3.4) | store `items` columnar (one array per payload column) so expansion reads only needed columns | payload column layout recorded per run |
| Zone maps on non-key columns (K `CheckZonemap`; M §3) | per-block min/max over each payload column, so `where v op c` after a lookup skips blocks inside large groups (duplicate-heavy keys) | summary entries on payload columns, used only when groups are large |
| Join filters from the build side (K `physical_hash_join.cpp:1837-1966`) | a run as build side gives exact `[min,max]` (first/last key) and exact membership for free; push both into probe-side scan pruning | STR-JoinFilter with exact keys from the run, on every target |
| Codec choice by measured size (K checkpointer, B sampling) | applies to *scan columns*, not to run components that must stay byte-addressable; offsets/keys may use delta-FOR only in scan-only copies | role fact (§8) decides |
| Exact statistics for planning (K stats; PL-Evidence) | a run knows key count, row count and max group size exactly at build; publish them as evidence for runtime `choose` (BE-Choose) and grading | published lengths/evidence facts |

None of these adds an observable. Each is either a representation law above,
an access decision recorded in Structures/Sequence, or published evidence.

## 10. Dictionaries and parallel behavior (open decision)

Code assignment is a **write to shared state** read by every worker,
partition and run. So a dictionary policy is a parallel-execution decision
first and a compression decision second.

**Today (evidence).**

| Dictionary | Assignment | Order | Writers |
|---|---|---|---|
| DOOP `IStr` (`features/datalog/doop/shared/src/interner.rs:35-51`) | first sight, process-global `Mutex` | `IStr` derives `Ord` **on the code** | every `sym()` call locks; loading is sequential today (no Rayon in `doop/shared`), so codes are deterministic only because nothing interns in parallel |
| GPU DOOP (`experiments/gpu/doop/src/lib.rs:3-5`) | same process-global dictionary | kernels see codes only | host only |
| device-columns `Dictionary` (`encoding.rs:4-24`) | first sight, `&mut self` | equality only (documented) | single writer |
| cq-load `StringDictionary` (`intern.rs:1-4`) | lexicographic rank of the full distinct set | order-preserving, deterministic, independent of row order | built once (static) |

**What depends on code values.** Each row is a parallel mechanism the policy
must keep valid:

1. Code-domain joins and membership need **one dictionary per domain**, shared
   by both sides. Per-column or per-block dictionaries force translation.
2. Sorted runs, merge-path and RadixRun sort by code. Merges across rounds
   need **stable codes**: re-coding means re-sorting every run.
3. Partition routing (key-partition shards, sharded membership, GPU hash
   partitions, MCCL partitions) routes by code. Stable codes keep partitions
   stable across rounds.
4. Dense structures (bitmaps, `DenseOrdinal`, RadixRun dense membership)
   need **dense codes** in `[0, n)`. Sharded or gapped code spaces break
   density.
5. `where` order predicates need order-preserving codes, or they decode
   (§1). `IStr`'s `Ord` is code order, so its order is assignment order: a
   representation order, which is legal under contract 7 but is not string
   order.
6. Determinism: first-sight codes under parallel interning depend on
   scheduling. Set answers do not change (only Eq/hash are used), but
   layouts, partition balance, merge shapes, checksums of ordered output and
   timings vary between runs.
7. Summaries on code columns: min/max over codes is valid for code-equality
   probes and join filters under any stable assignment.

**Policies and their parallel behavior.**

| Policy | Parallel build | Stable | Dense | Ordered | Deterministic | Joins on codes |
|---|---|---|---|---|---|---|
| A. first sight under a global lock (today) | serializes on the lock | yes | yes | no | only if single-threaded | yes |
| B. **assign at publication points**: workers intern into local provisional tables; at load end / round end the new strings are sorted and appended; that round's delta is remapped in one pass fused with its dedup/sort | no lock in hot loops; one barrier per round (already exists) | yes | yes | ordered within each epoch, not globally | yes | yes |
| C. global order-preserving (cq-load today) | parallel sort + dedup | only for static domains (a new value between two codes forces global re-coding) | yes | yes | yes | yes |
| D. per-block local dictionaries (Data Blocks §3.3) | fully independent per block | per block | per block | yes | yes | needs translation per block |
| E. order-preserving with gaps (Binnig et al., SIGMOD 2009) | needs coordination | until gaps run out | **no** | yes | yes | yes |

**Direction (to be measured before adopting):** code assignment is a
*publication*, so it belongs where every other cross-worker write already
is: round publication (Datalog Round/LayoutProtocol) and load completion.
- Static domains (TPC-H): C, as cq-load already does.
- Growing domains (Datalog): B.
- Scan-only string columns with order predicates and no joins: D is
  allowed.
- E rejected (breaks density).
- Closed constants (`sym("main")`, contract-7 closed Rust) are interned at
  invocation preparation, a publication point. Translating a constant for a
  `where` never inserts: an absent constant means an empty match (§1).

**Measurements that decide it:**
- lock contention of A under a parallel loader;
- the cost of B's remap per round (DOOP: rounds that create new symbols, and
  how many);
- GPU symbol creation (device hash with atomics gives provisional codes,
  canonicalized at the round barrier);
- run-to-run determinism of layouts and checksums under A vs B.

## 11. Targets: shared layout, per-target memory

Shared by all targets: the representation and its laws (§1, §9), and the
**byte layout** of every array, so any processor can read what another
built. Evidence: on Mac, CPU lookup reads completed Metal allocations
directly with no second host copy, and the same native CPU RadixRun reader
runs on CPU-built and GPU-built arrays (`MAC-IMPLEMENTATION.md`, remote,
around lines 361 and 1678).

Per target: memory placement and ownership, recorded as target facts next to
launch facts (Sequence layout/resources), never chosen by printers.

| | CUDA (discrete) | Metal (unified) |
|---|---|---|
| Allocation | host `Vec` / `cudaMalloc`, separate copies | Metal buffers (shared storage) read by both. No-copy wrapping needs a page-aligned region within one VM allocation; an arbitrary `Vec` is not valid (MAC doc ~819-822) |
| Copy reason at the boundary | crossing PCIe | none for shared buffers. A copy happens only if the data was not born in a Metal buffer, so loaders should allocate into Metal buffers |
| GPU-only intermediates | device memory | private storage is lawful for scratch and merge partitions; not measured as reliably faster (MAC doc access-role table ~826-833) |
| Synchronization | stream/event; download after completion | completion before publication is still required; unified memory does not permit concurrent mutation (~807-809) |
| What compression buys | PCIe transfer + VRAM capacity + bandwidth | bandwidth only, shared by CPU and GPU (matters for hybrid CPU+GPU execution); capacity is shared with the host |
| Codec constraint | device decoder suffices for device-only data | a structure read by both CPU and GPU needs decoders on both |
| CPU/GPU placement cost | transfer dominates | no transfer; hybrid placement is cheap; cost model from `experiments/mac` (mac-cost-model) |

So codec choice and placement are per-target cost decisions over shared laws.
Two binaries can disagree on whether to compress a column; they cannot
disagree on what a code means.

## 12. Evidence status: what experiments support, what they refute

The design above is a hypothesis until measured on each target. Codec and
placement choices are **performance decisions with target-specific cost
models**. They require controlled experiments; size alone (DuckDB's disk-
oriented `analyze` rule) is not an adequate objective in memory.

### 12.1 Data-property measurements (TPC-H SF1, dbgen order, 2026-10-28)

Source: `features/workloads/target/bench/data/tpch-1/dbgen.duckdb`. Columns
are exported to `target/storage-bench/tpch-sf1/` for the benchmarks.

**Zone-map pruning on literal predicates.** Disqualified/qualified block
fractions; every listed predicate gives 0.000/0.000 at B = 1024, 16384 and
122880 (DuckDB row group). Examples:

| Predicate | Selectivity | B=1024 | Same column sorted, B=1024 |
|---|---:|---|---|
| Q6 `l_shipdate` in 1994 | 0.152 | 0.000/0.000 | 0.848/0.151 |
| Q14 `l_shipdate` 1995-09 | 0.013 | 0.000/0.000 | 0.987/0.013 |
| Q4 `o_orderdate` 1993Q3 | 0.038 | 0.000/0.000 | 0.961/0.038 |
| Q6 `l_discount` .05–.07 | 0.273 | 0.000/0.000 | 0.727/0.273 |

**Join-filter block skipping** (`l_orderkey` against qualifying orders):
`corr(o_orderkey, o_orderdate) = −0.0005`. A min/max filter spans 100% of the
key range. An exact keyset skips 0.02% (Q4), 0.00% (Q5) and 3.7% (one-month
filter, B=1024) of lineitem blocks.

⇒ **Refuted for TPC-H in load order: summaries and join filters skip almost
nothing.** Skipping needs clustering (a sort or partition order chosen as
physical design), which then makes pruning ≈ selectivity. Summaries stay
cheap to build. Their benefit is workload- and order-dependent, so they are
selected, not default.

**Compressibility** (mean bits per value, 1024-value blocks):

| Column | Plain bits | FOR | delta-FOR | global dict | best/plain |
|---|---:|---:|---:|---:|---:|
| l_orderkey | 64 | 10.5 | 5.0 | 21 | 0.078 |
| l_partkey | 64 | 18.0 | 19.0 | 18 | 0.281 |
| l_suppkey | 64 | 14.0 | 15.0 | 14 | 0.219 |
| l_shipdate | 32 | 12.0 | 13.0 | 12 | 0.375 |
| l_quantity (cents) | 64 | 13.0 | 14.0 | 6 | 0.094 |
| l_discount (cents) | 64 | 4.0 | 5.0 | 4 | 0.062 |
| l_extendedprice | 64 | 24.0 | 25.0 | 20 | 0.312 |
| o_orderkey | 64 | 12.0 | 5.0 | 21 | 0.078 |

⇒ **Supported:** 3–16× fewer bytes are available; most of it is reachable
byte-aligned (2- or 4-byte truncation). Sortedness is the strongest property:
`l_orderkey` and `o_orderkey` are sorted, so lineitem already *is* a grouped
run on `l_orderkey` in load order (§9), at zero build cost.

### 12.2 Is scan bandwidth the bottleneck? (existing measurements)

| Query | CPU SF1 total (`performance_cpu_sf1.tsv`, default) | GPU SF1 / SF10 total (`performance_gpu.json`, advised) | Plain-bytes floor |
|---|---:|---:|---|
| h06 | 25.0 ms | 7.4 / 13.4 ms (≈all in reduce/dedup phase) | CPU ≈168 MB at ~15–20 GB/s per core ≈ 8–11 ms; GPU SF10 ≈1.7 GB at 960 GB/s ≈ 1.8 ms |
| h01 | 2676 ms | 66 / 592 ms (dedup/grouping) | far below; String grouping dominates |

⇒ On both targets the measured time is **not** dominated by scan bytes for
these queries. GPU is 7–20× above its bandwidth floor and its time sits in
reduction/grouping. CPU h01 is dominated by `String` group keys, where
dictionary codes help as *encoded execution* (group/compare on codes), not as
bandwidth. Compression for bandwidth can pay only after those costs drop.
Capacity (GPU SF100, Mac shared memory) and PCIe (non-resident) are separate
reasons, measured separately. [INFERENCE from totals; per-phase profiles in
§12.3 decide.]

### 12.3 Required experiments (per target: CPU t1/t24, CUDA RTX 6000 Ada; Metal on M3 Max by the Mac checkout)

Protocol: resident data; exact result check on every sample; ≥5 warmups,
≥21 samples; variants interleaved (ABAB) to cancel drift; median with
bootstrap 95% CI and ratio CIs; report clocks, other GPU tenants
(`nvidia-smi`) and machine load; real TPC-H columns plus synthetic width
sweeps.

| Id | Question | Variants | Output (cost-model parameter) |
|---|---|---|---|
| M1 | Scan + predicate + late-materialized sum | plain i64/i32, byte-truncated, FOR bit-packed (widths 4–24), dictionary codes; selectivity 1/15/50% | ns/value and effective GB/s; the crossover where decode cost exceeds bytes saved |
| M2 | Lookup + expand on a grouped run | payload plain vs byte-truncated vs bit-packed (same directory); offsets u32 vs u64 | probe latency/throughput; confirms or refutes "index runs never bit-packed" without the directory confound in the Mac MacRun comparison |
| M3 | Merge two sorted runs (Datalog) | plain words vs decode-merge-re-encode | merge GB/s; per-round re-encode cost |
| M4 | Transfer | plain vs compressed over PCIe + device decode | when compression pays for non-resident data |
| M5 | Encoded execution | group/compare on dictionary codes vs `String` (CPU h01-like) | group-by throughput |
| E1 | End-to-end | GPU TPC-H with plain columns vs current compressed default | whether today's GPU compression default pays (no prior controlled comparison exists) |
| E2 | End-to-end | per-phase profiles (CPU `perf`, GPU nsys/ncu) of h01/h06/h12/h14 | where time goes before optimizing storage |

A codec/role rule enters the planner only with an M/E result that supports it
on that target. Otherwise the default stays plain.

### 12.4 Results (2026-10-28; `experiments/storage-bench/results/{cpu,gpu}-2026-10-28.{md,json}`)

Machine: Threadripper PRO 5945WX (12c/24t); RTX 6000 Ada. Every sample was
checked exact; ratios are time/plain with bootstrap 95% CIs. GPU: 268
variants, 6,968 checks, no foreign tenants. CPU: 476 measurements.

| Experiment | CPU t1 | CPU t24 | CUDA (resident) |
|---|---|---|---|
| M1 Q6 real columns (SF1 / ×10) | bytes 0.651 / 0.641; plain hand scan 23.8 ms | bytes 0.480 / 0.573 | FOR 0.868 / 0.720; dict 0.881 / 0.737; plain 0.10 / 0.85 ms |
| M1 width sweep | bytes wins ≤16 bits, loses ≥20; **FOR loses at every width 4–32**; dictionary reconstruction loses everywhere | bytes and FOR win at every width | 60M values: bytes/FOR win at all widths; 6M: lose ≤16 bits (cache-resident regime) |
| M2 lookup+expand (same directory) | plain ≈ best (bytes/u32 0.960 on partkey only) | plain | plain; bytes/FOR 1.11–1.48× slower |
| M3 merge sorted runs | decode→merge→re-encode 4.24–4.27× slower | 2.30–2.73× slower | 1.58–1.89× slower |
| M4 transfer (PCIe) | — | — | bytes + device decode 0.30–0.60× |
| M5 group on codes vs `String` | **0.061** | **0.084** | — |

**What the evidence decides (supersedes §2's "codec by measured size"):**
1. **Default is plain** on every target. A non-plain choice needs a
   (target, role, access, parallelism) row above that supports it.
2. **Index runs and merges stay plain** on CPU and GPU. This confirms §8
   without the directory confound in the Mac comparison.
3. **CPU scan columns: byte-aligned truncation, not bit-packing.** Bit-
   packing pays only when the scan is bandwidth-bound (t24), so the codec
   choice depends on the plan's parallelism, which is a recorded fact; it is
   chosen jointly with Tasks placement.
4. **GPU: compression is for transfer and capacity** (1.7–3.4× on PCIe).
   Resident scans gain 13–28% at SF10 scale and can lose on cache-resident
   inputs.
5. **Encoded execution beats compression.** Grouping/comparing on dictionary
   codes is 12–16× on CPU. This is contract 7's `encoded` representation
   (`where`/group/join on codes), not a storage codec, and it is the largest
   measured storage-adjacent win.
6. **Summaries are not default** (§12.1: TPC-H load order prunes nothing).
   They are built only for a clustered order or recorded workload evidence.

**Engine gaps dwarf codec effects** (same queries, same machine):

| Query | Engine today | Hand microbenchmark | Gap |
|---|---:|---:|---:|
| h06 CPU SF1 | 25.0 ms (default) | 23.8 ms plain, 15.5 ms bytes | ≈1.05× (storage is the remaining lever) |
| h06 GPU SF1 / SF10 | 7.4 / 13.4 ms | 0.10 / 0.85 ms plain kernel | **74× / 16×** (reduce/dedup phase) |
| h01 CPU SF1 | 2676 ms | 18.8 ms `String` group count+sum (M5 is a narrower kernel than h01) | ~140× [INFERENCE: not storage; needs a profile] |

So storage work pays on CPU scan-bound queries (h06-like: ~1.5×), while
GPU reduction/dedup overhead and CPU h01's dominant cost must be profiled
(§12.3 E2) before storage slices.

## 13. Engine-wide rules adopted 2026-10-29 (user decisions + evidence)

1. **Strings are interned at input on every target.** No query column has
   type `String` inside the engine; relations carry codes. Systems differ
   in how far they go: Datalog engines intern globally (Soufflé symbol
   table, SRDatalog, our DOOP `IStr`); analytical stores dictionary-encode
   at the storage layer (DuckDB `dictionary_compression.cpp`, `dict_fsst.cpp`)
   and also keep inline/prefix string values in execution (DuckDB `string_t`,
   `string_type.hpp:33-34`). We take the stronger form. Code assignment
   follows §10 (a publication decision). CPU TPC-H loaders and GPU export
   use sorted per-column dictionaries; execution carries column-local
   codes, not owned strings. The M5 code-grouping result above is historical
   experimental evidence, not a measurement of this migration.
2. **String operations run once per dictionary entry at input**, producing
   code sets joined relationally (authored declarations, never compiler
   rewrites of opaque Rust). Prefix tests over a sorted dictionary are code
   intervals.
3. **Decoding is exceptional**: only (R) where the user's opaque Rust reads
   a value (contract 7 `rust_reads`, identity representation) and (P) at
   answer publication. All sites are flagged and registered
   (`doc/DECODE-SITES.tsv`, `doc/DECODE-AUDIT.md`, checked by `check.py`).
4. **Widths**: every GPU TPC-H input column fits 32 bits at SF1/SF10
   (`doc/DECODE-COLUMNS.md`); keys never decode; 64/128-bit widths only for
   accumulators sized by value bits + log₂ rows, and for authored Rust types.
5. **Self-deduplication is sort-based, not a hash set.** Canonicalization is
   a Structures/Sequence strategy, not a provider retype: `canon = sort;`
   and `head = sort;` are the defaults, including computed heads.
   On CPU, Sort collects the whole boundary stream, uses `sort_unstable_by` with
   Rust `Ord` comparisons of the fields, then removes adjacent equal rows.
   Tasks uses `par_sort_unstable_by`; its adjacent-unique pass is sequential.
   Device Sort may record `sort using Type`, naming an explicit device
   sort/unique implementation such as `WideCanon`. Catalog selection may
   refine that implementation but cannot replace Ordered with Sort. This
   does not restore the obsolete `retype(dedup => Type)` hash-set interface.
   No Rust type spelling selects a comparison, codec, or uniqueness fact;
   opaque authored types supply their ordering through Rust traits.
   Membership indexes remain hash-based where consumers probe membership:
   building such an index is not this self-dedup operation.
   Dedup against full data remains a separate merge/search operation on
   sorted plain runs (§8). GPU Datalog admission already uses that scheme
   (`experiments/gpu/datalog-runtime/src/runs.rs`). CPU Datalog's
   DL-InsertNew hash `seen` admission is explicitly outside this CQ strategy
   cutover and remains unchanged. Historical h01 SF1 t1: hash DISTINCT
   2542 ms (95% of the query; 36% of cycles in rehash hashing) vs DuckDB 819 ms.
   M6b (`experiments/storage-bench/results/cpu-h01-decomposition-2026-10-29.md`,
   exact vs DuckDB, t1 / t24):

   | Self-dedup variant | t1 | t24 |
   |---|---:|---:|
   | hash set on `String` tuples (engine) | 2541 ms | 140 ms |
   | hash set, reserved, codes | 660 ms | 29 ms (prefix-aligned partitions) |
   | packed u128 (78 bits) LSD radix, 10×8-bit passes, + unique | 463 ms | 130 ms |
   | packed u128 comparison sort + unique | 259 ms | 92 ms |
   | **load-verified prefix order + adjacent compare, codes** | **17.5 ms** | **3.3 ms** |

   The load-time order check costs 21–23 ms once. Our general radix sort is
   far from bandwidth-bound (≈330 ms t1 for 94 MB of keys): an untuned
   implementation, not evidence against sort-based dedup. Sort width and
   digit choice are open measurement work; this migration uses comparison
   sort, not a type-selected radix implementation.
   `ordered` is an explicit alternative only for a direct host scan of an
   immutable load-verified prefix-order carrier. The carrier retains the
   exact checked prefix comparator with its rows, exposes no mutable row
   access, and discards the witness when rows are recovered for mutation.
   Full-row equality must imply equal prefixes. Ordered sorts and uniques
   **within every equal-prefix run**; equal prefixes never imply unique rows.
   The borrowed path collects references, not cloned owned rows. This is not
   a promise of allocation-free streaming or unchecked order propagation:
   projection, filtering, joins and computed heads do not inherit the witness.
6. **Uniqueness is never assumed.** Only properties checked by construction
   (e.g. sortedness verified in one load pass, recorded with the data) may
   remove work.

### 13.1 CPU/GPU string interning implementation

`cq-load::StringCode` is a `Copy`, transparent `u32` newtype. Each CPU
TPC-H `Data` that contains string columns owns `cq_load::Dictionaries`,
keyed by declared column name. `StringDictionary` sorts and deduplicates
the complete column's source values before assigning zero-based codes.
Empty strings, case, and whitespace are preserved. Codes are comparable
as strings only within the same column dictionary, not across columns.

`StringCode` supplies `DenseOrdinal` with an associated `u64` ordinal word
and an ordered `KeyCodec` by its stored ordinal. Dense membership stores
and probes bounds in that declared word width; custom wide ordinal domains
may explicitly use `u128`. `STR-FullKey` records existence-only reads when a positive probe
binds every column of a set relation; `build!` then selects bounded bitmap
membership using ordinary Rust trait dispatch. Input uniqueness is not
assumed: constructing the membership set handles duplicate witnesses.

The loader-side `interned_relation_loaders!` opt-in uses ordinary Rust
field declarations; plain `relation_loaders!` expansion is unchanged.
No new query syntax or compiler type inspection is needed.
`InternedField` trait dispatch preserves ordinary `Load` fields unchanged
and maps `StringCode` fields from input-only `String` values.
`InternedRelation` prepares the dictionaries and encodes the rows;
`parquet_interned` is the load boundary. Representation facts come from
authored declarations/provenance, not guessed Rust type spellings.

`Dictionaries::literal(column, text)` performs a lookup without inserting
absent literals. Missing values or columns return `StringCode::MAX`,
which is reserved and never assigned to source text. Query setup may
translate equality literals into codes; it does not rebuild dictionaries.
Opaque string predicates use
`cq_load::dictionary_where(&dictionaries, "column", predicate)` during loading,
once per distinct dictionary entry, producing code-set relations stored
in `Data`. Execution binds those sets and joins codes instead of scanning
or cloning source strings. Prefix predicates currently use this same
code-set preparation; sorted order permits intervals but does not itself
establish that an interval optimization is implemented.

H13 prepares dense comment-code membership at load, then filters and
projects order/customer pairs before the count query. This preserves the
SQL pair identity even when one pair has several qualifying comments;
it does not assume an input key or add an intermediate CQ DISTINCT.
Default keyed aggregate maps record
`std::collections::HashMap<_, AccTuple, cq_storage::HashState>` in Structures,
matching the engine hash state used by membership indexes. Canonicalization
does not use a hasher. Authored provider types retain their own hashers.

Final string answers alone use `Dictionaries::publish`; intermediates,
grouping, and joins remain coded. The explicit load-time predicate reads
and final publication sites carry registered R/P decode comments.
Fixture source strings and expected textual answers remain legitimate
input/publication boundaries rather than string-valued query relations.

GPU export/import and CPU loading assign the same sorted, distinct
per-column order for the same source values; dictionary metadata supplies
the GPU literal/predicate preparation boundary. This describes the
implementation contract, not an executed CPU/GPU equivalence check.
No build, test, or performance result is claimed for this migration here.
7. **If a string value is ever needed inside the engine** (a DECODE R site:
   opaque Rust reads it), it is a reference into the interned dictionary
   (`&'static str`, or `&'data str` borrowed from the dictionary arena that
   outlives the query), never an owned `String`: decoding allocates nothing
   and copies nothing. No current query manipulates strings (string
   predicates run at input, rule 2), so no such site exists today; this rule
   governs the first one.
8. **Interning is input preparation, not query work.** It lives only in the
   loader (`runtime/cq-load/src/intern.rs`, opt-in `interned_relation_loaders!`);
   no compiler pass interns, and `run()` sees only codes. It is excluded from
   query timing and from acceptance. Its cost bound is one linear pass over
   the column's bytes (bytes ÷ bandwidth, plus hashing), plus a sort of the
   *distinct* values only when order-preserving codes are required (§10). The
   current implementation is naive (per-row `String`, sort of all values,
   per-row lookup; h13 load 188 → 2248 ms) and is a later P3c item: hash pass
   over Arrow string buffers, no per-row allocation, parallel.
