# TPC-H statistics and hand-authored GPU plans

Measurement scope: the original 17 runnable suite queries. h07, h10, h14 and h19 were excluded from those historical runs for the then-rejected-query reasons. The h07 section now includes a separate exact h07/h07_block run after dictionary disjunction support; its advice uses the existing h07_block statistics for their identical SQL predicate and output.

Measurements use read-only DuckDB 1.5.6 over `/data/tpch-gpu/{sf1,sf10}/dbgen.duckdb`, `SET threads=4`. Manifest row counts are cross-checked by SQL; the manifests contain no min/max fields, so key bounds below are measured with SQL rather than invented. DuckDB t24 benchmark timings are the suite's recorded independent timing files, not the four-thread statistics queries.

The objective used to choose a traversal is the sum of surviving prefix positions, including the root. These counts are not elapsed time predictions: provider builds, predicate scans, host evaluation, canonicalization, and launch costs remain visible in measurements. Connected candidate orders are enumerated in each table; this is a minimum among measured candidates, not an unsupported proof of global runtime optimality. Fully bound relations become membership tests where the checked lowerer permits, so their existential result does not need to enumerate all SQL join multiplicities. `items 4` controls processing granularity, not a four-match maximum.

Exact SQL and all raw counts are retained in [`tpch_statistics.json`](tpch_statistics.json). `OrderAtom` advice uses zero-based normalized body positions, including guards; `Lineitem@1` is not the first occurrence numbered from one. `block@5` schedules the whole checked H07 country-pair block without changing its branch order or relaxing opaque-Rust dependencies. Source queries are unchanged.

## Base tables and key domains

| Relation | SF1 rows | SF10 rows |
|---|---|---|
| region | 5 | 5 |
| nation | 25 | 25 |
| supplier | 10000 | 100000 |
| customer | 150000 | 1500000 |
| orders | 1500000 | 15000000 |
| part | 200000 | 2000000 |
| partsupp | 800000 | 8000000 |
| lineitem | 6001215 | 59986052 |

| Key | SF1 bounds / distinct | SF10 bounds / distinct |
|---|---|---|
| region.r_regionkey | [0, 4] / 5 | [0, 4] / 5 |
| nation.n_nationkey | [0, 24] / 25 | [0, 24] / 25 |
| nation.n_regionkey | [0, 4] / 5 | [0, 4] / 5 |
| supplier.s_suppkey | [1, 10000] / 10,000 | [1, 100000] / 100,000 |
| supplier.s_nationkey | [0, 24] / 25 | [0, 24] / 25 |
| customer.c_custkey | [1, 150000] / 150,000 | [1, 1500000] / 1,500,000 |
| customer.c_nationkey | [0, 24] / 25 | [0, 24] / 25 |
| orders.o_orderkey | [1, 6000000] / 1,500,000 | [1, 60000000] / 15,000,000 |
| orders.o_custkey | [1, 149999] / 99,996 | [1, 1499999] / 999,982 |
| part.p_partkey | [1, 200000] / 200,000 | [1, 2000000] / 2,000,000 |
| partsupp.ps_partkey | [1, 200000] / 200,000 | [1, 2000000] / 2,000,000 |
| partsupp.ps_suppkey | [1, 10000] / 10,000 | [1, 100000] / 100,000 |
| lineitem.l_orderkey | [1, 6000000] / 1,500,000 | [1, 60000000] / 15,000,000 |
| lineitem.l_partkey | [1, 200000] / 200,000 | [1, 2000000] / 2,000,000 |
| lineitem.l_suppkey | [1, 10000] / 10,000 | [1, 100000] / 100,000 |

## Representation and scheduling decisions

`tpch_dictionary` is an explicit loader profile for h03, h07, h07_block, h08, h21_positive, d19, d27 and d85: only literal-equality-compared or unused String columns use their existing manifest-local i32 dictionary representation. Classification traverses relational `Or` arms and `In` sets; opaque predicates and ordered String comparisons retain String storage. `encoded_query_with_types` resolves each literal through its own column dictionary; no global string IDs or string-method semantics are guessed. H09 substring, H16 string methods and H22 phone methods remain host Strings. H05 keeps its existing compressed-domain profile. Disjoint literal domains need actual row filtering rather than a host contiguous-range descriptor, so stored integer `Or`/`In` selects are graded as device publications.

The hand schedules retain selected forward prefixes, use full-key membership only where the checked live layout contains no payload, and place 1,024/items-4 segment boundaries at the measured expanding levels. Independent numeric filter publications are separately partitioned at 256/items-4; merely tiling the join would leave those producers scalar. Single-sided/multi-column range filters that the host range builder cannot express are explicit published device maps, with ancestor stores and recorded grades. Small prefixes stay untiled until an actual expansion or output partition. Count publication is retained.

## Execution and before/after comparison

**34/34 selected query/scale entries are exact.** Every warmup and each of five measured executions compared the full sorted DISTINCT answer with its exported DuckDB oracle. All 17 SF1 entries also passed `cq_testkit::conformance::assert_answers_equal` against Sequential of the same Physical program. Every selected serialized Sequence was matched to the final compiled plan.

The before column uses the fresh main-tree **advised** results from `/data/tpch-gpu/box11d-final-both.json` for the original 14 runnable entries, and the preserved exact advised grouped baseline for h01/h06/h12. Automatic-mode minima are not substituted. All baseline rows and samples are preserved in [`tpch_hand_baseline.json`](tpch_hand_baseline.json). This fresh baseline differs from the historical reference numbers in the task (for example H21 is 110.404 ms rather than the earlier 400.303 ms); the hand-plan speedup is not credited with intervening compiler improvements.

Final measurements: [`tpch_hand.results.json`](tpch_hand.results.json). Its 34 rows retain exact samples and per-row provenance from the full-suite [`tpch_hand.initial.json`](tpch_hand.initial.json), selected replacements from [`tpch_hand.retuned.json`](tpch_hand.retuned.json), and the final H04/H21 check [`tpch_hand.final_prefix.json`](tpch_hand.final_prefix.json). An earlier scalar-filter H21 experiment is preserved separately in [`tpch_hand_h21.initial.json`](tpch_hand_h21.initial.json). GPU measurements were exclusive; Box11d reported that native CPU compilation could overlap the initial full-suite run.

Times are five-sample CUDA-event medians including host evaluation/upload, provider build, kernels and final deduplication; initial input loading, answer download and comparison are excluded, matching the suite. The `kernels` interval can include host-side publication work; it is not evidence that opaque Rust arithmetic ran on the GPU. DuckDB t24 values come from the suite timing files, not the four-thread cardinality measurements.

| Query | Scale | Baseline advised ms | Hand plan ms | Before / after | DuckDB t24 ms | Exact |
|---|---|---|---|---|---|---|
| h01 | SF1 | 922.226 | 915.887 | 1.01× | 64.000 | yes |
| h03 | SF1 | 2.364 | 1.999 | 1.18× | 21.000 | yes |
| h04 | SF1 | 1.160 | 1.158 | 1.00× | 35.000 | yes |
| h04_not_exists | SF1 | 4.054 | 4.265 | 0.95× | 13.000 | yes |
| h05 | SF1 | 2.065 | 2.102 | 0.98× | 20.000 | yes |
| h06 | SF1 | 338.968 | 328.699 | 1.03× | 8.000 | yes |
| h07_block | SF1 | 3.683 | 2.782 | 1.32× | 22.000 | yes |
| h08 | SF1 | 8.419 | 5.034 | 1.67× | 17.000 | yes |
| h09 | SF1 | 6.136 | 5.707 | 1.08× | 453.000 | yes |
| h12 | SF1 | 11.954 | 12.007 | 1.00× | 12.000 | yes |
| h16 | SF1 | 1.567 | 1.647 | 0.95× | 106.000 | yes |
| h22 | SF1 | 1.005 | 0.982 | 1.02× | 46.000 | yes |
| h21_positive | SF1 | 11.077 | 4.312 | 2.57× | 851.000 | yes |
| d19 | SF1 | 3.315 | 2.643 | 1.25× | 31.000 | yes |
| d27 | SF1 | 2.863 | 2.596 | 1.10× | 10.000 | yes |
| d72_inner | SF1 | 5.809 | 4.636 | 1.25× | 83.000 | yes |
| d85 | SF1 | 3.840 | 3.627 | 1.06× | 28.000 | yes |
| h01 | SF10 | 9491.752 | 9318.143 | 1.02× | 524.000 | yes |
| h03 | SF10 | 20.282 | 17.615 | 1.15× | 162.000 | yes |
| h04 | SF10 | 10.333 | 10.260 | 1.01× | 296.000 | yes |
| h04_not_exists | SF10 | 49.186 | 47.331 | 1.04× | 95.000 | yes |
| h05 | SF10 | 10.242 | 10.256 | 1.00× | 158.000 | yes |
| h06 | SF10 | 3534.769 | 3412.623 | 1.04× | 40.000 | yes |
| h07_block | SF10 | 27.084 | 14.948 | 1.81× | 132.000 | yes |
| h08 | SF10 | 80.436 | 27.663 | 2.91× | 110.000 | yes |
| h09 | SF10 | 46.240 | 44.952 | 1.03× | 4397.000 | yes |
| h12 | SF10 | 44.738 | 43.730 | 1.02× | 54.000 | yes |
| h16 | SF10 | 10.081 | 9.966 | 1.01× | 1025.000 | yes |
| h22 | SF10 | 6.717 | 6.793 | 0.99× | 128.000 | yes |
| h21_positive | SF10 | 110.404 | 40.118 | 2.75× | 3797.000 | yes |
| d19 | SF10 | 26.755 | 19.324 | 1.38× | 202.000 | yes |
| d27 | SF10 | 24.960 | 19.474 | 1.28× | 86.000 | yes |
| d72_inner | SF10 | 49.984 | 25.933 | 1.93× | 287.000 | yes |
| d85 | SF10 | 26.781 | 27.083 | 0.99× | 189.000 | yes |

H01/H06/H12 retain their accepted schedules because the attempted arithmetic/host-consumer routes were rejected; their small timing differences are not claimed as plan improvements. H05 likewise retains authored R. Remaining small regressions (including SF1 h04_not_exists/h16 and SF10 d85/h22) are shown, not hidden. This is the best selected set of measured accepted plans, not a proof of globally optimal runtime.

Checks exercised: scoped oxide builds of all 17 queries at both scales; one complete 34-entry exact run; restricted 12-entry and 4-entry exact tuning runs; four explicit-order/block contract tests, the occurrence-qualified ordering test, the facade order parser test, and the facade reduction-selector parser/resolution test. Scoped rustfmt was run on the changed Rust files.

## Resource and output decisions

The accepted schedules use `store flatten ... tile 1024 items 4` only at authored expansion boundaries, and count output. Independent device numeric filters use separately partitioned scans. Grouped host arithmetic retains the previously checked 256-row publication boundaries. All selected plans passed PL-Cost, machine grading and CUDA translation; no attempted candidate produced a BE-Resource/shared-byte rejection, so no unobserved shared-byte total is invented. The BE-Scope rejections below are incorrect intermediate publication grades corrected by explicit ancestor stores and grades, not resource-limit measurements.

## Forward provider experiments

Index-build rows are separate from traversal positions. These cardinalities come from the measured matching-prefix SQL, not selectivity guesses. Extra providers are retained only where their measured schedule is preferable; H04/D19/D85 materialization experiments were rejected on performance, not falsely labeled compiler failures.

| Query | Filtered provider | SF1 rows | SF10 rows | Selection |
|---|---|---|---|---|
| h04 | late Lineitem supported by quarter Orders | 144869 | 1450571 | not retained: measured slower |
| h21_positive | late Lineitem@1 supported by Saudi Supplier | 156739 | 1522366 | retained |
| d19 | Lineitem supported by US customer / quarter Orders | 8963 | 90015 | not retained: measured slower |
| d27 | Lineitem supported by US Supplier, quarter Orders and Brand#12 Part | 357 | 3575 | retained |
| d85 | Lineitem supported by US customer / quarter Orders (Masks) | 8963 | 90015 | not retained: measured slower |

## h01

Single lineitem scan with shipdate cutoff, then exact grouped sum(quantity_units), sum(price_units), count by (returnflag,linestatus). Predicate retains almost the whole relation; optimize scan/reduction, not fictitious selective expansion. Fuse contribution formation into grouped reduction where possible.

| Predicate / cumulative guard | SF1 rows (% of base) | SF10 rows (% of base) |
|---|---|---|
| constants: l_shipdate <= DATE '1998-09-02' | 5,916,591 (98.5899%) | 59,142,609 (98.5939%) |
| guards: l_shipdate <= DATE '1998-09-02' | 5,916,591 (98.5899%) | 59,142,609 (98.5939%) |
| cumulative: (l_shipdate <= DATE '1998-09-02') | 5,916,591 (98.5899%) | 59,142,609 (98.5939%) |

| Candidate order | SF1 prefixes | SF1 sum | SF10 prefixes | SF10 sum |
|---|---|---|---|---|
| lineitem | [5916591] | 5916591 | [59142609] | 59142609 |

| Scale | Scanned Lineitem rows | Aggregate inputs | Output groups |
|---|---|---|---|
| 1 | 6001215 | 5916591 | 4 |
| 10 | 59986052 | 59142609 | 4 |

**Segmentation:** No join-expansion segmentation boundary: this query has no join.

| Filtered key cardinality | SF1 | SF10 |
|---|---|---|
| rows | 5916591 | 59142609 |
| distinct_order_keys | 1493405 | 14934546 |
| distinct_line_numbers | 7 | 7 |

**Accepted boundary:** exact integer bind maps stay on host, then distinct valuation publication and grouped reduction. Direct device binding was rejected with `BE-Partial: opaque Physical Rust must be evaluated at host before dispatch`; no translator workaround is introduced.

## h03

Drive selected BUILDING customers, expand date-qualified orders, then shipdate-qualified lineitems. The selected customer root beats date/order and lineitem roots at both scales. Preserve DISTINCT(order,customer).

Aliases: `c`=customer, `o`=orders, `l`=lineitem.

| Predicate | SF1 independent / base | SF10 independent / base | SF1 cumulative | SF10 cumulative |
|---|---|---|---|---|
| c: c.c_mktsegment='BUILDING' | 30,142 / 150,000 | 300,276 / 1,500,000 | 30,142 | 300,276 |
| o: o.o_orderdate < DATE '1995-03-15' | 727,305 / 1,500,000 | 7,289,442 / 15,000,000 | 727,305 | 7,289,442 |
| l: l.l_shipdate > DATE '1995-03-15' | 3,241,776 / 6,001,215 | 32,334,250 / 59,986,052 | 3,241,776 | 32,334,250 |

| Candidate order | SF1 prefixes | SF1 sum | SF10 prefixes | SF10 sum |
|---|---|---|---|---|
| c → o → l | 30,142 → 147,126 → 30,519 | 207,787 | 300,276 → 1,461,923 → 302,114 | 2,064,313 |
| o → c → l | 727,305 → 147,126 → 30,519 | 904,950 | 7,289,442 → 1,461,923 → 302,114 | 9,053,479 |
| o → l → c | 727,305 → 151,331 → 30,519 | 909,155 | 7,289,442 → 1,498,960 → 302,114 | 9,090,516 |
| l → o → c | 3,241,776 → 151,331 → 30,519 | 3,423,626 | 32,334,250 → 1,498,960 → 302,114 | 34,135,324 |

