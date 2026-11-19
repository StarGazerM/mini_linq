# Packed radix runs in the xalan fixpoint

**Result:** both configurations matched all 26 CPU outputs. Packing cut end-of-fixpoint device pool live bytes by **56.15%** and peak bytes by **53.08%**, but the median fixpoint rose **36.023 ms (+6.49%)**. Keep it opt-in.

## Xalan fixpoint: five fresh processes per configuration

Local NVIDIA RTX 6000 Ada, `sm_89`, driver 580.178.04, CUDA 13.0 / HPC SDK 25.9. The final crossover ran after our builds, tests and parent control were finished. Five blocks rotated each configuration through all five launch positions; A and A′ are the **same unmodified parent executable** in separately launched processes. The driver's synchronized `phase fixpoint` timer excludes setup and host output. The compute-process list was empty immediately before and after every sample, both exact comparisons, both censuses and both profiler captures.

| Configuration | Fixpoint samples (ms), in block order | Median ms | Pool live at end (bytes) | Peak through fixpoint (bytes) |
|---|---|---:|---:|---:|
| Parent `fca91bf7`, A | 563.140, 561.591, 567.374, 556.409, 574.342 | 563.140 | 275,147,264 | 316,326,912 |
| Same parent, A′ | 554.506, 558.411, 555.938, 565.075, 569.691 | 558.411 | — | — |
| **Default off** | 554.666, 543.841, 573.041, 546.454, 576.191 | **554.666** | **275,147,264** | **316,326,912** |
| **Packed on** | 591.748, 590.689, 583.400, 599.147, 585.616 | **590.689** | **120,649,984** | **148,414,976** |
| Packed-capable binary, storage forced off | 534.895, 545.708, 528.960, 526.383, 539.478 | 534.895 | 275,147,264 | 316,265,472 |

Parent memory comes from our separate parent ownership trace; for each successor configuration all five processes gave the same pair of byte counts. Default-off memory matched the parent exactly. Its median time was 1.50% below A, within the observed A/A paired process spread (−2.02% to +1.56%); this is not a formal equivalence bound. Packed-on was slower than default-off in every block. Against the *same binary* with storage forced off, packed-on was 10.43% slower by medians. The memory savings are actual pool bytes, not the sum of per-relation compression estimates.

### One Nsight Systems capture each (not acceptance timings)

The trace window is bounded by the same synchronized setup-end and fixpoint-end boundaries as the timer. Kernel-duration sums can overlap across lanes; only the middle three rows below partition wall time.

| Profile observation | Default off | Packed on |
|---|---:|---:|
| Profiled fixpoint, ms | 650.791 | 709.806 |
| Wall with generated rule kernels running, ms | 254.631 | 256.675 |
| Wall with support kernels running, ms | 74.883 | 82.433 |
| Wall with no kernel running, ms | 321.277 | 370.698 |
| Summed rule-kernel duration / launches | 379.429 ms / 8,882 | 372.979 ms / 8,882 |
| Support launches, including admission/publication | 22,921 | 23,398 |

The largest difference in this captured pair was time with no kernel running (+49.421 ms), not total rule execution. With copies/memsets excluded, device-idle intervals with at least one lane inside a CUDA API grew from 163.962 to 191.083 ms; device-idle intervals outside CUDA APIs grew from 135.202 to 156.869 ms. Neither interval is a claim about CPU utilization. On the admission side, `absent` took 6.303 → 2.787 ms summed, with 4.316 ms in packed `membership` and 4.311 ms in `write_leaf_compact` on. There were 652 `packed_merge` and 844 `packed_seal_adopt` host ranges (13.355 and 8.267 ms inclusive, not additive to kernel time). A single instrumented pair does not identify a unique cause of the unprofiled slowdown.

## Exactness and regression gates

**Xalan: exact full-tuple sets for all 26 outputs**, separately against `doop-mini` with default off and packed on. The fixpoint pool counters in those full-mode runs also matched the GPU-only memory readings. `python3 doc/formal/check.py` passed: 522 registered decode comments, including the updated demanded-field site; no checker change.

* Default-off Datalog/runtime/Sequence/DOOP suites: 364 passed; two standard ignored tests. This includes the packed prototype suites. The SF10 grouped H01 fixture exceeded the 100-second cap using the DuckDB CLI's row formatter. The same unmodified test passed within the cap using its `DUCKDB` hook to stream both its live SQL queries with DuckDB `COPY ... TO '/dev/stdout' (FORMAT CSV, HEADER false)` from the real SF10 database; no precomputed rows or oracle.
* Final runtime suites alone: 137 passed off, 138 passed in the packed build (one normally ignored in each). The additional test retains a packed snapshot across a merge, permutes it, replaces a delta, and checks the complete outputs. It also passed with the packed-capable build forced off.
* Other downstream checks: SRD DOOP fixture passed; all 64 non-ignored TPC-H checks passed. The compiled examples had four passes and **one pre-existing failure**. In `gpu-h05-compiled`, `tests::compiled_triangle_rejects_unsupported_dense_span` panics at `h05-compiled/src/bin/triangle-compiled.rs:270:26`: `unsupported dense key span was accepted`. It also failed with exactly that message on unmodified parent `fca91bf7` under the required RCCL override. No triangle test, Sequence code, or RCCL source was changed.

Parent control reproduction, from its `experiments/gpu` with the prescribed environment:

```sh
flock -s /tmp/mini_linq-timing.lock timeout 900 cargo +1.99.1 oxide test --arch sm_89 --materialize-cubin -- --release --no-run -p gpu-h05-compiled --bin triangle-compiled
flock /tmp/mini_linq-gpu.lock timeout 100 target/release/deps/triangle_compiled-0e8d0a714dd3afd0 tests::compiled_triangle_rejects_unsupported_dense_span --exact
```

## Selection and scope

Packed ownership is absent from default host states and snapshots. The default driver was 17,830,032 bytes versus 17,749,960 for the parent, compared with 20,064,456 before specializing the host ownership path. These executable sizes are not device-memory measurements.

The default build uses the original fixed-word readers. Build `doop-gpu` with the Cargo feature `gpu-datalog-runtime/packed-radix` to enable packed full-key states. `GPU_DATALOG_PACKED_RUN=0` disables new packed states in that *same* binary; it cannot remove registers from already-compiled kernels. The compile-time gate matters: the rule kernels inline the runtime reader functions and the host cannot change their register allocation at launch.

The forced-off control retains the packed-capable kernel image, including the disjoint-run shortcut for full-key distinct enumeration. It measures plain storage under that image, not the default binary.

* `radix_run.rs`: `new_set` records the policy; typed publication provides the field packing (active bit count alone is insufficient). Small states keep the original radix path. On promotion, existing sorted, disjoint runs are replayed in their original order and retained only after a successful append. The prototype keeps runs of up to 32,768 rows plain and packs larger ones. The same policy applies independently to maintained full-heading permutations and large typed delta replacements. No emitter, IR, plan, or generated source changed.
* `packed_run/set.rs`: immutable subtraction and ratio-law append/merge operate on block codes. The lazily uploaded view table is now shared; invalidation drops the writer's reference, not a reader's. Each published snapshot retains both the descriptor table and its immutable run owners across later appends and merges. The ordinary cross-stream fork/join and owner-lifetime contract is unchanged.
* The radix descriptor size is unchanged: a null key pointer denotes packed storage, in which case the otherwise-unused unit-offset pointer addresses the packed descriptor. Search and interval bounds encode probes into the leaf frame. `locate_key` needs only run lengths. `distinct` and `distinct_at` use disjoint intervals for full-key sets; occurrence runs still account for older duplicates. At the existing `decode_key` site only the demanded column's residual is loaded and its origin added. The explicit host answer/diagnostic export is separate from operator reads.
* A standalone `permuted_provider` can also receive a packed snapshot. It transposes residuals into a common per-column frame, sorts those codes, seals the resulting leaves, and shifts their frame/fence metadata. It never reconstructs source rows to sort them. Normal Datalog additional layouts are built from fresh batches, not via this slower general operation.
* Grouped/item-column providers, runtime-width rows, and other non-radix providers are unchanged. Untyped `new_set` calls without a field layout cannot seal framed runs. In the generated engine typed publication precedes admission.

## Reproduction

Local GPU only. Build from `experiments/gpu` after `source /dev/shm/gpu-env.sh`:

```sh
flock -s /tmp/mini_linq-timing.lock timeout 900 cargo +1.99.1 oxide build --arch sm_89 --materialize-cubin -- --release -p doop-gpu --features cq-gpu-runtime/nvtx
# Repeat with --features cq-gpu-runtime/nvtx,gpu-datalog-runtime/packed-radix
flock /tmp/mini_linq-gpu.lock env GPU_DATALOG_PHASES=1 target/release/doop-gpu /data/doop-bench/doop-data/xalan
```

Full mode compares CPU/GPU tuple sets; `--gpu-only` skips that comparison for timing. `phases.rs` samples the pool after the synchronized fixpoint boundary. The separate ownership census uses `GPU_DATALOG_TRACE_MEMORY=1`, not the timed runs. None of the memory numbers are process NVML usage or reserved pool capacity. Raw builds, commands, tenant checks, samples, resource dumps, ownership tables and profiles are in `/data/design-notes/doop-memory/wire-in/`.

## Fused rule register cost

`registers.py` maps each hole in the built executable to its generated dispatch symbol, extracts its embedded CUDA ELF, and reads `cuobjdump --dump-resource-usage`. These are static registers **per thread**, not registers measured on a sampled warp. The packed-capable binary has the same allocation when its storage is forced off.

| Hole | Unmodified parent | Default off | Packed-capable |
|---:|---:|---:|---:|
| 122 | 64 | 64 | 100 |
| 119 | 40 | 40 | 64 |
| 145 | 120 | 120 | 128 |
| 132 | 40 | 40 | 80 |
| 136 | 128 | 128 | 128 |

All five default allocations and their stack sizes match the parent. Out-of-line packed helpers do not completely isolate register pressure in the packed-capable build.

## Which physical states packed?

At fixpoint 92 of 131 physical owners had packed runs: 71 canonical and 21 additional-layout owners. Of 189 retained runs, 96 were packed (73 canonical, 23 additional). In the off census these 96 runs occupied **265,233,696 bytes**: 98.09% of the 270,410,656 bytes of full-state run storage and 96.40% of end pool live bytes. Their packed storage used **110,666,856 bytes**. This is a run-size cutoff, not a relation-size cutoff: e.g. only two of `varpointsto`'s eight runs per owner packed. Each reported physical owner is counted once, irrespective of logical aliases. The off run allocations have no duplicate nonzero addresses across these owners.

Counting **whole eligible owners** rather than just their large runs includes their plain tails: 265,946,592 bytes off (98.35% of full-state run storage; 96.66% of end pool live) versus 111,373,552 on.

| All retained runs, requested storage bytes | Off | On |
|---|---:|---:|
| Canonical states | 189,996,984 | 79,575,556 |
| Additional full-heading layout states | 80,413,672 | 36,262,060 |
| Total | 270,410,656 | 115,837,616 |

Separate from this end census, the trace also recorded 91 large replacement publications (typed deltas/adopted sorted batches). Across their lifetimes the input prefixes contained 102,635,720 fixed-width bytes and the initial sealed runs requested 39,171,308 bytes. These are **not concurrent allocations** and are not added to the table. Grouped providers and non-radix representations were not converted.

<details><summary>All 92 packed owners: whole-state run storage including any plain tail runs</summary>

C = canonical; L = additional layout. MB are decimal and rounded; `states.tsv` and `runs.tsv` contain exact bytes and layout/run IDs for all owners, including the 39 entirely plain ones. Role labels are matched by owner ID and relation against the Xalan plan catalogue; the byte counts come exclusively from this run. Run storage includes payload, frames, fences and local metadata, not pool rounding or snapshot descriptor tables.

| Owner | Relation | Role | Packed/total runs | Off MB | On MB |
|---:|---|:---:|---:|---:|---:|
| 56 | `method_declaringtype` | L | 1/1 | 0.815 | 0.349 |
| 57 | `method_simplename` | C | 1/1 | 0.815 | 0.356 |
| 58 | `method_descriptor` | C | 1/1 | 0.815 | 0.319 |
| 59 | `method_modifier` | C | 1/1 | 1.054 | 0.225 |
| 78 | `assignheapallocation` | L | 1/1 | 2.868 | 1.060 |
| 80 | `instruction_method` | L | 1/1 | 9.795 | 4.346 |
| 81 | `isstaticmethodinvocation_insn` | C | 1/1 | 0.374 | 0.082 |
| 84 | `loadstaticfield` | L | 1/1 | 0.648 | 0.239 |
| 86 | `formalparam` | L | 1/1 | 1.791 | 0.328 |
| 88 | `returnvar` | L | 1/1 | 0.584 | 0.291 |
| 90 | `varpointsto` | L | 2/8 | 21.933 | 13.106 |
| 92 | `assignlocal` | L | 1/1 | 3.158 | 1.279 |
| 95 | `assigncast` | L | 1/1 | 0.710 | 0.395 |
| 97 | `heapallocation_type` | L | 1/1 | 0.976 | 0.276 |
| 98 | `varpointsto` | C | 2/8 | 21.933 | 12.337 |
| 99 | `assigncast` | L | 1/1 | 0.710 | 0.370 |
| 100 | `heapallocation_type` | C | 1/1 | 0.976 | 0.340 |
| 101 | `storearrayindex` | L | 1/1 | 1.437 | 0.429 |
| 103 | `supertypeof` | C | 1/1 | 0.425 | 0.170 |
| 104 | `storearrayindex` | L | 1/1 | 1.437 | 0.402 |
| 111 | `loadinstancefield` | L | 1/1 | 2.407 | 1.253 |
| 112 | `instancefieldpointsto` | L | 2/5 | 22.973 | 7.595 |
| 114 | `loadinstancefield` | L | 1/1 | 2.407 | 1.250 |
| 122 | `virtualmethodinvocation_base` | L | 1/1 | 1.835 | 0.977 |
| 123 | `storeinstancefield` | L | 1/1 | 0.870 | 0.467 |
| 125 | `storeinstancefield` | L | 1/1 | 0.870 | 0.424 |
| 127 | `staticmethodinvocation` | L | 1/1 | 0.747 | 0.298 |
| 129 | `specialmethodinvocation_base` | L | 1/1 | 0.697 | 0.382 |
| 133 | `formalparam` | C | 1/1 | 1.791 | 0.341 |
| 135 | `assignreturnvalue` | C | 1/1 | 1.684 | 0.535 |
| 136 | `actualparam` | C | 1/1 | 5.834 | 1.512 |
| 137 | `var_type` | C | 1/1 | 9.799 | 4.397 |
| 142 | `field_declaringtype` | C | 1/1 | 0.423 | 0.190 |
| 143 | `method_declaringtype` | C | 1/1 | 0.815 | 0.355 |
| 144 | `method_returntype` | C | 1/1 | 0.815 | 0.355 |
| 145 | `method_paramtypes` | C | 1/1 | 0.815 | 0.356 |
| 146 | `thisvar` | C | 1/1 | 0.698 | 0.249 |
| 147 | `var_declaringmethod` | C | 1/1 | 9.799 | 3.037 |
| 148 | `instruction_method` | C | 1/1 | 9.795 | 3.164 |
| 149 | `isvirtualmethodinvocation_insn` | C | 1/1 | 1.835 | 0.343 |
| 150 | `fieldinstruction_signature` | C | 1/1 | 2.050 | 0.822 |
| 151 | `loadinstancefield_base` | C | 1/1 | 1.205 | 0.566 |
| 152 | `loadinstancefield_to` | C | 1/1 | 1.205 | 0.564 |
| 153 | `storeinstancefield_from` | C | 1/1 | 0.436 | 0.206 |
| 154 | `storeinstancefield_base` | C | 1/1 | 0.436 | 0.205 |
| 155 | `loadstaticfield_to` | C | 1/1 | 0.324 | 0.152 |
| 159 | `storearrayindex_from` | C | 1/1 | 0.725 | 0.317 |
| 160 | `storearrayindex_base` | C | 1/1 | 0.725 | 0.320 |
| 161 | `assigninstruction_to` | C | 1/1 | 3.372 | 1.599 |
| 162 | `assigncast_from` | C | 1/1 | 0.355 | 0.168 |
| 163 | `assigncast_type` | C | 1/1 | 0.355 | 0.158 |
| 164 | `assignlocal_from` | C | 1/1 | 1.582 | 0.766 |
| 165 | `assignheapallocation_heap` | C | 1/1 | 1.435 | 0.702 |
| 166 | `returnnonvoid_var` | C | 1/1 | 0.588 | 0.278 |
| 167 | `methodinvocation_method` | C | 1/1 | 2.906 | 1.342 |
| 168 | `virtualmethodinvocation_base` | C | 1/1 | 1.835 | 0.931 |
| 169 | `virtualmethodinvocation_simplename` | C | 1/1 | 1.834 | 0.871 |
| 170 | `virtualmethodinvocation_descriptor` | C | 1/1 | 1.834 | 0.782 |
| 171 | `specialmethodinvocation_base` | C | 1/1 | 0.697 | 0.353 |
| 172 | `methodinvocation_base` | C | 1/1 | 2.532 | 1.267 |
| 173 | `loadinstancefield` | C | 1/1 | 2.411 | 1.120 |
| 174 | `storeinstancefield` | C | 1/1 | 0.873 | 0.425 |
| 175 | `loadstaticfield` | C | 1/1 | 0.649 | 0.242 |
| 178 | `storearrayindex` | C | 1/1 | 1.451 | 0.461 |
| 179 | `assigncast` | C | 1/1 | 0.710 | 0.344 |
| 180 | `assignlocal` | C | 1/1 | 3.165 | 1.054 |
| 181 | `assignheapallocation` | C | 1/1 | 2.869 | 0.997 |
| 182 | `returnvar` | C | 1/1 | 0.588 | 0.233 |
| 183 | `staticmethodinvocation` | C | 1/1 | 0.747 | 0.261 |
| 187 | `_var_declaringmethod` | C | 1/1 | 9.799 | 3.037 |
| 189 | `_thisvar` | C | 1/1 | 0.698 | 0.249 |
| 190 | `_normalheap` | C | 1/1 | 0.623 | 0.281 |
| 191 | `_stringconstant` | C | 1/1 | 0.353 | 0.055 |
| 192 | `_assignheapallocation` | C | 1/1 | 4.304 | 1.900 |
| 193 | `_assignlocal` | C | 1/1 | 4.747 | 1.720 |
| 194 | `_assigncast` | C | 1/1 | 1.066 | 0.502 |
| 195 | `_field` | C | 1/1 | 0.846 | 0.466 |
| 196 | `_staticmethodinvocation` | C | 1/1 | 0.747 | 0.317 |
| 197 | `_specialmethodinvocation` | C | 1/1 | 2.092 | 0.790 |
| 198 | `_virtualmethodinvocation` | C | 1/1 | 5.504 | 2.032 |
| 199 | `_method` | C | 1/1 | 3.260 | 1.286 |
| 200 | `_storeinstancefield` | C | 1/1 | 1.309 | 0.590 |
| 201 | `_loadinstancefield` | C | 1/1 | 3.616 | 1.607 |
| 203 | `_loadstaticfield` | C | 1/1 | 0.973 | 0.345 |
| 204 | `_storearrayindex` | C | 1/1 | 2.176 | 0.771 |
| 206 | `_return` | C | 1/1 | 1.175 | 0.463 |
| 207 | `methodlookup` | C | 1/5 | 6.931 | 2.976 |
| 208 | `methodimplemented` | C | 1/1 | 1.491 | 0.679 |
| 213 | `subtypeof` | C | 1/3 | 0.518 | 0.209 |
| 214 | `subtypeofdifferent` | C | 1/1 | 0.331 | 0.163 |
| 218 | `assign` | C | 1/6 | 0.356 | 0.222 |
| 219 | `instancefieldpointsto` | C | 2/5 | 22.973 | 9.031 |

