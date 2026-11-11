# Compressed-code GPU operators — 2026-11-11

Measurement-only microbenchmarks, not engine or end-to-end query changes.

## Findings

- E_b_semijoin: lowest-median winners {'V4_bitmap_zone': 4, 'V3_bitmap': 2}; speedup over u32 2.71–5.58×.
- E_b_set_build: lowest-median winners {'bitmap_atomicOr_zero': 3}; speedup over u32 3.36–3.43×.
- E_b_set_intersection: lowest-median winners {'bitmap_AND': 3}; speedup over u32 1.73–7.87×.
- O1: lowest-median winners {'V3_for': 7, 'V2_bytes': 1, 'V5_bitmap_rank': 16}; speedup over u32 1.14–23.80×.
- O2: lowest-median winners {'V5_bitmap_rank': 1}; speedup over u32 2.56–2.56×.
- O3: lowest-median winners {'V3_for': 1, 'V1_u32': 1}; speedup over u32 1.00–3.13×.
- O4: lowest-median winners {'V1_u32_significant_bits': 1}; speedup over u32 1.20–1.20×.
- O4_seal: lowest-median winners {'V3_for': 1}; speedup over u32 1.48–1.48×.
- O5: lowest-median winners {'V5_bitmap_rank': 2}; speedup over u32 1.53–5.24×.
- O1 random probes: FOR beats byte-aligned in 9/12 cases with CI wholly below 1, loses in 3/12; remaining CIs overlap 1.
- O1 sorted probes: FOR beats byte-aligned in 3/12 cases with CI wholly below 1, loses in 9/12; remaining CIs overlap 1.
- The design note's structural/code-domain hypothesis is tested directly, not inferred from old decode-first M3. Disjoint O3 copies encoded blocks with descriptor planning; overlapping O3 co-ranks directly into packed output words using registers only. Neither path materializes an uncompressed tile, even in shared memory. Compare both regimes and decode controls before generalizing.
- O3 disjoint: direct packed merge takes 0.32× u32 time and 0.09× decode-merge-reencode time. The design note's 'bit-packing is right for merges' is conditional: skipping decode does not guarantee a faster overlap merge.
- O3 interleaved: direct packed merge takes 5.65× u32 time and 1.16× decode-merge-reencode time. The design note's 'bit-packing is right for merges' is conditional: skipping decode does not guarantee a faster overlap merge.
- O2 with the same V3 key directory: FOR offsets+payload take 1.16× plain offsets+payload time. Key-layout improvements do not eliminate the earlier M2 payload-access penalty.
- E-b 3% on FOR probes: bitmap+zone/plain bitmap median time = 1.160. Zone grading is exact but is not an automatic speedup.
- E-b 15% on FOR probes: bitmap+zone/plain bitmap median time = 0.993. Zone grading is exact but is not an automatic speedup.
- E-b 50% on FOR probes: bitmap+zone/plain bitmap median time = 0.978. Zone grading is exact but is not an automatic speedup.
- Strict-definition caveat: CUB sort+unique, u32 hash-table build and sorted-set output materialize plain code arrays and therefore are L2, despite staying in the canonical domain. Calling every narrow-code operation L1 would contradict the note's stated definition.
- O4 seal controls are device copies, not a claim that retaining already-plain output requires a copy. Plain output can be retained for zero additional seal cost; V2/V3 measured seal time is additional work. Combined sort+seal was not timed as one event region.
- Noise/order floor: identical u32 duplicate median ratios range 0.761–1.120. Rotation preserves variant adjacency, so this diagnostic includes cache/order bias, not just random timing noise. Bootstrap CIs do not remove that bias; small layout differences comparable to the same-case duplicate discrepancy are inconclusive.

## Protocol

Five warmups and 21 measured CUDA-event samples per variant, rotated within paired rounds. Every sample is checked against a CPU oracle. Ratios below are median paired time ratios (lower is better), with 5,000-bootstrap 95% percentile CIs. Allocations, file loading, CPU encoding and oracle construction are excluded from operator timing; separate wall records include preparation. All GPU jobs take `/tmp/mini_linq-gpu.lock` and release it between jobs; builds take the shared timing lock only.

L0: descriptors/bitmap words only; L1: code extraction/rebasing in registers without global plain materialization; L2: logical-width reconstruction or global plain-array materialization. Plain u64 is an L2 control. CUB sort necessarily emits plain arrays and is labeled L2 under this strict definition even though its comparison domain is canonical codes.

E-a FOR uses contiguous packed residuals; V2 uses actual u8/u16/u32 loads. Headers are four u32 fields (base, exact max, word offset, width), plus a u32 fence per 1024 keys. Delta restarts every 64 keys and uses one such header per restart. V5 includes one u32 rank prefix per bitmap word for locate. E-b scan encoding and details are recorded in group metadata.

## Lowest-median variants

| Operation / case | Winner | ms | ratio / V0 [95% CI] | ratio / V1 [95% CI] |
|---|---|---:|---:|---:|
|E_b_semijoin / sf10_s15_for1024|V4_bitmap_zone|0.1933|0.180 [0.179, 0.182]|0.194 [0.193, 0.196]|
|E_b_semijoin / sf10_s15_plain_u32|V3_bitmap|0.3065|0.304 [0.302, 0.305]|0.328 [0.326, 0.331]|
|E_b_semijoin / sf10_s3_for1024|V3_bitmap|0.1691|0.176 [0.174, 0.177]|0.188 [0.187, 0.190]|
|E_b_semijoin / sf10_s3_plain_u32|V4_bitmap_zone|0.3073|0.341 [0.340, 0.344]|0.369 [0.366, 0.372]|
|E_b_semijoin / sf10_s50_for1024|V4_bitmap_zone|0.1921|0.166 [0.165, 0.170]|0.179 [0.178, 0.185]|
|E_b_semijoin / sf10_s50_plain_u32|V4_bitmap_zone|0.3029|0.278 [0.275, 0.279]|0.299 [0.297, 0.303]|
|E_b_set_build / sf10_s15|bitmap_atomicOr_zero|0.0388|0.162 [0.156, 0.164]|0.298 [0.281, 0.312]|
|E_b_set_build / sf10_s3|bitmap_atomicOr_zero|0.0261|0.205 [0.190, 0.219]|0.292 [0.278, 0.314]|
|E_b_set_build / sf10_s50|bitmap_atomicOr_zero|0.0769|0.083 [0.079, 0.085]|0.297 [0.287, 0.308]|
|E_b_set_intersection / sf10_s15|bitmap_AND|0.0291|0.288 [0.260, 0.309]|0.339 [0.305, 0.364]|
|E_b_set_intersection / sf10_s3|bitmap_AND|0.0190|0.559 [0.540, 0.619]|0.579 [0.526, 0.621]|
|E_b_set_intersection / sf10_s50|bitmap_AND|0.0371|0.089 [0.082, 0.104]|0.127 [0.120, 0.148]|
|O1 / clustered_h10_random|V3_for|0.2035|0.111 [0.110, 0.112]|0.140 [0.139, 0.141]|
|O1 / clustered_h10_sorted|V3_for|0.0823|0.558 [0.543, 0.573]|0.617 [0.606, 0.657]|
|O1 / clustered_h90_random|V3_for|0.6658|0.342 [0.340, 0.345]|0.449 [0.443, 0.451]|
|O1 / clustered_h90_sorted|V2_bytes|0.1166|0.548 [0.544, 0.565]|0.783 [0.772, 0.853]|
|O1 / customers_h10_random|V5_bitmap_rank|0.0554|0.112 [0.110, 0.117]|0.115 [0.113, 0.120]|
|O1 / customers_h10_sorted|V5_bitmap_rank|0.0216|0.299 [0.276, 0.310]|0.219 [0.211, 0.221]|
|O1 / customers_h90_random|V5_bitmap_rank|0.0905|0.185 [0.184, 0.187]|0.188 [0.187, 0.191]|
|O1 / customers_h90_sorted|V5_bitmap_rank|0.0233|0.308 [0.295, 0.330]|0.230 [0.216, 0.248]|
|O1 / dense_h10_random|V5_bitmap_rank|0.0622|0.032 [0.031, 0.033]|0.042 [0.041, 0.043]|
|O1 / dense_h10_sorted|V5_bitmap_rank|0.0262|0.129 [0.126, 0.131]|0.179 [0.175, 0.183]|
|O1 / dense_h90_random|V5_bitmap_rank|0.0957|0.049 [0.048, 0.049]|0.065 [0.064, 0.065]|
|O1 / dense_h90_sorted|V5_bitmap_rank|0.0295|0.140 [0.132, 0.145]|0.201 [0.187, 0.206]|
|O1 / orders_h10_random|V5_bitmap_rank|0.0693|0.037 [0.036, 0.037]|0.049 [0.048, 0.050]|
|O1 / orders_h10_sorted|V5_bitmap_rank|0.0297|0.148 [0.147, 0.154]|0.205 [0.200, 0.217]|
|O1 / orders_h90_random|V5_bitmap_rank|0.1073|0.056 [0.055, 0.057]|0.075 [0.074, 0.077]|
|O1 / orders_h90_sorted|V5_bitmap_rank|0.0321|0.156 [0.152, 0.167]|0.222 [0.213, 0.237]|
|O1 / parts_h10_random|V5_bitmap_rank|0.0283|0.350 [0.337, 0.367]|0.280 [0.268, 0.289]|
|O1 / parts_h10_sorted|V5_bitmap_rank|0.0191|0.769 [0.691, 0.824]|0.755 [0.676, 0.799]|
|O1 / parts_h90_random|V5_bitmap_rank|0.0906|0.181 [0.181, 0.183]|0.182 [0.180, 0.184]|
|O1 / parts_h90_sorted|V5_bitmap_rank|0.0228|0.320 [0.301, 0.335]|0.244 [0.229, 0.253]|
|O1 / sparse_h10_random|V3_for|0.7675|0.391 [0.390, 0.392]|0.514 [0.513, 0.516]|
|O1 / sparse_h10_sorted|V3_for|0.1308|0.623 [0.620, 0.630]|0.878 [0.869, 0.886]|
|O1 / sparse_h90_random|V3_for|0.7670|0.390 [0.389, 0.392]|0.513 [0.511, 0.518]|
|O1 / sparse_h90_sorted|V3_for|0.1300|0.608 [0.605, 0.611]|0.865 [0.863, 0.883]|
|O2 / sf10_order_part|V5_bitmap_rank|0.7431|0.299 [0.298, 0.299]|0.391 [0.390, 0.393]|
|O3 / disjoint|V3_for|0.0773|0.160 [0.153, 0.170]|0.319 [0.307, 0.329]|
|O3 / interleaved|V1_u32|0.1446|0.469 [0.460, 0.487]|1.000 [1.000, 1.000]|
|O4 / unsorted_25pct_duplicates|V1_u32_significant_bits|0.5570|0.185 [0.185, 0.186]|0.831 [0.822, 0.842]|
|O4_seal / sorted_unique|V3_for|0.0500|0.215 [0.203, 0.224]|0.676 [0.623, 0.751]|
|O5 / run16M_batch1000000|V5_bitmap_rank|0.0325|0.709 [0.675, 0.785]|0.654 [0.573, 0.740]|
|O5 / run16M_batch16000000|V5_bitmap_rank|0.0855|0.227 [0.218, 0.231]|0.191 [0.188, 0.195]|

