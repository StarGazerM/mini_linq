# H05 stable compaction and clustered merge probe

Measured 2026-10-18 on RTX 6000 Ada, `sm_89`, using the workspace's pinned
cuda-oxide toolchain. These are hand-written physical alternatives, not changes
to the compiler planner. Point remains the default: merge was **slower** in both
measured cases.

## Implementation and reproduction

`filter_orders` first writes one count per 256-row block. The blocked-run
hierarchical exclusive scan produces deterministic block offsets and a device
selected-row total. A second filter pass scatters each selected row at its block
offset plus its lane's exclusive rank. The result preserves input order and
customer payload alignment without atomic reservations or cardinality readback.
The same scan implementation is reused by run construction. SF1/SF10 Plan A
orders now take the device-detected sorted-adjacency build path. Plan B still
sorts orders by customer, as before.

Plan A's point and merge entry points instantiate the same
`plan_a_impl<const MERGE: bool>` source. Merge sorts 1024 probe keys in shared
memory using `gpu_tiles::sort`, binary-searches the run's 2048-row fences for the
tile span, and stages each covered index block in shared memory. Threads own
merge-path diagonal partitions, with probe-before-index ordering on equal keys,
so repeated lineitem keys all match the same unique orders row. Shared matches
are mapped back to the original input lanes before the remaining joins. The
per-tile runtime guard is **K = 4 blocks**; wider spans use the ordinary rank
point probe. Input keys are already checked by `Data::load` to fit 26 bits,
allowing the existing u32 tile sort and an unambiguous MAX sentinel. Orders use
the TPC-H unique-key contract.

`--unfiltered-orders` (Plan A only) bypasses the date-filter build entirely,
builds all orders, and carries clustered dates for the identical predicate in
the probe kernel. At SF10 the filtered key/customer columns have 2,275,919 rows
(36,414,704 bytes), versus 15,000,000 rows (240,000,000 bytes) unfiltered. The
unfiltered run additionally has 120,000,000 date bytes, plus its directory; it
really exceeds the GPU's 96 MiB L2. Allocations still use the pre-reserved pool,
never CUDA allocation/free in timed regions.

From `experiments/gpu/legacy-datalog`:

```sh
source /data/rccl/scripts/oxide-env.sh
cargo oxide build --arch sm_89 -- --release -p gpu-h05 --bin gpu-h05 --bin filter-oracle --bin blocked-run-oracle
./target/release/filter-oracle
./target/release/blocked-run-oracle
./target/release/gpu-h05 /data/tpch-gpu/sf1 A h05/merge.results.json --merge
./target/release/gpu-h05 /data/tpch-gpu/sf10 A h05/merge.results.json
./target/release/gpu-h05 /data/tpch-gpu/sf10 A h05/merge.results.json --merge
./target/release/gpu-h05 /data/tpch-gpu/sf10 A h05/merge.results.json --unfiltered-orders
./target/release/gpu-h05 /data/tpch-gpu/sf10 A h05/merge.results.json --unfiltered-orders --merge
```

The stable-filter oracle checks exact key/payload order in 28 cases: empty,
full, sparse, partial blocks and recursive scan boundaries. The blocked-run
oracle passed all cases after sharing the scan helper. Synthetic, SF1 and SF10
all passed both probe modes with both filter placements, one warmup plus five
measured runs each; distinct answers were respectively 579, 7,242 and 72,983.
Plan B also passed SF1/SF10 after the shared filter change. `exactness.txt`
retains every final runner line; exactness includes both deduplicated keys and
materialized three-column results.

## Stable compaction: SF10 Plan A build

CUDA-event build medians are uninstrumented (five runs after one warmup).
Nsight Systems sums are a separate capture of **all six iterations**, not
per-iteration medians; they exclude memsets, copies, launch gaps and host waits.
The build kernel sum excludes Plan A and dedup kernels. Individual kernel sums
include the customer/supplier builds as well as orders. Raw summaries are
`nsys-before.csv`, `nsys-after.csv`; structured sums are in
`build-profile.results.json`, with pre-change event samples in
`baseline.results.json`.