</details>

## Host-side admission cost

**The control changes the diagnosis.** In the same packed-capable executable, with storage forced off versus on, the median wall time with a generated rule running was **222.434 vs 258.011 ms**. Kernel suffix `c23983d3a8568ae8_115` alone took 66.982 vs 87.185 ms summed (174 launches in each first capture). Its summed-time increases in the other pairs were 20.241 and 19.648 ms. The earlier default-image comparison does not establish that packed reads are free under the same image. Nor can the remaining difference in a driver timer be assigned causally to CUDA calls by adding API or NVTX durations: lanes overlap and copies can wait for earlier rule work.

### Original wiring: three fresh Nsight Systems processes per storage policy

The `admission-trace` feature enables static, nested CPU operation and row-class ranges, separately from the acceptance build. Profile with `-t cuda,nvtx --sample=none --cpuctxsw=none`; clip at the end of `setup_end` and start of `fixpoint_end`. Each row below is a **wall partition** in ms: rule has precedence over support when both run; copy/memset is counted only outside kernels. The last two columns have no recorded device activity. GPU process lists before and after all six captures were empty. The third packed capture was slow; shown rather than discarded.

| Storage / repetition | Fixpoint | Rule | Support | Copy/memset | Idle, lane in CUDA | Idle, outside CUDA |
|---|---:|---:|---:|---:|---:|---:|
| Forced off 1 | 666.159 | 221.158 | 74.665 | 21.918 | 179.262 | 169.156 |
| Packed 1 | 731.560 | 255.709 | 82.232 | 22.568 | 195.164 | 175.888 |
| Forced off 2 | 686.550 | 223.296 | 74.711 | 22.067 | 190.814 | 175.662 |
| Packed 2 | 741.588 | 258.011 | 82.223 | 22.737 | 197.706 | 180.911 |
| Forced off 3 | 710.278 | 222.434 | 74.805 | 22.187 | 202.839 | 188.013 |
| Packed 3 | 894.380 | 269.907 | 82.370 | 23.423 | 252.412 | 266.267 |

`idle.py` further partitions *only* idle-device wall by the simultaneous set of host ranges and issuing CUDA calls (one label set per interval in `.idle.tsv`, not one copy per lane). With some marked admission/snapshot scope active, median idle time inside CUDA was 91.563 → 97.153 ms and outside CUDA 72.917 → 77.859 ms. With **no** marked scope active, the corresponding medians were 99.250 → 100.553 ms and 102.745 → 103.052 ms. These measurements identify coincidence, not which thread caused a device stall.

CPU-side attribution uses the innermost operation on the CUDA call's issuing thread; copy direction and exact size come from its CUPTI correlation ID, **not** the NVTX duration. Per-process medians below are independent and are not additive. These are lane times, not wall shares. Logical allocations are separate count-only GDB runs (breakpoints at the statistics allocator's `allocate` entries between the same phase marks); they include zero-length requests, not just upstream `cuMemAlloc` calls. No GDB time is used.

| Host scope | Calls off / on | Inclusive ms off / on | Exclusive ms off / on | Logical allocation requests off / on |
|---|---:|---:|---:|---:|
| `fresh` (includes sort/subtract) | 1,549 / 1,549 | 182.005 / 183.623 | 103.043 / 104.762 | 7,818 / 7,818 |
| Plain subtract | 1,115 / 714 | 66.092 / 42.366 | 50.445 / 32.307 | 5,719 / 3,709 |
| Packed subtract | 0 / 401 | — / 23.738 | — / 17.925 | 0 / 2,010 |
| Membership descriptor upload | 1,115 / 1,115 | 5.010 / 5.233 | 5.010 / 5.233 | 1,115 / 1,115 |
| Unique/fresh count readback | 2,302 / 2,302 | 23.552 / 23.418 | 23.552 / 23.418 | 0 / 0 |
| Append (including empty) | 2,124 / 2,124 | 29.057 / 36.870 | 3.008 / 2.519 | 1,466 / 719 |
| Replace | 1,200 / 1,200 | 1.076 / 5.295 | 1.076 / 1.236 | 1,043 / 952 |
| Packed append (including promotion replay) | 0 / 844 | — / 24.194 | — / 2.049 | 0 / 0 |
| Adopt, including actual seal | 0 / 844 | — / 8.859 | — / 1.507 | 0 / 3,461 |
| Actual seal | 0 / 171 | — / 7.412 | — / 5.224 | 0 / 1,197 |
| Seal sizing, 8 B D2H + sync | 0 / 171 | — / 2.189 | — / 2.189 | 0 / 0 |
| Ordinary ratio merge | 1,314 / 662 | 26.049 / 13.453 | 26.049 / 13.453 | 2,628 / 1,324 |
| Ratio merge in packed set | 0 / 652 | — / 13.636 | — / 3.251 | 0 / 2,454 |
| Leaf merge (subset of preceding) | 0 / 243 | — / 10.377 | — / 5.252 | 0 / 1,517 |
| Merge sizing, 24 B D2H + sync | 0 / 243 | — / 5.112 | — / 5.112 | 0 / 0 |
| Construct packed module bundle | 0 / 159 | — / 0.838 | — / 0.838 | 0 / 0 |
| Snapshot cache miss | 2,666 / 2,666 | 24.478 / 28.460 | 24.478 / 24.173 | 7,998 / 7,998 |
| Packed view upload within snapshot | 0 / 838 | — / 4.448 | — / 4.448 | 0 / 838 |
| Outside these ranges | — | — | — | 5,476 / 5,476 |
| **Total pool requests** | | | | **33,263 / 40,588** |