| Selected distinct key | SF1 | SF10 |
|---|---|---|
| c.c_custkey | 30142 | 300276 |
| c.c_nationkey | 25 | 25 |
| o.o_orderkey | 727305 | 7289442 |
| o.o_custkey | 99606 | 996205 |
| l.l_orderkey | 829958 | 8278099 |
| l.l_partkey | 200000 | 2000000 |
| l.l_suppkey | 10000 | 100000 |
| l.(l_partkey,l_suppkey) | 786241 | 7863420 |

| Chosen keyed step | SF1 post-filter mean / max | SF10 post-filter mean / max |
|---|---|---|
| c → o | 4.88110 / 23 | 4.86860 / 25 |
| c → o → l | 0.20743 / 7 | 0.20666 / 7 |

Final DISTINCT output rows: **11,620 / 114,003**.

**Authored schedule:** retain constant-only strings as dictionary IDs, publish the single-sided Orders/Lineitem date filters as explicit numeric device maps, segment the 4.88× Orders expansion, and use membership for the payload-free Lineitem existence test.

## h04

Filter Orders by the quarter first; probe Lineitem by orderkey and apply commitdate<receiptdate before emitting. The reverse order gets a unique Orders lookup but starts from far more late Lineitem rows. Preserve DISTINCT order_key; if a semijoin executor is available, one existence probe per selected order avoids materializing duplicate matches without changing the authored set result.

| Local predicate | SF1 selected / base | SF10 selected / base |
|---|---|---|
| Orders lower_bound_rows | 1,160,553 / 1,500,000 (77.3702%) | 11,588,985 / 15,000,000 (77.2599%) |
| Orders upper_bound_rows | 396,665 / 1,500,000 (26.4443%) | 3,984,686 / 15,000,000 (26.5646%) |
| Orders cumulative_rows | 57,218 / 1,500,000 (3.8145%) | 573,671 / 15,000,000 (3.8245%) |
| Lineitem selected_rows | 3,793,296 / 6,001,215 (63.2088%) | 37,929,348 / 59,986,052 (63.2303%) |
| Orders_conditional_upper_bound | 57,218 / 1,160,553 (4.9302%) | 573,671 / 11,588,985 (4.9501%) |

| Candidate order | SF1 prefixes | SF1 sum | SF10 prefixes | SF10 sum |
|---|---|---|---|---|
| Orders → Lineitem | [57218, 144869] | 202087 | [573671, 1450571] | 2024242 |
| Lineitem → Orders | [3793296, 144869] | 3938165 | [37929348, 1450571] | 39379919 |

| Correlation/output metric | SF1 | SF10 |
|---|---|---|
| positive_root_rows | 57218 | 573671 |
| late_rows | 3793296 | 37929348 |
| late_distinct_correlation_keys | 1375365 | 13753474 |
| joined_positions | 144869 | 1450571 |
| output_rows | 52523 | 526040 |
| exists_selectivity_local | 52,523 / 57,218 (91.7945%) | 526,040 / 573,671 (91.6972%) |
| exists_selectivity_all_orders | 1,375,365 / 1,500,000 (91.6910%) | 13,753,474 / 15,000,000 (91.6898%) |
| cumulative_output_selectivity | 52,523 / 1,500,000 (3.5015%) | 526,040 / 15,000,000 (3.5069%) |
| late_key_domain_selectivity | 1,375,365 / 1,500,000 (91.6910%) | 13,753,474 / 15,000,000 (91.6898%) |
| output_correlation_keys | 52523 | 526040 |
| upper_bound_after_lower_bound_selectivity | 57,218 / 1,160,553 (4.9302%) | 573,671 / 11,588,985 (4.9501%) |

**Segmentation:** {"filtered_index": "No mean>4 boundary (mean 2.532/2.529); fanout tail still reaches 7, so bounded-width implementations must handle >4 buckets.", "unfiltered_index": "Place a boundary after filtered Orders and before raw Lineitem expansion: measured local raw mean is 4.0143 at SF1 and 4.00030 at SF10, both >4; apply the line guard before subsequent work.", "existence_lookup": "No expansion boundary needed for true existence membership."}

| Join-key/fanout measurement | SF1 | SF10 |
|---|---|---|
| fanout_order_lineitem: parent_rows | 1500000 | 15000000 |
| fanout_order_lineitem: matched_parent_keys | 1500000 | 15000000 |
| fanout_order_lineitem: child_positions | 6001215 | 59986052 |
| fanout_order_lineitem: min_fanout | 1 | 1 |
| fanout_order_lineitem: max_fanout | 7 | 7 |
| fanout_order_lineitem: mean_fanout_all_parents | 4.00081 | 3.9990701333333334 |
| fanout_order_lineitem: mean_fanout_matched_parents | 4.00081 | 3.9990701333333334 |
| fanout_order_lineitem: parent_keys_fanout_gt4 | 643287 | 6425574 |
| fanout_order_lineitem: positions_in_gt4_buckets | 3860126 | 38551645 |
| fanout_selected_order_lineitem: parent_rows | 57218 | 573671 |
| fanout_selected_order_lineitem: matched_parent_keys | 57218 | 573671 |
| fanout_selected_order_lineitem: child_positions | 229691 | 2294856 |
| fanout_selected_order_lineitem: min_fanout | 1 | 1 |
| fanout_selected_order_lineitem: max_fanout | 7 | 7 |
| fanout_selected_order_lineitem: mean_fanout_all_parents | 4.014313677514069 | 4.0002998234179525 |
| fanout_selected_order_lineitem: mean_fanout_matched_parents | 4.014313677514069 | 4.0002998234179525 |
| fanout_selected_order_lineitem: parent_keys_fanout_gt4 | 24743 | 245896 |
| fanout_selected_order_lineitem: positions_in_gt4_buckets | 148395 | 1475106 |
| fanout_order_late_lineitem: parent_rows | 1500000 | 15000000 |
| fanout_order_late_lineitem: matched_parent_keys | 1375365 | 13753474 |
| fanout_order_late_lineitem: child_positions | 3793296 | 37929348 |
| fanout_order_late_lineitem: min_fanout | 0 | 0 |
| fanout_order_late_lineitem: max_fanout | 7 | 7 |
| fanout_order_late_lineitem: mean_fanout_all_parents | 2.528864 | 2.5286232 |
| fanout_order_late_lineitem: mean_fanout_matched_parents | 2.7580285960454134 | 2.7578012653384882 |
| fanout_order_late_lineitem: parent_keys_fanout_gt4 | 188507 | 1884500 |
| fanout_order_late_lineitem: positions_in_gt4_buckets | 1008972 | 10084473 |
| fanout_selected_order_late_lineitem: parent_rows | 57218 | 573671 |
| fanout_selected_order_late_lineitem: matched_parent_keys | 52523 | 526040 |
| fanout_selected_order_late_lineitem: child_positions | 144869 | 1450571 |
| fanout_selected_order_late_lineitem: min_fanout | 0 | 0 |
| fanout_selected_order_late_lineitem: max_fanout | 7 | 7 |
| fanout_selected_order_late_lineitem: mean_fanout_all_parents | 2.5318780803243737 | 2.528576483733708 |
| fanout_selected_order_late_lineitem: mean_fanout_matched_parents | 2.758201169011671 | 2.757529845639115 |
| fanout_selected_order_late_lineitem: parent_keys_fanout_gt4 | 7116 | 71795 |
| fanout_selected_order_late_lineitem: positions_in_gt4_buckets | 38071 | 384031 |

**Provider experiment (not selected):** selective forward materialization was exact, but its extra scans/publications cost more than the selected accepted schedule. Its measurements are retained in `tpch_hand.retuned.json`; no predicate was deleted.

**Selected runtime fallback:** retain the accepted Lineitem → Orders plan. Orders-first has fewer logical prefixes, but the materialized late-line provider measured 20.332 ms SF10 versus the fresh 10.333 ms advised fact-scan baseline; raw prefix count is not an index-build cost model.

## h04_not_exists

Quarter-filter Orders, then anti-probe the late-lineitem orderkey set. Deduplicate only the correlation key for membership; never seed positive rows from LateLineitem.

| Local predicate | SF1 selected / base | SF10 selected / base |
|---|---|---|
| Orders lower_bound_rows | 1,160,553 / 1,500,000 (77.3702%) | 11,588,985 / 15,000,000 (77.2599%) |
| Orders upper_bound_rows | 396,665 / 1,500,000 (26.4443%) | 3,984,686 / 15,000,000 (26.5646%) |
| Orders cumulative_rows | 57,218 / 1,500,000 (3.8145%) | 573,671 / 15,000,000 (3.8245%) |
| Lineitem selected_rows | 3,793,296 / 6,001,215 (63.2088%) | 37,929,348 / 59,986,052 (63.2303%) |
| Orders_conditional_upper_bound | 57,218 / 1,160,553 (4.9302%) | 573,671 / 11,588,985 (4.9501%) |

| Candidate order | SF1 prefixes | SF1 sum | SF10 prefixes | SF10 sum |
|---|---|---|---|---|
| Orders → ANTI LateLineitem | [57218] | 57218 | [573671] | 573671 |

| Correlation/output metric | SF1 | SF10 |
|---|---|---|
| positive_root_rows | 57218 | 573671 |
| anti_build_rows | 3793296 | 37929348 |
| anti_build_distinct_correlation_keys | 1375365 | 13753474 |
| anti_rejected_rows | 52523 | 526040 |
| output_rows | 4695 | 47631 |
| anti_selectivity_local | 4,695 / 57,218 (8.2055%) | 47,631 / 573,671 (8.3028%) |
| anti_selectivity_all_orders | 124,635 / 1,500,000 (8.3090%) | 1,246,526 / 15,000,000 (8.3102%) |
| cumulative_output_selectivity | 4,695 / 1,500,000 (0.3130%) | 47,631 / 15,000,000 (0.3175%) |
| output_correlation_keys | 4695 | 47631 |

**Segmentation:** "No positive expanding edge. LateLineitem duplicate-key fanout is only an anti membership lookup and must not cause output expansion or a false fanout>4 segment."

| Join-key/fanout measurement | SF1 | SF10 |
|---|---|---|
| fanout_order_lineitem: parent_rows | 1500000 | 15000000 |
| fanout_order_lineitem: matched_parent_keys | 1500000 | 15000000 |
| fanout_order_lineitem: child_positions | 6001215 | 59986052 |
| fanout_order_lineitem: min_fanout | 1 | 1 |
| fanout_order_lineitem: max_fanout | 7 | 7 |
| fanout_order_lineitem: mean_fanout_all_parents | 4.00081 | 3.9990701333333334 |
| fanout_order_lineitem: mean_fanout_matched_parents | 4.00081 | 3.9990701333333334 |
| fanout_order_lineitem: parent_keys_fanout_gt4 | 643287 | 6425574 |
| fanout_order_lineitem: positions_in_gt4_buckets | 3860126 | 38551645 |
| fanout_selected_order_lineitem: parent_rows | 57218 | 573671 |
| fanout_selected_order_lineitem: matched_parent_keys | 57218 | 573671 |
| fanout_selected_order_lineitem: child_positions | 229691 | 2294856 |
| fanout_selected_order_lineitem: min_fanout | 1 | 1 |
| fanout_selected_order_lineitem: max_fanout | 7 | 7 |
| fanout_selected_order_lineitem: mean_fanout_all_parents | 4.014313677514069 | 4.0002998234179525 |
| fanout_selected_order_lineitem: mean_fanout_matched_parents | 4.014313677514069 | 4.0002998234179525 |
| fanout_selected_order_lineitem: parent_keys_fanout_gt4 | 24743 | 245896 |
| fanout_selected_order_lineitem: positions_in_gt4_buckets | 148395 | 1475106 |
| fanout_order_late_lineitem: parent_rows | 1500000 | 15000000 |
| fanout_order_late_lineitem: matched_parent_keys | 1375365 | 13753474 |
| fanout_order_late_lineitem: child_positions | 3793296 | 37929348 |
| fanout_order_late_lineitem: min_fanout | 0 | 0 |
| fanout_order_late_lineitem: max_fanout | 7 | 7 |
| fanout_order_late_lineitem: mean_fanout_all_parents | 2.528864 | 2.5286232 |
| fanout_order_late_lineitem: mean_fanout_matched_parents | 2.7580285960454134 | 2.7578012653384882 |
| fanout_order_late_lineitem: parent_keys_fanout_gt4 | 188507 | 1884500 |
| fanout_order_late_lineitem: positions_in_gt4_buckets | 1008972 | 10084473 |
| fanout_selected_order_late_lineitem: parent_rows | 57218 | 573671 |
| fanout_selected_order_late_lineitem: matched_parent_keys | 52523 | 526040 |
| fanout_selected_order_late_lineitem: child_positions | 144869 | 1450571 |
| fanout_selected_order_late_lineitem: min_fanout | 0 | 0 |
| fanout_selected_order_late_lineitem: max_fanout | 7 | 7 |
| fanout_selected_order_late_lineitem: mean_fanout_all_parents | 2.5318780803243737 | 2.528576483733708 |
| fanout_selected_order_late_lineitem: mean_fanout_matched_parents | 2.758201169011671 | 2.757529845639115 |
| fanout_selected_order_late_lineitem: parent_keys_fanout_gt4 | 7116 | 71795 |
| fanout_selected_order_late_lineitem: positions_in_gt4_buckets | 38071 | 384031 |

## h05

Aliases: `r` = `region`, `n` = `nation`, `c` = `customer`, `o` = `orders`, `l` = `lineitem`, `s` = `supplier`. All prefix counts apply every predicate whose aliases are bound.

| Predicate | SF1 selected / base | SF10 selected / base |
|---|---|---|
| r.r_name='ASIA' | 1 / 5 (20.0000%) | 1 / 5 (20.0000%) |
| o.o_orderdate >= DATE '1994-01-01' | 1,046,266 / 1,500,000 (69.7511%) | 10,442,157 / 15,000,000 (69.6144%) |
| o.o_orderdate < DATE '1995-01-01' | 681,331 / 1,500,000 (45.4221%) | 6,833,762 / 15,000,000 (45.5584%) |
| o.o_orderdate >= DATE '1994-01-01' AND o.o_orderdate < DATE '1995-01-01' | 227,597 / 1,500,000 (15.1731%) | 2,275,919 / 15,000,000 (15.1728%) |

| Candidate order | SF1 prefixes | SF1 sum | SF10 prefixes | SF10 sum |
|---|---|---|---|---|
| r n c o l s | 1 → 5 → 30,183 → 46,008 → 184,082 → 7,243 | 267,522 | 1 → 5 → 300,270 → 456,771 → 1,825,856 → 72,985 | 2,655,888 |
| r n s l o c | 1 → 5 → 2,003 → 1,201,113 → 182,183 → 7,243 | 1,392,548 | 1 → 5 → 20,037 → 12,021,065 → 1,825,042 → 72,985 | 13,939,135 |
| o c n r l s | 227,597 → 227,597 → 227,597 → 46,008 → 184,082 → 7,243 | 920,124 | 2,275,919 → 2,275,919 → 2,275,919 → 456,771 → 1,825,856 → 72,985 | 9,183,369 |
| l o c n r s | 6,001,215 → 910,519 → 910,519 → 910,519 → 184,082 → 7,243 | 8,924,097 | 59,986,052 → 9,103,367 → 9,103,367 → 9,103,367 → 1,825,856 → 72,985 | 89,194,994 |
| s n r l o c | 10,000 → 10,000 → 2,003 → 1,201,113 → 182,183 → 7,243 | 1,412,542 | 100,000 → 100,000 → 20,037 → 12,021,065 → 1,825,042 → 72,985 | 14,139,129 |