Median rankings do not by themselves establish a statistically significant win. See noise duplicates below.

## Full measurements and footprints

For O1/O5, bytes/key is complete stored run bytes divided by distinct run keys. O2 includes keys, offsets and payload divided by distinct group keys, not payload rows. Build/intersection/seal footprint definitions are in each group record.

| Operation / case | Variant | Level | bytes/key | ms [95% CI] | ratio / V0 [95% CI] | ratio / V1 [95% CI] |
|---|---|---|---:|---:|---:|---:|
|E_b_semijoin / sf10_s15_for1024|V0_u64|L2_u64_key_control|1.6296|1.0712 [1.0678, 1.0757]|1.000 [1.000, 1.000]|1.076 [1.071, 1.079]|
|E_b_semijoin / sf10_s15_for1024|V1_noise|L1|1.4794|0.9897 [0.9845, 0.9960]|0.923 [0.921, 0.926]|0.995 [0.987, 1.000]|
|E_b_semijoin / sf10_s15_for1024|V1_u32|L1|1.4794|0.9974 [0.9930, 0.9991]|0.929 [0.927, 0.934]|1.000 [1.000, 1.000]|
|E_b_semijoin / sf10_s15_for1024|V2_hash_u32|L1|1.8886|0.3809 [0.3788, 0.3832]|0.356 [0.354, 0.357]|0.383 [0.381, 0.384]|
|E_b_semijoin / sf10_s15_for1024|V3_bitmap|L1|1.4543|0.1948 [0.1906, 0.1975]|0.183 [0.178, 0.184]|0.196 [0.192, 0.198]|
|E_b_semijoin / sf10_s15_for1024|V4_bitmap_zone|L0+L1|1.4543|0.1933 [0.1923, 0.1956]|0.180 [0.179, 0.182]|0.194 [0.193, 0.196]|
|E_b_semijoin / sf10_s15_for1024|V5_bloom_exact_u32|L1|1.5494|0.9560 [0.9537, 0.9581]|0.894 [0.885, 0.897]|0.958 [0.955, 0.969]|
|E_b_semijoin / sf10_s15_plain_u32|V0_u64|L2_u64_key_control|4.3004|1.0088 [1.0063, 1.0129]|1.000 [1.000, 1.000]|1.080 [1.076, 1.084]|
|E_b_semijoin / sf10_s15_plain_u32|V1_noise|L1|4.1502|0.9333 [0.9323, 0.9375]|0.927 [0.922, 0.930]|0.998 [0.996, 0.999]|
|E_b_semijoin / sf10_s15_plain_u32|V1_u32|L1|4.1502|0.9355 [0.9316, 0.9367]|0.926 [0.923, 0.929]|1.000 [1.000, 1.000]|
|E_b_semijoin / sf10_s15_plain_u32|V2_hash_u32|L1|4.5594|0.4309 [0.4286, 0.4322]|0.426 [0.424, 0.428]|0.461 [0.460, 0.463]|
|E_b_semijoin / sf10_s15_plain_u32|V3_bitmap|L1|4.1250|0.3065 [0.3049, 0.3091]|0.304 [0.302, 0.305]|0.328 [0.326, 0.331]|
|E_b_semijoin / sf10_s15_plain_u32|V4_bitmap_zone|L0+L1|4.1406|0.3068 [0.3039, 0.3087]|0.303 [0.302, 0.305]|0.328 [0.326, 0.330]|
|E_b_semijoin / sf10_s15_plain_u32|V5_bloom_exact_u32|L1|4.2201|0.9077 [0.9043, 0.9091]|0.900 [0.895, 0.902]|0.971 [0.969, 0.977]|
|E_b_semijoin / sf10_s3_for1024|V0_u64|L2_u64_key_control|1.3891|0.9646 [0.9630, 0.9661]|1.000 [1.000, 1.000]|1.074 [1.070, 1.077]|
|E_b_semijoin / sf10_s3_for1024|V1_noise|L1|1.3592|0.9001 [0.8964, 0.9020]|0.933 [0.930, 0.935]|1.002 [0.998, 1.006]|
|E_b_semijoin / sf10_s3_for1024|V1_u32|L1|1.3592|0.8977 [0.8964, 0.8992]|0.931 [0.929, 0.935]|1.000 [1.000, 1.000]|
|E_b_semijoin / sf10_s3_for1024|V2_hash_u32|L1|1.3992|0.3382 [0.3364, 0.3386]|0.351 [0.349, 0.352]|0.376 [0.374, 0.378]|
|E_b_semijoin / sf10_s3_for1024|V3_bitmap|L1|1.4543|0.1691 [0.1679, 0.1701]|0.176 [0.174, 0.177]|0.188 [0.187, 0.190]|
|E_b_semijoin / sf10_s3_for1024|V4_bitmap_zone|L0+L1|1.4543|0.1962 [0.1950, 0.1978]|0.204 [0.202, 0.204]|0.219 [0.217, 0.220]|
|E_b_semijoin / sf10_s3_for1024|V5_bloom_exact_u32|L1|1.3767|0.4671 [0.4655, 0.4697]|0.485 [0.483, 0.487]|0.520 [0.517, 0.523]|
|E_b_semijoin / sf10_s3_plain_u32|V0_u64|L2_u64_key_control|4.0598|0.8987 [0.8952, 0.9019]|1.000 [1.000, 1.000]|1.081 [1.077, 1.086]|
|E_b_semijoin / sf10_s3_plain_u32|V1_noise|L1|4.0299|0.8333 [0.8313, 0.8341]|0.926 [0.924, 0.930]|1.002 [0.998, 1.006]|
|E_b_semijoin / sf10_s3_plain_u32|V1_u32|L1|4.0299|0.8328 [0.8271, 0.8356]|0.925 [0.921, 0.929]|1.000 [1.000, 1.000]|
|E_b_semijoin / sf10_s3_plain_u32|V2_hash_u32|L1|4.0699|0.3419 [0.3400, 0.3455]|0.381 [0.378, 0.384]|0.412 [0.409, 0.414]|
|E_b_semijoin / sf10_s3_plain_u32|V3_bitmap|L1|4.1250|0.3089 [0.3058, 0.3110]|0.343 [0.340, 0.346]|0.371 [0.366, 0.373]|
|E_b_semijoin / sf10_s3_plain_u32|V4_bitmap_zone|L0+L1|4.1406|0.3073 [0.3042, 0.3101]|0.341 [0.340, 0.344]|0.369 [0.366, 0.372]|
|E_b_semijoin / sf10_s3_plain_u32|V5_bloom_exact_u32|L1|4.0474|0.4549 [0.4516, 0.4570]|0.507 [0.503, 0.508]|0.547 [0.541, 0.550]|
|E_b_semijoin / sf10_s50_for1024|V0_u64|L2_u64_key_control|2.3295|1.1612 [1.1593, 1.1641]|1.000 [1.000, 1.000]|1.081 [1.075, 1.085]|
|E_b_semijoin / sf10_s50_for1024|V1_noise|L1|1.8294|1.0763 [1.0737, 1.0799]|0.925 [0.921, 0.931]|1.001 [0.997, 1.005]|
|E_b_semijoin / sf10_s50_for1024|V1_u32|L1|1.8294|1.0748 [1.0718, 1.0793]|0.925 [0.922, 0.930]|1.000 [1.000, 1.000]|
|E_b_semijoin / sf10_s50_for1024|V2_hash_u32|L1|2.4480|0.6365 [0.6331, 0.6391]|0.550 [0.547, 0.550]|0.590 [0.589, 0.595]|
|E_b_semijoin / sf10_s50_for1024|V3_bitmap|L1|1.4543|0.1965 [0.1950, 0.1982]|0.170 [0.168, 0.171]|0.183 [0.180, 0.184]|
|E_b_semijoin / sf10_s50_for1024|V4_bitmap_zone|L0+L1|1.4543|0.1921 [0.1916, 0.1979]|0.166 [0.165, 0.170]|0.179 [0.178, 0.185]|
|E_b_semijoin / sf10_s50_for1024|V5_bloom_exact_u32|L1|2.1090|1.2908 [1.2880, 1.2948]|1.111 [1.109, 1.117]|1.201 [1.196, 1.205]|
|E_b_semijoin / sf10_s50_plain_u32|V0_u64|L2_u64_key_control|5.0002|1.0950 [1.0938, 1.0976]|1.000 [1.000, 1.000]|1.084 [1.081, 1.086]|
|E_b_semijoin / sf10_s50_plain_u32|V1_noise|L1|4.5001|1.0105 [1.0074, 1.0115]|0.922 [0.920, 0.922]|0.998 [0.997, 1.001]|
|E_b_semijoin / sf10_s50_plain_u32|V1_u32|L1|4.5001|1.0099 [1.0089, 1.0125]|0.922 [0.921, 0.925]|1.000 [1.000, 1.000]|
|E_b_semijoin / sf10_s50_plain_u32|V2_hash_u32|L1|5.1187|0.7055 [0.7024, 0.7078]|0.644 [0.642, 0.647]|0.698 [0.696, 0.699]|
|E_b_semijoin / sf10_s50_plain_u32|V3_bitmap|L1|4.1250|0.3052 [0.3026, 0.3071]|0.278 [0.276, 0.280]|0.301 [0.300, 0.303]|
|E_b_semijoin / sf10_s50_plain_u32|V4_bitmap_zone|L0+L1|4.1406|0.3029 [0.3006, 0.3045]|0.278 [0.275, 0.279]|0.299 [0.297, 0.303]|
|E_b_semijoin / sf10_s50_plain_u32|V5_bloom_exact_u32|L1|4.7798|1.2449 [1.2418, 1.2464]|1.136 [1.132, 1.139]|1.232 [1.229, 1.233]|
|E_b_set_build / sf10_s15|V0_u64|L2_materialized_codes|16.0000|0.2378 [0.2353, 0.2406]|1.000 [1.000, 1.000]|1.867 [1.798, 1.927]|
|E_b_set_build / sf10_s15|V1_noise|L2_materialized_codes|8.0000|0.1212 [0.1173, 0.1252]|0.506 [0.497, 0.530]|0.936 [0.909, 0.965]|
|E_b_set_build / sf10_s15|V1_u32|L2_materialized_codes|8.0000|0.1279 [0.1246, 0.1324]|0.536 [0.519, 0.556]|1.000 [1.000, 1.000]|
|E_b_set_build / sf10_s15|bitmap_atomicOr_zero|L1|7.3298|0.0388 [0.0370, 0.0408]|0.162 [0.156, 0.164]|0.298 [0.281, 0.312]|
|E_b_set_build / sf10_s15|hash_zero_insert_u32|L2_materialized_codes|18.8974|0.0963 [0.0936, 0.0995]|0.403 [0.398, 0.420]|0.745 [0.726, 0.784]|
|E_b_set_build / sf10_s3|V0_u64|L2_materialized_codes|16.0000|0.1250 [0.1234, 0.1287]|1.000 [1.000, 1.000]|1.482 [1.400, 1.497]|
|E_b_set_build / sf10_s3|V1_noise|L2_materialized_codes|8.0000|0.0882 [0.0858, 0.0915]|0.682 [0.671, 0.711]|1.004 [0.969, 1.031]|
|E_b_set_build / sf10_s3|V1_u32|L2_materialized_codes|8.0000|0.0868 [0.0847, 0.0909]|0.675 [0.668, 0.714]|1.000 [1.000, 1.000]|
|E_b_set_build / sf10_s3|bitmap_atomicOr_zero|L1|20.7115|0.0261 [0.0246, 0.0277]|0.205 [0.190, 0.219]|0.292 [0.278, 0.314]|
|E_b_set_build / sf10_s3|hash_zero_insert_u32|L2_materialized_codes|13.3457|0.0325 [0.0308, 0.0351]|0.254 [0.244, 0.273]|0.369 [0.343, 0.413]|
|E_b_set_build / sf10_s50|V0_u64|L2_materialized_codes|16.0000|0.9374 [0.9368, 0.9422]|1.000 [1.000, 1.000]|3.643 [3.596, 3.661]|
|E_b_set_build / sf10_s50|V1_noise|L2_materialized_codes|8.0000|0.2618 [0.2606, 0.2672]|0.279 [0.276, 0.285]|1.018 [0.997, 1.031]|
|E_b_set_build / sf10_s50|V1_u32|L2_materialized_codes|8.0000|0.2578 [0.2568, 0.2603]|0.275 [0.273, 0.278]|1.000 [1.000, 1.000]|
|E_b_set_build / sf10_s50|bitmap_atomicOr_zero|L1|5.0000|0.0769 [0.0738, 0.0811]|0.083 [0.079, 0.085]|0.297 [0.287, 0.308]|
|E_b_set_build / sf10_s50|hash_zero_insert_u32|L2_materialized_codes|12.9481|0.6546 [0.6538, 0.6617]|0.701 [0.694, 0.705]|2.541 [2.513, 2.569]|
|E_b_set_intersection / sf10_s15|V0_u64|L2_materialized_codes|12.0024|0.1015 [0.0989, 0.1037]|1.000 [1.000, 1.000]|1.175 [1.156, 1.228]|
|E_b_set_intersection / sf10_s15|V1_noise|L2_materialized_codes|6.0012|0.0753 [0.0728, 0.0841]|0.815 [0.765, 0.844]|0.973 [0.925, 0.997]|
|E_b_set_intersection / sf10_s15|V1_u32|L2_materialized_codes|6.0012|0.0834 [0.0765, 0.0849]|0.851 [0.815, 0.865]|1.000 [1.000, 1.000]|
|E_b_set_intersection / sf10_s15|bitmap_AND|L0|4.9978|0.0291 [0.0255, 0.0300]|0.288 [0.260, 0.309]|0.339 [0.305, 0.364]|
|E_b_set_intersection / sf10_s3|V0_u64|L2_materialized_codes|12.0009|0.0315 [0.0301, 0.0336]|1.000 [1.000, 1.000]|1.033 [0.956, 1.069]|
|E_b_set_intersection / sf10_s3|V1_noise|L2_materialized_codes|6.0004|0.0296 [0.0273, 0.0315]|0.926 [0.869, 1.004]|0.921 [0.817, 0.979]|
|E_b_set_intersection / sf10_s3|V1_u32|L2_materialized_codes|6.0004|0.0332 [0.0289, 0.0370]|0.968 [0.935, 1.047]|1.000 [1.000, 1.000]|
|E_b_set_intersection / sf10_s3|bitmap_AND|L0|25.0727|0.0190 [0.0170, 0.0205]|0.559 [0.540, 0.619]|0.579 [0.526, 0.621]|
|E_b_set_intersection / sf10_s50|V0_u64|L2_materialized_codes|11.9987|0.4110 [0.4091, 0.4132]|1.000 [1.000, 1.000]|1.437 [1.430, 1.454]|
|E_b_set_intersection / sf10_s50|V1_noise|L2_materialized_codes|5.9993|0.2152 [0.2143, 0.2224]|0.524 [0.519, 0.545]|0.761 [0.747, 0.772]|
|E_b_set_intersection / sf10_s50|V1_u32|L2_materialized_codes|5.9993|0.2851 [0.2834, 0.2897]|0.696 [0.688, 0.699]|1.000 [1.000, 1.000]|
|E_b_set_intersection / sf10_s50|bitmap_AND|L0|1.4995|0.0371 [0.0340, 0.0421]|0.089 [0.082, 0.104]|0.127 [0.120, 0.148]|
|O1 / clustered_h10_random|V0_u64|L2|8.0000|1.8315 [1.8292, 1.8341]|1.000 [1.000, 1.000]|1.261 [1.253, 1.263]|
|O1 / clustered_h10_random|V1_noise|L1|4.0000|1.4510 [1.4471, 1.4562]|0.792 [0.790, 0.795]|0.999 [0.996, 1.002]|
|O1 / clustered_h10_random|V1_u32|L1|4.0000|1.4554 [1.4538, 1.4596]|0.793 [0.792, 0.798]|1.000 [1.000, 1.000]|
|O1 / clustered_h10_random|V2_bytes|L1|2.0195|0.6687 [0.6625, 0.6736]|0.365 [0.362, 0.367]|0.460 [0.455, 0.463]|
|O1 / clustered_h10_random|V3_for|L1|1.2695|0.2035 [0.2029, 0.2052]|0.111 [0.110, 0.112]|0.140 [0.139, 0.141]|
|O1 / clustered_h10_random|V4_delta|L1|0.3789|2.6426 [2.6370, 2.6459]|1.443 [1.438, 1.446]|1.815 [1.811, 1.817]|
|O1 / clustered_h10_sorted|V0_u64|L2|8.0000|0.1484 [0.1477, 0.1488]|1.000 [1.000, 1.000]|1.095 [1.075, 1.134]|
|O1 / clustered_h10_sorted|V1_noise|L1|4.0000|0.1307 [0.1300, 0.1319]|0.876 [0.865, 0.889]|0.971 [0.947, 1.022]|
|O1 / clustered_h10_sorted|V1_u32|L1|4.0000|0.1354 [0.1349, 0.1366]|0.913 [0.882, 0.931]|1.000 [1.000, 1.000]|
|O1 / clustered_h10_sorted|V2_bytes|L1|2.0195|0.1090 [0.1085, 0.1106]|0.742 [0.729, 0.749]|0.807 [0.798, 0.833]|
|O1 / clustered_h10_sorted|V3_for|L1|1.2695|0.0823 [0.0815, 0.0834]|0.558 [0.543, 0.573]|0.617 [0.606, 0.657]|
|O1 / clustered_h10_sorted|V4_delta|L1|0.3789|0.3634 [0.3615, 0.3646]|2.448 [2.440, 2.471]|2.684 [2.657, 2.792]|
|O1 / clustered_h90_random|V0_u64|L2|8.0000|1.9461 [1.9429, 1.9491]|1.000 [1.000, 1.000]|1.301 [1.299, 1.303]|
|O1 / clustered_h90_random|V1_noise|L1|4.0000|1.4939 [1.4869, 1.5015]|0.769 [0.763, 0.771]|1.000 [0.993, 1.010]|
|O1 / clustered_h90_random|V1_u32|L1|4.0000|1.4940 [1.4904, 1.4991]|0.769 [0.767, 0.770]|1.000 [1.000, 1.000]|
|O1 / clustered_h90_random|V2_bytes|L1|2.0195|0.7257 [0.7242, 0.7297]|0.374 [0.371, 0.375]|0.487 [0.484, 0.494]|
|O1 / clustered_h90_random|V3_for|L1|1.2695|0.6658 [0.6601, 0.6710]|0.342 [0.340, 0.345]|0.449 [0.443, 0.451]|
|O1 / clustered_h90_random|V4_delta|L1|0.3789|1.1776 [1.1708, 1.2054]|0.607 [0.602, 0.620]|0.789 [0.783, 0.813]|
|O1 / clustered_h90_sorted|V0_u64|L2|8.0000|0.2106 [0.2099, 0.2150]|1.000 [1.000, 1.000]|1.417 [1.409, 1.439]|
|O1 / clustered_h90_sorted|V1_noise|L1|4.0000|0.1402 [0.1388, 0.1439]|0.660 [0.652, 0.673]|0.945 [0.932, 0.988]|
|O1 / clustered_h90_sorted|V1_u32|L1|4.0000|0.1483 [0.1458, 0.1494]|0.706 [0.695, 0.709]|1.000 [1.000, 1.000]|
|O1 / clustered_h90_sorted|V2_bytes|L1|2.0195|0.1166 [0.1148, 0.1195]|0.548 [0.544, 0.565]|0.783 [0.772, 0.853]|
|O1 / clustered_h90_sorted|V3_for|L1|1.2695|0.1260 [0.1253, 0.1269]|0.599 [0.587, 0.612]|0.856 [0.839, 0.886]|
|O1 / clustered_h90_sorted|V4_delta|L1|0.3789|0.3572 [0.3564, 0.3592]|1.699 [1.685, 1.703]|2.412 [2.391, 2.448]|
|O1 / customers_h10_random|V0_u64|L2|8.0000|0.4920 [0.4908, 0.4954]|1.000 [1.000, 1.000]|1.026 [1.018, 1.029]|
|O1 / customers_h10_random|V1_noise|L1|4.0000|0.4868 [0.4847, 0.4877]|0.990 [0.979, 0.995]|1.008 [1.004, 1.012]|
|O1 / customers_h10_random|V1_u32|L1|4.0000|0.4814 [0.4797, 0.4834]|0.975 [0.972, 0.982]|1.000 [1.000, 1.000]|
|O1 / customers_h10_random|V2_bytes|L1|2.0205|0.4663 [0.4648, 0.4688]|0.948 [0.941, 0.954]|0.966 [0.964, 0.972]|
|O1 / customers_h10_random|V3_for|L1|1.3950|0.4562 [0.4537, 0.4609]|0.928 [0.924, 0.938]|0.951 [0.940, 0.959]|
|O1 / customers_h10_random|V4_delta|L1|0.5039|0.4998 [0.4978, 0.5016]|1.017 [1.012, 1.022]|1.041 [1.035, 1.046]|
|O1 / customers_h10_random|V5_bitmap_rank|L1|0.3750|0.0554 [0.0547, 0.0581]|0.112 [0.110, 0.117]|0.115 [0.113, 0.120]|
|O1 / customers_h10_sorted|V0_u64|L2|8.0000|0.0733 [0.0718, 0.0768]|1.000 [1.000, 1.000]|0.747 [0.722, 0.784]|
|O1 / customers_h10_sorted|V1_noise|L1|4.0000|0.0972 [0.0968, 0.0990]|1.349 [1.288, 1.374]|0.990 [0.937, 1.014]|
|O1 / customers_h10_sorted|V1_u32|L1|4.0000|0.0981 [0.0973, 0.1016]|1.339 [1.276, 1.386]|1.000 [1.000, 1.000]|
|O1 / customers_h10_sorted|V2_bytes|L1|2.0205|0.0899 [0.0887, 0.0933]|1.229 [1.179, 1.262]|0.906 [0.898, 0.927]|
|O1 / customers_h10_sorted|V3_for|L1|1.3950|0.1082 [0.1062, 0.1096]|1.493 [1.410, 1.545]|1.103 [1.079, 1.112]|
|O1 / customers_h10_sorted|V4_delta|L1|0.5039|0.2523 [0.2516, 0.2543]|3.462 [3.292, 3.510]|2.574 [2.484, 2.585]|
|O1 / customers_h10_sorted|V5_bitmap_rank|L1|0.3750|0.0216 [0.0215, 0.0226]|0.299 [0.276, 0.310]|0.219 [0.211, 0.221]|
|O1 / customers_h90_random|V0_u64|L2|8.0000|0.4883 [0.4875, 0.4904]|1.000 [1.000, 1.000]|1.017 [1.014, 1.021]|
|O1 / customers_h90_random|V1_noise|L1|4.0000|0.4811 [0.4800, 0.4826]|0.983 [0.980, 0.990]|1.002 [0.996, 1.005]|
|O1 / customers_h90_random|V1_u32|L1|4.0000|0.4807 [0.4798, 0.4819]|0.984 [0.980, 0.986]|1.000 [1.000, 1.000]|
|O1 / customers_h90_random|V2_bytes|L1|2.0205|0.4707 [0.4691, 0.4724]|0.964 [0.955, 0.968]|0.978 [0.973, 0.983]|
|O1 / customers_h90_random|V3_for|L1|1.3950|0.4539 [0.4532, 0.4582]|0.930 [0.923, 0.939]|0.944 [0.938, 0.951]|
|O1 / customers_h90_random|V4_delta|L1|0.5039|0.4888 [0.4877, 0.4904]|1.000 [0.996, 1.006]|1.017 [1.014, 1.020]|
|O1 / customers_h90_random|V5_bitmap_rank|L1|0.3750|0.0905 [0.0899, 0.0922]|0.185 [0.184, 0.187]|0.188 [0.187, 0.191]|
|O1 / customers_h90_sorted|V0_u64|L2|8.0000|0.0735 [0.0718, 0.0747]|1.000 [1.000, 1.000]|0.745 [0.731, 0.763]|
|O1 / customers_h90_sorted|V1_noise|L1|4.0000|0.0974 [0.0963, 0.0998]|1.343 [1.323, 1.372]|0.994 [0.984, 1.029]|
|O1 / customers_h90_sorted|V1_u32|L1|4.0000|0.0972 [0.0969, 0.0985]|1.343 [1.311, 1.367]|1.000 [1.000, 1.000]|
|O1 / customers_h90_sorted|V2_bytes|L1|2.0205|0.0904 [0.0891, 0.0924]|1.238 [1.231, 1.262]|0.917 [0.912, 0.931]|
|O1 / customers_h90_sorted|V3_for|L1|1.3950|0.1076 [0.1070, 0.1121]|1.486 [1.465, 1.571]|1.109 [1.092, 1.150]|
|O1 / customers_h90_sorted|V4_delta|L1|0.5039|0.2495 [0.2487, 0.2508]|3.408 [3.347, 3.482]|2.557 [2.514, 2.586]|
|O1 / customers_h90_sorted|V5_bitmap_rank|L1|0.3750|0.0233 [0.0211, 0.0246]|0.308 [0.295, 0.330]|0.230 [0.216, 0.248]|
|O1 / dense_h10_random|V0_u64|L2|8.0000|1.9577 [1.9547, 1.9634]|1.000 [1.000, 1.000]|1.311 [1.306, 1.317]|
|O1 / dense_h10_random|V1_noise|L1|4.0000|1.4945 [1.4911, 1.4976]|0.763 [0.760, 0.765]|1.001 [0.997, 1.004]|
|O1 / dense_h10_random|V1_u32|L1|4.0000|1.4917 [1.4901, 1.5006]|0.763 [0.759, 0.766]|1.000 [1.000, 1.000]|
|O1 / dense_h10_random|V2_bytes|L1|2.0195|0.7435 [0.7313, 0.7455]|0.379 [0.374, 0.380]|0.495 [0.491, 0.501]|
|O1 / dense_h10_random|V3_for|L1|1.3945|0.7189 [0.7132, 0.7278]|0.368 [0.364, 0.371]|0.483 [0.476, 0.489]|
|O1 / dense_h10_random|V4_delta|L1|0.5039|1.2260 [1.2182, 1.2291]|0.625 [0.623, 0.627]|0.818 [0.817, 0.823]|
|O1 / dense_h10_random|V5_bitmap_rank|L1|0.2750|0.0622 [0.0607, 0.0644]|0.032 [0.031, 0.033]|0.042 [0.041, 0.043]|
|O1 / dense_h10_sorted|V0_u64|L2|8.0000|0.2044 [0.2033, 0.2058]|1.000 [1.000, 1.000]|1.381 [1.377, 1.401]|
|O1 / dense_h10_sorted|V1_noise|L1|4.0000|0.1391 [0.1378, 0.1399]|0.680 [0.676, 0.687]|0.936 [0.932, 0.949]|
|O1 / dense_h10_sorted|V1_u32|L1|4.0000|0.1477 [0.1473, 0.1486]|0.724 [0.714, 0.726]|1.000 [1.000, 1.000]|
|O1 / dense_h10_sorted|V2_bytes|L1|2.0195|0.1141 [0.1133, 0.1148]|0.558 [0.556, 0.563]|0.771 [0.765, 0.785]|
|O1 / dense_h10_sorted|V3_for|L1|1.3945|0.1250 [0.1239, 0.1257]|0.610 [0.605, 0.615]|0.842 [0.840, 0.850]|
|O1 / dense_h10_sorted|V4_delta|L1|0.5039|0.3550 [0.3535, 0.3558]|1.735 [1.729, 1.748]|2.396 [2.388, 2.414]|
|O1 / dense_h10_sorted|V5_bitmap_rank|L1|0.2750|0.0262 [0.0256, 0.0268]|0.129 [0.126, 0.131]|0.179 [0.175, 0.183]|
|O1 / dense_h90_random|V0_u64|L2|8.0000|1.9665 [1.9641, 1.9685]|1.000 [1.000, 1.000]|1.314 [1.312, 1.318]|
|O1 / dense_h90_random|V1_noise|L1|4.0000|1.4946 [1.4916, 1.4999]|0.759 [0.759, 0.762]|1.001 [0.998, 1.005]|
|O1 / dense_h90_random|V1_u32|L1|4.0000|1.4979 [1.4925, 1.4998]|0.761 [0.759, 0.762]|1.000 [1.000, 1.000]|
|O1 / dense_h90_random|V2_bytes|L1|2.0195|0.7524 [0.7514, 0.7555]|0.383 [0.382, 0.384]|0.503 [0.503, 0.506]|
|O1 / dense_h90_random|V3_for|L1|1.3945|0.7380 [0.7331, 0.7393]|0.375 [0.373, 0.376]|0.493 [0.491, 0.496]|
|O1 / dense_h90_random|V4_delta|L1|0.5039|1.1228 [1.1202, 1.1266]|0.571 [0.570, 0.572]|0.750 [0.749, 0.755]|
|O1 / dense_h90_random|V5_bitmap_rank|L1|0.2750|0.0957 [0.0952, 0.0966]|0.049 [0.048, 0.049]|0.065 [0.064, 0.065]|
|O1 / dense_h90_sorted|V0_u64|L2|8.0000|0.2139 [0.2123, 0.2149]|1.000 [1.000, 1.000]|1.420 [1.407, 1.432]|
|O1 / dense_h90_sorted|V1_noise|L1|4.0000|0.1418 [0.1404, 0.1436]|0.666 [0.657, 0.680]|0.949 [0.941, 0.970]|
|O1 / dense_h90_sorted|V1_u32|L1|4.0000|0.1499 [0.1495, 0.1510]|0.704 [0.698, 0.711]|1.000 [1.000, 1.000]|
|O1 / dense_h90_sorted|V2_bytes|L1|2.0195|0.1166 [0.1163, 0.1179]|0.546 [0.540, 0.553]|0.777 [0.764, 0.786]|
|O1 / dense_h90_sorted|V3_for|L1|1.3945|0.1273 [0.1259, 0.1279]|0.595 [0.593, 0.597]|0.846 [0.838, 0.865]|
|O1 / dense_h90_sorted|V4_delta|L1|0.5039|0.3568 [0.3561, 0.3596]|1.680 [1.670, 1.691]|2.381 [2.364, 2.403]|
|O1 / dense_h90_sorted|V5_bitmap_rank|L1|0.2750|0.0295 [0.0278, 0.0308]|0.140 [0.132, 0.145]|0.201 [0.187, 0.206]|
|O1 / orders_h10_random|V0_u64|L2|8.0000|1.9098 [1.9056, 1.9117]|1.000 [1.000, 1.000]|1.321 [1.314, 1.325]|
|O1 / orders_h10_random|V1_noise|L1|4.0000|1.4452 [1.4441, 1.4484]|0.758 [0.757, 0.760]|1.001 [0.994, 1.004]|
|O1 / orders_h10_random|V1_u32|L1|4.0000|1.4468 [1.4432, 1.4518]|0.757 [0.755, 0.761]|1.000 [1.000, 1.000]|
|O1 / orders_h10_random|V2_bytes|L1|2.0196|0.6778 [0.6765, 0.6799]|0.355 [0.353, 0.357]|0.470 [0.467, 0.471]|
|O1 / orders_h10_random|V3_for|L1|1.5196|0.7025 [0.6983, 0.7059]|0.368 [0.367, 0.370]|0.484 [0.483, 0.488]|
|O1 / orders_h10_random|V4_delta|L1|0.8789|1.3204 [1.3166, 1.3234]|0.691 [0.689, 0.693]|0.912 [0.911, 0.916]|
|O1 / orders_h10_random|V5_bitmap_rank|L1|1.0000|0.0693 [0.0689, 0.0708]|0.037 [0.036, 0.037]|0.049 [0.048, 0.050]|
|O1 / orders_h10_sorted|V0_u64|L2|8.0000|0.1998 [0.1994, 0.2013]|1.000 [1.000, 1.000]|1.368 [1.352, 1.381]|
|O1 / orders_h10_sorted|V1_noise|L1|4.0000|0.1388 [0.1382, 0.1399]|0.698 [0.687, 0.700]|0.949 [0.945, 0.961]|
|O1 / orders_h10_sorted|V1_u32|L1|4.0000|0.1459 [0.1448, 0.1478]|0.731 [0.724, 0.740]|1.000 [1.000, 1.000]|
|O1 / orders_h10_sorted|V2_bytes|L1|2.0196|0.1146 [0.1130, 0.1165]|0.575 [0.561, 0.586]|0.783 [0.773, 0.803]|
|O1 / orders_h10_sorted|V3_for|L1|1.5196|0.1263 [0.1255, 0.1279]|0.632 [0.626, 0.641]|0.862 [0.854, 0.877]|
|O1 / orders_h10_sorted|V4_delta|L1|0.8789|0.3618 [0.3604, 0.3627]|1.802 [1.797, 1.826]|2.471 [2.464, 2.496]|
|O1 / orders_h10_sorted|V5_bitmap_rank|L1|1.0000|0.0297 [0.0292, 0.0309]|0.148 [0.147, 0.154]|0.205 [0.200, 0.217]|
|O1 / orders_h90_random|V0_u64|L2|8.0000|1.9099 [1.9076, 1.9192]|1.000 [1.000, 1.000]|1.324 [1.317, 1.337]|
|O1 / orders_h90_random|V1_noise|L1|4.0000|1.4427 [1.4386, 1.4452]|0.755 [0.751, 0.758]|1.000 [0.998, 1.004]|
|O1 / orders_h90_random|V1_u32|L1|4.0000|1.4431 [1.4385, 1.4473]|0.756 [0.748, 0.759]|1.000 [1.000, 1.000]|
|O1 / orders_h90_random|V2_bytes|L1|2.0196|0.6964 [0.6906, 0.7007]|0.365 [0.360, 0.366]|0.481 [0.476, 0.488]|
|O1 / orders_h90_random|V3_for|L1|1.5196|0.7058 [0.7033, 0.7087]|0.369 [0.367, 0.371]|0.488 [0.488, 0.490]|
|O1 / orders_h90_random|V4_delta|L1|0.8789|1.1460 [1.1443, 1.1503]|0.599 [0.598, 0.601]|0.797 [0.791, 0.801]|
|O1 / orders_h90_random|V5_bitmap_rank|L1|1.0000|0.1073 [0.1064, 0.1091]|0.056 [0.055, 0.057]|0.075 [0.074, 0.077]|
|O1 / orders_h90_sorted|V0_u64|L2|8.0000|0.2066 [0.2046, 0.2078]|1.000 [1.000, 1.000]|1.400 [1.380, 1.422]|
|O1 / orders_h90_sorted|V1_noise|L1|4.0000|0.1409 [0.1399, 0.1433]|0.687 [0.677, 0.694]|0.959 [0.946, 0.969]|
|O1 / orders_h90_sorted|V1_u32|L1|4.0000|0.1476 [0.1460, 0.1488]|0.714 [0.703, 0.725]|1.000 [1.000, 1.000]|
|O1 / orders_h90_sorted|V2_bytes|L1|2.0196|0.1143 [0.1138, 0.1168]|0.555 [0.553, 0.568]|0.779 [0.765, 0.800]|
|O1 / orders_h90_sorted|V3_for|L1|1.5196|0.1279 [0.1258, 0.1305]|0.620 [0.609, 0.629]|0.882 [0.849, 0.894]|
|O1 / orders_h90_sorted|V4_delta|L1|0.8789|0.3592 [0.3583, 0.3605]|1.749 [1.723, 1.766]|2.438 [2.413, 2.466]|
|O1 / orders_h90_sorted|V5_bitmap_rank|L1|1.0000|0.0321 [0.0315, 0.0343]|0.156 [0.152, 0.167]|0.222 [0.213, 0.237]|
|O1 / parts_h10_random|V0_u64|L2|8.0000|0.0803 [0.0789, 0.0820]|1.000 [1.000, 1.000]|0.797 [0.787, 0.814]|
|O1 / parts_h10_random|V1_noise|L1|4.0000|0.1004 [0.0995, 0.1026]|1.271 [1.228, 1.297]|1.003 [1.001, 1.035]|
|O1 / parts_h10_random|V1_u32|L1|4.0000|0.1005 [0.0994, 0.1011]|1.255 [1.229, 1.270]|1.000 [1.000, 1.000]|
|O1 / parts_h10_random|V2_bytes|L1|2.0199|0.0916 [0.0906, 0.0921]|1.135 [1.119, 1.148]|0.909 [0.898, 0.914]|
|O1 / parts_h10_random|V3_for|L1|1.2699|0.1098 [0.1090, 0.1103]|1.374 [1.339, 1.396]|1.094 [1.086, 1.109]|
|O1 / parts_h10_random|V4_delta|L1|0.3789|0.2788 [0.2771, 0.2803]|3.487 [3.379, 3.538]|2.787 [2.761, 2.802]|
|O1 / parts_h10_random|V5_bitmap_rank|L1|0.2500|0.0283 [0.0273, 0.0287]|0.350 [0.337, 0.367]|0.280 [0.268, 0.289]|
|O1 / parts_h10_sorted|V0_u64|L2|8.0000|0.0250 [0.0240, 0.0265]|1.000 [1.000, 1.000]|1.003 [0.956, 1.009]|
|O1 / parts_h10_sorted|V1_noise|L1|4.0000|0.0256 [0.0253, 0.0261]|1.000 [0.909, 1.100]|1.045 [0.926, 1.078]|
|O1 / parts_h10_sorted|V1_u32|L1|4.0000|0.0251 [0.0243, 0.0271]|0.997 [0.991, 1.046]|1.000 [1.000, 1.000]|
|O1 / parts_h10_sorted|V2_bytes|L1|2.0199|0.0249 [0.0230, 0.0253]|0.947 [0.905, 1.029]|0.953 [0.921, 1.005]|
|O1 / parts_h10_sorted|V3_for|L1|1.2699|0.0271 [0.0256, 0.0293]|1.061 [1.001, 1.157]|1.063 [1.007, 1.131]|
|O1 / parts_h10_sorted|V4_delta|L1|0.3789|0.0510 [0.0500, 0.0533]|2.070 [1.920, 2.182]|2.016 [1.926, 2.098]|
|O1 / parts_h10_sorted|V5_bitmap_rank|L1|0.2500|0.0191 [0.0182, 0.0196]|0.769 [0.691, 0.824]|0.755 [0.676, 0.799]|
|O1 / parts_h90_random|V0_u64|L2|8.0000|0.4979 [0.4975, 0.5010]|1.000 [1.000, 1.000]|1.005 [0.990, 1.015]|
|O1 / parts_h90_random|V1_noise|L1|4.0000|0.4987 [0.4971, 0.5003]|1.001 [0.989, 1.007]|1.004 [0.998, 1.010]|
|O1 / parts_h90_random|V1_u32|L1|4.0000|0.4953 [0.4943, 0.5019]|0.995 [0.986, 1.010]|1.000 [1.000, 1.000]|
|O1 / parts_h90_random|V2_bytes|L1|2.0199|0.4542 [0.4521, 0.4564]|0.911 [0.906, 0.913]|0.915 [0.901, 0.923]|
|O1 / parts_h90_random|V3_for|L1|1.2699|0.4386 [0.4378, 0.4404]|0.881 [0.876, 0.883]|0.883 [0.874, 0.893]|
|O1 / parts_h90_random|V4_delta|L1|0.3789|0.4439 [0.4432, 0.4463]|0.891 [0.884, 0.895]|0.895 [0.890, 0.903]|
|O1 / parts_h90_random|V5_bitmap_rank|L1|0.2500|0.0906 [0.0900, 0.0910]|0.181 [0.181, 0.183]|0.182 [0.180, 0.184]|
|O1 / parts_h90_sorted|V0_u64|L2|8.0000|0.0698 [0.0692, 0.0713]|1.000 [1.000, 1.000]|0.758 [0.750, 0.765]|
|O1 / parts_h90_sorted|V1_noise|L1|4.0000|0.0928 [0.0916, 0.0940]|1.323 [1.289, 1.330]|1.002 [0.995, 1.008]|
|O1 / parts_h90_sorted|V1_u32|L1|4.0000|0.0923 [0.0915, 0.0930]|1.320 [1.307, 1.334]|1.000 [1.000, 1.000]|
|O1 / parts_h90_sorted|V2_bytes|L1|2.0199|0.0844 [0.0833, 0.0854]|1.192 [1.179, 1.222]|0.919 [0.904, 0.932]|
|O1 / parts_h90_sorted|V3_for|L1|1.2699|0.1020 [0.1011, 0.1036]|1.466 [1.440, 1.475]|1.104 [1.091, 1.116]|
|O1 / parts_h90_sorted|V4_delta|L1|0.3789|0.2626 [0.2612, 0.2634]|3.739 [3.664, 3.766]|2.839 [2.810, 2.858]|
|O1 / parts_h90_sorted|V5_bitmap_rank|L1|0.2500|0.0228 [0.0208, 0.0236]|0.320 [0.301, 0.335]|0.244 [0.229, 0.253]|
|O1 / sparse_h10_random|V0_u64|L2|8.0000|1.9600 [1.9580, 1.9615]|1.000 [1.000, 1.000]|1.312 [1.310, 1.318]|
|O1 / sparse_h10_random|V1_noise|L1|4.0000|1.4976 [1.4885, 1.5024]|0.763 [0.760, 0.765]|1.001 [0.997, 1.007]|
|O1 / sparse_h10_random|V1_u32|L1|4.0000|1.4943 [1.4894, 1.4962]|0.762 [0.759, 0.764]|1.000 [1.000, 1.000]|
|O1 / sparse_h10_random|V2_bytes|L1|4.0195|1.0026 [1.0006, 1.0049]|0.511 [0.509, 0.513]|0.672 [0.669, 0.674]|
|O1 / sparse_h10_random|V3_for|L1|2.2610|0.7675 [0.7658, 0.7694]|0.391 [0.390, 0.392]|0.514 [0.513, 0.516]|
|O1 / sparse_h10_random|V4_delta|L1|1.4764|1.1603 [1.1582, 1.1633]|0.592 [0.591, 0.594]|0.777 [0.775, 0.780]|
|O1 / sparse_h10_sorted|V0_u64|L2|8.0000|0.2102 [0.2092, 0.2108]|1.000 [1.000, 1.000]|1.411 [1.395, 1.423]|
|O1 / sparse_h10_sorted|V1_noise|L1|4.0000|0.1423 [0.1410, 0.1428]|0.676 [0.671, 0.683]|0.958 [0.954, 0.968]|
|O1 / sparse_h10_sorted|V1_u32|L1|4.0000|0.1489 [0.1472, 0.1497]|0.709 [0.703, 0.717]|1.000 [1.000, 1.000]|
|O1 / sparse_h10_sorted|V2_bytes|L1|4.0195|0.1384 [0.1372, 0.1394]|0.660 [0.657, 0.666]|0.933 [0.925, 0.950]|
|O1 / sparse_h10_sorted|V3_for|L1|2.2610|0.1308 [0.1299, 0.1316]|0.623 [0.620, 0.630]|0.878 [0.869, 0.886]|
|O1 / sparse_h10_sorted|V4_delta|L1|1.4764|0.3614 [0.3605, 0.3636]|1.718 [1.713, 1.738]|2.435 [2.409, 2.449]|
|O1 / sparse_h90_random|V0_u64|L2|8.0000|1.9634 [1.9624, 1.9683]|1.000 [1.000, 1.000]|1.316 [1.312, 1.322]|
|O1 / sparse_h90_random|V1_noise|L1|4.0000|1.4977 [1.4958, 1.5024]|0.762 [0.760, 0.765]|1.003 [0.998, 1.006]|
|O1 / sparse_h90_random|V1_u32|L1|4.0000|1.4949 [1.4873, 1.4989]|0.760 [0.757, 0.762]|1.000 [1.000, 1.000]|
|O1 / sparse_h90_random|V2_bytes|L1|4.0195|1.0010 [0.9968, 1.0029]|0.509 [0.509, 0.511]|0.671 [0.669, 0.676]|
|O1 / sparse_h90_random|V3_for|L1|2.2610|0.7670 [0.7655, 0.7695]|0.390 [0.389, 0.392]|0.513 [0.511, 0.518]|
|O1 / sparse_h90_random|V4_delta|L1|1.4764|1.1309 [1.1268, 1.1321]|0.575 [0.574, 0.576]|0.755 [0.753, 0.760]|
|O1 / sparse_h90_sorted|V0_u64|L2|8.0000|0.2141 [0.2125, 0.2151]|1.000 [1.000, 1.000]|1.434 [1.415, 1.454]|
|O1 / sparse_h90_sorted|V1_noise|L1|4.0000|0.1434 [0.1420, 0.1444]|0.667 [0.663, 0.671]|0.966 [0.947, 0.989]|
|O1 / sparse_h90_sorted|V1_u32|L1|4.0000|0.1494 [0.1479, 0.1500]|0.697 [0.688, 0.707]|1.000 [1.000, 1.000]|
|O1 / sparse_h90_sorted|V2_bytes|L1|4.0195|0.1383 [0.1376, 0.1397]|0.649 [0.646, 0.652]|0.933 [0.917, 0.964]|
|O1 / sparse_h90_sorted|V3_for|L1|2.2610|0.1300 [0.1292, 0.1306]|0.608 [0.605, 0.611]|0.865 [0.863, 0.883]|
|O1 / sparse_h90_sorted|V4_delta|L1|1.4764|0.3589 [0.3580, 0.3600]|1.679 [1.669, 1.690]|2.409 [2.393, 2.436]|
|O2 / sf10_order_part|V0_u64|L2|27.9963|2.4892 [2.4835, 2.4911]|1.000 [1.000, 1.000]|1.309 [1.304, 1.315]|
|O2 / sf10_order_part|V1_noise|L1|23.9963|1.9073 [1.9032, 1.9082]|0.767 [0.767, 0.769]|1.004 [1.003, 1.006]|
|O2 / sf10_order_part|V1_offsets_delta_payload_for|L1|15.2047|2.0343 [2.0216, 2.0378]|0.817 [0.814, 0.818]|1.069 [1.063, 1.072]|
|O2 / sf10_order_part|V1_offsets_delta_payload_u32|L1|20.6252|1.9096 [1.9038, 1.9123]|0.767 [0.766, 0.769]|1.006 [1.001, 1.009]|
|O2 / sf10_order_part|V1_offsets_for_payload_for|L1|16.1549|1.9519 [1.9486, 1.9536]|0.784 [0.783, 0.785]|1.027 [1.023, 1.030]|
|O2 / sf10_order_part|V1_offsets_for_payload_u32|L1|21.5754|1.8830 [1.8737, 1.8900]|0.756 [0.754, 0.760]|0.991 [0.986, 0.999]|
|O2 / sf10_order_part|V1_offsets_u32_payload_for|L1|18.5758|1.9517 [1.9475, 1.9562]|0.784 [0.783, 0.787]|1.029 [1.026, 1.032]|
|O2 / sf10_order_part|V1_u32|L1|23.9963|1.8994 [1.8939, 1.9025]|0.764 [0.761, 0.767]|1.000 [1.000, 1.000]|
|O2 / sf10_order_part|V2_bytes|L1|22.0159|0.9559 [0.9522, 0.9577]|0.384 [0.383, 0.385]|0.503 [0.501, 0.504]|
|O2 / sf10_order_part|V3_for|L1|21.5159|0.9153 [0.9144, 0.9192]|0.369 [0.368, 0.370]|0.482 [0.481, 0.487]|
|O2 / sf10_order_part|V3_offsets_delta_payload_for|L1|12.7243|1.2642 [1.2499, 1.2842]|0.506 [0.501, 0.517]|0.672 [0.659, 0.676]|
|O2 / sf10_order_part|V3_offsets_for_payload_for|L1|13.6745|1.0644 [1.0603, 1.0668]|0.428 [0.427, 0.430]|0.561 [0.558, 0.562]|
|O2 / sf10_order_part|V4_delta|L1|20.8752|1.3820 [1.3767, 1.3836]|0.555 [0.554, 0.556]|0.728 [0.724, 0.730]|
|O2 / sf10_order_part|V5_bitmap_rank|L1|20.9963|0.7431 [0.7423, 0.7437]|0.299 [0.298, 0.299]|0.391 [0.390, 0.393]|
|O3 / disjoint|V0_u64|L2|16.0000|0.4871 [0.4832, 0.4950]|1.000 [1.000, 1.000]|2.029 [1.995, 2.046]|
|O3 / disjoint|V1_noise|L2|8.0000|0.2428 [0.2421, 0.2433]|0.500 [0.491, 0.502]|1.003 [0.991, 1.007]|
|O3 / disjoint|V1_u32|L2|8.0000|0.2428 [0.2415, 0.2452]|0.493 [0.489, 0.501]|1.000 [1.000, 1.000]|
|O3 / disjoint|V2_bytes|L0+L1|4.0391|0.0901 [0.0881, 0.0911]|0.184 [0.180, 0.189]|0.371 [0.364, 0.378]|
|O3 / disjoint|V2_bytes_decode_merge_reencode|L2|4.0391|0.8884 [0.8870, 0.8942]|1.831 [1.802, 1.854]|3.665 [3.632, 3.700]|
|O3 / disjoint|V3_for|L0+L1|2.7891|0.0773 [0.0755, 0.0809]|0.160 [0.153, 0.170]|0.319 [0.307, 0.329]|
|O3 / disjoint|V3_for_decode_merge_reencode|L2|2.7891|0.8740 [0.8718, 0.8765]|1.794 [1.753, 1.813]|3.596 [3.556, 3.614]|
|O3 / disjoint|V4_delta|L0+L1|1.2578|0.3100 [0.3080, 0.3127]|0.640 [0.627, 0.643]|1.277 [1.265, 1.286]|
|O3 / disjoint|V4_delta_decode_merge_reencode|L2|1.2578|1.8518 [1.8446, 1.8561]|3.790 [3.734, 3.842]|7.587 [7.572, 7.698]|
|O3 / interleaved|V0_u64|L2|16.0000|0.3090 [0.3052, 0.3138]|1.000 [1.000, 1.000]|2.132 [2.055, 2.176]|
|O3 / interleaved|V1_noise|L2|8.0000|0.1444 [0.1375, 0.1488]|0.465 [0.447, 0.479]|0.986 [0.968, 0.994]|
|O3 / interleaved|V1_u32|L2|8.0000|0.1446 [0.1420, 0.1532]|0.469 [0.460, 0.487]|1.000 [1.000, 1.000]|
|O3 / interleaved|V2_bytes|L0+L1|4.0391|0.8972 [0.8888, 0.9027]|2.906 [2.880, 2.917]|6.163 [5.943, 6.320]|
|O3 / interleaved|V2_bytes_decode_merge_reencode|L2|4.0391|0.7384 [0.7324, 0.7413]|2.394 [2.368, 2.424]|5.074 [4.844, 5.248]|
|O3 / interleaved|V3_for|L0+L1|2.6641|0.8232 [0.8140, 0.8314]|2.675 [2.646, 2.688]|5.648 [5.444, 5.841]|
|O3 / interleaved|V3_for_decode_merge_reencode|L2|2.6641|0.7120 [0.7084, 0.7203]|2.309 [2.287, 2.317]|4.900 [4.699, 5.000]|
|O3 / interleaved|V4_delta|L0+L1|1.2578|35.7586 [35.7481, 35.7695]|115.694 [113.959, 117.157]|247.184 [233.515, 251.829]|
|O3 / interleaved|V4_delta_decode_merge_reencode|L2|1.2578|1.6827 [1.6778, 1.6844]|5.448 [5.374, 5.483]|11.617 [10.978, 11.795]|
|O4 / unsorted_25pct_duplicates|V0_u64|L2|14.0000|3.0060 [3.0029, 3.0096]|1.000 [1.000, 1.000]|4.535 [4.508, 4.555]|
|O4 / unsorted_25pct_duplicates|V1_noise|L2|7.0000|0.6814 [0.6802, 0.6828]|0.227 [0.226, 0.227]|1.028 [1.019, 1.035]|
|O4 / unsorted_25pct_duplicates|V1_u32|L2|7.0000|0.6615 [0.6608, 0.6686]|0.221 [0.220, 0.222]|1.000 [1.000, 1.000]|
|O4 / unsorted_25pct_duplicates|V1_u32_significant_bits|L2|7.0000|0.5570 [0.5547, 0.5601]|0.185 [0.185, 0.186]|0.831 [0.822, 0.842]|
|O4_seal / sorted_unique|V0_u64|L2|16.0000|0.2313 [0.2282, 0.2326]|1.000 [1.000, 1.000]|3.062 [2.987, 3.120]|
|O4_seal / sorted_unique|V1_noise|L2|8.0000|0.0871 [0.0811, 0.0880]|0.370 [0.353, 0.381]|1.120 [1.074, 1.167]|
|O4_seal / sorted_unique|V1_u32|L2|8.0000|0.0756 [0.0743, 0.0771]|0.327 [0.321, 0.335]|1.000 [1.000, 1.000]|
|O4_seal / sorted_unique|V2_bytes|L1|6.0195|0.0731 [0.0704, 0.0758]|0.312 [0.304, 0.327]|0.983 [0.949, 1.018]|
|O4_seal / sorted_unique|V3_for|L1|5.5195|0.0500 [0.0466, 0.0519]|0.215 [0.203, 0.224]|0.676 [0.623, 0.751]|
|O5 / run16M_batch1000000|V0_u64|L2|8.0000|0.0442 [0.0426, 0.0484]|1.000 [1.000, 1.000]|0.859 [0.846, 0.901]|
|O5 / run16M_batch1000000|V1_noise|L1|4.0000|0.0515 [0.0501, 0.0552]|1.166 [1.132, 1.181]|1.001 [0.950, 1.046]|
|O5 / run16M_batch1000000|V1_u32|L1|4.0000|0.0509 [0.0494, 0.0547]|1.164 [1.110, 1.182]|1.000 [1.000, 1.000]|
|O5 / run16M_batch1000000|V2_bytes|L1|2.0195|0.0479 [0.0456, 0.0490]|1.068 [0.971, 1.148]|0.932 [0.896, 0.976]|
|O5 / run16M_batch1000000|V3_for|L1|1.3945|0.0520 [0.0505, 0.0538]|1.136 [1.075, 1.242]|1.021 [0.967, 1.064]|
|O5 / run16M_batch1000000|V4_delta|L1|0.5039|0.0914 [0.0909, 0.0934]|2.058 [1.948, 2.160]|1.776 [1.741, 1.890]|
|O5 / run16M_batch1000000|V5_bitmap_rank|L1|0.5000|0.0325 [0.0303, 0.0359]|0.709 [0.675, 0.785]|0.654 [0.573, 0.740]|
|O5 / run16M_batch16000000|V0_u64|L2|8.0000|0.3791 [0.3772, 0.3811]|1.000 [1.000, 1.000]|0.841 [0.834, 0.846]|
|O5 / run16M_batch16000000|V1_noise|L1|4.0000|0.4412 [0.4369, 0.4429]|1.163 [1.155, 1.173]|0.977 [0.974, 0.988]|
|O5 / run16M_batch16000000|V1_u32|L1|4.0000|0.4502 [0.4477, 0.4532]|1.189 [1.181, 1.198]|1.000 [1.000, 1.000]|
|O5 / run16M_batch16000000|V2_bytes|L1|2.0195|0.3890 [0.3863, 0.3927]|1.023 [1.019, 1.038]|0.865 [0.862, 0.872]|
|O5 / run16M_batch16000000|V3_for|L1|1.3945|0.4530 [0.4517, 0.4560]|1.196 [1.190, 1.199]|1.008 [1.001, 1.013]|
|O5 / run16M_batch16000000|V4_delta|L1|0.5039|1.0591 [1.0558, 1.0610]|2.800 [2.779, 2.811]|2.346 [2.341, 2.371]|
|O5 / run16M_batch16000000|V5_bitmap_rank|L1|0.5000|0.0855 [0.0831, 0.0872]|0.227 [0.218, 0.231]|0.191 [0.188, 0.195]|