Of **844** seal/adopt ranges, **171** actually packed; **zero** appends both sealed the incoming batch and merged it immediately. That shortcut was already implemented. **91** seals were replacements, not ratio merges. The **243** actual leaf merges each paid a geometry download and stream barrier; the other **409** within packed sets produced plain words without that download. Size classes below count append **inputs** and merge **outputs** independently; K = 1,024, M = 1,000,000, upper bounds inclusive. Times are inclusive lane ms.

| Rows | Append calls off = on | Packed adopts | Actual seals (ms) | Packed-set merges (ms) | Of which leaf merges (ms) | Ordinary merges off / on |
|---|---:|---:|---:|---:|---:|---:|
| 0 | 658 | 0 | 0 | 0 | 0 | 0 / 0 |
| 1–256 | 782 | 246 | 0 | 151 (1.100) | 0 | 525 / 374 |
| 257–32K | 534 | 357 | 0 | 258 (1.901) | 0 | 546 / 288 |
| >32K–128K | 126 | 199 | 130 (5.436) | 142 (5.873) | 142 (5.742) | 142 / 0 |
| >128K–2M | 24 | 42 | 41 (1.977) | 99 (4.614) | 99 (4.510) | 99 / 0 |
| >2M | 0 | 0 | 0 | 2 (0.136) | 2 (0.132) | 2 / 0 |

| CUDA API | Calls off / on | Lane ms off / on | Correlated device bytes off / on |
|---|---:|---:|---:|
| Kernel launch | 31,803 / 32,280 | 212.001 / 229.743 | — |
| D2H async | 11,197 / 11,611 | 543.842 / 605.141 | 357,992 / 365,192 |
| H2D async | 6,738 / 7,806 | 18.600 / 21.387 | 322,944 / 601,408 |
| D2D async | 4,991 / 5,087 | 20.791 / 20.888 | 385,658,984 / 393,319,776 |
| Stream synchronize | 11,225 / 11,639 | 6.638 / 6.973 | — |
| Event record | 33,778 / 37,054 | 16.977 / 18.635 | — |
| Stream wait event | 2,935 / 3,047 | 2.317 / 2.217 | — |
| Memset D8 | 10,588 / 9,936 | 30.037 / 30.634 | — |
| Device malloc / free | 2 / 3 malloc; 0 / 1 free | 0.411 / 0.576 malloc; 0 / 0.210 free | — |
| Module load | 5 / 5 | 27.349 / 32.180 | — |
| Stream get ID | 1,519 / 1,519 | 0.176 / 0.219 | — |
| Event create / query | 3 / 3 create; 7 / 11 query | 0.004 / 0.005 create; 0.006 / 0.009 query | — |

There were exactly **8,882** generated rule launches in each original capture; their summed GPU times in pair order were 326.265/371.993, 326.139/372.770 and 326.291/368.355 ms (off/on). The five driver module loads were also fixed; constructing a packed module *bundle* was instead per new promoted/replacement set, with no `cuModuleLoadData` inside those 159 ranges.

The **entire D2H count and byte increase** is 171 × 8 B seal sizing plus 243 × 24 B leaf sizing. It does not explain the entire increase in blocked D2H API time. Subtraction uploads were fixed per subtract, 51,280 → 135,312 B across 1,115 calls; the ordinary snapshot's 5,175 H2D calls / 259,120 B were unchanged. Packed snapshots added 838 table copies (187,760 B); schemas added 171 copies / 4,872 B at seals and 59 copies / 1,800 B at leaf bootstraps. Trimming retained plain tails added 96 D2D copies / 7,660,792 B. These counts distinguish publication costs from costs per actual seal or leaf merge.

Sizing is mostly **not the transfer itself**: joining the readback to its own CUDA stream, median merge-sizing intervals included 0.253 ms of copies, 1.527 ms of `plan_team`, 0.665 ms of `plan_bounds`, 0.115 ms of `tile_offsets` and 2.320 ms with *no operation on that stream*. For seal sizing the corresponding copies occupied 0.170 ms and no same-stream operation occupied 1.543 ms. `sizing.py` emits the full decomposition; a quiet stream does not imply a quiet GPU or a CPU spinning.

Raw data and runnable helpers: `/data/design-notes/doop-memory/wire-in/host/`. `base-trace-{0,1}-{1,2,3}.sqlite` are the original-wiring captures. `analyze.py` writes `.scope.tsv` (inclusive/exclusive/inside CUDA by operation **and** row class), `.api.tsv` (actual API kind, count and ns for each class and operation), `.copy.tsv` (histogram of exact copy sizes), and `.wall.tsv`. `alloc.py` and `base-alloc-{0,1}.tsv` supply the separate suballocation census. `summary.py` aggregates the captures; `run.sh` rotates fresh processes with before/after tenant records, and `gate.sh` retains full comparisons, suites and formal logs for each change.

### Changes, measured in order

1. **Share replaceable delta words (`45bef643`).** `with_sorted` adopts the immutable batch rather than sealing a one-generation delta. Long-lived `from_sorted` still packs; the ownership test covers both contracts and unchanged-identity reuse. Packed median 602.855 → 592.894 ms in the local five-process blocks; live/peak stayed 120,649,984 / 148,414,976 B. More robust than this noisy wall difference: the second set of three captures per policy showed 171 → 81 actual seals, 838 → 748 packed snapshot uploads and 159 → 69 bundle constructions. This removed **90** complete seal pipelines, **270** launches, **90** sizing barriers and **180** H2D calls, without changing the 243 leaf merges.
2. **Batch immutable publication (`98cb5630`).** In the packed-capable image, publish the count, fixed-ABI radix descriptors and optional packed descriptors in one aligned table, with its pointers fixed before one staged H2D. Its owner survives republishing; grouped layouts and the default image still use separate buffers. Packed median 592.894 → 585.437 ms, live/peak 120,594,688 / 148,357,376 B. Forced-off changed 536.446 → 532.794 ms; it benefits from batching the ordinary snapshot metadata too. All 349 default CUDA kernel entries retained identical machine code and register/stack use; six generic entries needed joining across changed type-ID suffixes by family and identical `.text`.
3. **Keep coalesced tails plain through 1 MiB (`da3e1cb9`).** Applies only to maintained merges of two already-plain inputs. Large independent seeds keep the original seal cutoff; a packed input is never expanded. Packed median 585.437 → 569.278 ms, live/peak **118,557,696 / 145,518,336 B**. Surprisingly, retaining three previously encoded end runs in plain form did not increase total live memory: the two `varpointsto` owners together used 2,108,304 fewer requested run bytes with the new merge history. The `assign` owner requested 134,624 more.

At this point a new three-pair capture series showed 243 → **136** leaf merges and sizing barriers. Against forced-off, packed now had **54 fewer** launches (31,749 vs 31,803), not 477 more. Packed snapshot CPU time fell to 13.819 ms exclusive with no nested packed-view upload, versus the original 24.173 ms plus 4.448 ms in the nested upload. Wall with rules running was still 246.961 vs 222.807 ms; idle inside/outside CUDA was 182.363/159.416 vs 176.407/156.142 ms. The remaining 662 ordinary small merges took 13.219 ms on the packed host before promotion, motivating use of the same one-launch disjoint merge used by plain tails after promotion.

**Rejected experiment:** also use the one-launch merge for plain states before promotion. Both exact comparisons, both runtime suites, the checker and register gate passed, but its five-process packed median was 580.936 ms. A separate alternating, packed-only head-to-head gave **575.052 ms** at 1 MiB without this change vs **576.979 ms** with it, with no memory saving. It was not kept.

4. **Raise the coalesced-only limit to 4 MiB (`f5b04a8e`).** After the 1 MiB policy, only 35 leaf merges remained in the >32K–128K-row class, but 99 still had >128K–2M output rows. The new five-process packed median was **562.900 ms**, down from 569.278 ms, with **116,970,496 / 144,765,696 B** live/peak. This trial did not change the first-admission cutoff. The separately counted 1 MiB phase had 27,931 / 33,839 logical allocation requests off/on, down from 33,263 / 40,588 initially; snapshot requests were 2,666 in either mode rather than 7,998 plus the separate packed view allocations.
5. **Reuse the snapshot for subtraction (`d5a7f6ec`).** The packed descriptor array is already in the immutable table. `fresh` retains that publication while probing it; if the state has never published this generation, it uses the ordinary transient upload rather than building a snapshot. On the 4 MiB policy packed median was 562.900 → 560.567 ms at identical live and peak bytes.