| Key measurement SQL | SF1 rows / distinct / mean fanout | SF10 rows / distinct / mean fanout |
|---|---|---|
| SELECT count(*),count(DISTINCT c_nationkey) FROM customer | 150,000 / 25 / 6000.00000 | 1,500,000 / 25 / 60000.00000 |
| SELECT count(*),count(DISTINCT o_custkey) FROM orders WHERE o_orderdate>=DATE '1994-01-01' AND o_orderdate<DATE '1995-01-01' | 227,597 / 86,608 / 2.62790 | 2,275,919 / 866,336 / 2.62706 |
| SELECT count(*),count(DISTINCT l_orderkey) FROM lineitem | 6,001,215 / 1,500,000 / 4.00081 | 59,986,052 / 15,000,000 / 3.99907 |

**Hand-plan reasoning:** Choose Region → Nation → Customer → Orders → Lineitem → Supplier. The ASIA root is one row, nation→customer fanout is 6,036.6 / 60,054, and only 1.52 dated orders per retained customer survive. Preserve the existing retained customer stream, the 1,024×4 Orders/Lineitem boundaries, count output, compressed u32 domains and full-key Supplier membership. The final nation cycle removes about 96% of the Lineitem prefix. This is the existing authored-R baseline, not a fabricated new improvement.

## h06

Single lineitem scan; date window -> discount window -> quantity is the measured minimum-position staged guard order. Reduce price.scaled()*discount.scaled() directly into a single scale-four integer sum; no join and no expanded tuple materialization needed.

| Predicate / cumulative guard | SF1 rows (% of base) | SF10 rows (% of base) |
|---|---|---|
| constants: l_shipdate >= DATE '1994-01-01' | 4,336,142 (72.2544%) | 43,280,901 (72.1516%) |
| constants: l_shipdate < DATE '1995-01-01' | 2,574,528 (42.9001%) | 25,804,316 (43.0172%) |
| constants: l_discount >= 0.05 | 3,273,484 (54.5470%) | 32,721,660 (54.5488%) |
| constants: l_discount <= 0.07 | 4,365,288 (72.7401%) | 43,625,954 (72.7268%) |
| constants: l_quantity < 24 | 2,758,822 (45.9711%) | 27,590,886 (45.9955%) |
| guards: l_shipdate >= DATE '1994-01-01' AND l_shipdate < DATE '1995-01-01' | 909,455 (15.1545%) | 9,099,165 (15.1688%) |
| guards: l_discount >= 0.05 AND l_discount <= 0.07 | 1,637,557 (27.2871%) | 16,361,562 (27.2756%) |
| guards: l_quantity < 24 | 2,758,822 (45.9711%) | 27,590,886 (45.9955%) |
| cumulative: (l_shipdate >= DATE '1994-01-01' AND l_shipdate < DATE '1995-01-01') | 909,455 (15.1545%) | 9,099,165 (15.1688%) |
| cumulative: (l_shipdate >= DATE '1994-01-01' AND l_shipdate < DATE '1995-01-01') AND (l_discount >= 0.05 AND l_discount <= 0.07) | 248,078 (4.1338%) | 2,480,002 (4.1343%) |
| cumulative: (l_shipdate >= DATE '1994-01-01' AND l_shipdate < DATE '1995-01-01') AND (l_discount >= 0.05 AND l_discount <= 0.07) AND (l_quantity < 24) | 114,160 (1.9023%) | 1,139,264 (1.8992%) |

| Candidate order | SF1 prefixes | SF1 sum | SF10 prefixes | SF10 sum |
|---|---|---|---|---|
| lineitem | [114160] | 114160 | [1139264] | 1139264 |

| Scale | Scanned Lineitem rows | Aggregate inputs | Output groups |
|---|---|---|---|
| 1 | 6001215 | 114160 | 1 |
| 10 | 59986052 | 1139264 | 1 |

**Segmentation:** No join-expansion segmentation boundary: this query has no join.

| Filtered key cardinality | SF1 | SF10 |
|---|---|---|
| rows | 114160 | 1139264 |
| distinct_order_keys | 91970 | 919421 |
| distinct_line_numbers | 7 | 7 |

**Accepted boundary:** retain the host arithmetic boundary. Device binding has the same BE-Partial rejection as H01. A forward-filter store before host arithmetic additionally reached generated-Rust errors E0277 (`RangeProvider` cannot be built from the row iterator) and E0599 (`RangeProvider` has no `iter`). The exact diagnostics are in the rejection log; this is not a successful optimization.

## h07

**Authored disjunction plan:** `no_reduce, order(Nation@6, Nation@5, Customer, Orders, Lineitem, Supplier)`. In `retail_h07.rs`, the date predicate occupies occurrence 1, Supplier is occurrence 2, and the supplier/customer Nation roles are occurrences 5/6 respectively. Both Nation occurrences bind before the unchanged France/Germany disjunction is evaluated; the surviving opposite-country pair drives nation-indexed Customer, customer-indexed Orders, date-qualified order-indexed Lineitem, and the opposite Supplier lookup. Two independent IN filters would admit same-country pairs and are not a substitute.

The identical SQL in h07_block supplies the measured candidate comparison below: the customer branch has prefix sums **287,630 / 2,848,416**, versus **298,132 / 2,986,893** for the supplier branch. The two-row pair restricts Customer to **12,008 / 120,469**, then Orders to **121,324 / 1,205,808**, dated Lineitem to **148,370 / 1,463,770**, and opposite Supplier witnesses to **5,924 / 58,365** before final DISTINCT. These are reused SQL statistics, not measurements of h07 execution. The first single-Nation prefix counts in the table apply the implied country restriction; the authored disjunction need only filter once both Nation roles are available.

Unlike h07_block's `block@5` root, h07 explicitly orders its two Nation occurrences. It uses the same dictionary profile, host-read construction, retained Customer prefix, and 1,024/items-4 Orders segmentation as h07_block. Retaining Customer exposes the selected forward stream before the measured ~10× Orders expansion (maximum 37 / 44); the subsequent date-filtered Lineitem step and full-key Supplier membership retain the existing block scheduling policy. The generic checked membership selection applies only when no Supplier payload remains. No reverse reductions, string-method encoding, or new disjunction semantics are introduced.

**Dictionary-disjunction acceptance run (release, sm_89):** both queries are exact at both scales, with **5,501 SF1 / 54,142 SF10** DISTINCT answers. Every warmup and all five measured runs compare complete answers against `answer.retail_h07.tsv` or `answer.retail_h07_block.tsv` and their count files. Both SF1 runs also pass Sequential-of-the-same-Physical conformance. H07's stored Nation filter and coupled pair select execute over column-local i32 codes on the device; the literal pair is not evaluated as an opaque host String guard.

| Query | Scale | GPU total ms | Build / kernels / dedup ms | DuckDB t24 ms | Exact |
|---|---|---:|---|---:|---|
| h07 | SF1 | 2.828 | 1.668 / 1.018 / 0.136 | 23.000 | yes |
| h07_block | SF1 | 2.913 | 1.803 / 0.974 / 0.136 | 22.000 | yes |
| h07 | SF10 | 15.025 | 13.100 / 1.753 / 0.161 | 130.000 | yes |
| h07_block | SF10 | 15.159 | 13.242 / 1.737 / 0.162 | 132.000 | yes |

Evidence: [`h07_dictionary.results.json`](h07_dictionary.results.json) preserves samples, recorded plans, exactness, conformance, and DuckDB provenance. The full runner artifact is `/data/tpch-gpu/h07-dictionary.results.json`. GPU use was exclusive; unrelated CPU builds could overlap. These are medians of five stage-sum measurements, with initial loading, answer download and comparison excluded. DuckDB values are the existing complete-SQL t24 measurements, not new or timing-boundary-identical benchmarks. H07 is approximately level with h07_block, not evidence of a substantial performance improvement.

```sh
cd experiments/gpu
export CUDA_HOME=/opt/nvidia/hpc_sdk/Linux_x86_64/25.5/cuda/12.9
export CARGO_BUILD_JOBS=1 CQ_GPU_TPCH_QUERIES=h07,h07_block
cargo oxide build --arch sm_89 -- --release -p cq-gpu-tpch
./target/release/cq-gpu-tpch /data/tpch-gpu/h07-dictionary.results.json \
  --mode advised --query h07 --query h07_block --scale sf1 --scale sf10
```

Verification: `cargo test --release -p cq-sequence-plan --lib` passed **21 tests**, including integer Or/In streaming and stored-device placement and String host-placement regressions. With the same two-query build filter, `cargo oxide test --arch sm_89 -- --release -p cq-gpu-tpch --lib -- --test-threads=1` passed **36 tests**, including positive h07/h07_block conformance, schedule replay, dictionary Or/In encoding, and opaque String retention. Non-selected query conformance cases have no generated executors; this is not a full-suite GPU execution claim. The H05 schedule-inspection test now follows the same build-selected inventory rule.

`cargo oxide test --arch sm_89 -- --release -p cq-gpu-sequence --test rows` passed **9 translation tests**, including an Or of two encoded equalities emitted as a native device `||` condition without host equality helpers or range selection. Scoped rustfmt was applied to the changed Rust files; existing CUDA exported-inline warnings remain unchanged.

## h07_block

Precompute exact two-row opposite-country support; use reduced customers, orders, date-qualified lineitems, then supplier lookup against the opposite nation. Customer branch beats supplier branch at both scales despite the much smaller supplier root. Do not replace opposite-country coupling with two independent IN filters. If nested Nation occurrences cannot be plan roots, use Customer→Orders→Lineitem→Supplier with the country-pair support retained in existing masks.

Aliases: `nc`=nation, `ns`=nation, `c`=customer, `o`=orders, `l`=lineitem, `s`=supplier.

| Predicate | SF1 independent / base | SF10 independent / base | SF1 cumulative | SF10 cumulative |
|---|---|---|---|---|
| nc: nc.n_name IN ('FRANCE','GERMANY') | 2 / 25 | 2 / 25 | 2 | 2 |
| ns: ns.n_name IN ('FRANCE','GERMANY') | 2 / 25 | 2 / 25 | 2 | 2 |
| l: l.l_shipdate >= DATE '1995-01-01' | 3,426,687 / 6,001,215 | 34,181,736 / 59,986,052 | 3,426,687 | 34,181,736 |
| l: l.l_shipdate <= DATE '1996-12-31' | 4,402,978 / 6,001,215 | 44,034,641 / 59,986,052 | 1,828,450 | 18,230,325 |

| Candidate order | SF1 prefixes | SF1 sum | SF10 prefixes | SF10 sum |
|---|---|---|---|---|
| nc → ns → c → o → l → s | 2 → 2 → 12,008 → 121,324 → 148,370 → 5,924 | 287,630 | 2 → 2 → 120,469 → 1,205,808 → 1,463,770 → 58,365 | 2,848,416 |
| ns → nc → s → l → o → c | 2 → 2 → 798 → 145,703 → 145,703 → 5,924 | 298,132 | 2 → 2 → 8,010 → 1,460,257 → 1,460,257 → 58,365 | 2,986,893 |
| l → s → ns → nc → o → c | 1,828,450 → 1,828,450 → 145,703 → 145,703 → 145,703 → 5,924 | 4,099,933 | 18,230,325 → 18,230,325 → 1,460,257 → 1,460,257 → 1,460,257 → 58,365 | 40,899,786 |
| l → o → c → nc → ns → s | 1,828,450 → 1,828,450 → 1,828,450 → 148,370 → 148,370 → 5,924 | 5,788,014 | 18,230,325 → 18,230,325 → 18,230,325 → 1,463,770 → 1,463,770 → 58,365 | 57,676,880 |
| ns → nc → c → o → l → s | 2 → 2 → 12,008 → 121,324 → 148,370 → 5,924 | 287,630 | 2 → 2 → 120,469 → 1,205,808 → 1,463,770 → 58,365 | 2,848,416 |

| Selected distinct key | SF1 | SF10 |
|---|---|---|
| nc.n_nationkey | 2 | 2 |
| ns.n_nationkey | 2 | 2 |
| l.l_orderkey | 495603 | 4942798 |
| l.l_partkey | 199982 | 1999806 |
| l.l_suppkey | 10000 | 100000 |
| l.(l_partkey,l_suppkey) | 718828 | 7188115 |

| Chosen keyed step | SF1 post-filter mean / max | SF10 post-filter mean / max |
|---|---|---|
| nc → ns | 1.00000 / 1 | 1.00000 / 1 |
| nc → ns → c | 6004.00000 / 6100 | 60234.50000 / 60316 |
| nc → ns → c → o | 10.10360 / 37 | 10.00928 / 44 |
| nc → ns → c → o → l | 1.22292 / 7 | 1.21393 / 7 |
| nc → ns → c → o → l → s | 0.03993 / 1 | 0.03987 / 1 |

Final DISTINCT output rows: **5,501 / 54,142**.

**Authored block-root plan:** `order(block@5, Customer, Orders, Lineitem, Supplier)` with `no_reduce` selects the two-row country-pair block as one checked unit. Its root replaces the two SQL nation prefixes by a single two-row prefix (subtract two from the displayed candidate sums). Carrying both nation keys makes Supplier a full-key cycle-closing membership lookup; neither a both-IN approximation nor reverse Supplier→Lineitem→Orders masks is used. The retained customer stream precedes the ~10× Orders expansion.

| Individual branch constant | SF1 selected / base | SF10 selected / base |
|---|---|---|
| nc.n_name='FRANCE' | 1 / 25 | 1 / 25 |
| nc.n_name='GERMANY' | 1 / 25 | 1 / 25 |
| ns.n_name='FRANCE' | 1 / 25 | 1 / 25 |
| ns.n_name='GERMANY' | 1 / 25 | 1 / 25 |

## h08

Aliases: `r` = `region`, `n` = `nation`, `c` = `customer`, `o` = `orders`, `l` = `lineitem`, `p` = `part`, `s` = `supplier`, `t` = `nation`. All prefix counts apply every predicate whose aliases are bound.

| Predicate | SF1 selected / base | SF10 selected / base |
|---|---|---|
| r.r_name='AMERICA' | 1 / 5 (20.0000%) | 1 / 5 (20.0000%) |
| p.p_type='ECONOMY ANODIZED STEEL' | 1,451 / 200,000 (0.7255%) | 13,452 / 2,000,000 (0.6726%) |
| o.o_orderdate>=DATE '1995-01-01' | 818,669 / 1,500,000 (54.5779%) | 8,166,238 / 15,000,000 (54.4416%) |
| o.o_orderdate<=DATE '1996-12-31' | 1,138,594 / 1,500,000 (75.9063%) | 11,391,275 / 15,000,000 (75.9418%) |
| o.o_orderdate>=DATE '1995-01-01' AND o.o_orderdate<=DATE '1996-12-31' | 457,263 / 1,500,000 (30.4842%) | 4,557,513 / 15,000,000 (30.3834%) |