## Bit-packed versus byte-aligned

| Operation / case | FOR / bytes paired time ratio [95% CI] |
|---|---:|
|O1 / clustered_h10_random|0.306 [0.303, 0.308]|
|O1 / clustered_h10_sorted|0.757 [0.739, 0.771]|
|O1 / clustered_h90_random|0.911 [0.903, 0.923]|
|O1 / clustered_h90_sorted|1.072 [1.052, 1.095]|
|O1 / customers_h10_random|0.975 [0.969, 0.986]|
|O1 / customers_h10_sorted|1.200 [1.175, 1.219]|
|O1 / customers_h90_random|0.966 [0.959, 0.969]|
|O1 / customers_h90_sorted|1.200 [1.183, 1.266]|
|O1 / dense_h10_random|0.978 [0.965, 0.983]|
|O1 / dense_h10_sorted|1.096 [1.092, 1.099]|
|O1 / dense_h90_random|0.975 [0.969, 0.981]|
|O1 / dense_h90_sorted|1.089 [1.072, 1.104]|
|O1 / orders_h10_random|1.034 [1.028, 1.044]|
|O1 / orders_h10_sorted|1.098 [1.084, 1.116]|
|O1 / orders_h90_random|1.017 [1.008, 1.022]|
|O1 / orders_h90_sorted|1.109 [1.095, 1.134]|
|O1 / parts_h10_random|1.196 [1.185, 1.220]|
|O1 / parts_h10_sorted|1.100 [1.065, 1.180]|
|O1 / parts_h90_random|0.970 [0.960, 0.976]|
|O1 / parts_h90_sorted|1.204 [1.183, 1.239]|
|O1 / sparse_h10_random|0.765 [0.763, 0.768]|
|O1 / sparse_h10_sorted|0.945 [0.936, 0.954]|
|O1 / sparse_h90_random|0.767 [0.765, 0.770]|
|O1 / sparse_h90_sorted|0.940 [0.935, 0.943]|
|O2 / sf10_order_part|0.958 [0.956, 0.962]|
|O3 / disjoint|0.863 [0.821, 0.909]|
|O3 / interleaved|0.920 [0.913, 0.925]|
|O4_seal / sorted_unique|0.663 [0.615, 0.786]|
|O5 / run16M_batch1000000|1.105 [1.073, 1.137]|
|O5 / run16M_batch16000000|1.161 [1.158, 1.172]|

