# Wave 3 workloads evidence

Branch ext/w3-workloads. No commits. Core and other features were not edited.
Only workloads pending lines were removed and 23 regression rows appended.

| Evidence | Before redesign | Current evidence |
|---|---:|---|
| Primary corpus | 7,948 executed scenarios | Full matrix blocked by Hypergraph correspondence; 14,256 planned pipeline configurations, execution count not claimed |
| Full products | Four kernels, 864 configurations each | Four kernels, 3,456 pipelines each; mandatory and currently blocked |
| Pairwise | 16 queries, 27 rows each | Same 27 rows cover all nine pipeline/host/printer axes |
| Authored pairs | 49; 17 extension queries | 20 primary + 29 extended pairs retained; all 29 extended queries execute on all three scenarios (87); 17 extension queries use surface passes |
| Typed corpus | Five CSV/Parquet pairs | All ten format/query tests pass |
| Goldens | 151 | All 151 independently recomputed and byte compared by cq-testkit |
| Compile failures | 1,234 matrix configurations | Matrix count blocked; all 17 separately authored negatives pass (five minimal + twelve extensions) |
| Fuzz | 2,000 executed cases | 200-case gate stalls at generated seed 31 / Hypergraph; no executed batch count claimed; 2,000 includes the same blocker |
| Synthetic benchmark | Instrumented timing | Uninstrumented release timings and separate counted executions pass at default and scale 512 / seed 7 |
| Growth | WCOJ, Yannakakis, canon, native reads, Fine orders | All non-fan predictions pass unchanged; full WCOJ assertion remains failing, smallest rewrite inactive in generated Fine |

Formatting, whitespace, source/dependency scans and clippy on all unblocked
packages (including oracle/Parquet/counter code) pass. Full workspace test and
clippy remain blocked by the mandatory corpus proof. The external gate fails at workloads Cargo metadata because sibling feature paths
were not copied. The root xtask alias and dependency closure need core changes. Exact requests and
commands are in `/tmp/codex/w2/workloads.core_requests.md`; the acceptance ledger
is `/tmp/codex/w2/workloads.ledger.md`. No blockers were converted to passing tests.

TPC timings below are observed medians of five release executions with storage
instrumentation compiled out. Every warm/timed result agrees exactly with DuckDB;
two separately compiled counted executions also agree. Inputs reuse official
DuckDB v1.5.5 dbgen caches; all answers and measurements are fresh. Results are
from this shared host. Individually selected reports do not imply full-catalog
coverage. Hypergraph h07/h08/d19 remain blocked; nine of twelve kernels have both scales and both pipelines restored (36 of 48 timing rows).

| Query | SF | Pipeline | Rust ms | DuckDB one thread ms | DuckDB 24 threads ms |
|---|---:|---|---:|---:|---:|
| retail_d27 | 0.1 | default | 11.125 | 3.398 | 3.502 |
| retail_d27 | 0.1 | hypergraph | 9.744 | 3.398 | 3.502 |
| retail_d72_inner | 0.1 | default | 74.831 | 10.758 | 11.482 |
| retail_d72_inner | 0.1 | generic_smallest | 95.336 | 10.758 | 11.482 |
| retail_d85 | 0.1 | default | 101.333 | 4.644 | 4.747 |
| retail_d85 | 0.1 | generic_smallest | 880.215 | 4.644 | 4.747 |
| retail_h03 | 0.1 | default | 30.835 | 6.400 | 7.210 |
| retail_h03 | 0.1 | hypergraph | 6.781 | 6.400 | 7.210 |
| retail_h05 | 0.1 | default | 76.368 | 6.233 | 5.716 |
| retail_h05 | 0.1 | generic_smallest | 78.425 | 6.233 | 5.716 |
| retail_h09 | 0.1 | default | 143.566 | 10.152 | 10.516 |
| retail_h09 | 0.1 | generic_smallest | 939.123 | 10.152 | 10.516 |
| retail_h21_positive | 0.1 | default | 221.137 | 11.344 | 8.377 |
| retail_h21_positive | 0.1 | generic_smallest | 2463.832 | 11.344 | 8.377 |
| tpch_q06 | 0.1 | default | 4.639 | 1.747 | 0.732 |
| tpch_q06 | 0.1 | generic-smallest | 30.456 | 1.747 | 0.732 |
| tpch_q19 | 0.1 | default | 7.769 | 16.720 | 6.791 |
| tpch_q19 | 0.1 | generic-smallest | 129.447 | 16.720 | 6.791 |
| retail_d27 | 1 | default | 141.178 | 25.529 | 8.714 |
| retail_d27 | 1 | hypergraph | 94.065 | 25.529 | 8.714 |
| retail_d72_inner | 1 | default | 1741.592 | 115.487 | 71.428 |
| retail_d72_inner | 1 | generic_smallest | 2174.627 | 115.487 | 71.428 |
| retail_d85 | 1 | default | 3222.784 | 38.690 | 14.942 |
| retail_d85 | 1 | generic_smallest | 14688.310 | 38.690 | 14.942 |
| retail_h03 | 1 | default | 1110.726 | 42.148 | 11.804 |
| retail_h03 | 1 | hypergraph | 87.142 | 42.148 | 11.804 |
| retail_h05 | 1 | default | 1516.483 | 42.158 | 10.958 |
| retail_h05 | 1 | generic_smallest | 1732.407 | 42.158 | 10.958 |
| retail_h09 | 1 | default | 4966.104 | 217.833 | 50.042 |
| retail_h09 | 1 | generic_smallest | 20847.877 | 217.833 | 50.042 |
| retail_h21_positive | 1 | default | 3478.965 | 102.044 | 25.932 |
| retail_h21_positive | 1 | generic_smallest | 32536.240 | 102.044 | 25.932 |
| tpch_q06 | 1 | default | 51.889 | 19.923 | 3.002 |
| tpch_q06 | 1 | generic-smallest | 574.777 | 19.923 | 3.002 |
| tpch_q19 | 1 | default | 196.772 | 159.755 | 23.727 |
| tpch_q19 | 1 | generic-smallest | 3202.749 | 159.755 | 23.727 |

23 restored regression targets: growth; restored; oracle-unit; golden; fuzz; typed; census; provider_candidates; compile_fail; elimination_cost; planner_0 through planner_8; extended_catalog; extended; extension_laws; extension_compile_fail. Exact TSV rows are in `/tmp/codex/w2/workloads.rows_added.tsv`.