| Candidate order | SF1 prefixes | SF1 sum | SF10 prefixes | SF10 sum |
|---|---|---|---|---|
| p l o c n r s t | 1,451 → 43,693 → 13,389 → 13,389 → 13,389 → 2,603 → 2,603 → 2,603 | 93,120 | 13,452 → 403,487 → 122,404 → 122,404 → 122,404 → 24,254 → 24,254 → 24,254 | 856,913 |
| r n c o l p s t | 1 → 5 → 29,952 → 91,179 → 365,091 → 2,603 → 2,603 → 2,603 | 494,037 | 1 → 5 → 299,436 → 910,360 → 3,638,929 → 24,254 → 24,254 → 24,254 | 4,921,493 |
| o c n r l p s t | 457,263 → 457,263 → 457,263 → 91,179 → 365,091 → 2,603 → 2,603 → 2,603 | 1,835,868 | 4,557,513 → 4,557,513 → 4,557,513 → 910,360 → 3,638,929 → 24,254 → 24,254 → 24,254 | 18,294,590 |
| l p o c n r s t | 6,001,215 → 43,693 → 13,389 → 13,389 → 13,389 → 2,603 → 2,603 → 2,603 | 6,092,884 | 59,986,052 → 403,487 → 122,404 → 122,404 → 122,404 → 24,254 → 24,254 → 24,254 | 60,829,513 |
| s t l p o c n r | 10,000 → 10,000 → 6,001,215 → 43,693 → 13,389 → 13,389 → 13,389 → 2,603 | 6,107,678 | 100,000 → 100,000 → 59,986,052 → 403,487 → 122,404 → 122,404 → 122,404 → 24,254 | 60,981,005 |

| Key measurement SQL | SF1 rows / distinct / mean fanout | SF10 rows / distinct / mean fanout |
|---|---|---|
| SELECT count(*),count(DISTINCT l_partkey) FROM lineitem | 6,001,215 / 200,000 / 30.00607 | 59,986,052 / 2,000,000 / 29.99303 |
| SELECT count(*),count(DISTINCT o_custkey) FROM orders WHERE o_orderdate>=DATE '1995-01-01' AND o_orderdate<=DATE '1996-12-31' | 457,263 / 97,493 / 4.69021 | 4,557,513 / 975,382 / 4.67254 |

**Hand-plan reasoning:** Choose Part → Lineitem → Orders → Customer → customer Nation → Region → Supplier → supplier Nation. The restricted Part stream (0.7255% / 0.6726%) is far smaller than the 20% regional customer stream. Retain its numeric selection, segment the approximately 30× Lineitem expansion, and perform dated-order and region checks before unrelated supplier lookups. Occurrence-qualified Nation@6 and Nation@8 distinguish the customer and supplier roles.

## h09

Aliases: `p` = `part`, `l` = `lineitem`, `x` = `partsupp`, `o` = `orders`, `s` = `supplier`, `n` = `nation`. All prefix counts apply every predicate whose aliases are bound.

| Predicate | SF1 selected / base | SF10 selected / base |
|---|---|---|
| p.p_name LIKE '%green%' | 10,664 / 200,000 (5.3320%) | 108,782 / 2,000,000 (5.4391%) |

| Candidate order | SF1 prefixes | SF1 sum | SF10 prefixes | SF10 sum |
|---|---|---|---|---|
| p l x o s n | 10,664 → 319,404 → 319,404 → 319,404 → 319,404 → 319,404 | 1,607,684 | 108,782 → 3,261,613 → 3,261,613 → 3,261,613 → 3,261,613 → 3,261,613 | 16,416,847 |
| p x l o s n | 10,664 → 42,656 → 319,404 → 319,404 → 319,404 → 319,404 | 1,330,936 | 108,782 → 435,128 → 3,261,613 → 3,261,613 → 3,261,613 → 3,261,613 | 13,590,362 |
| l p x o s n | 6,001,215 → 319,404 → 319,404 → 319,404 → 319,404 → 319,404 | 7,598,235 | 59,986,052 → 3,261,613 → 3,261,613 → 3,261,613 → 3,261,613 → 3,261,613 | 76,294,117 |
| x p l o s n | 800,000 → 42,656 → 319,404 → 319,404 → 319,404 → 319,404 | 2,120,272 | 8,000,000 → 435,128 → 3,261,613 → 3,261,613 → 3,261,613 → 3,261,613 | 21,481,580 |
| s n x p l o | 10,000 → 10,000 → 800,000 → 42,656 → 319,404 → 319,404 | 1,501,464 | 100,000 → 100,000 → 8,000,000 → 435,128 → 3,261,613 → 3,261,613 | 15,158,354 |
| o l p x s n | 1,500,000 → 6,001,215 → 319,404 → 319,404 → 319,404 → 319,404 | 8,778,831 | 15,000,000 → 59,986,052 → 3,261,613 → 3,261,613 → 3,261,613 → 3,261,613 | 88,032,504 |
| p x s n l o | 10,664 → 42,656 → 42,656 → 42,656 → 319,404 → 319,404 | 777,440 | 108,782 → 435,128 → 435,128 → 435,128 → 3,261,613 → 3,261,613 | 7,937,392 |
| p x s l o n | 10,664 → 42,656 → 42,656 → 319,404 → 319,404 → 319,404 | 1,054,188 | 108,782 → 435,128 → 435,128 → 3,261,613 → 3,261,613 → 3,261,613 | 10,763,877 |

| Key measurement SQL | SF1 rows / distinct / mean fanout | SF10 rows / distinct / mean fanout |
|---|---|---|
| SELECT count(*),count(DISTINCT ps_partkey) FROM partsupp | 800,000 / 200,000 / 4.00000 | 8,000,000 / 2,000,000 / 4.00000 |
| SELECT count(*),count(DISTINCT l_partkey) FROM lineitem | 6,001,215 / 200,000 / 30.00607 | 59,986,052 / 2,000,000 / 29.99303 |
| SELECT count(*),count(DISTINCT (l_partkey,l_suppkey)) FROM lineitem | 6,001,215 / 799,541 / 7.50583 | 59,986,052 / 7,995,955 / 7.50205 |
| SELECT count(*),count(DISTINCT p_partkey) FROM part WHERE p_name LIKE '%green%' | 10,664 / 10,664 / 1.00000 | 108,782 / 108,782 / 1.00000 |

**Accepted-plan reasoning:** PartSupp-first is the measured minimum-prefix candidate, but its full composite Lineitem index alone took 79.138 ms at SF10 in the initial exact run, exceeding both the historical 66.535 ms and fresh 46.240 ms totals of the single-key baseline. The initial retained Supplier publication also lacked a parallel partition; its inflated kernel time is not attributed solely to join order. A forward Lineitem filter by the opaque green-Part support was rejected with `T-Gpu: host publication r1 needs recorded device-representable code columns`. Keep the accepted Part → Lineitem → PartSupp → Orders → Supplier → Nation route, retain the numeric green Part leaf, and use checked full-key membership for cycle-closing providers. The report does not claim that fewer prefix positions alone imply lower wall time.

## h12

Filter lineitem completely, then probe unique orders key for priority, then two shipmode grouped indicator sums. Never filter out low-priority orders: priority is a classification, not an acceptance guard. For staged guard compaction, date_order -> receiptdate_window -> shipmode minimizes measured intermediate positions; fuse all masks when feasible.

| Predicate / cumulative guard | SF1 rows (% of base) | SF10 rows (% of base) |
|---|---|---|
| constants: l_shipmode = 'MAIL' | 857,401 (14.2871%) | 8,569,053 (14.2851%) |
| constants: l_shipmode = 'SHIP' | 858,036 (14.2977%) | 8,571,402 (14.2890%) |
| constants: l_receiptdate >= DATE '1994-01-01' | 4,374,812 (72.8988%) | 43,666,415 (72.7943%) |
| constants: l_receiptdate < DATE '1995-01-01' | 2,536,247 (42.2622%) | 25,418,064 (42.3733%) |
| atomic_guards: l_shipdate < l_commitdate | 2,926,558 (48.7661%) | 29,246,972 (48.7563%) |
| atomic_guards: l_commitdate < l_receiptdate | 3,793,296 (63.2088%) | 37,929,348 (63.2303%) |
| guards: l_shipmode IN ('MAIL','SHIP') | 1,715,437 (28.5848%) | 17,140,455 (28.5741%) |
| guards: l_shipdate < l_commitdate AND l_commitdate < l_receiptdate | 718,639 (11.9749%) | 7,190,268 (11.9866%) |
| guards: l_receiptdate >= DATE '1994-01-01' AND l_receiptdate < DATE '1995-01-01' | 909,844 (15.1610%) | 9,098,427 (15.1676%) |
| cumulative: (l_shipmode IN ('MAIL','SHIP')) | 1,715,437 (28.5848%) | 17,140,455 (28.5741%) |
| cumulative: (l_shipmode IN ('MAIL','SHIP')) AND (l_shipdate < l_commitdate AND l_commitdate < l_receiptdate) | 205,778 (3.4289%) | 2,054,567 (3.4251%) |
| cumulative: (l_shipmode IN ('MAIL','SHIP')) AND (l_shipdate < l_commitdate AND l_commitdate < l_receiptdate) AND (l_receiptdate >= DATE '1994-01-01' AND l_receiptdate < DATE '1995-01-01') | 30,988 (0.5164%) | 310,803 (0.5181%) |

| Candidate order | SF1 prefixes | SF1 sum | SF10 prefixes | SF10 sum |
|---|---|---|---|---|
| lineitem → orders | [30988, 30988] | 61976 | [310803, 310803] | 621606 |
| orders → lineitem | [1500000, 30988] | 1530988 | [15000000, 310803] | 15310803 |

| Scale | Scanned Lineitem rows | Aggregate inputs | Output groups |
|---|---|---|---|
| 1 | 6001215 | 30988 | 2 |
| 10 | 59986052 | 310803 | 2 |

**Segmentation:** No fanout boundary on filtered lineitem -> orders: unique-key lookup emits exactly one match for each surviving lineitem.

| Filtered key cardinality | SF1 | SF10 |
|---|---|---|
| rows | 30988 | 310803 |
| distinct_order_keys | 29099 | 291909 |
| distinct_line_numbers | 7 | 7 |

**Accepted boundary:** the Orders lookup is unique and the priority-expression binding remains on host. Forcing its numeric bind producer onto device is rejected by BE-Partial. Preserve the baseline checked publication rather than silently changing exact aggregation.

| Priority classification (not a filter) | SF1 Orders / fraction | SF10 Orders / fraction |
|---|---|---|
| urgent | 300,343 / 20.0229% | 3,003,093 / 20.0206% |
| high | 300,091 / 20.0061% | 3,000,061 / 20.0004% |
| high_priority | 600,434 / 40.0289% | 6,003,154 / 40.0210% |
| low_priority | 899,566 / 59.9711% | 8,996,846 / 59.9790% |

## h16

Apply all Part guards before expanding to four Partsupp rows; once suppkey is bound, probe the tiny BadSupplier key set before emission. Partsupp-first permits unique Part lookup but loses on root size. BadSupplier-first is not a legal positive root: anti suppliers remove, not produce, output.

| Local predicate | SF1 selected / base | SF10 selected / base |
|---|---|---|
| Part brand_rows | 192,022 / 200,000 (96.0110%) | 1,919,874 / 2,000,000 (95.9937%) |
| Part type_rows | 193,290 / 200,000 (96.6450%) | 1,933,121 / 2,000,000 (96.6561%) |
| Part size_rows | 31,826 / 200,000 (15.9130%) | 319,686 / 2,000,000 (15.9843%) |
| Part brand_type_rows | 185,566 / 200,000 (92.7830%) | 1,855,696 / 2,000,000 (92.7848%) |
| Part cumulative_rows | 29,581 / 200,000 (14.7905%) | 296,824 / 2,000,000 (14.8412%) |
| Part size_49_rows | 3,945 / 200,000 (1.9725%) | 39,958 / 2,000,000 (1.9979%) |
| Part size_14_rows | 4,153 / 200,000 (2.0765%) | 39,913 / 2,000,000 (1.9956%) |
| Part size_23_rows | 3,932 / 200,000 (1.9660%) | 40,177 / 2,000,000 (2.0088%) |
| Part size_45_rows | 3,893 / 200,000 (1.9465%) | 39,601 / 2,000,000 (1.9800%) |
| Part size_19_rows | 3,851 / 200,000 (1.9255%) | 39,837 / 2,000,000 (1.9918%) |
| Part size_3_rows | 4,123 / 200,000 (2.0615%) | 40,028 / 2,000,000 (2.0014%) |
| Part size_36_rows | 3,895 / 200,000 (1.9475%) | 40,242 / 2,000,000 (2.0121%) |
| Part size_9_rows | 4,034 / 200,000 (2.0170%) | 39,930 / 2,000,000 (1.9965%) |
| Supplier selected_rows | 4 / 10,000 (0.0400%) | 56 / 100,000 (0.0560%) |
| Supplier_constant_components contains_customer_rows | 9 / 10,000 (0.0900%) | 107 / 100,000 (0.1070%) |
| Supplier_constant_components contains_complaints_rows | 4 / 10,000 (0.0400%) | 56 / 100,000 (0.0560%) |
| Supplier_constant_components contains_both_rows | 4 / 10,000 (0.0400%) | 56 / 100,000 (0.0560%) |
| Supplier_constant_components ordered_cumulative_rows | 4 / 10,000 (0.0400%) | 56 / 100,000 (0.0560%) |

| Candidate order | SF1 prefixes | SF1 sum | SF10 prefixes | SF10 sum |
|---|---|---|---|---|
| Part → Partsupp → ANTI BadSupplier | [29581, 118274] | 147855 | [296824, 1186602] | 1483426 |
| Partsupp → Part → ANTI BadSupplier | [800000, 118274] | 918274 | [8000000, 1186602] | 9186602 |
| Partsupp with ANTI BadSupplier → Part | [799680, 118274] | 917954 | [7995520, 1186602] | 9182122 |

| Correlation/output metric | SF1 | SF10 |
|---|---|---|
| positive_root_rows | 29581 | 296824 |
| anti_build_rows | 4 | 56 |
| anti_build_distinct_correlation_keys | 4 | 56 |
| expanded_positions_before_anti | 118324 | 1187296 |
| output_rows | 118274 | 1186602 |
| output_distinct_pairs | 118274 | 1186602 |
| pre_anti_distinct_correlation_keys | 10000 | 100000 |
| post_anti_distinct_correlation_keys | 9996 | 99944 |
| rejected_distinct_correlation_keys | 4 | 56 |
| anti_rejected_positions | 50 | 694 |
| anti_selectivity_local | 118,274 / 118,324 (99.9577%) | 1,186,602 / 1,187,296 (99.9415%) |
| anti_selectivity_all_partsupp | 799,680 / 800,000 (99.9600%) | 7,995,520 / 8,000,000 (99.9440%) |
| cumulative_output_selectivity_vs_partsupp | 118,274 / 800,000 (14.7842%) | 1,186,602 / 8,000,000 (14.8325%) |
| brand_then_type_conditional | 185,566 / 192,022 (96.6379%) | 1,855,696 / 1,919,874 (96.6572%) |
| then_size_conditional | 29,581 / 185,566 (15.9410%) | 296,824 / 1,855,696 (15.9953%) |

**Segmentation:** "Part->Partsupp fanout is exactly 4 (including maximum), so strict >4 creates no boundary. The reverse Supplier->Partsupp fanout is 80 but Supplier is an anti domain, not a chosen positive expansion. Anti-probe is unique-key and nonexpanding."