## Noise floor and contamination

Observed tenant waits: 0; discarded contaminated samples: 0. Before/after sampling cannot detect a foreign process that starts and exits entirely inside a sample. Clock/load snapshots and all tenant snapshots remain in raw JSONL.

Identical `V1_noise` vs `V1_u32` paired ratios:

- E_b_semijoin / sf10_s15_for1024: 0.995 [0.987, 1.000].
- E_b_semijoin / sf10_s15_plain_u32: 0.998 [0.996, 0.999].
- E_b_semijoin / sf10_s3_for1024: 1.002 [0.998, 1.006].
- E_b_semijoin / sf10_s3_plain_u32: 1.002 [0.998, 1.006].
- E_b_semijoin / sf10_s50_for1024: 1.001 [0.997, 1.005].
- E_b_semijoin / sf10_s50_plain_u32: 0.998 [0.997, 1.001].
- E_b_set_build / sf10_s15: 0.936 [0.909, 0.965].
- E_b_set_build / sf10_s3: 1.004 [0.969, 1.031].
- E_b_set_build / sf10_s50: 1.018 [0.997, 1.031].
- E_b_set_intersection / sf10_s15: 0.973 [0.925, 0.997].
- E_b_set_intersection / sf10_s3: 0.921 [0.817, 0.979].
- E_b_set_intersection / sf10_s50: 0.761 [0.747, 0.772].
- O1 / clustered_h10_random: 0.999 [0.996, 1.002].
- O1 / clustered_h10_sorted: 0.971 [0.947, 1.022].
- O1 / clustered_h90_random: 1.000 [0.993, 1.010].
- O1 / clustered_h90_sorted: 0.945 [0.932, 0.988].
- O1 / customers_h10_random: 1.008 [1.004, 1.012].
- O1 / customers_h10_sorted: 0.990 [0.937, 1.014].
- O1 / customers_h90_random: 1.002 [0.996, 1.005].
- O1 / customers_h90_sorted: 0.994 [0.984, 1.029].
- O1 / dense_h10_random: 1.001 [0.997, 1.004].
- O1 / dense_h10_sorted: 0.936 [0.932, 0.949].
- O1 / dense_h90_random: 1.001 [0.998, 1.005].
- O1 / dense_h90_sorted: 0.949 [0.941, 0.970].
- O1 / orders_h10_random: 1.001 [0.994, 1.004].
- O1 / orders_h10_sorted: 0.949 [0.945, 0.961].
- O1 / orders_h90_random: 1.000 [0.998, 1.004].
- O1 / orders_h90_sorted: 0.959 [0.946, 0.969].
- O1 / parts_h10_random: 1.003 [1.001, 1.035].
- O1 / parts_h10_sorted: 1.045 [0.926, 1.078].
- O1 / parts_h90_random: 1.004 [0.998, 1.010].
- O1 / parts_h90_sorted: 1.002 [0.995, 1.008].
- O1 / sparse_h10_random: 1.001 [0.997, 1.007].
- O1 / sparse_h10_sorted: 0.958 [0.954, 0.968].
- O1 / sparse_h90_random: 1.003 [0.998, 1.006].
- O1 / sparse_h90_sorted: 0.966 [0.947, 0.989].
- O2 / sf10_order_part: 1.004 [1.003, 1.006].
- O3 / disjoint: 1.003 [0.991, 1.007].
- O3 / interleaved: 0.986 [0.968, 0.994].
- O4 / unsorted_25pct_duplicates: 1.028 [1.019, 1.035].
- O4_seal / sorted_unique: 1.120 [1.074, 1.167].
- O5 / run16M_batch1000000: 1.001 [0.950, 1.046].
- O5 / run16M_batch16000000: 0.977 [0.974, 0.988].