**Policy sensitivity:** a 16 MiB coalesced-only limit passed both exact runs, both suites and the checker. Its packed five-process median was **554.018 ms**; live/peak were **113,168,384 / 145,885,696 B**. That meets the latency target and both *ceilings*, but live bytes fell **6.20% below** the reference: outside a literal symmetric ±5% interval. We did not pad the pool or call that a pass on all three gates.

The census explained the lower memory at 16 MiB: **no retained run changed its plain/packed classification** compared with 4 MiB. Different merge inputs led to different final packed frames. The two `instancefieldpointsto` owners requested 3,046,904 fewer bytes together, and the two `varpointsto` owners 761,336 fewer; other owners were unchanged. A threshold's retained memory cost cannot be predicted just by counting end runs below that threshold.

6. **Select 8 MiB (`d8865abd`).** The intermediate cutoff kept initial-admission and standalone replay policies unchanged. In the three-way, five-fresh-process acceptance series, packed median was **554.234 ms** (4 MiB with resident probes: 560.567 ms); live **114,855,424 B** (−4.803%) and peak **142,289,408 B** (−4.127%) were both within 5% of the packed reference. This is 58.257% less live and 55.018% less peak than the default build.

| Selected build, crossover in three configurations | Five fixpoint samples (ms) | Median ms | End live B | Peak B |
|---|---|---:|---:|---:|
| Default | 565.805, 566.408, 555.407, 560.845, 573.173 | 565.805 | 275,147,264 | 316,326,912 |
| Packed-capable, storage forced off | 533.596, 540.980, 531.773, 546.495, 521.954 | 533.596 | 275,113,728 | 316,229,888 |
| Packed | 571.069, 553.834, 554.234, 553.941, 555.283 | **554.234** | **114,855,424** | **142,289,408** |

**Held-out confirmation.** After freezing the cutoff, `final.sh` rotated the three original and three selected configurations together. These were 30 additional fresh processes, five per configuration, all with empty before/after compute lists. Packed beat the original packed binary in **every block**. It met the fixed 554.7 ms median target by **0.442 ms**, not a large margin; it did not beat default in every individual process.

| Configuration | Original median ms | Selected five fixpoints (ms) | Selected median ms |
|---|---:|---|---:|
| Default | 556.791 | 557.614, 571.199, 571.539, 547.197, 572.905 | 571.199 |
| Forced off | 537.405 | 530.814, 532.377, 528.590, 530.602, 536.848 | 530.814 |
| Packed | 603.436 | 543.451, 554.258, 553.032, 556.173, 573.098 | **554.258** |

Live/peak matched the three-way table in all selected processes. The same-image packed premium fell **66.031 → 23.444 ms**; selected packed was **8.15% faster** than original packed. In the final three-pair profiles, all 401 packed subtractions used resident descriptors: total membership uploads were 714 on vs 1,115 off. Actual seals fell 171 → **81** and leaf merges 243 → **68**; their remaining exact sizing barriers accounted for the 149 extra D2H calls and synchronizations, 0.953 + 1.662 ms inclusive. Packed H2D calls fell 7,806 → **3,911** (final forced: 4,229), event records 37,054 → **28,118** (forced: 28,875), launches 32,280 → **31,627** (forced: 31,803), and logical pool requests 40,588 → **33,442** (forced: 27,931). Snapshot CPU time was 14.179 vs 13.504 ms and no separate packed snapshot upload remained.

**What remains:** final rule-running wall was **241.330 vs 223.258 ms** packed/forced; support wall was 77.435 vs 74.751 ms. Idle inside CUDA was 179.924 vs 177.575 ms, outside CUDA 162.976 vs 154.767 ms. Packing is therefore not free relative to plain storage in the same executable. The remaining difference is not predominantly a pile of per-snapshot uploads or small leaf merges. These correlated lane/device observations are not a causal partition of unprofiled wall time.

**Gates.** For the annotation baseline and every retained change: full CPU/GPU tuple-set equality for **all 26 outputs** independently with packed storage on and forced off; runtime tests **137 passed default / 138 passed enabled**, one standard ignored in each; repository formal checker passed with **522** registered decode comments. Device operations do not introduce any data inversion: no decode marker changed. The final default image has unchanged live/peak, and all **349** embedded kernel entries have identical `.text`, register count and stack bytes to the original default (including the six rehashed symbol names).

**Not implemented: faster `fresh`.** The existing 1,024-row `fresh_small` sorts, uniques and subtracts in one kernel/count readback. The engine needs a read-only interface accepting current descriptors (including plain, mixed and empty states) rather than the replay's mutable timing counters and per-call table upload. For larger inputs `sort_varying` computes per-lane variation masks and downloads them before selecting stable radix passes; that extra barrier is not free. Reuse lane-owned, pool-keyed scratch and release it at the existing engine scratch boundary. Preserve the wider-count path beyond its `u32` index limit, sort-window/padding preconditions, and `fresh`'s immutable sorted-prefix contract. [INFERENCE] Select between sort paths using measured candidate sizes and varying digits; a replay speedup is not evidence of a whole-fixpoint win. Include all-constant lanes, duplicates across runs, retained snapshots and multi-lane ordering in the full-engine comparison.

## Rule-kernel cost of packed reads

### Which real kernels changed?

Three new Nsight Systems processes per storage policy, alternating which policy goes first, using the same `tail8-packed-driver` with ordinary phase marks (no admission instrumentation). Clip execution to the end of `gpu_datalog/setup_end` and start of `gpu_datalog/fixpoint_end`. Each capture contained 8,882 generated launches. The median **union** of their device intervals was **222.537 ms forced off vs 241.831 ms packed (+19.294 ms)**. Median **sum** was 328.920 vs 358.189 ms (+29.269 ms); concurrent lanes make that a different quantity.

`/data/design-notes/doop-memory/wire-in/rules/baseline-kernels.tsv` lists **all 164 dispatch/reduce symbols**, their counts, registers, all six summed times, and the difference of the two medians. `rules.py` generates it from the scoped SQLite exports. The heaviest increases were:

| Host hole | Launches | Forced ms | Packed ms | Δ summed ms |
|---:|---:|---:|---:|---:|
| 122 | 174 | 67.006 | 75.277 | +8.271 |
| 123 | 204 | 10.592 | 13.411 | +2.819 |
| 132 | 196 | 16.204 | 18.002 | +1.798 |
| 145 | 197 | 16.178 | 17.631 | +1.453 |
| 143 | 197 | 15.730 | 17.073 | +1.343 |
| 131 | 208 | 9.165 | 10.277 | +1.112 |
| 136 | 201 | 16.316 | 17.407 | +1.091 |
| 138 | 198 | 7.963 | 9.012 | +1.049 |
| 153 | 206 | 7.338 | 8.252 | +0.914 |
| 139 | 190 | 6.445 | 7.350 | +0.905 |

The executable's Rust symbol contains `__cq_gpu_hole_12242___cq_sequence_kernels_c23983d3a8568ae8_115`: **`_115` is hole 122**, not hole 115.

### Paired hardware counters

For each of the top three, choose the time-weighted median packed launch among fixpoint ordinals with matching grids in both policies. Count preceding launches of that kernel across the whole trace: this production driver has NVTX **boundary marks**, not nested per-rule ranges. `capture.py` selects the unique generated symbol and absolute ordinal, runs `ncu --set full` on the unmodified engine, and `counters.py` checks the captured grid. Cache/clock controls were `none`; runtime replay time is not application time. All raw NCU reports, source counters and a machine-readable table (`baseline-counters.tsv`) are beside the SQLite files.