| Join-key/fanout measurement | SF1 | SF10 |
|---|---|---|
| fanout_part_partsupp: parent_rows | 200000 | 2000000 |
| fanout_part_partsupp: matched_parent_keys | 200000 | 2000000 |
| fanout_part_partsupp: child_positions | 800000 | 8000000 |
| fanout_part_partsupp: min_fanout | 4 | 4 |
| fanout_part_partsupp: max_fanout | 4 | 4 |
| fanout_part_partsupp: mean_fanout_all_parents | 4.0 | 4.0 |
| fanout_part_partsupp: mean_fanout_matched_parents | 4.0 | 4.0 |
| fanout_part_partsupp: parent_keys_fanout_gt4 | 0 | 0 |
| fanout_part_partsupp: positions_in_gt4_buckets | None | None |
| fanout_selected_part_partsupp: parent_rows | 29581 | 296824 |
| fanout_selected_part_partsupp: matched_parent_keys | 29581 | 296824 |
| fanout_selected_part_partsupp: child_positions | 118324 | 1187296 |
| fanout_selected_part_partsupp: min_fanout | 4 | 4 |
| fanout_selected_part_partsupp: max_fanout | 4 | 4 |
| fanout_selected_part_partsupp: mean_fanout_all_parents | 4.0 | 4.0 |
| fanout_selected_part_partsupp: mean_fanout_matched_parents | 4.0 | 4.0 |
| fanout_selected_part_partsupp: parent_keys_fanout_gt4 | 0 | 0 |
| fanout_selected_part_partsupp: positions_in_gt4_buckets | None | None |
| fanout_selected_part_good_partsupp: parent_rows | 29581 | 296824 |
| fanout_selected_part_good_partsupp: matched_parent_keys | 29581 | 296824 |
| fanout_selected_part_good_partsupp: child_positions | 118274 | 1186602 |
| fanout_selected_part_good_partsupp: min_fanout | 3 | 2 |
| fanout_selected_part_good_partsupp: max_fanout | 4 | 4 |
| fanout_selected_part_good_partsupp: mean_fanout_all_parents | 3.998309725837531 | 3.9976619141309326 |
| fanout_selected_part_good_partsupp: mean_fanout_matched_parents | 3.998309725837531 | 3.9976619141309326 |
| fanout_selected_part_good_partsupp: parent_keys_fanout_gt4 | 0 | 0 |
| fanout_selected_part_good_partsupp: positions_in_gt4_buckets | None | None |
| fanout_supplier_partsupp: parent_rows | 10000 | 100000 |
| fanout_supplier_partsupp: matched_parent_keys | 10000 | 100000 |
| fanout_supplier_partsupp: child_positions | 800000 | 8000000 |
| fanout_supplier_partsupp: min_fanout | 80 | 80 |
| fanout_supplier_partsupp: max_fanout | 80 | 80 |
| fanout_supplier_partsupp: mean_fanout_all_parents | 80.0 | 80.0 |
| fanout_supplier_partsupp: mean_fanout_matched_parents | 80.0 | 80.0 |
| fanout_supplier_partsupp: parent_keys_fanout_gt4 | 10000 | 100000 |
| fanout_supplier_partsupp: positions_in_gt4_buckets | 800000 | 8000000 |
| fanout_bad_supplier_partsupp: parent_rows | 4 | 56 |
| fanout_bad_supplier_partsupp: matched_parent_keys | 4 | 56 |
| fanout_bad_supplier_partsupp: child_positions | 320 | 4480 |
| fanout_bad_supplier_partsupp: min_fanout | 80 | 80 |
| fanout_bad_supplier_partsupp: max_fanout | 80 | 80 |
| fanout_bad_supplier_partsupp: mean_fanout_all_parents | 80.0 | 80.0 |
| fanout_bad_supplier_partsupp: mean_fanout_matched_parents | 80.0 | 80.0 |
| fanout_bad_supplier_partsupp: parent_keys_fanout_gt4 | 4 | 56 |
| fanout_bad_supplier_partsupp: positions_in_gt4_buckets | 320 | 4480 |

## h22

Filter Customer by the seven phone prefixes, then anti-probe Orders indexed/deduplicated by o_custkey. Orders has no valid positive-root role and o_orderkey is existentially unbound. No scalar balance-average predicate should be introduced.

| Local predicate | SF1 selected / base | SF10 selected / base |
|---|---|---|
| Customer selected_rows | 42,015 / 150,000 (28.0100%) | 419,974 / 1,500,000 (27.9983%) |
| Customer code_13_rows | 6,020 / 150,000 (4.0133%) | 59,849 / 1,500,000 (3.9899%) |
| Customer code_31_rows | 6,008 / 150,000 (4.0053%) | 59,997 / 1,500,000 (3.9998%) |
| Customer code_23_rows | 6,033 / 150,000 (4.0220%) | 59,909 / 1,500,000 (3.9939%) |
| Customer code_29_rows | 6,100 / 150,000 (4.0667%) | 60,048 / 1,500,000 (4.0032%) |
| Customer code_30_rows | 5,904 / 150,000 (3.9360%) | 59,803 / 1,500,000 (3.9869%) |
| Customer code_18_rows | 6,042 / 150,000 (4.0280%) | 60,215 / 1,500,000 (4.0143%) |
| Customer code_17_rows | 5,908 / 150,000 (3.9387%) | 60,153 / 1,500,000 (4.0102%) |

| Candidate order | SF1 prefixes | SF1 sum | SF10 prefixes | SF10 sum |
|---|---|---|---|---|
| Customer → ANTI Orders | [42015] | 42015 | [419974] | 419974 |

| Correlation/output metric | SF1 | SF10 |
|---|---|---|
| positive_root_rows | 42015 | 419974 |
| anti_build_rows | 1500000 | 15000000 |
| anti_build_distinct_correlation_keys | 99996 | 999982 |
| anti_rejected_rows | 27929 | 279485 |
| output_rows | 14086 | 140489 |
| anti_selectivity_local | 14,086 / 42,015 (33.5261%) | 140,489 / 419,974 (33.4518%) |
| anti_selectivity_all_customers | 50,004 / 150,000 (33.3360%) | 500,018 / 1,500,000 (33.3345%) |
| cumulative_output_selectivity | 14,086 / 150,000 (9.3907%) | 140,489 / 1,500,000 (9.3659%) |
| pre_anti_distinct_correlation_keys | 42015 | 419974 |
| rejected_distinct_correlation_keys | 27929 | 279485 |
| output_distinct_correlation_keys | 14086 | 140489 |

**Segmentation:** "No expanding positive edge. Although Customer->Orders has mean about 10 over all selected customers (15 over matched customers) and maximum 41/46, this is anti membership, not enumeration. Do not create a fanout>4 expansion segment for the anti block; if an implementation currently expands Orders, replace that with a key existence check."

| Join-key/fanout measurement | SF1 | SF10 |
|---|---|---|
| fanout_customer_orders: parent_rows | 150000 | 1500000 |
| fanout_customer_orders: matched_parent_keys | 99996 | 999982 |
| fanout_customer_orders: child_positions | 1500000 | 15000000 |
| fanout_customer_orders: min_fanout | 0 | 0 |
| fanout_customer_orders: max_fanout | 41 | 46 |
| fanout_customer_orders: mean_fanout_all_parents | 10.0 | 10.0 |
| fanout_customer_orders: mean_fanout_matched_parents | 15.00060002400096 | 15.000270004860088 |
| fanout_customer_orders: parent_keys_fanout_gt4 | 98529 | 985593 |
| fanout_customer_orders: positions_in_gt4_buckets | 1494803 | 14949008 |
| fanout_selected_customer_orders: parent_rows | 42015 | 419974 |
| fanout_selected_customer_orders: matched_parent_keys | 27929 | 279485 |
| fanout_selected_customer_orders: child_positions | 420579 | 4192748 |
| fanout_selected_customer_orders: min_fanout | 0 | 0 |
| fanout_selected_customer_orders: max_fanout | 41 | 46 |
| fanout_selected_customer_orders: mean_fanout_all_parents | 10.01021063905748 | 9.983351350321687 |
| fanout_selected_customer_orders: mean_fanout_matched_parents | 15.058863546850944 | 15.001692398518705 |
| fanout_selected_customer_orders: parent_keys_fanout_gt4 | 27552 | 275520 |
| fanout_selected_customer_orders: positions_in_gt4_buckets | 419252 | 4178711 |

## h21_positive

Aliases: `n` = `nation`, `s` = `supplier`, `a` = `lineitem`, `o` = `orders`, `b` = `lineitem`. All prefix counts apply every predicate whose aliases are bound.

| Predicate | SF1 selected / base | SF10 selected / base |
|---|---|---|
| n.n_name='SAUDI ARABIA' | 1 / 25 (4.0000%) | 1 / 25 (4.0000%) |
| a.l_receiptdate>a.l_commitdate | 3,793,296 / 6,001,215 (63.2088%) | 37,929,348 / 59,986,052 (63.2303%) |
| o.o_orderstatus='F' | 729,413 / 1,500,000 (48.6275%) | 7,309,184 / 15,000,000 (48.7279%) |

| Candidate order | SF1 prefixes | SF1 sum | SF10 prefixes | SF10 sum |
|---|---|---|---|---|
| n s a o b | 1 → 411 → 156,739 → 75,871 → 302,383 | 535,405 | 1 → 4,010 → 1,522,366 → 734,523 → 2,930,383 | 5,191,283 |
| s n a o b | 10,000 → 411 → 156,739 → 75,871 → 302,383 | 545,404 | 100,000 → 4,010 → 1,522,366 → 734,523 → 2,930,383 | 5,291,282 |
| a s n o b | 3,793,296 → 3,793,296 → 156,739 → 75,871 → 302,383 | 8,121,585 | 37,929,348 → 37,929,348 → 1,522,366 → 734,523 → 2,930,383 | 81,045,968 |
| o a s n b | 729,413 → 1,828,911 → 1,828,911 → 75,871 → 302,383 | 4,765,489 | 7,309,184 → 18,321,400 → 18,321,400 → 734,523 → 2,930,383 | 47,616,890 |
| b o a s n | 6,001,215 → 2,901,744 → 7,301,853 → 7,301,853 → 302,383 | 23,809,048 | 59,986,052 → 29,058,120 → 73,104,966 → 73,104,966 → 2,930,383 | 238,184,487 |
| n s b a o | 1 → 411 → 2,466,499,365 → 626,799 → 302,383 | 2,467,428,959 | 1 → 4,010 → 240,544,068,520 → 6,085,661 → 2,930,383 | 240,553,088,575 |

| Key measurement SQL | SF1 rows / distinct / mean fanout | SF10 rows / distinct / mean fanout |
|---|---|---|
| SELECT count(*),count(DISTINCT l_suppkey) FROM lineitem WHERE l_receiptdate>l_commitdate | 3,793,296 / 10,000 / 379.32960 | 37,929,348 / 100,000 / 379.29348 |
| SELECT count(*),count(DISTINCT l_orderkey) FROM lineitem | 6,001,215 / 1,500,000 / 4.00081 | 59,986,052 / 15,000,000 / 3.99907 |
| SELECT count(*),count(DISTINCT s_nationkey) FROM supplier | 10,000 / 25 / 400.00000 | 100,000 / 25 / 4000.00000 |

**Hand-plan reasoning:** Choose Nation → Supplier → late Lineitem@1 → finalized Orders → other Lineitem@0. This needs occurrence-qualified advice: the two Lineitem roles are not interchangeable under the late predicate. Retain the Saudi supplier prefix; segment high-fanout Supplier and late-Lineitem expansions and the approximately-four final expansion, subject to the exact resource diagnostics. Orders is a fully bound membership lookup, so finalized-order rejection happens before expanding other suppliers. The inequality remains an exact predicate, never a guessed FK.

| Other-supplier guard after Saudi + late + finalized prefix | SF1 | SF10 |
|---|---|---|
| before inequality | 378274 | 3664942 |
| after inequality | 302383 | 2930383 |
| selectivity | 0.7993755848934899 | 0.7995714529725163 |

**Provider choice:** use the measured selective support to materialize a forward filtered provider before indexing; see the provider table. No measured predicate is deleted merely because it happens to pass every exported row.

**Selected publication boundary:** retain the late-Lineitem joined prefix globally before finalized-order / other-supplier work. That gives 156,739 / 1,522,366 independent roots instead of 411 / 4,010 supplier roots. The exact measured change was 21.277 → 4.312 ms at SF1 and 61.189 → 40.118 ms at SF10. This is a scheduling improvement with unchanged join-prefix cardinalities, not a new predicate.

## d19

US nation→customer→date-qualified orders→lineitem→part/supplier lookups. The two terminal dimension lookups tie and cannot precede lineitem because their keys are not bound. Preserve DISTINCT output: SF1 has one duplicate witness.

Aliases: `n`=nation, `c`=customer, `o`=orders, `l`=lineitem, `p`=part, `s`=supplier.

| Predicate | SF1 independent / base | SF10 independent / base | SF1 cumulative | SF10 cumulative |
|---|---|---|---|---|
| n: n.n_name='UNITED STATES' | 1 / 25 | 1 / 25 | 1 | 1 |
| o: o.o_orderdate >= DATE '1995-01-01' | 818,669 / 1,500,000 | 8,166,238 / 15,000,000 | 818,669 | 8,166,238 |
| o: o.o_orderdate < DATE '1995-04-01' | 737,837 / 1,500,000 | 7,395,374 / 15,000,000 | 56,506 | 561,612 |

| Candidate order | SF1 prefixes | SF1 sum | SF10 prefixes | SF10 sum |
|---|---|---|---|---|
| n → c → o → l → p → s | 1 → 5,983 → 2,231 → 8,963 → 8,963 → 8,963 | 35,104 | 1 → 60,006 → 22,579 → 90,015 → 90,015 → 90,015 | 352,631 |
| o → c → n → l → p → s | 56,506 → 56,506 → 2,231 → 8,963 → 8,963 → 8,963 | 142,132 | 561,612 → 561,612 → 22,579 → 90,015 → 90,015 → 90,015 | 1,415,848 |
| l → o → c → n → p → s | 6,001,215 → 226,122 → 226,122 → 8,963 → 8,963 → 8,963 | 6,480,348 | 59,986,052 → 2,246,588 → 2,246,588 → 90,015 → 90,015 → 90,015 | 64,749,273 |

| Selected distinct key | SF1 | SF10 |
|---|---|---|
| n.n_nationkey | 1 | 1 |
| o.o_orderkey | 56506 | 561612 |
| o.o_custkey | 42141 | 420744 |

| Chosen keyed step | SF1 post-filter mean / max | SF10 post-filter mean / max |
|---|---|---|
| n → c | 5983.00000 / 5983 | 60006.00000 / 60006 |
| n → c → o | 0.37289 / 5 | 0.37628 / 6 |
| n → c → o → l | 4.01748 / 7 | 3.98667 / 7 |
| n → c → o → l → p | 1.00000 / 1 | 1.00000 / 1 |
| n → c → o → l → p → s | 1.00000 / 1 | 1.00000 / 1 |

Final DISTINCT output rows: **8,962 / 90,015**.

**Authored schedule:** retain the selected customer prefix after the one-row US nation. Its 0.37× quarter-order lookup precedes the roughly four-line expansion; bound Part/Supplier are membership checks, not new expansion streams.

**Provider experiment (not selected):** selective forward materialization was exact, but its extra scans/publications cost more than the selected accepted schedule. Its measurements are retained in `tpch_hand.retuned.json`; no predicate was deleted.

**Selected runtime fallback:** retain the initial unmasked hand traversal (19.324 ms SF10), rather than the exact forward-provider variant (20.698 ms).

## d27

US nation→supplier→lineitem→date-qualified order lookup→Brand#12 part lookup. Order before part minimizes the next prefix at both scales (8969 versus 9332 at SF1; 90158 versus 96857 at SF10). Supplier-to-lineitem fanout is about 600, so segment that expansion.

Aliases: `n`=nation, `s`=supplier, `l`=lineitem, `o`=orders, `p`=part.

