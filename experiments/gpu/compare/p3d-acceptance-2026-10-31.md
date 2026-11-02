# P3d final acceptance timing: M0 baseline 4112249a vs codex/p3d-final 085f9a0f

> Point-in-time measurement: `4112249a` versus `085f9a0f`, 2026-10-31 (report preserved by `1031adf6`, 2026-10-31). **Superseded by:** [PLANNING-REDESIGN](../../../doc/PLANNING-REDESIGN.md) P4/P6b progress for subsequent CPU-storage/GPU-DOOP evidence; this exact acceptance matrix has not been re-measured since.

Interleaved pairs (baseline then new, fresh process each). Ratio = median(new)/median(base); CI = percentile bootstrap of the median of resampled paired ratios (10000 resamples, seed 20261030). **↑** = CI entirely above 1.0 (regression).

## 1. CPU TPC-H SF1

### CPU TPC-H SF1 default, single-query timing-worker (median_ms of 5 in-process runs)

| item | n | median base ms | median new ms | new/base | 95% CI | exact |
|---|---:|---:|---:|---:|---|---|
| retail_h02 | 5 | 52.648 | 50.420 | 0.958 | [0.945, 1.009] | answers identical |
| retail_h12 **↑** | 5 | 112.158 | 144.167 | 1.285 | [1.277, 1.286] | answers identical |
| retail_h13 | 5 | 177.583 | 177.737 | 1.001 | [0.989, 1.009] | answers identical |

## 2. CPU DOOP

### DOOP xalan compute seconds

| item | n | median base s | median new s | new/base | 95% CI | exact |
|---|---:|---:|---:|---:|---|---|
| mini | 5 | 9.569 | 8.893 | 0.929 | [0.925, 0.967] | sizes identical |
| mini-keyed | 5 | 12.634 | 10.817 | 0.856 | [0.835, 0.866] | sizes identical |
| mini-par | 5 | 2.356 | 2.360 | 1.002 | [0.958, 1.012] | sizes identical |
| mini-par-keyed | 5 | 2.565 | 2.637 | 1.028 | [0.982, 1.043] | sizes identical |

### DOOP batik compute seconds

| item | n | median base s | median new s | new/base | 95% CI | exact |
|---|---:|---:|---:|---:|---|---|
| mini-par **↑** | 5 | 7.787 | 8.538 | 1.096 | [1.089, 1.125] | sizes identical |
| mini-par-keyed **↑** | 5 | 8.393 | 9.885 | 1.178 | [1.125, 1.222] | sizes identical |

## 3. GPU

### GPU TPC-H SF1 advised (median_ms.total)

| item | n | median base ms | median new ms | new/base | 95% CI | exact |
|---|---:|---:|---:|---:|---|---|
| h06@sf1/advised | 5 | 0.435 | 0.109 | 0.250 | [0.249, 0.252] | exact conf=passed |
| h11@sf1/advised | 5 | 2.352 | 2.223 | 0.945 | [0.936, 0.952] | exact conf=passed |
| h15@sf1/advised | 5 | 1.135 | 0.987 | 0.870 | [0.861, 0.878] | exact conf=passed |
| h19@sf1/advised | 5 | 1.128 | 1.078 | 0.955 | [0.947, 0.959] | exact conf=passed |

### GPU srd-doop-gpu xalan fixpoint seconds

| item | n | median base s | median new s | new/base | 95% CI | exact |
|---|---:|---:|---:|---:|---|---|
| fixpoint | 5 | 1.485 | 1.440 | 0.970 | [0.963, 0.979] | 43 relation counts identical across all runs; export tuple compare base vs new: 43 files, all sorted-tuple digests identical |

### GPU srd-doop-gpu batik fixpoint seconds

| item | n | median base s | median new s | new/base | 95% CI | exact |
|---|---:|---:|---:|---:|---|---|
| fixpoint | 5 | 4.363 | 4.298 | 0.985 | [0.982, 0.985] | 43 relation counts identical across all runs; export tuple compare base vs new: 43 files, all sorted-tuple digests identical |

## CI entirely above 1.0

- CPU TPC-H worker: retail_h12 
- CPU DOOP batik: mini-par 
- CPU DOOP batik: mini-par-keyed 