Rotation changes the first variant but preserves cyclic adjacency. An identical duplicate frequently follows the original u32 variant; a reproducible difference can therefore be a cache/order effect. These are within-schedule bootstrap intervals, not bias-corrected cross-machine guarantees. Near-ties must be read against the **same-case** duplicate, not merely whether a CI excludes 1.

Initial O3/O4 raw files are preserved with `initial-` prefixes. These three jobs were remeasured after adding preparation wall-time records and excluding the unused legacy main from library inclusion; timed kernels were unchanged. Tables use the complete later runs, not selected samples.

## Applicability and limits

- Bitmap membership/locate is included only at span <= 32 × cardinality. This is a storage policy, not an assertion of a performance crossover.
- Dense real sets with no substantial holes necessarily use outside-span misses; other O1 cases sample misses uniformly from holes inside the span. Hit rates are exact, probe sequences identical across layouts.
- O5 uses sorted unique even/odd admission batches with exactly 50% overlap against the 16M even-key run. Results do not establish arbitrary Datalog distributions.
- No hardware traffic counters, cache-flush experiments or implementation tuning sweep were performed. Interpret implementation-specific kernel costs rather than universal codec costs.

- {"case": "sparse_h10_random", "experiment": "O1", "reason": "span exceeds 32*n", "type": "not_applicable", "variant": "V5_bitmap_rank"}
- {"case": "sparse_h10_sorted", "experiment": "O1", "reason": "span exceeds 32*n", "type": "not_applicable", "variant": "V5_bitmap_rank"}
- {"case": "sparse_h90_random", "experiment": "O1", "reason": "span exceeds 32*n", "type": "not_applicable", "variant": "V5_bitmap_rank"}
- {"case": "sparse_h90_sorted", "experiment": "O1", "reason": "span exceeds 32*n", "type": "not_applicable", "variant": "V5_bitmap_rank"}
- {"case": "clustered_h10_random", "experiment": "O1", "reason": "span exceeds 32*n", "type": "not_applicable", "variant": "V5_bitmap_rank"}
- {"case": "clustered_h10_sorted", "experiment": "O1", "reason": "span exceeds 32*n", "type": "not_applicable", "variant": "V5_bitmap_rank"}
- {"case": "clustered_h90_random", "experiment": "O1", "reason": "span exceeds 32*n", "type": "not_applicable", "variant": "V5_bitmap_rank"}
- {"case": "clustered_h90_sorted", "experiment": "O1", "reason": "span exceeds 32*n", "type": "not_applicable", "variant": "V5_bitmap_rank"}