| Predicate | SF1 independent / base | SF10 independent / base | SF1 cumulative | SF10 cumulative |
|---|---|---|---|---|
| n: n.n_name='UNITED STATES' | 1 / 25 | 1 / 25 | 1 | 1 |
| o: o.o_orderdate >= DATE '1995-01-01' | 818,669 / 1,500,000 | 8,166,238 / 15,000,000 | 818,669 | 8,166,238 |
| o: o.o_orderdate < DATE '1995-04-01' | 737,837 / 1,500,000 | 7,395,374 / 15,000,000 | 56,506 | 561,612 |
| p: p.p_brand='Brand#12' | 8,167 / 200,000 | 79,980 / 2,000,000 | 8,167 | 79,980 |

| Candidate order | SF1 prefixes | SF1 sum | SF10 prefixes | SF10 sum |
|---|---|---|---|---|
| n → s → l → o → p | 1 → 393 → 236,346 → 8,969 → 357 | 246,066 | 1 → 4,004 → 2,402,516 → 90,158 → 3,575 | 2,500,254 |
| n → s → l → p → o | 1 → 393 → 236,346 → 9,332 → 357 | 246,429 | 1 → 4,004 → 2,402,516 → 96,857 → 3,575 | 2,506,953 |
| o → l → p → s → n | 56,506 → 226,122 → 9,225 → 9,225 → 357 | 301,435 | 561,612 → 2,246,588 → 90,279 → 90,279 → 3,575 | 2,992,333 |
| p → l → o → s → n | 8,167 → 245,425 → 9,225 → 9,225 → 357 | 272,399 | 79,980 → 2,396,463 → 90,279 → 90,279 → 3,575 | 2,660,576 |
| l → o → p → s → n | 6,001,215 → 226,122 → 9,225 → 9,225 → 357 | 6,246,144 | 59,986,052 → 2,246,588 → 90,279 → 90,279 → 3,575 | 62,416,773 |

| Selected distinct key | SF1 | SF10 |
|---|---|---|
| n.n_nationkey | 1 | 1 |
| o.o_orderkey | 56506 | 561612 |
| o.o_custkey | 42141 | 420744 |
| p.p_partkey | 8167 | 79980 |

| Chosen keyed step | SF1 post-filter mean / max | SF10 post-filter mean / max |
|---|---|---|
| n → s | 393.00000 / 393 | 4004.00000 / 4004 |
| n → s → l | 601.38931 / 680 | 600.02897 / 679 |
| n → s → l → o | 0.03795 / 1 | 0.03753 / 1 |
| n → s → l → o → p | 0.03980 / 1 | 0.03965 / 1 |

Final DISTINCT output rows: **357 / 3,575**.

**Authored schedule:** retain the 393 / 4,004 US suppliers, segment their approximately 600× Lineitem expansion at 1,024 items 4, then reject by the narrower order-date window before brand membership. Constant-bearing providers that lack a membership certificate keep the accepted sorted provider.

**Provider choice:** use the measured selective support to materialize a forward filtered provider before indexing; see the provider table. No measured predicate is deleted merely because it happens to pass every exported row.

## d72_inner

Aliases: `l` = `lineitem`, `o` = `orders`, `x` = `partsupp`, `s` = `supplier`, `p` = `part`, `c` = `customer`, `n` = `nation`. All prefix counts apply every predicate whose aliases are bound.

| Predicate | SF1 selected / base | SF10 selected / base |
|---|---|---|
| l.l_shipdate>=DATE '1995-01-01' | 3,426,687 / 6,001,215 (57.0999%) | 34,181,736 / 59,986,052 (56.9828%) |
| l.l_shipdate<DATE '1995-04-01' | 2,799,781 / 6,001,215 (46.6536%) | 28,051,631 / 59,986,052 (46.7636%) |
| l.l_shipdate>=DATE '1995-01-01' AND l.l_shipdate<DATE '1995-04-01' | 225,253 / 6,001,215 (3.7535%) | 2,247,315 / 59,986,052 (3.7464%) |
| l.l_quantity>20.00 | 3,602,799 / 6,001,215 (60.0345%) | 35,995,193 / 59,986,052 (60.0059%) |
| l.l_shipdate>=DATE '1995-01-01' AND l.l_shipdate<DATE '1995-04-01' AND l.l_quantity>20.00 | 135,638 / 6,001,215 (2.2602%) | 1,348,779 / 59,986,052 (2.2485%) |
| x.ps_availqty>0 | 800,000 / 800,000 (100.0000%) | 8,000,000 / 8,000,000 (100.0000%) |

| Candidate order | SF1 prefixes | SF1 sum | SF10 prefixes | SF10 sum |
|---|---|---|---|---|
| l o c s x p n | 135,638 → 135,638 → 135,638 → 5,344 → 5,344 → 5,344 → 5,344 | 428,290 | 1,348,779 → 1,348,779 → 1,348,779 → 53,966 → 53,966 → 53,966 → 53,966 | 4,262,201 |
| l s o c x p n | 135,638 → 135,638 → 135,638 → 5,344 → 5,344 → 5,344 → 5,344 | 428,290 | 1,348,779 → 1,348,779 → 1,348,779 → 53,966 → 53,966 → 53,966 → 53,966 | 4,262,201 |
| l x s p o c n | 135,638 → 135,638 → 135,638 → 135,638 → 135,638 → 5,344 → 5,344 | 688,878 | 1,348,779 → 1,348,779 → 1,348,779 → 1,348,779 → 1,348,779 → 53,966 → 53,966 | 6,851,827 |
| o l c s x p n | 1,500,000 → 135,638 → 135,638 → 5,344 → 5,344 → 5,344 → 5,344 | 1,792,652 | 15,000,000 → 1,348,779 → 1,348,779 → 53,966 → 53,966 → 53,966 → 53,966 | 17,913,422 |
| x l o c s n p | 800,000 → 135,638 → 135,638 → 135,638 → 5,344 → 5,344 → 5,344 | 1,222,946 | 8,000,000 → 1,348,779 → 1,348,779 → 1,348,779 → 53,966 → 53,966 → 53,966 | 12,208,235 |
| s l o c x p n | 10,000 → 135,638 → 135,638 → 5,344 → 5,344 → 5,344 → 5,344 | 302,652 | 100,000 → 1,348,779 → 1,348,779 → 53,966 → 53,966 → 53,966 → 53,966 | 3,013,422 |
| c o l s x p n | 150,000 → 1,500,000 → 135,638 → 5,344 → 5,344 → 5,344 → 5,344 | 1,807,014 | 1,500,000 → 15,000,000 → 1,348,779 → 53,966 → 53,966 → 53,966 → 53,966 | 18,064,643 |
| n s l o c x p | 25 → 10,000 → 135,638 → 135,638 → 5,344 → 5,344 → 5,344 | 297,333 | 25 → 100,000 → 1,348,779 → 1,348,779 → 53,966 → 53,966 → 53,966 | 2,959,481 |
| n c o l s x p | 25 → 150,000 → 1,500,000 → 135,638 → 5,344 → 5,344 → 5,344 | 1,801,695 | 25 → 1,500,000 → 15,000,000 → 1,348,779 → 53,966 → 53,966 → 53,966 | 18,010,702 |
| p x s l o c n | 200,000 → 800,000 → 800,000 → 135,638 → 135,638 → 5,344 → 5,344 | 2,081,964 | 2,000,000 → 8,000,000 → 8,000,000 → 1,348,779 → 1,348,779 → 53,966 → 53,966 | 20,805,490 |

| Key measurement SQL | SF1 rows / distinct / mean fanout | SF10 rows / distinct / mean fanout |
|---|---|---|
| SELECT count(*),count(DISTINCT (l_partkey,l_suppkey)) FROM lineitem WHERE l_shipdate>=DATE '1995-01-01' AND l_shipdate<DATE '1995-04-01' AND l_quantity>20.00 | 135,638 / 124,846 / 1.08644 | 1,348,779 / 1,241,941 / 1.08603 |
| SELECT count(*),count(DISTINCT l_orderkey) FROM lineitem WHERE l_shipdate>=DATE '1995-01-01' AND l_shipdate<DATE '1995-04-01' AND l_quantity>20.00 | 135,638 / 75,908 / 1.78687 | 1,348,779 / 756,258 / 1.78349 |

**Hand-plan reasoning:** Choose Nation → Supplier → filtered Lineitem → Orders → Customer → PartSupp → Part. This small dimension root has a lower prefix sum than scanning all surviving Lineitem positions as the root. The quantity/window filter is evaluated forward once. Customer closes both customer key and nation, rejecting approximately 96%, before the unrestricted availability and Part checks. Segment Supplier and Lineitem fanouts; keep PartSupp/Part and cycle closure as membership where all keys are bound. The availability predicate keeps every measured Partsupp row but is retained semantically.

## d85

US customer-nation occurrence→customer→date-qualified orders→lineitem→composite (part,supplier) partsupp lookup→part→supplier→supplier-nation occurrence. Availability >0 selects every partsupp row at both scales and is not a useful driving predicate. Partsupp must bind both keys; a part-only or supplier-only join is not an equivalent lookup. Part and partsupp checks tie in measured cardinality; prefer guarded composite lookup first.

Aliases: `nc`=nation, `c`=customer, `o`=orders, `l`=lineitem, `ps`=partsupp, `p`=part, `s`=supplier, `ns`=nation.

| Predicate | SF1 independent / base | SF10 independent / base | SF1 cumulative | SF10 cumulative |
|---|---|---|---|---|
| nc: nc.n_name='UNITED STATES' | 1 / 25 | 1 / 25 | 1 | 1 |
| o: o.o_orderdate >= DATE '1995-01-01' | 818,669 / 1,500,000 | 8,166,238 / 15,000,000 | 818,669 | 8,166,238 |
| o: o.o_orderdate < DATE '1995-04-01' | 737,837 / 1,500,000 | 7,395,374 / 15,000,000 | 56,506 | 561,612 |
| ps: ps.ps_availqty > 0 | 800,000 / 800,000 | 8,000,000 / 8,000,000 | 800,000 | 8,000,000 |

| Candidate order | SF1 prefixes | SF1 sum | SF10 prefixes | SF10 sum |
|---|---|---|---|---|
| nc → c → o → l → ps → p → s → ns | 1 → 5,983 → 2,231 → 8,963 → 8,963 → 8,963 → 8,963 → 8,963 | 53,030 | 1 → 60,006 → 22,579 → 90,015 → 90,015 → 90,015 → 90,015 → 90,015 | 532,661 |
| o → c → nc → l → ps → p → s → ns | 56,506 → 56,506 → 2,231 → 8,963 → 8,963 → 8,963 → 8,963 → 8,963 | 160,058 | 561,612 → 561,612 → 22,579 → 90,015 → 90,015 → 90,015 → 90,015 → 90,015 | 1,595,878 |
| ps → l → o → c → nc → p → s → ns | 800,000 → 6,001,215 → 226,122 → 226,122 → 8,963 → 8,963 → 8,963 → 8,963 | 7,289,311 | 8,000,000 → 59,986,052 → 2,246,588 → 2,246,588 → 90,015 → 90,015 → 90,015 → 90,015 | 72,839,288 |
| l → o → c → nc → ps → p → s → ns | 6,001,215 → 226,122 → 226,122 → 8,963 → 8,963 → 8,963 → 8,963 → 8,963 | 6,498,274 | 59,986,052 → 2,246,588 → 2,246,588 → 90,015 → 90,015 → 90,015 → 90,015 → 90,015 | 64,929,303 |

| Selected distinct key | SF1 | SF10 |
|---|---|---|
| nc.n_nationkey | 1 | 1 |
| o.o_orderkey | 56506 | 561612 |
| o.o_custkey | 42141 | 420744 |
| ps.ps_partkey | 200000 | 2000000 |
| ps.ps_suppkey | 10000 | 100000 |
| ps.(ps_partkey,ps_suppkey) | 800000 | 8000000 |

| Chosen keyed step | SF1 post-filter mean / max | SF10 post-filter mean / max |
|---|---|---|
| nc → c | 5983.00000 / 5983 | 60006.00000 / 60006 |
| nc → c → o | 0.37289 / 5 | 0.37628 / 6 |
| nc → c → o → l | 4.01748 / 7 | 3.98667 / 7 |
| nc → c → o → l → ps | 1.00000 / 1 | 1.00000 / 1 |
| nc → c → o → l → ps → p | 1.00000 / 1 | 1.00000 / 1 |
| nc → c → o → l → ps → p → s | 1.00000 / 1 | 1.00000 / 1 |
| nc → c → o → l → ps → p → s → ns | 1.00000 / 1 | 1.00000 / 1 |

Final DISTINCT output rows: **8,962 / 90,015**.

**Authored schedule:** retain the US customer prefix and expand dated orders before lineitems; the 100%-passing availability predicate is preserved as an explicitly published numeric device filter, not deleted from semantics. Part and composite PartSupp are bound-key membership; Supplier supplies the output nation before its final nation-existence lookup. The equally sized Part/PartSupp prefix order is a cardinality tie.

**Provider experiment (not selected):** selective forward materialization was exact, but its extra scans/publications cost more than the selected accepted schedule. Its measurements are retained in `tpch_hand.retuned.json`; no predicate was deleted.

**Selected runtime fallback:** retain the initial `no_reduce` hand traversal (27.083 ms SF10). Default Masks reintroduced the unselective availability-side work and measured 75.338 ms; it is not selected. The fresh baseline was 26.781 ms, so this small remaining regression is disclosed rather than called a speedup.

## Compiler-rejected candidates

Every rejected attempt is retained, including both scales, in [`tpch_hand_rejections.json`](tpch_hand_rejections.json). The following groups identical diagnostics without truncating their text. No rejection is treated as an executed plan.

### initial hand schedule — h08 SF1, h08 SF10

```text
no numeric filtered leaf for Part; keep its predicate before joins in the source
```

### initial hand schedule — h09 SF1, h09 SF10

```text
hoist cannot move an opaque Rust dependency
```

### initial hand schedule — d72_inner SF1, d72_inner SF10

```text
T-Gpu: recorded range build selection must be a half-open interval on one source column
```

### device publication / opaque-map candidate — h01 SF1, h06 SF1, h01 SF10, h06 SF10