| Measurement | Atomic filter before | Stable filter after |
|---|---:|---:|
| CUDA-event `build_ms` median | 4.906112 | 5.467968 |
| nsys build kernel sum, ms / six iterations | 21.354764 | 11.297078 |
| filter kernel sum, ms (6 → 12 calls) | 2.317618 | 3.103384 |
| histogram clear sum, ms | 3.692158 | 0.022592 |
| histogram sum, ms | 2.124657 | 0.223938 |
| word offsets sum, ms | 4.349796 | 0.497250 |
| scatter sum, ms | 2.684150 | 0.223361 |
| provenance order-bins sum, ms | 2.206771 | 0.765894 |
| sorted-starts sum, ms | 0.442242 | 2.482001 |
| sorted-directory sum, ms | 0.433796 | 1.557133 |
| scan-blocks + add-prefix sum, ms | 0.251426 | 0.254915 |

The GPU build-kernel sum decreased 47.10%, but the measured end-to-end build
interval did not improve. The machine was CPU-loaded: final measurement load
averages were (8.39, 6.12, 5.00) before and (8.17, 6.15, 5.02) after, while other
corpus/out-of-tree and reference work could be active. GPU work was serialized
with the triangle experiment. CUDA-event build intervals include CPU launch
and synchronization gaps; these measurements do **not** establish an
end-to-end build speedup or an idle-machine comparison.

## Point versus merge: SF10 Plan A

Kernel times are final uninstrumented five-run medians, taken after profiling.
NCU metrics are from the first measured kernel (`--launch-skip 1 --launch-count
1`, regex `^plan_a$` or `^plan_a_merge$`), using default replay/cache controls.
They are hardware DRAM counters, not logical lineitem bytes; their peak
percentage belongs to the instrumented invocation, not the CUDA-event timing.
No NCU-instrumented timing was included in the medians. Register counts are
`ptxas -v -arch=sm_89` (CUDA 13.0); raw output is `ptxas.txt`. Spill stores,
spill loads and stack frames are all zero bytes.

| Orders build | Probe | `kernel_ms` | DRAM read bytes | DRAM write bytes | DRAM % peak sustained | Registers | Spill bytes (store/load) |
|---|---|---:|---:|---:|---:|---:|---:|
| Filtered, L2-sized key/customer | point | 0.944352 | 568,608,128 | 4,658,304 | 39.52 | 36 | 0 / 0 |
| Filtered, L2-sized key/customer | merge, K=4 | 2.577696 | 567,158,016 | 4,665,216 | 9.85 | 40 | 0 / 0 |
| Unfiltered, >L2 | point | 1.229824 | 789,702,656 | 4,740,352 | 45.20 | 36 | 0 / 0 |
| Unfiltered, >L2 | merge, K=4 | 2.856736 | 843,165,184 | 5,084,288 | 13.62 | 40 | 0 / 0 |

Point uses 9,216 dynamic shared bytes; merge uses 17,408. Merge is 2.73× slower
on filtered orders and 2.32× slower on unfiltered orders. It does not demonstrate
an advantage over the dense rank directory: sorting, shared staging and merge
work outweigh any locality benefit in these cases. Raw NCU CSVs are named
`ncu-{filtered,unfiltered}-{point,merge}.csv`; `merge.results.json` retains all
samples, exactness, live order bytes, mode, K, hardware counters and resources.

Example profile command (replace mode and flags for each table row):

```sh
NCU=/opt/nvidia/hpc_sdk/Linux_x86_64/25.5/compilers/bin/ncu
$NCU --target-processes all --kernel-name 'regex:^plan_a_merge$' \
  --launch-skip 1 --launch-count 1 \
  --metrics dram__bytes_read.sum,dram__bytes_write.sum,dram__throughput.avg.pct_of_peak_sustained_elapsed \
  --csv --log-file /tmp/h05-ncu.csv \
  ./target/release/gpu-h05 /data/tpch-gpu/sf10 A /tmp/h05-profile.json --merge --unfiltered-orders
```