| Counter, off / on | 122 (ordinal 48) | 123 (77) | 132 (60) |
|---|---:|---:|---:|
| Grid × block | 284 × 256 | 426 × 256 | 426 × 256 |
| NCU duration, µs | 2021 / 2198 | 43.71 / 55.46 | 156.35 / 209.18 |
| Registers/thread | 100 / 100 | 80 / 80 | 80 / 80 |
| Achieved occupancy, % | 17.18 / 17.10 | 46.56 / 46.69 | 47.53 / 46.20 |
| Issue active, % | 23.07 / 22.96 | 23.63 / 28.21 | 26.24 / 27.36 |
| Warp instructions, M | 10.426 / 11.286 | 10.286 / 14.657 | 48.872 / 71.956 |
| Thread instructions, M | 329.217 / 354.529 | 276.195 / 364.942 | 1390.543 / 1708.061 |
| Active lanes/instruction | 31.58 / 31.41 | 26.85 / 24.90 | 28.45 / 23.74 |
| L1 / L2 hit %, forced | 75.02 / 93.18 | 80.59 / 94.04 | 91.10 / 100.29 |
| L1 / L2 hit %, packed | 82.94 / 85.00 | 89.37 / 90.26 | 93.28 / 94.04 |
| Global load sectors | 897914 / 981537 | 1803781 / 3330801 | 2257205 / 3332292 |
| Global store sectors | 13 / 13 | 2322 / 2292 | 6455 / 6462 |
| Local load sectors | 224 / 224 | 230336 / 230336 | 4838405 / 5596741 |
| Local store sectors | 224 / 224 | 107936 / 107936 | 3291328 / 3761344 |

Sectors are 32 B at L1/TEX, not DRAM bytes. The >100% L2 rate is the raw multipass counter on a short launch, not a physical probability. All scheduler stall reasons below are warp cycles per issued instruction (`smsp__average_warps_issue_stalled_*_per_issue_active.ratio`); `selected` is the reference, not itself a stall. Rounded zeros can be nonzero in the TSV.

| Stall, off / on | 122 | 123 | 132 |
|---|---:|---:|---:|
| Barrier | 1.489 / 1.551 | 11.854 / 10.328 | 13.516 / 13.387 |
| Branch resolving | .270 / .327 | .534 / .540 | .502 / .493 |
| Dispatch | .007 / .011 | .016 / .043 | .021 / .035 |
| Drain | .000 / .000 | .001 / .000 | .000 / .000 |
| Immediate cache miss | .073 / .067 | .125 / .094 | .026 / .021 |
| LG throttle | .000 / .008 | .007 / .017 | .042 / .038 |
| Long scoreboard | 2.739 / 2.844 | 5.850 / 3.787 | 2.972 / 2.287 |
| Math pipe throttle | .094 / .177 | .338 / .421 | .197 / .189 |
| Memory barrier | .073 / .067 | .118 / .086 | .030 / .021 |
| MIO throttle | .006 / .005 | .045 / .025 | .123 / .101 |
| Miscellaneous | .000 / .000 | .001 / .001 | .000 / .000 |
| No instruction | .158 / .193 | .254 / .270 | .340 / .354 |
| Not selected | .111 / .105 | .511 / .583 | .395 / .362 |
| Selected | 1.000 / 1.000 | 1.000 / 1.000 | .953 / .976 |
| Short scoreboard | .438 / .434 | .765 / .536 | 1.164 / 1.006 |
| Sleeping; TEX throttle | 0 / 0 | 0 / 0 | 0 / 0 |
| Fixed-latency dependency (`wait`) | 2.591 / 2.619 | 2.685 / 2.412 | 2.362 / 2.437 |

**122: demanded fields, not the bool membership helper.** Joining the packed cubin's local function symbols to NCU's per-PC predicated thread counts gives 265,138 calls to the 64-bit `field_code`, versus 1,565 total `packed_interval` calls and no packed `search` calls on this launch. That reader alone executed 823,514 warp instructions (86.8 predicated thread instructions and 7.1 load instructions per field). In every one of those 265,138 calls the executed fixed-block path converted the runtime divisor to float, computed a reciprocal and corrected the quotient before loading the header. Then the header and frame feed the payload address. The same-image register count and local load/store sectors do **not** change with storage policy.

**123: interval instruction and load traffic.** 92,586 wide-word and 53,289 narrow-word packed ranges, respectively 1,205 and 1,070 predicated thread instructions and 60.6 and 55.0 load instructions per call inside `range`. There are serial fence, header, frame and midpoint payload loads. The L1 hit rate *increases*; this is more work, not mainly worse cache locality. Local traffic does not increase.

**132: more divergent wide comparisons, plus local traffic.** 215,369 wide-word and 64,713 narrow-word packed ranges; 1,414 and 1,115 predicated thread instructions and 74.2 and 58.1 load instructions per call. The wide-frame midpoint reads both low and high code lanes on each endpoint search. Lanes per instruction fall from 28.45 to 23.74. Local load/store sectors increase by 758,336 / 470,016; local traffic includes call-frame traffic, so these are **not** all proven register spills. None of these three paired probes changes *static* occupancy: both policies run the same image.

### Which packed probes can skip searching?

An untimed diagnostic image counts one actual **packed run visit**, not one Datalog binding, indexed by PTX `%gridid`. The count table was allocated separately, never read on the production path, and joined to Nsight Systems `gridId` in the same process: all 5,365 nonempty grids belonged to scoped generated rules. `stage-counts.tsv` gives every rule (including zero rows), separated into general intervals, point intervals and positional searches. This census measures **opportunities**, not claims that production already skips each case. Test the two endpoints of an interval, not just the low one. `above last` reads the last row in code space in the diagnostic: there is **no stored maximum** in the production descriptor. A later column outside its frame is not a direct positional bound if an earlier column varies. The conservative `frame` column counts only cases where the insertion position follows from the leading frame(s).

| Packed run visits | Below first | Above last | Fence decides | Frame decides | Needs in-block comparison |
|---|---:|---:|---:|---:|---:|
| Hole 122 | 285 | 2 | 0 | 35 | 382614 |
| Hole 123 | 598898 | 24664 | 275 | 548 | 10466345 |
| Hole 132 | 0 | 0 | 663 | 93 | 19397051 |
| All generated rules | 17919462 | 4495708 | 1097 | 2852 | 91330185 |

Every counted visit came through `interval`, even when `low == high`; none came through positional `search`. A new `contains` specialization alone would therefore miss this workload. The wider logical probe population, which also includes plain-backed runs and non-radix providers, is not the denominator of this table.

Of the 113,749,304 packed run visits, 13,610,177 were point intervals. A second diagnostic marked `distinct`'s point-count call site separately: **zero** visits there (`count-only.tsv`). Thus even that count-only entry is not the way to specialize those point intervals. The general interface must preserve both insertion positions, including for misses.

The independent `shapes.tsv` census asks a different question: if **only a Boolean** were consumed for each point, any out-of-frame component proves absence even when it does not prove an insertion position. Across all rules, 278 points fell below the first key, 53,463 above the last, 1,097 were decided by a fence, 4,910,056 by a frame and 8,645,283 still needed an in-block comparison. In hole 123 alone the frame would settle 4,907,561 Boolean probes; the positional census settled only 548 visits by frame. Of the **non-point** visits in the shape census, 95,847,222 bound only the leading column, 3,625,851 had another shape, and none covered the entire domain. Counts are per diagnostic image; do not subtract totals from different instruments.

**Cross-check on NVTX-filtered launches.** A separate host-only build extended the interval between phase marks into one push/pop range. With `--nvtx --nvtx-push-pop-scope process --nvtx-include 'rules-fixpoint/' --set full`, both storage policies selected the same ordinal and grid as above; there were zero preceding setup launches of any of these three symbols. Times off/on were 1998.4/2281.5, 43.78/55.23, and 158.72/209.73 µs. `scoped-counters.tsv` has all counters. This check used the same executable for both policies; its separately built cubins have matching register/stack for the three rules but not byte-identical text to production. The production measurements above remain the primary comparison.

### Read changes and controls