```text
P-Decompose candidate stage=0 tile=Some(256) items=Some(4) segmented=Some(false): BE-Partial: opaque Physical Rust must be evaluated at host before dispatch
P-Decompose candidate stage=0 tile=Some(256) items=Some(2) segmented=Some(false): BE-Partial: opaque Physical Rust must be evaluated at host before dispatch
P-Decompose candidate stage=0 tile=Some(256) items=Some(1) segmented=Some(false): BE-Partial: opaque Physical Rust must be evaluated at host before dispatch
P-Decompose candidate stage=0 tile=Some(128) items=Some(1) segmented=Some(false): BE-Partial: opaque Physical Rust must be evaluated at host before dispatch
P-Decompose candidate stage=0 tile=Some(64) items=Some(1) segmented=Some(false): BE-Partial: opaque Physical Rust must be evaluated at host before dispatch
P-Decompose candidate stage=0 tile=Some(32) items=Some(1) segmented=Some(false): BE-Partial: opaque Physical Rust must be evaluated at host before dispatch
P-Decompose candidate stage=0 tile=Some(16) items=Some(1) segmented=Some(false): BE-Partial: opaque Physical Rust must be evaluated at host before dispatch
P-Decompose candidate stage=0 tile=Some(8) items=Some(1) segmented=Some(false): BE-Partial: opaque Physical Rust must be evaluated at host before dispatch
P-Decompose candidate stage=0 tile=Some(4) items=Some(1) segmented=Some(false): BE-Partial: opaque Physical Rust must be evaluated at host before dispatch
P-Decompose candidate stage=0 tile=Some(2) items=Some(1) segmented=Some(false): BE-Partial: opaque Physical Rust must be evaluated at host before dispatch
P-Decompose candidate stage=0 tile=Some(1) items=Some(1) segmented=Some(false): BE-Partial: opaque Physical Rust must be evaluated at host before dispatch
P-Decompose candidate stage=1 tile=Some(256) items=Some(4) segmented=Some(false): BE-Partial: opaque Physical Rust must be evaluated at host before dispatch
P-Decompose candidate stage=1 tile=Some(256) items=Some(2) segmented=Some(false): BE-Partial: opaque Physical Rust must be evaluated at host before dispatch
P-Decompose candidate stage=1 tile=Some(256) items=Some(1) segmented=Some(false): BE-Partial: opaque Physical Rust must be evaluated at host before dispatch
P-Decompose candidate stage=1 tile=Some(128) items=Some(1) segmented=Some(false): BE-Partial: opaque Physical Rust must be evaluated at host before dispatch
P-Decompose candidate stage=1 tile=Some(64) items=Some(1) segmented=Some(false): BE-Partial: opaque Physical Rust must be evaluated at host before dispatch
P-Decompose candidate stage=1 tile=Some(32) items=Some(1) segmented=Some(false): BE-Partial: opaque Physical Rust must be evaluated at host before dispatch
P-Decompose candidate stage=1 tile=Some(16) items=Some(1) segmented=Some(false): BE-Partial: opaque Physical Rust must be evaluated at host before dispatch
P-Decompose candidate stage=1 tile=Some(8) items=Some(1) segmented=Some(false): BE-Partial: opaque Physical Rust must be evaluated at host before dispatch
P-Decompose candidate stage=1 tile=Some(4) items=Some(1) segmented=Some(false): BE-Partial: opaque Physical Rust must be evaluated at host before dispatch
P-Decompose candidate stage=1 tile=Some(2) items=Some(1) segmented=Some(false): BE-Partial: opaque Physical Rust must be evaluated at host before dispatch
P-Decompose candidate stage=1 tile=Some(1) items=Some(1) segmented=Some(false): BE-Partial: opaque Physical Rust must be evaluated at host before dispatch
P-Decompose candidate stage=2 tile=Some(256) items=Some(4) segmented=Some(false): BE-Partial: opaque Physical Rust must be evaluated at host before dispatch
P-Decompose candidate stage=2 tile=Some(256) items=Some(2) segmented=Some(false): BE-Partial: opaque Physical Rust must be evaluated at host before dispatch
P-Decompose candidate stage=2 tile=Some(256) items=Some(1) segmented=Some(false): BE-Partial: opaque Physical Rust must be evaluated at host before dispatch
P-Decompose candidate stage=2 tile=Some(128) items=Some(1) segmented=Some(false): BE-Partial: opaque Physical Rust must be evaluated at host before dispatch
P-Decompose candidate stage=2 tile=Some(64) items=Some(1) segmented=Some(false): BE-Partial: opaque Physical Rust must be evaluated at host before dispatch
P-Decompose candidate stage=2 tile=Some(32) items=Some(1) segmented=Some(false): BE-Partial: opaque Physical Rust must be evaluated at host before dispatch
P-Decompose candidate stage=2 tile=Some(16) items=Some(1) segmented=Some(false): BE-Partial: opaque Physical Rust must be evaluated at host before dispatch
P-Decompose candidate stage=2 tile=Some(8) items=Some(1) segmented=Some(false): BE-Partial: opaque Physical Rust must be evaluated at host before dispatch
P-Decompose candidate stage=2 tile=Some(4) items=Some(1) segmented=Some(false): BE-Partial: opaque Physical Rust must be evaluated at host before dispatch
P-Decompose candidate stage=2 tile=Some(2) items=Some(1) segmented=Some(false): BE-Partial: opaque Physical Rust must be evaluated at host before dispatch
P-Decompose candidate stage=2 tile=Some(1) items=Some(1) segmented=Some(false): BE-Partial: opaque Physical Rust must be evaluated at host before dispatch
```

### device publication / opaque-map candidate — h12 SF1, h12 SF10

```text
P-Decompose candidate stage=0 tile=Some(256) items=Some(4) segmented=Some(true): BE-Partial: opaque Physical Rust must be evaluated at host before dispatch
P-Decompose candidate stage=0 tile=Some(256) items=Some(4) segmented=Some(false): BE-Partial: opaque Physical Rust must be evaluated at host before dispatch
P-Decompose candidate stage=0 tile=Some(256) items=Some(2) segmented=Some(true): BE-Partial: opaque Physical Rust must be evaluated at host before dispatch
P-Decompose candidate stage=0 tile=Some(256) items=Some(2) segmented=Some(false): BE-Partial: opaque Physical Rust must be evaluated at host before dispatch
P-Decompose candidate stage=0 tile=Some(256) items=Some(1) segmented=Some(true): BE-Partial: opaque Physical Rust must be evaluated at host before dispatch
P-Decompose candidate stage=0 tile=Some(256) items=Some(1) segmented=Some(false): BE-Partial: opaque Physical Rust must be evaluated at host before dispatch
P-Decompose candidate stage=0 tile=Some(128) items=Some(1) segmented=Some(true): BE-Partial: opaque Physical Rust must be evaluated at host before dispatch
P-Decompose candidate stage=0 tile=Some(128) items=Some(1) segmented=Some(false): BE-Partial: opaque Physical Rust must be evaluated at host before dispatch
P-Decompose candidate stage=0 tile=Some(64) items=Some(1) segmented=Some(true): BE-Partial: opaque Physical Rust must be evaluated at host before dispatch
P-Decompose candidate stage=0 tile=Some(64) items=Some(1) segmented=Some(false): BE-Partial: opaque Physical Rust must be evaluated at host before dispatch
P-Decompose candidate stage=0 tile=Some(32) items=Some(1) segmented=Some(true): BE-Partial: opaque Physical Rust must be evaluated at host before dispatch
P-Decompose candidate stage=0 tile=Some(32) items=Some(1) segmented=Some(false): BE-Partial: opaque Physical Rust must be evaluated at host before dispatch
P-Decompose candidate stage=0 tile=Some(16) items=Some(1) segmented=Some(true): BE-Partial: opaque Physical Rust must be evaluated at host before dispatch
P-Decompose candidate stage=0 tile=Some(16) items=Some(1) segmented=Some(false): BE-Partial: opaque Physical Rust must be evaluated at host before dispatch
P-Decompose candidate stage=0 tile=Some(8) items=Some(1) segmented=Some(true): BE-Partial: opaque Physical Rust must be evaluated at host before dispatch
P-Decompose candidate stage=0 tile=Some(8) items=Some(1) segmented=Some(false): BE-Partial: opaque Physical Rust must be evaluated at host before dispatch
P-Decompose candidate stage=0 tile=Some(4) items=Some(1) segmented=Some(true): BE-Partial: opaque Physical Rust must be evaluated at host before dispatch
P-Decompose candidate stage=0 tile=Some(4) items=Some(1) segmented=Some(false): BE-Partial: opaque Physical Rust must be evaluated at host before dispatch
P-Decompose candidate stage=0 tile=Some(2) items=Some(1) segmented=Some(true): BE-Partial: opaque Physical Rust must be evaluated at host before dispatch
P-Decompose candidate stage=0 tile=Some(2) items=Some(1) segmented=Some(false): BE-Partial: opaque Physical Rust must be evaluated at host before dispatch
P-Decompose candidate stage=0 tile=Some(1) items=Some(1) segmented=Some(true): BE-Partial: opaque Physical Rust must be evaluated at host before dispatch
P-Decompose candidate stage=0 tile=Some(1) items=Some(1) segmented=Some(false): BE-Partial: opaque Physical Rust must be evaluated at host before dispatch
P-Decompose candidate stage=1 tile=Some(256) items=Some(4) segmented=Some(true): BE-Partial: opaque Physical Rust must be evaluated at host before dispatch
P-Decompose candidate stage=1 tile=Some(256) items=Some(4) segmented=Some(false): BE-Partial: opaque Physical Rust must be evaluated at host before dispatch
P-Decompose candidate stage=1 tile=Some(256) items=Some(2) segmented=Some(true): BE-Partial: opaque Physical Rust must be evaluated at host before dispatch
P-Decompose candidate stage=1 tile=Some(256) items=Some(2) segmented=Some(false): BE-Partial: opaque Physical Rust must be evaluated at host before dispatch
P-Decompose candidate stage=1 tile=Some(256) items=Some(1) segmented=Some(true): BE-Partial: opaque Physical Rust must be evaluated at host before dispatch
P-Decompose candidate stage=1 tile=Some(256) items=Some(1) segmented=Some(false): BE-Partial: opaque Physical Rust must be evaluated at host before dispatch
P-Decompose candidate stage=1 tile=Some(128) items=Some(1) segmented=Some(true): BE-Partial: opaque Physical Rust must be evaluated at host before dispatch
P-Decompose candidate stage=1 tile=Some(128) items=Some(1) segmented=Some(false): BE-Partial: opaque Physical Rust must be evaluated at host before dispatch
P-Decompose candidate stage=1 tile=Some(64) items=Some(1) segmented=Some(true): BE-Partial: opaque Physical Rust must be evaluated at host before dispatch
P-Decompose candidate stage=1 tile=Some(64) items=Some(1) segmented=Some(false): BE-Partial: opaque Physical Rust must be evaluated at host before dispatch
P-Decompose candidate stage=1 tile=Some(32) items=Some(1) segmented=Some(true): BE-Partial: opaque Physical Rust must be evaluated at host before dispatch
P-Decompose candidate stage=1 tile=Some(32) items=Some(1) segmented=Some(false): BE-Partial: opaque Physical Rust must be evaluated at host before dispatch
P-Decompose candidate stage=1 tile=Some(16) items=Some(1) segmented=Some(true): BE-Partial: opaque Physical Rust must be evaluated at host before dispatch
P-Decompose candidate stage=1 tile=Some(16) items=Some(1) segmented=Some(false): BE-Partial: opaque Physical Rust must be evaluated at host before dispatch
P-Decompose candidate stage=1 tile=Some(8) items=Some(1) segmented=Some(true): BE-Partial: opaque Physical Rust must be evaluated at host before dispatch
P-Decompose candidate stage=1 tile=Some(8) items=Some(1) segmented=Some(false): BE-Partial: opaque Physical Rust must be evaluated at host before dispatch
P-Decompose candidate stage=1 tile=Some(4) items=Some(1) segmented=Some(true): BE-Partial: opaque Physical Rust must be evaluated at host before dispatch
P-Decompose candidate stage=1 tile=Some(4) items=Some(1) segmented=Some(false): BE-Partial: opaque Physical Rust must be evaluated at host before dispatch
P-Decompose candidate stage=1 tile=Some(2) items=Some(1) segmented=Some(true): BE-Partial: opaque Physical Rust must be evaluated at host before dispatch
P-Decompose candidate stage=1 tile=Some(2) items=Some(1) segmented=Some(false): BE-Partial: opaque Physical Rust must be evaluated at host before dispatch
P-Decompose candidate stage=1 tile=Some(1) items=Some(1) segmented=Some(true): BE-Partial: opaque Physical Rust must be evaluated at host before dispatch
P-Decompose candidate stage=1 tile=Some(1) items=Some(1) segmented=Some(false): BE-Partial: opaque Physical Rust must be evaluated at host before dispatch
P-Decompose candidate stage=2 tile=Some(256) items=Some(4) segmented=Some(true): BE-Partial: opaque Physical Rust must be evaluated at host before dispatch
P-Decompose candidate stage=2 tile=Some(256) items=Some(4) segmented=Some(false): BE-Partial: opaque Physical Rust must be evaluated at host before dispatch
P-Decompose candidate stage=2 tile=Some(256) items=Some(2) segmented=Some(true): BE-Partial: opaque Physical Rust must be evaluated at host before dispatch
P-Decompose candidate stage=2 tile=Some(256) items=Some(2) segmented=Some(false): BE-Partial: opaque Physical Rust must be evaluated at host before dispatch
P-Decompose candidate stage=2 tile=Some(256) items=Some(1) segmented=Some(true): BE-Partial: opaque Physical Rust must be evaluated at host before dispatch
P-Decompose candidate stage=2 tile=Some(256) items=Some(1) segmented=Some(false): BE-Partial: opaque Physical Rust must be evaluated at host before dispatch
P-Decompose candidate stage=2 tile=Some(128) items=Some(1) segmented=Some(true): BE-Partial: opaque Physical Rust must be evaluated at host before dispatch
P-Decompose candidate stage=2 tile=Some(128) items=Some(1) segmented=Some(false): BE-Partial: opaque Physical Rust must be evaluated at host before dispatch
P-Decompose candidate stage=2 tile=Some(64) items=Some(1) segmented=Some(true): BE-Partial: opaque Physical Rust must be evaluated at host before dispatch
P-Decompose candidate stage=2 tile=Some(64) items=Some(1) segmented=Some(false): BE-Partial: opaque Physical Rust must be evaluated at host before dispatch
P-Decompose candidate stage=2 tile=Some(32) items=Some(1) segmented=Some(true): BE-Partial: opaque Physical Rust must be evaluated at host before dispatch
P-Decompose candidate stage=2 tile=Some(32) items=Some(1) segmented=Some(false): BE-Partial: opaque Physical Rust must be evaluated at host before dispatch
P-Decompose candidate stage=2 tile=Some(16) items=Some(1) segmented=Some(true): BE-Partial: opaque Physical Rust must be evaluated at host before dispatch
P-Decompose candidate stage=2 tile=Some(16) items=Some(1) segmented=Some(false): BE-Partial: opaque Physical Rust must be evaluated at host before dispatch
P-Decompose candidate stage=2 tile=Some(8) items=Some(1) segmented=Some(true): BE-Partial: opaque Physical Rust must be evaluated at host before dispatch
P-Decompose candidate stage=2 tile=Some(8) items=Some(1) segmented=Some(false): BE-Partial: opaque Physical Rust must be evaluated at host before dispatch
P-Decompose candidate stage=2 tile=Some(4) items=Some(1) segmented=Some(true): BE-Partial: opaque Physical Rust must be evaluated at host before dispatch
P-Decompose candidate stage=2 tile=Some(4) items=Some(1) segmented=Some(false): BE-Partial: opaque Physical Rust must be evaluated at host before dispatch
P-Decompose candidate stage=2 tile=Some(2) items=Some(1) segmented=Some(true): BE-Partial: opaque Physical Rust must be evaluated at host before dispatch
P-Decompose candidate stage=2 tile=Some(2) items=Some(1) segmented=Some(false): BE-Partial: opaque Physical Rust must be evaluated at host before dispatch
P-Decompose candidate stage=2 tile=Some(1) items=Some(1) segmented=Some(true): BE-Partial: opaque Physical Rust must be evaluated at host before dispatch
P-Decompose candidate stage=2 tile=Some(1) items=Some(1) segmented=Some(false): BE-Partial: opaque Physical Rust must be evaluated at host before dispatch
P-Decompose candidate stage=3 tile=Some(256) items=Some(4) segmented=Some(true): BE-Partial: opaque Physical Rust must be evaluated at host before dispatch
P-Decompose candidate stage=3 tile=Some(256) items=Some(4) segmented=Some(false): BE-Partial: opaque Physical Rust must be evaluated at host before dispatch
P-Decompose candidate stage=3 tile=Some(256) items=Some(2) segmented=Some(true): BE-Partial: opaque Physical Rust must be evaluated at host before dispatch
P-Decompose candidate stage=3 tile=Some(256) items=Some(2) segmented=Some(false): BE-Partial: opaque Physical Rust must be evaluated at host before dispatch
P-Decompose candidate stage=3 tile=Some(256) items=Some(1) segmented=Some(true): BE-Partial: opaque Physical Rust must be evaluated at host before dispatch
P-Decompose candidate stage=3 tile=Some(256) items=Some(1) segmented=Some(false): BE-Partial: opaque Physical Rust must be evaluated at host before dispatch
P-Decompose candidate stage=3 tile=Some(128) items=Some(1) segmented=Some(true): BE-Partial: opaque Physical Rust must be evaluated at host before dispatch
P-Decompose candidate stage=3 tile=Some(128) items=Some(1) segmented=Some(false): BE-Partial: opaque Physical Rust must be evaluated at host before dispatch
P-Decompose candidate stage=3 tile=Some(64) items=Some(1) segmented=Some(true): BE-Partial: opaque Physical Rust must be evaluated at host before dispatch
P-Decompose candidate stage=3 tile=Some(64) items=Some(1) segmented=Some(false): BE-Partial: opaque Physical Rust must be evaluated at host before dispatch
P-Decompose candidate stage=3 tile=Some(32) items=Some(1) segmented=Some(true): BE-Partial: opaque Physical Rust must be evaluated at host before dispatch
P-Decompose candidate stage=3 tile=Some(32) items=Some(1) segmented=Some(false): BE-Partial: opaque Physical Rust must be evaluated at host before dispatch
P-Decompose candidate stage=3 tile=Some(16) items=Some(1) segmented=Some(true): BE-Partial: opaque Physical Rust must be evaluated at host before dispatch
P-Decompose candidate stage=3 tile=Some(16) items=Some(1) segmented=Some(false): BE-Partial: opaque Physical Rust must be evaluated at host before dispatch
P-Decompose candidate stage=3 tile=Some(8) items=Some(1) segmented=Some(true): BE-Partial: opaque Physical Rust must be evaluated at host before dispatch
P-Decompose candidate stage=3 tile=Some(8) items=Some(1) segmented=Some(false): BE-Partial: opaque Physical Rust must be evaluated at host before dispatch
P-Decompose candidate stage=3 tile=Some(4) items=Some(1) segmented=Some(true): BE-Partial: opaque Physical Rust must be evaluated at host before dispatch
P-Decompose candidate stage=3 tile=Some(4) items=Some(1) segmented=Some(false): BE-Partial: opaque Physical Rust must be evaluated at host before dispatch
P-Decompose candidate stage=3 tile=Some(2) items=Some(1) segmented=Some(true): BE-Partial: opaque Physical Rust must be evaluated at host before dispatch
P-Decompose candidate stage=3 tile=Some(2) items=Some(1) segmented=Some(false): BE-Partial: opaque Physical Rust must be evaluated at host before dispatch
P-Decompose candidate stage=3 tile=Some(1) items=Some(1) segmented=Some(true): BE-Partial: opaque Physical Rust must be evaluated at host before dispatch
P-Decompose candidate stage=3 tile=Some(1) items=Some(1) segmented=Some(false): BE-Partial: opaque Physical Rust must be evaluated at host before dispatch
```