## Preparation (not query time)

| Case | load/encode/oracle wall ms |
|---|---:|
|orders_h10_random|3015.33|
|orders_h10_sorted|1881.85|
|orders_h90_random|1516.82|
|orders_h90_sorted|1020.10|
|parts_h10_random|3615.81|
|parts_h10_sorted|3785.63|
|parts_h90_random|3971.50|
|parts_h90_sorted|3789.54|
|customers_h10_random|3089.90|
|customers_h10_sorted|2264.41|
|customers_h90_random|1439.97|
|customers_h90_sorted|1279.21|
|dense_h10_random|9691.32|
|dense_h10_sorted|9106.29|
|dense_h90_random|2381.17|
|dense_h90_sorted|1891.96|
|sparse_h10_random|5253.59|
|sparse_h10_sorted|4727.23|
|sparse_h90_random|4622.42|
|sparse_h90_sorted|4092.40|
|clustered_h10_random|2044.99|
|clustered_h10_sorted|1580.42|
|clustered_h90_random|1635.04|
|clustered_h90_sorted|1099.13|
|sf10_order_part|3628.39|
|run16M_batch1000000|365.88|
|run16M_batch16000000|762.11|
|O3_disjoint|398.28|
|O3_interleaved|412.76|
|O4_unsorted_25pct_duplicates|1336.25|
|O4_seal_sorted_unique|10.58|