* **Rejected: inline the entire fixed-leaf field path.** The gap in rule wall narrowed to 15.806 ms, but hole 122 grew to 104 registers. In a six-way rotated crossover against the original binaries, forced median went 521.760 → 527.462 ms and packed 555.456 → 559.841 ms. Not retained.
* **Pass the demanded-field descriptor by address.** Keep the reader out of line. Fixed-size leaves use quotient/remainder by the compile-time geometry; other leaves still consult their row directory. The first column's shift is total row width minus its frame width. No stored field is inverted here. At the hot hole-122 call site the caller used to execute seven 64-bit descriptor loads, **263,620 times each**, after loading the packed descriptor pointer. The new caller passes that pointer without those seven loads. Packed global-load sectors went **981,537 → 946,991**, warp instructions **11.286M → 11.102M**, and registers **100 → 99** (no residency step). Hole 122's aggregate storage premium fell 8.271 → 6.348 ms; rule wall off/on was **222.716 / 239.909 ms**. For hole 132 the extra local traffic was in the *caller*: in the baseline SASS the counted helper `LDL`/`STL` thread instructions were zero in both modes.
* In the first crossover for the descriptor change, forced median moved 527.322 → 528.235 ms and packed 559.986 → 554.886 ms. In the separate uncontended confirmation they moved **535.518 → 531.889 ms forced and 552.553 → 549.745 ms packed**. The effect on the driver is smaller than process-to-process variation. Both tuple-set comparisons were exact; runtime suites passed 137 default / 138 packed (one ignored each); formal inventory passed with 522 decode markers. All 349 default entries and five embedded cubins matched the reference, including text, registers and stacks; live/peak were unchanged.
* **Rejected: interval dispatch changes by themselves.** Passing the interval descriptor by address took hole 122 down to 80 registers (grid 284 → 426), but its packed launch had 19,524 local load sectors versus 224 before; rule wall was 223.864 / 243.586 ms. A first-fence guard alone gave a 19.551 ms premium. Inlining only a fast last-column field path gave 224.626 / 240.371 ms; the largest hole still cost 6.625 ms extra. Lower static registers or more theoretically rejectable queries do not guarantee lower wall time.
* **Rejected: separate rank and Boolean membership for point intervals.** Preserving the insertion index by returning `(rank, rank + present)` let the compiler remove neither lookup in the real hole-123 kernel: the same 34,671 lanes called both helpers (850,034 and 769,167 warp instructions). Rule wall 222.876 / 241.565 ms; crossover forced 527.089 → 540.021 and packed 553.026 → 567.276 ms. This was exact but slower. Returning `(0, 0)` on an arbitrary miss to avoid the rank would be incorrect.
* **Rejected at the forced-off gate: compare aligned leading-column prefixes in native code space.** A specialized fence cut and leaf search shared the two bounds and read only the leading residual. Rule wall fell to **223.822 / 234.835 ms (+11.013)**. Versus the descriptor change, hole 123's packed warp instructions fell 14.586M → 10.906M and global load sectors 3.287M → 1.876M; hole 132's corresponding counts fell 70.243M → 54.603M and 3.376M → 2.806M. Exact for all 26 outputs both ways; 137/139 tests passed, formal passed, default cubins unchanged. But hole 122 grew to 102 registers and remained +7.082 ms, and the rotated crossover moved forced-off **529.012 → 535.808 ms**, despite packed improving 562.687 → 551.824 ms. Not promoted. The six traces, six NCU profiles and 30 uncontended timer processes are under `leading-*` / `held-leading-*`.

### Held-out controls and remaining cost

The following five-process medians are from the descriptor change's independent six-way rotated series (`desc-confirm-held.log`); MB are decimal. Every before/after compute-process snapshot was empty.

| Configuration | Fixpoint ms | Pool live MB | Pool peak MB |
|---|---:|---:|---:|
| Default | 562.978 | 275.147 | 316.327 |
| Packed-capable, forced off | 531.889 | 275.114 | 316.230 |
| Packed on | 549.745 | 114.855 | 142.289 |

Registers in hole order **122 / 119 / 145 / 132 / 136**: default **64 / 40 / 120 / 40 / 128**; baseline packed-capable **100 / 64 / 128 / 80 / 128**; descriptor change **99 / 64 / 128 / 80 / 128**. Hole 119 is not a residency-step crossing: both 40 and 64 registers allow 32 resident warps at 256 threads/block here. The 122 and 132 crossings persist. Rule wall still differed by 17.194 ms and packed fixpoint exceeded 540 ms by 9.745 ms after the descriptor change; the memory target was met exactly in bytes. See `descriptor-kernels.tsv`, `descriptor-counters.tsv` and the SASS attribution for the remaining hole-122 metadata dependency, hole-123 extra range instructions and hole-132 divergent comparisons and increased caller local traffic.

## Runtime row cursor

**All the packed field demand in the three profiled holes came from range expansion, not the scan.** An untimed runtime-only census tagged the ordinal returned by `locate_key` and counted calls at `decode_key`, by grid, physical column, storage and position type. Nsight Systems identified the generated symbols and their absolute launch ordinals. A second pass recorded run address, slot, block, column and hardware thread for the selected grids; the sum of recorded events had to equal the first-pass counts. These are demands, not estimates from instruction counts. In production the packed field-reader entry counts agree exactly. The default/forced/packed timings and hardware counters below do **not** use the census binary.

| Hole / ordinal (zero-based) | Scan calls (all plain) | Range calls (all packed) | Scan rows × fields | Packed range rows × fields |
|---|---:|---:|---|---|
| 122 / 48 | 25 | 265,204 | 25 × 1 | 265,138 × 1; 22 × 3 |
| 123 / 77 | 185,172 | 90,525 | 92,586 × 2 | 34,671 × 1; 18,618 × 3 |
| 132 / 60 | 651,840 | 342,414 | 217,280 × 3 | 114,138 × 3 |

The three-column range reads demanded columns 1, 2, 3; the single-column reads demanded column 1. Across **all** 174 / 204 / 196 launches in the field census, scan calls were 4,425 / 6,078,618 / 43,450,800, range calls 6,470,775 / 8,643,359 / 28,929,987, with the same storage split. `cursor/census-*-launches.tsv` retains counts per diagnostic launch. Hole 132 repeated a different amount of plain scan work under instrumentation (164,096 rows at grid 568). The table uses the three **production** `LD.E.128` scan PCs, offsets `da0`, `1080`, `13b0`, each executed by 217,280 lanes at grid 426. The packed range count did not change. The 122 and 123 plain scan PCs also agree with the census.

The sequence engine distributes adjacent range slots **across lanes**. On the representative launches, grouping by warp, run and lane-local visit ordinal, the mean contiguous same-block segment among neighboring lanes was **25.70 / 1.50 / 5.91** rows (maximum 32 in each). Of the physically consecutive neighbor pairs, 254,844/255,865, 17,876/17,891 and 94,821/94,832 shared a block. Within one hardware thread, successive visits advanced by exactly one slot just 0 / 0 / 16 times. A per-thread last-block cache is not suggested by these measurements. Nor can an interval carry an auxiliary block pointer through the emitted pair of exact `u32` endpoints: the positions are also used for empty ranges, cardinalities and insertion indices.

A separate interval census counted contiguous physical block segments *within* the returned nonempty ranges. In hole 122 the two decoded sources returned 265,138 slots in 2,412 segments (1,384 nonempty intervals) and 22 slots in seven segments (seven intervals). In hole 123 the two sources returned 34,671 slots in 18,633 segments (18,618 intervals) and 18,618 slots in 18,608 segments (18,608 intervals). In a run-address join for hole 132 the decoded source returned 14,947,743 slots in 217,681 segments across 217,670 nonempty intervals, but only 114,138 rows were actually decoded: do not treat returned range lengths as work done. A separate source returned 4,228 singleton intervals without a single field demand. See `census-segments.txt` and `census-owner-segments.txt`.

**Cursor layout experiments.** All runs here use fresh binaries and xalan processes; the `before` packed build matched all five cubins and all 403 entries of the descriptor reference. Times below compare each candidate with `before` in its own six-way rotated, five-process-per-configuration crossover. Rule wall is instead the median union of the 8,882 scoped generated launches in three Nsight Systems traces per policy. Registers are in hole order 122 / 119 / 145 / 132 / 136.