### device publication / opaque-map candidate — d72_inner SF1, d72_inner SF10

```text
BE-Scope: crossing into an inner level requires an ancestor store
```

### full-key membership and staged filters — h08 SF1, h08 SF10

```text
r10: filtered storage requires a rename, selection or union
```

### full-key membership and staged filters — h21_positive SF1, h21_positive SF10

```text
r6: filtered storage requires a rename, selection or union
```

### full-key membership and staged filters — d72_inner SF1, d72_inner SF10

```text
BE-Scope: crossing into an inner level requires an ancestor store
```

### H06 forward-filter publication before host arithmetic — h06 SF1, h06 SF10

```text
error[E0277]: a value of type `RangeProvider` cannot be built from an iterator over elements of type `(&i64, &i64, &i64, &i64, &i64, &i32)`
error[E0599]: no method named `iter` found for reference `&RangeProvider` in the current scope
```

### statistics-sized forward semijoin provider — h09 SF1, h09 SF10

```text
T-Gpu: host publication r1 needs recorded device-representable code columns
```

## Reproduction

```sh
cd experiments/gpu
export CUDA_HOME=/opt/nvidia/hpc_sdk/Linux_x86_64/25.5/cuda/12.9
export CARGO_BUILD_JOBS=1
export CQ_GPU_TPCH_AUTOMATIC=0
export CQ_GPU_TPCH_QUERIES=h01,h03,h04,h04_not_exists,h05,h06,h07_block,h08,h09,h12,h16,h22,h21_positive,d19,d27,d72_inner,d85
cargo oxide build --arch sm_89 -- --release -p cq-gpu-tpch
./target/release/cq-gpu-tpch compare/tpch_hand.reproduced.json \
  --mode advised --scale sf1 --scale sf10 \
  --baseline compare/tpch_hand_baseline.json
```

Only one GPU measurement process should run at a time. The default advice lives in `tpch/src/advice.rs`; the JSON override remains available for replay. Query sources and translator semantics are unchanged. The explicit-selector API was extended, with checked rejection tests, for `Relation@clause`, `block@clause`, and occurrence-qualified reduce/skip endpoints.

## Box 14b: statistics-driven staged plans

The following measurements use the same read-only DuckDB 1.5.6 databases and
`SET threads=4` method as above. Exact SQL and raw results are retained in
[`tpch_staged.statistics.json`](tpch_staged.statistics.json). The historical
Box 14 before measurements include WideCanon but not CQ-Arith/key-aware grouping.
Stage numbers below are zero-based; order names are the authored relations,
not guessed physical operator identifiers.

| Query/stage | Measured SF1 positions → output | Measured SF10 positions → output | Selected order and segmentation |
|---|---|---|---|
| H02/0 minimum | 1 → 5 → 1,987 → 158,960 → 117,422 groups | 1 → 5 → 20,033 → 1,602,640 → 1,183,098 groups | Region → Nation → Supplier → Partsupp. Region-first avoids starting at 800,000/8,000,000 Partsupp rows. Retain the checked grouped-input publication schedule: a separate 1,024/items-4 join-segment attempt was rejected after group extraction consumed the flatten. |
| H02/1 final | 747 Parts → 2,988 Partsupp → 642 European matches | 7,854 → 31,416 → 6,351 | Part → Partsupp → MinimumCost → Supplier → Nation → Region. The four-row Partsupp fanout needs no >4 boundary; Europe-first starts with 158,960/1,602,640 rows instead. Nullable minimum and string methods remain subject to checked placement. |
| H11/0 Stock | 1 Nation → 396 Suppliers → 31,680 Stock | 1 → 4,049 → 323,920 | Nation → Supplier → Partsupp; segment both expanding lookups, rather than scanning all Partsupp. |
| H11/1 PartValue | 31,680 → 29,818 groups | 323,920 → 304,774 | Stock scan and grouped multiplication/sum; no join boundary. |
| H11/2 TotalValue | 31,680 → 1 | 323,920 → 1 | Stock scan and global multiplication/sum; no join boundary. |
| H11/3 final | 1 total × 29,818 parts | 1 × 304,774 | TotalValue → PartValue, with checked widened arithmetic guard; partition output, no keyed fanout. |
| H13/0 counts | 150,000 Customers → 1,484,298 qualified orders → 99,996 groups | 1,500,000 → 14,845,369 → 999,981 | Customer → QualifiedOrders; expanding mean per customer 9.895/9.897, matched mean 14.844/14.846, max 41/46. Retain the checked grouped-input publication schedule; post-extraction join segmentation was rejected. Reverse order requires an extra unique Customer probe for every qualified order. |
| H13/1 zeros | 150,000 Customers → 50,004 | 1,500,000 → 500,019 | Customer root with anti-membership, not expansion. Preserve the one-time authored comment preprocessing and terminal union. |
| H15/0 Revenue | 225,954 quarter lines → 10,000 suppliers | 2,265,714 → 100,000 | Lineitem scan, date guard and exact revenue arithmetic into keyed sum; no join boundary. |
| H15/1 Maximum | 10,000 revenues → 1 nullable maximum | 100,000 → 1 | Revenue scan and global `max`, replacing quadratic anti-dominance. |
| H15/2 final | 1 maximum × 10,000 revenues → 1 winner | 1 × 100,000 → 1 | Maximum → Revenue → Supplier. Compare maximum before supplier lookup, preserving all ties and empty-input semantics. |
| H18/0 Quantity | 6,001,215 lines → 1,500,000 orders | 59,986,052 → 15,000,000 | Key-covered line scan and grouped exact quantity sum; no join boundary. |
| H18/1 final | 57 selected quantities → 57 Orders → 57 Customers | 624 → 624 → 624 | Quantity guard first, then unique Orders/Customer lookups; do not expand all customers/orders. |
| H20/0 Shipped | 909,455 dated lines → 543,210 part/supplier groups | 9,099,165 → 5,441,669 | Key-covered Lineitem scan and quantity sum; no join boundary. |
| H20/1 final | 2,127 forest Parts → 8,508 Partsupp → 5,843 shipped → 5,833 threshold → 263 Canadian matches | 21,551 → 86,204 → 58,782 → 58,655 → 2,377 | Part → Partsupp → Shipped → Supplier → Nation; four-row fanout, no >4 segment. Canada-root would enumerate 32,960/324,320 Partsupp before Part selection, versus 8,508/86,204. Final DISTINCT suppliers are 186/1,804. |

Semantic key assertions are separate from these statistics. Grouped
MinimumCost/PartValue/Revenue/Quantity/Shipped publications have their group
keys. Stock preserves the dbgen Partsupp composite key, and QualifiedOrders
preserves Orders' key through filtering/projection. These premises are supplied
as ordinary checked Sequence evidence, not inferred from sampled uniqueness.
The H15 CPU oracle SQL is unchanged; duplicate lines, equal-valued distinct
lines, tied maxima, zero revenue, empty input and quarter boundaries remain
covered by its exact DuckDB fixture.

### Placement, rejected candidates, and exact measurements

The accepted H02/0 and H13/0 grouped-input publications use
`partition(r7, 256, items 4)` and `partition(r2, 256, items 4)`, respectively.
H11/0 uses `segment(r3, 1024, items 4)` and
`segment(r6, 1024, items 4)`. Other non-grouped device terminal stages use
256-row output partitions. No arbitrary fanout limit is introduced.

All eight grouped nodes remove their valuation canon with checked key evidence.
H11/1–2, H15/0–1, H18/0 and H20/0 use resident group-argument kernels:
the group orchestrator and source placeholders have host grades, while numeric
argument formation, date filtering, key ordering and folding execute through
the existing GPU group implementation. This is not a claim that every trace
entry is device-graded. H11/0's scaled-cost bind and H11/3's widened threshold
guard, H18/1's quantity guard/price bind, and H20/1's widened availability guard
are explicitly graded `host.device.block`. H13's comment preprocessing and
H20's `starts_with` remain authored host work.

H02's minimum and H15's maximum are GPU-computed but nullable publications are
downloaded to host rows. Their final stages remain host-planned because the
existing translator refuses those nullable inputs. No compiler or translator
was patched, no rejection suppressed, and no host stage is labeled device-only.

| Candidate | Scale | Phase | Exact rejection |
|---|---|---|---|
| H02/0 extra post-extraction segment | SF1, SF10 | plan(machine) | `P-Decompose segment (r7 , 1024 , items 4): segment requires a named flatten level` |
| H13/0 extra post-extraction segment | SF1, SF10 | plan(machine) | `P-Decompose segment (r2 , 1024 , items 4): segment requires a named flatten level` |
| H02/1 device final stage | SF1, SF10 | translate | `T-Gpu: host publication r5 needs recorded device-representable code columns` |
| H15/2 device final stage | SF1, SF10 | translate | `T-Gpu: host publication r0 needs recorded device-representable code columns` |

The first two rejected candidates and their exact fallback measurements remain
in [`tpch_staged.initial.json`](tpch_staged.initial.json). The selected plans,
full replay traces, nullable-input rejections, five samples, independent DuckDB
t24 timing provenance, and exact row counts are in
[`tpch_staged.results.json`](tpch_staged.results.json).

**12/12 selected query/scale entries are exact**: one warmup and five measured
runs each compare the complete answer against the exported DuckDB oracle.
All six SF1 entries also pass same-Physical Sequential conformance. Timing is
the median of per-run sums of stage phase times, exactly as in Box 14; initial
loading, answer comparison and the authored one-time H13 preprocessing are
outside that interval. Group argument/reduction work is in the historical
`dedup` timing bucket even when valuation canon is removed. GPU runs were
exclusive; CPU compilation/testing did not overlap the final measurement.

| Query | Scale | Box 14 before ms | Box 14b after ms | Speedup | DuckDB t24 ms | Exact rows |
|---|---|---:|---:|---:|---:|---:|
| H02 | SF1 | 22.600 | 25.566 | 0.88× | 11 | 460 |
| H11 | SF1 | 33.102 | 15.872 | 2.09× | 6 | 1,048 |
| H13 | SF1 | 15.164 | 14.597 | 1.04× | 184 | 150,000 |
| H15 | SF1 | 664.863 | 12.177 | 54.60× | 11 | 1 |
| H18 | SF1 | 493.079 | 22.890 | 21.54× | 87 | 57 |
| H20 | SF1 | 399.399 | 16.097 | 24.81× | 21 | 186 |
| H02 | SF10 | 496.737 | 400.515 | 1.24× | 42 | 4,667 |
| H11 | SF10 | 521.764 | 41.827 | 12.47× | 15 | 0 |
| H13 | SF10 | 157.185 | 116.182 | 1.35× | 1,588 | 1,500,000 |
| H15 | SF10 | 33,827.704 | 18.305 | 1847.96× | 65 | 1 |
| H18 | SF10 | 5,393.855 | 172.927 | 31.19× | 771 | 624 |
| H20 | SF10 | 4,219.653 | 97.352 | 43.34× | 128 | 1,804 |

H02's SF1 regression is retained, not replaced by an automatic-mode minimum.
The same statistics-derived plan is used at both scales, and its SF10 minimum
publication remains dominated by a large nullable host boundary. The table
does not claim universal GPU-only execution or a globally optimal plan.

Reproduce from `experiments/gpu` with the CUDA_HOME and CARGO_BUILD_JOBS values
above, `CQ_GPU_TPCH_QUERIES=h02,h11,h13,h15,h18,h20`, then
`cargo oxide build --arch sm_89 -- --release -p cq-gpu-tpch` and
`./target/release/cq-gpu-tpch compare/tpch_staged.reproduced.json --mode advised --scale sf1 --scale sf10`.

Verification: the restricted release CUDA build passed, followed by the
12-entry exact benchmark above. In `features/workloads`,
`CARGO_BUILD_JOBS=1 cargo test --release -p cq-workloads --test grouped_h01 --test grouped_h02_h11_h13_h20 --test grouped_h06_h10_h12 --test grouped_h14_h18_h19 --test catalog`
passed **23 tests**: 14 grouped tests (including the unchanged exact H15 MAX
oracle cases) and nine catalog tests. Rustfmt was scoped to changed Rust ranges.
Existing unused-variable/exported-inline warnings were not suppressed.