| Operation / case | load ms | CPU selection/oracle ms | encoding/upload ms |
|---|---:|---:|---:|
|E_b_semijoin / sf10_s15_for1024|277.05|604.77|274.00|
|E_b_semijoin / sf10_s15_plain_u32|277.05|604.77|274.00|
|E_b_semijoin / sf10_s3_for1024|607.91|642.13|221.65|
|E_b_semijoin / sf10_s3_plain_u32|607.91|642.13|221.65|
|E_b_semijoin / sf10_s50_for1024|276.01|735.46|305.77|
|E_b_semijoin / sf10_s50_plain_u32|276.01|735.46|305.77|
|E_b_set_build / sf10_s15|118.66|694.31|41.77|
|E_b_set_build / sf10_s3|119.00|606.65|8.74|
|E_b_set_build / sf10_s50|119.69|904.98|155.07|
|E_b_set_intersection / sf10_s15|118.66|694.31|41.77|
|E_b_set_intersection / sf10_s3|119.00|606.65|8.74|
|E_b_set_intersection / sf10_s50|119.69|904.98|155.07|

## Exact commands

```sh
python3 experiments/storage-bench/gpu/compressed_run.py --build
flock -s /tmp/mini_linq-timing.lock /opt/nvidia/hpc_sdk/Linux_x86_64/25.9/cuda/13.0/bin/nvcc -O3 -std=c++17 -arch=sm_89 /data/mini_linq-wt/compression-exp/experiments/storage-bench/gpu/compressed_ops.cu -o /data/mini_linq-wt/compression-exp/experiments/storage-bench/gpu/compressed_ops
flock /tmp/mini_linq-gpu.lock nice -n 19 /data/mini_linq-wt/compression-exp/experiments/storage-bench/gpu/compressed_ops lookup orders 10 random
flock /tmp/mini_linq-gpu.lock nice -n 19 /data/mini_linq-wt/compression-exp/experiments/storage-bench/gpu/compressed_ops lookup orders 10 sorted
flock /tmp/mini_linq-gpu.lock nice -n 19 /data/mini_linq-wt/compression-exp/experiments/storage-bench/gpu/compressed_ops lookup orders 90 random
flock /tmp/mini_linq-gpu.lock nice -n 19 /data/mini_linq-wt/compression-exp/experiments/storage-bench/gpu/compressed_ops lookup orders 90 sorted
flock /tmp/mini_linq-gpu.lock nice -n 19 /data/mini_linq-wt/compression-exp/experiments/storage-bench/gpu/compressed_ops lookup parts 10 random
flock /tmp/mini_linq-gpu.lock nice -n 19 /data/mini_linq-wt/compression-exp/experiments/storage-bench/gpu/compressed_ops lookup parts 10 sorted
flock /tmp/mini_linq-gpu.lock nice -n 19 /data/mini_linq-wt/compression-exp/experiments/storage-bench/gpu/compressed_ops lookup parts 90 random
flock /tmp/mini_linq-gpu.lock nice -n 19 /data/mini_linq-wt/compression-exp/experiments/storage-bench/gpu/compressed_ops lookup parts 90 sorted
flock /tmp/mini_linq-gpu.lock nice -n 19 /data/mini_linq-wt/compression-exp/experiments/storage-bench/gpu/compressed_ops lookup customers 10 random
flock /tmp/mini_linq-gpu.lock nice -n 19 /data/mini_linq-wt/compression-exp/experiments/storage-bench/gpu/compressed_ops lookup customers 10 sorted
flock /tmp/mini_linq-gpu.lock nice -n 19 /data/mini_linq-wt/compression-exp/experiments/storage-bench/gpu/compressed_ops lookup customers 90 random
flock /tmp/mini_linq-gpu.lock nice -n 19 /data/mini_linq-wt/compression-exp/experiments/storage-bench/gpu/compressed_ops lookup customers 90 sorted
flock /tmp/mini_linq-gpu.lock nice -n 19 /data/mini_linq-wt/compression-exp/experiments/storage-bench/gpu/compressed_ops lookup dense 10 random
flock /tmp/mini_linq-gpu.lock nice -n 19 /data/mini_linq-wt/compression-exp/experiments/storage-bench/gpu/compressed_ops lookup dense 10 sorted
flock /tmp/mini_linq-gpu.lock nice -n 19 /data/mini_linq-wt/compression-exp/experiments/storage-bench/gpu/compressed_ops lookup dense 90 random
flock /tmp/mini_linq-gpu.lock nice -n 19 /data/mini_linq-wt/compression-exp/experiments/storage-bench/gpu/compressed_ops lookup dense 90 sorted
flock /tmp/mini_linq-gpu.lock nice -n 19 /data/mini_linq-wt/compression-exp/experiments/storage-bench/gpu/compressed_ops lookup sparse 10 random
flock /tmp/mini_linq-gpu.lock nice -n 19 /data/mini_linq-wt/compression-exp/experiments/storage-bench/gpu/compressed_ops lookup sparse 10 sorted
flock /tmp/mini_linq-gpu.lock nice -n 19 /data/mini_linq-wt/compression-exp/experiments/storage-bench/gpu/compressed_ops lookup sparse 90 random
flock /tmp/mini_linq-gpu.lock nice -n 19 /data/mini_linq-wt/compression-exp/experiments/storage-bench/gpu/compressed_ops lookup sparse 90 sorted
flock /tmp/mini_linq-gpu.lock nice -n 19 /data/mini_linq-wt/compression-exp/experiments/storage-bench/gpu/compressed_ops lookup clustered 10 random
flock /tmp/mini_linq-gpu.lock nice -n 19 /data/mini_linq-wt/compression-exp/experiments/storage-bench/gpu/compressed_ops lookup clustered 10 sorted
flock /tmp/mini_linq-gpu.lock nice -n 19 /data/mini_linq-wt/compression-exp/experiments/storage-bench/gpu/compressed_ops lookup clustered 90 random
flock /tmp/mini_linq-gpu.lock nice -n 19 /data/mini_linq-wt/compression-exp/experiments/storage-bench/gpu/compressed_ops lookup clustered 90 sorted
flock /tmp/mini_linq-gpu.lock nice -n 19 /data/mini_linq-wt/compression-exp/experiments/storage-bench/gpu/compressed_ops expand
flock /tmp/mini_linq-gpu.lock nice -n 19 /data/mini_linq-wt/compression-exp/experiments/storage-bench/gpu/compressed_ops admission 1000000
flock /tmp/mini_linq-gpu.lock nice -n 19 /data/mini_linq-wt/compression-exp/experiments/storage-bench/gpu/compressed_ops admission 16000000
flock /tmp/mini_linq-gpu.lock nice -n 19 /data/mini_linq-wt/compression-exp/experiments/storage-bench/gpu/compressed_ops merge disjoint
flock /tmp/mini_linq-gpu.lock nice -n 19 /data/mini_linq-wt/compression-exp/experiments/storage-bench/gpu/compressed_ops merge interleaved
flock /tmp/mini_linq-gpu.lock nice -n 19 /data/mini_linq-wt/compression-exp/experiments/storage-bench/gpu/compressed_ops sort
flock /tmp/mini_linq-gpu.lock nice -n 19 /data/mini_linq-wt/compression-exp/experiments/storage-bench/gpu/compressed_ops semijoin 3
flock /tmp/mini_linq-gpu.lock nice -n 19 /data/mini_linq-wt/compression-exp/experiments/storage-bench/gpu/compressed_ops semijoin 15
flock /tmp/mini_linq-gpu.lock nice -n 19 /data/mini_linq-wt/compression-exp/experiments/storage-bench/gpu/compressed_ops semijoin 50
flock /tmp/mini_linq-gpu.lock nice -n 19 /data/mini_linq-wt/compression-exp/experiments/storage-bench/gpu/compressed_ops setops 3
flock /tmp/mini_linq-gpu.lock nice -n 19 /data/mini_linq-wt/compression-exp/experiments/storage-bench/gpu/compressed_ops setops 15
flock /tmp/mini_linq-gpu.lock nice -n 19 /data/mini_linq-wt/compression-exp/experiments/storage-bench/gpu/compressed_ops setops 50
python3 experiments/storage-bench/gpu/compressed_run.py --summarize
```

## Completeness

All campaign jobs completed; all accepted samples passed their exactness checks.