* `row`: payload pointer, frame pointer, absolute `u64` row bit; a frame's former reserved word held its inclusive column width. Resolution was inlined and the residual reader stayed out of line. Scan resolution was shared by construction, but **range resolution was not commoned**. In hole 132 three distinct instruction PCs each loaded the same block geometry 114,138 times. Per packed row (122 / 123 / 132), the baseline took 80.8 / 157.1 / 321.1 predicated thread instructions and 11.07 / 20.34 / 37.74 global load instructions; this layout took 85.8 / 139.7 / 260.1 and 12.07 / 20.99 / 37.74. These counts include the packed arm from the storage branch through the field call, plus its callees, not unrelated expression work. Registers 100 / 64 / 128 / 80 / 128 versus baseline 99 / 64 / 128 / 80 / 128. Rule wall 222.634 / 240.606 ms off/on; hole 122 +6.386 ms. Crossover medians forced 512.860 → 512.892, packed 535.789 → 537.830 ms. Rejected.
* `slot`: a slot address for plain data, or a row-word pointer, frame pointer and word-relative `u32` bit for packed data. Aligned frame storage let the reader fetch base, width and location in one 128-bit load. The pure out-of-line range resolver still ran once **per field**, not per row (342,414 calls for 114,138 hole-132 rows); it also cost the plain range path a call. The field helper alone took 33.75 / 33.82 / 33.69 instructions and 2.06 / 2.36 / 2.58 global loads per field, but the resolver alone added ten loads per call. Registers 102 / 64 / 128 / 80 / 128; wall 226.788 / 242.787 ms; hole 122 +6.257 ms. Forced 510.084 → 519.570, packed 529.919 → 540.110 ms. Rejected.
* `direct`: put a fixed leaf's low 16 payload-word offset bits, stride, and inclusive field width in the otherwise reserved frame word. The snapshot exposes the frame origin and column count. Single-page runs need no per-row header load; larger fixed runs fetch only the high page from the header. Registers 100 / 64 / 128 / 80 / 128; rule wall 225.998 / 238.742 ms; hole 122 +4.582 ms. Crossover forced 511.519 → 514.862 ms, packed 531.988 → 535.310 ms; independent confirmation forced 508.537 → 515.292 ms, packed 539.847 → 534.081 ms. The signs of the packed fixpoint deltas disagree. Per field *after resolution*, 31.69 / 32.05 / 32.10 instructions and 2.06 / 2.36 / 2.58 loads; including setup, **63.70 / 101.47 / 180.31 instructions and 6.06 / 7.66 / 16.74 loads per row**. In hole 132 each of three fields still loaded the same page independently. `direct-inline-cost.tsv` lists the measured PC ranges (base addition excluded).
* `affine`: move the first row's field bit address into each narrow frame, beside its row stride. A local ordinal now needs one multiply-add. Raw and variable narrow leaves use the same frame representation, and the scan cursor needs no variant tag apart from its frame pointer. The payload loads use a single crossing check. Rule wall 223.521 / 235.653 ms; hole 122 +3.010 ms. Forced 507.658 → 516.709, packed 532.985 → 531.818 ms: the packed gain was smaller than the forced-off cost. Per field after resolution, 23.26 / 24.23 / 25.32 instructions and 2.06 / 2.36 / 2.58 loads. Including repeated resolution this was 55.26 / 88.18 / 159.96 instructions and 6.06 / 7.66 / 16.74 loads per row. The subsequent two columns of hole 132 cost 53.92 and 52.29 instructions, not 30, once their duplicated setup was included.
* `tagged`: distinguish storage in the run's existing offset-kind word; a fixed descriptor can now supply both the payload and frame origins directly. Ordinals use their own packed arm rather than constructing a cursor and then retesting its tag. The 122 hot site no longer loaded a `PackedView` payload pointer; its packed launch used 883,629 global-load sectors (946,991 before). Registers 100 / 64 / 128 / 80 / 128. Rule wall 225.688 / 235.342 ms; hole 122 +2.788 ms. Forced 509.364 → 513.652, packed 529.916 → 526.840 ms. Per packed row including setup: 34.26 / 56.35 / 114.96 thread instructions and 3.06 / 4.66 / 13.74 loads. In hole 132 two repeated metadata loads per field remained on the multi-page arm.
* `address`: the published 16-byte narrow frame stores a 64-bit base, byte width, byte row stride, 16-bit field shift and the low 32 bits of the leaf's payload word offset. Read an aligned 64-bit payload word and fetch the low half of the next word only on a crossing. The frame-load-to-code costs were 27.13 / 27.89 / 28.13 instructions and 2.03 / 2.17 / 2.28 loads per field. Per row, including setup: 40.13 / 65.27 / 114.38 instructions and 3.03 / 4.34 / 6.85 loads. In particular the repeated high-page loads in 132 disappeared, **not** all its address arithmetic. Registers 100 / 64 / 128 / 80 / 128; rule wall 225.049 / 236.150 ms; hole 122 +3.119 ms. Forced 508.194 → 517.155, packed 537.745 → 535.762 ms.

The reference in this series was 226.698 / 240.326 ms off/on rule wall. Its single-launch packed `field_code` body, averaged across demanded columns and carrier widths, was 72.76 / 84.31 / 97.35 predicated instructions and 10.06 / 10.97 / 11.58 global loads per field. This is an instruction count, not a count of sectors fetched from DRAM. `before-row-cost.tsv` includes the caller's setup separately. On the same selected ordinals, `affine` reduced packed warp instructions from 11.102M / 14.586M / 68.723M to 10.942M / 14.416M / 67.331M. Its registers were 100 / 64 / 128 / 80 / 128. Reducing field arithmetic did not remove the 132 occupancy step or its caller spills: local-load sectors off/on were 3.592M / 4.046M.

**Retained: `flat` (`7d808306a3`).** Resolve a scan once in `locate_key`: a plain cursor carries only a meaningful slot address; a narrow packed cursor carries a payload origin, block-frame address and local ordinal. For wider compressed leaves it carries the first row word, frame address and relative row-end bit; an identity leaf needs only its row-word address and the identity tag. No field is reconstructed while locating a row. `decode_key` alone combines the demanded residual and base and applies signedness. With the feature off the position is still `u64`. An ordinal on the common fixed path resolves with arithmetic on the 256-row geometry and immutable run metadata: **no header load**. Runs that cannot fit the frame's word offset, nondefault geometry and variable leaves use the general resolver, not truncated addresses. No emitted code, inventory entry, or resident allocation was added.

| Representative hole | Before → final instructions / packed row | Before → final global loads / packed row | Final instructions / field from frame load | Final loads / field from frame load |
|---|---:|---:|---:|---:|
| 122 | 80.78 → 35.16 | 11.07 → 4.03 | 27.13 | 2.03 |
| 123 | 157.11 → 66.72 | 20.34 → 7.09 | 27.27 | 2.17 |
| 132 | 321.06 → 120.38 | 37.74 → 12.85 | 28.13 | 2.28 |

**The compiler did not share the entire resolver.** In 132 the direct arms for columns 1, 2 and 3 each loaded the same run column count (`+0x28`) and frame origin (`+0x10`), and each shifted the ordinal and multiplied by the column count, 114,138 times per PC. Including setup, the second/third field cost **40.86 / 39.71 instructions and 4.47 / 4.18 loads**, versus the requested ~30 and ~2. The single aligned frame load and one (occasionally two) payload loads achieve that target only once the row is resolved. See `flat-inline-cost.tsv` and `flat-h132-caller.tsv`. These regions exclude consumer expressions and the final base addition.

| Five-process median | Before fixpoint ms | Final fixpoint ms | Final pool live / peak bytes |
|---|---:|---:|---:|
| Default | 537.171 | 541.925 | 275,147,264 / 316,326,912 |
| Forced off | 508.282 | 510.696 | 275,113,728 / 316,229,888 |
| Packed on | 529.734 | **526.877** | **114,855,424 / 142,289,408** |

The packed gain (2.857 ms) exceeded the forced-off loss (2.414 ms) in this crossover; both deltas are small compared with the individual process spread. Memory was unchanged in bytes in all configurations. Final rule wall forced/packed was **224.003 / 234.259 ms (+10.256)** versus 226.698 / 240.326 before. Hole 122's aggregate premium fell **6.936 → 2.643 ms**, missing the 2 ms goal; its packed launch fell from 11.102M to 10.618M warp instructions but still used 102 registers and measured 17.2% occupancy. Final registers were **102 / 64 / 128 / 80 / 128**. The 8 ms rule-gap target was also missed: 123's packed interval bodies alone executed 8.457M warp instructions, and 132's executed 25.408M; 132's local-load sectors were 4.591M / 5.500M off/on. These are separate costs from the repeated per-field setup.

**Reproduction and gates.** Under `/data/design-notes/doop-memory/wire-in/cursor/`, `run.sh STAGE packed default formal registers exact trace attribution tests counters analysis` drove the build, gates, three traces per storage policy and six `ncu --set full` captures. Counters used the same symbol families/ordinals, not whichever kernel ranked first in a new trace. `crossover.sh before flat held-flat` rotated five fresh processes per build and policy; `held-flat.txt` has every timer. All GPU process snapshots were empty. The runtime tests passed with and without the feature at each step (final: 139 / 137, one ignored each). Every candidate matched all 26 xalan full tuple sets both ways. The unmodified checker reported 522 registered decode sites, and each default comparison matched all five cubins and all 349 kernel texts, register counts and stacks.
