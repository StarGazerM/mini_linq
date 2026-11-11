#!/usr/bin/env python3
"""Reproducible E-a/E-b campaign; raw records are never filtered on disk."""
import argparse
import datetime
import json
from pathlib import Path
import random
import shlex
import statistics
import subprocess
import time

HERE = Path(__file__).resolve().parent
RESULTS = HERE.parent / 'results'
STEM = 'gpu-2026-11-11-compressed-ops'
RAW = RESULTS / (STEM + '-raw')
NVCC = '/opt/nvidia/hpc_sdk/Linux_x86_64/25.9/cuda/13.0/bin/nvcc'
BINARY = HERE / 'compressed_ops'
BUILD = ['flock', '-s', '/tmp/mini_linq-timing.lock', NVCC, '-O3', '-std=c++17', '-arch=sm_89', str(HERE / 'compressed_ops.cu'), '-o', str(BINARY)]
JOBS = [('lookup', case, str(pct), order) for case in ('orders', 'parts', 'customers', 'dense', 'sparse', 'clustered') for pct in (10, 90) for order in ('random', 'sorted')]
JOBS += [('expand',), ('admission', '1000000'), ('admission', '16000000'), ('merge', 'disjoint'), ('merge', 'interleaved'), ('sort',)]
JOBS += [(op, str(pct)) for op in ('semijoin', 'setops') for pct in (3, 15, 50)]

def validate_manifest():
    root = Path('/data/tpch-gpu/sf10')
    m = json.loads((root / 'manifest.json').read_text())
    wanted = {'lineitem': ('l_orderkey', 'l_partkey'), 'orders': ('o_orderkey', 'o_custkey', 'o_orderdate')}
    columns = []
    for table, names in wanted.items():
        rel = m['relations'][table]
        for name in names:
            c = next(c for c in rel['columns'] if c['name'] == name)
            expected_type = 'i32' if name == 'o_orderdate' else 'i64'
            assert c['type'] == expected_type and c['file'] == f'{table}.{name}.bin', c
            path = root / c['file']
            assert path.stat().st_size == rel['rows'] * (4 if c['type'] == 'i32' else 8), path
            columns.append(dict(table=table, rows=rel['rows'], **c))
    return columns

def percentile(values, q):
    values = sorted(values)
    x = (len(values) - 1) * q
    i = int(x)
    return values[i] + (values[min(i+1, len(values)-1)] - values[i]) * (x-i)

def interval(values):
    return [percentile(values, .025), percentile(values, .975)]

def summarize():
    records, failures, commands = [], [], [shlex.join(BUILD)]
    for job in JOBS:
        path = RAW / ('-'.join(job) + '.jsonl')
        status = path.with_suffix('.status.json')
        if not status.exists():
            failures.append({'job': job, 'reason': 'not completed'})
        else:
            info = json.loads(status.read_text())
            commands.append(info['command'])
            if info['returncode']:
                failures.append(info)
        if path.exists():
            for line in path.read_text().splitlines():
                try:
                    records.append(json.loads(line))
                except json.JSONDecodeError:
                    failures.append({'job': job, 'reason': 'invalid JSONL record'})
    groups, layouts, samples = {}, {}, {}
    for r in records:
        if r['type'] == 'group':
            groups[r['experiment'], r['case']] = r
        elif r['type'] == 'layout':
            layouts[r['experiment'], r['case'], r['variant']] = r
        elif r['type'] == 'sample' and r['valid'] and r['round'] >= 0:
            samples.setdefault((r['experiment'], r['case'], r['variant']), []).append(r)
    rng = random.Random(20261111)
    rows = []
    for key, values in sorted(samples.items()):
        op, case, variant = key
        times = [r['ms'] for r in values]
        if len(values) != 21 or len({r['round'] for r in values}) != 21 or not all(r['exact'] for r in values):
            failures.append({'key': key, 'reason': 'sample count or exactness failure'})
        med = statistics.median(times)
        boot = [statistics.median(rng.choices(times, k=len(times))) for _ in range(5000)]
        row = dict(experiment=op, case=case, variant=variant, samples=len(times), median_ms=med, time_ci95=interval(boot), exact=all(r['exact'] for r in values), sample_ms=times, stored_bytes=values[0]['stored_bytes'])
        row.update({k: v for k, v in layouts.get(key, {}).items() if k in ('level', 'bytes_per_key')})
        for base in ('V0_u64', 'V1_u32', 'V2_bytes'):
            baseline = samples.get((op, case, base))
            if not baseline:
                row['ratio_' + base] = None
                continue
            rounds = {r['round']: r['ms'] for r in baseline}
            ratios = [r['ms'] / rounds[r['round']] for r in values if r['round'] in rounds]
            rb = [statistics.median(rng.choices(ratios, k=len(ratios))) for _ in range(5000)]
            row['ratio_' + base] = dict(median_paired_ratio=statistics.median(ratios), ci95=interval(rb), ratio_of_medians=med/statistics.median(r['ms'] for r in baseline))
        rows.append(row)
    report = dict(date='2026-11-11', target='RTX 6000 Ada sm_89', protocol=dict(warmups=5, samples=21, rotation='first variant advances once per round', timing='CUDA events; allocations/load/CPU oracle outside timed region', bootstrap='5000 paired-round resamples, percentile 95% CI of median sample ratios; ratio of medians also reported', exactness='Every warmup/sample checked against CPU oracle; output arrays compared element by element where produced', tenant_policy='nvidia-smi before and after sample; wait before foreign tenant, invalidate and retry when foreign tenant afterward', lock='GPU subprocess flock /tmp/mini_linq-gpu.lock, released between jobs; compile shared timing lock only'), commands=commands, results=rows, groups=list(groups.values()), layouts=list(layouts.values()), load_encode=[r for r in records if r['type']=='load_encode'], environments=[r for r in records if r['type']=='environment'], tenant_waits=[r for r in records if r['type']=='tenant_wait'], contaminated_samples=[r for r in records if r['type']=='sample' and not r['valid']], not_applicable=[r for r in records if r['type']=='not_applicable'], failures=failures)
    winners = []
    for op, case in sorted(groups):
        candidates = [r for r in rows if r['experiment']==op and r['case']==case and 'noise' not in r['variant']]
        if candidates:
            best = min(candidates, key=lambda r:r['median_ms'])
            winners.append({k:best[k] for k in ('experiment','case','variant','median_ms','ratio_V0_u64','ratio_V1_u32')})
    report['winners'] = winners
    report['noise_floor'] = [r for r in rows if 'noise' in r['variant']]
    conclusions = []
    for op in sorted({r['experiment'] for r in winners}):
        selected = [r for r in winners if r['experiment'] == op]
        counts = {}
        speedups = []
        for r in selected:
            counts[r['variant']] = counts.get(r['variant'], 0) + 1
            if r['ratio_V1_u32']:
                speedups.append(1/r['ratio_V1_u32']['median_paired_ratio'])
        span = f"; speedup over u32 {min(speedups):.2f}–{max(speedups):.2f}×" if speedups else ''
        conclusions.append(f"{op}: lowest-median winners {counts}{span}.")
    for order in ('random', 'sorted'):
        compared = [r['ratio_V2_bytes'] for r in rows if r['experiment']=='O1' and r['variant']=='V3_for' and r['case'].endswith(order)]
        if compared:
            faster = sum(r['ci95'][1] < 1 for r in compared)
            slower = sum(r['ci95'][0] > 1 for r in compared)
            conclusions.append(f"O1 {order} probes: FOR beats byte-aligned in {faster}/{len(compared)} cases with CI wholly below 1, loses in {slower}/{len(compared)}; remaining CIs overlap 1.")
    conclusions.append("The design note's structural/code-domain hypothesis is tested directly, not inferred from old decode-first M3. Disjoint O3 copies encoded blocks with descriptor planning; overlapping O3 co-ranks directly into packed output words using registers only. Neither path materializes an uncompressed tile, even in shared memory. Compare both regimes and decode controls before generalizing.")
    indexed = {(r['experiment'],r['case'],r['variant']):r for r in rows}
    for regime in ('disjoint','interleaved'):
        packed = indexed.get(('O3',regime,'V3_for'))
        control = indexed.get(('O3',regime,'V3_for_decode_merge_reencode'))
        if packed and control:
            conclusions.append(f"O3 {regime}: direct packed merge takes {packed['ratio_V1_u32']['median_paired_ratio']:.2f}× u32 time and {packed['median_ms']/control['median_ms']:.2f}× decode-merge-reencode time. The design note's 'bit-packing is right for merges' is conditional: skipping decode does not guarantee a faster overlap merge.")
    grouped = indexed.get(('O2','sf10_order_part','V3_for'))
    packed_payload = indexed.get(('O2','sf10_order_part','V3_offsets_for_payload_for'))
    if grouped and packed_payload:
        conclusions.append(f"O2 with the same V3 key directory: FOR offsets+payload take {packed_payload['median_ms']/grouped['median_ms']:.2f}× plain offsets+payload time. Key-layout improvements do not eliminate the earlier M2 payload-access penalty.")
    for pct in (3,15,50):
        plain_bitmap = indexed.get(('E_b_semijoin',f'sf10_s{pct}_for1024','V3_bitmap'))
        zone_bitmap = indexed.get(('E_b_semijoin',f'sf10_s{pct}_for1024','V4_bitmap_zone'))
        if plain_bitmap and zone_bitmap:
            conclusions.append(f"E-b {pct}% on FOR probes: bitmap+zone/plain bitmap median time = {zone_bitmap['median_ms']/plain_bitmap['median_ms']:.3f}. Zone grading is exact but is not an automatic speedup.")
    conclusions.append("Strict-definition caveat: CUB sort+unique, u32 hash-table build and sorted-set output materialize plain code arrays and therefore are L2, despite staying in the canonical domain. Calling every narrow-code operation L1 would contradict the note's stated definition.")
    conclusions.append("O4 seal controls are device copies, not a claim that retaining already-plain output requires a copy. Plain output can be retained for zero additional seal cost; V2/V3 measured seal time is additional work. Combined sort+seal was not timed as one event region.")
    noise_ratios = [r['ratio_V1_u32']['median_paired_ratio'] for r in report['noise_floor'] if r['ratio_V1_u32']]
    if noise_ratios:
        conclusions.append(f"Noise/order floor: identical u32 duplicate median ratios range {min(noise_ratios):.3f}–{max(noise_ratios):.3f}. Rotation preserves variant adjacency, so this diagnostic includes cache/order bias, not just random timing noise. Bootstrap CIs do not remove that bias; small layout differences comparable to the same-case duplicate discrepancy are inconclusive.")
    report['conclusions'] = conclusions
    (RESULTS / (STEM+'.json')).write_text(json.dumps(report, indent=2)+'\n')
    def ratio(row, base):
        x=row.get('ratio_'+base)
        return '—' if not x else f"{x['median_paired_ratio']:.3f} [{x['ci95'][0]:.3f}, {x['ci95'][1]:.3f}]"
    lines=['# Compressed-code GPU operators — 2026-11-11', '', 'Measurement-only microbenchmarks, not engine or end-to-end query changes.', '', '## Protocol', '', 'Five warmups and 21 measured CUDA-event samples per variant, rotated within paired rounds. Every sample is checked against a CPU oracle. Ratios below are median paired time ratios (lower is better), with 5,000-bootstrap 95% percentile CIs. Allocations, file loading, CPU encoding and oracle construction are excluded from operator timing; separate wall records include preparation. All GPU jobs take `/tmp/mini_linq-gpu.lock` and release it between jobs; builds take the shared timing lock only.', '', 'L0: descriptors/bitmap words only; L1: code extraction/rebasing in registers without global plain materialization; L2: logical-width reconstruction or global plain-array materialization. Plain u64 is an L2 control. CUB sort necessarily emits plain arrays and is labeled L2 under this strict definition even though its comparison domain is canonical codes.', '', 'E-a FOR uses contiguous packed residuals; V2 uses actual u8/u16/u32 loads. Headers are four u32 fields (base, exact max, word offset, width), plus a u32 fence per 1024 keys. Delta restarts every 64 keys and uses one such header per restart. V5 includes one u32 rank prefix per bitmap word for locate. E-b scan encoding and details are recorded in group metadata.', '', '## Lowest-median variants', '', '| Operation / case | Winner | ms | ratio / V0 [95% CI] | ratio / V1 [95% CI] |', '|---|---|---:|---:|---:|']
    lines[4:4] = ['## Findings', ''] + ['- '+s for s in conclusions] + ['']
    for r in winners:
        lines.append(f"|{r['experiment']} / {r['case']}|{r['variant']}|{r['median_ms']:.4f}|{ratio(r,'V0_u64')}|{ratio(r,'V1_u32')}|")
    lines += ['', 'Median rankings do not by themselves establish a statistically significant win. See noise duplicates below.', '', '## Full measurements and footprints', '', 'For O1/O5, bytes/key is complete stored run bytes divided by distinct run keys. O2 includes keys, offsets and payload divided by distinct group keys, not payload rows. Build/intersection/seal footprint definitions are in each group record.', '', '| Operation / case | Variant | Level | bytes/key | ms [95% CI] | ratio / V0 [95% CI] | ratio / V1 [95% CI] |', '|---|---|---|---:|---:|---:|---:|']
    for r in rows:
        ci=r['time_ci95']
        lines.append(f"|{r['experiment']} / {r['case']}|{r['variant']}|{r.get('level','unspecified')}|{r.get('bytes_per_key',float('nan')):.4f}|{r['median_ms']:.4f} [{ci[0]:.4f}, {ci[1]:.4f}]|{ratio(r,'V0_u64')}|{ratio(r,'V1_u32')}|")
    lines += ['', '## Bit-packed versus byte-aligned', '', '| Operation / case | FOR / bytes paired time ratio [95% CI] |', '|---|---:|']
    for r in rows:
        if r['variant'] == 'V3_for' and r.get('ratio_V2_bytes'):
            lines.append(f"|{r['experiment']} / {r['case']}|{ratio(r,'V2_bytes')}|")
    lines += ['', '## Noise floor and contamination', '', f"Observed tenant waits: {len(report['tenant_waits'])}; discarded contaminated samples: {len(report['contaminated_samples'])}. Before/after sampling cannot detect a foreign process that starts and exits entirely inside a sample. Clock/load snapshots and all tenant snapshots remain in raw JSONL.", '', 'Identical `V1_noise` vs `V1_u32` paired ratios:', '']
    for r in report['noise_floor']:
        lines.append(f"- {r['experiment']} / {r['case']}: {ratio(r,'V1_u32')}.")
    lines += ['', 'Rotation changes the first variant but preserves cyclic adjacency. An identical duplicate frequently follows the original u32 variant; a reproducible difference can therefore be a cache/order effect. These are within-schedule bootstrap intervals, not bias-corrected cross-machine guarantees. Near-ties must be read against the **same-case** duplicate, not merely whether a CI excludes 1.', '', 'Initial O3/O4 raw files are preserved with `initial-` prefixes. These three jobs were remeasured after adding preparation wall-time records and excluding the unused legacy main from library inclusion; timed kernels were unchanged. Tables use the complete later runs, not selected samples.']
    lines += ['', '## Applicability and limits', '', '- Bitmap membership/locate is included only at span <= 32 × cardinality. This is a storage policy, not an assertion of a performance crossover.', '- Dense real sets with no substantial holes necessarily use outside-span misses; other O1 cases sample misses uniformly from holes inside the span. Hit rates are exact, probe sequences identical across layouts.', '- O5 uses sorted unique even/odd admission batches with exactly 50% overlap against the 16M even-key run. Results do not establish arbitrary Datalog distributions.', '- No hardware traffic counters, cache-flush experiments or implementation tuning sweep were performed. Interpret implementation-specific kernel costs rather than universal codec costs.', '']
    for r in report['not_applicable']:
        lines.append('- '+json.dumps(r,sort_keys=True))
    lines += ['', '## Preparation (not query time)', '', '| Case | load/encode/oracle wall ms |', '|---|---:|']
    for r in report['load_encode']:
        lines.append(f"|{r['case']}|{r.get('wall_ms',0):.2f}|")
    lines += ['', '| Operation / case | load ms | CPU selection/oracle ms | encoding/upload ms |', '|---|---:|---:|---:|']
    for (op, case), g in sorted(groups.items()):
        if 'load_wall_ms' in g:
            lines.append(f"|{op} / {case}|{g['load_wall_ms']:.2f}|{g.get('cpu_selection_oracle_wall_ms',0):.2f}|{g.get('encoding_upload_wall_ms',g.get('encoding_upload_allocation_wall_ms',0)):.2f}|")
    lines += ['', '## Exact commands', '', '```sh', 'python3 experiments/storage-bench/gpu/compressed_run.py --build', *commands, 'python3 experiments/storage-bench/gpu/compressed_run.py --summarize', '```', '', '## Completeness', '']
    lines += [json.dumps(x) for x in failures] if failures else ['All campaign jobs completed; all accepted samples passed their exactness checks.']
    (RESULTS / (STEM+'.md')).write_text('\n'.join(lines)+'\n')
    return report

def main():
    p=argparse.ArgumentParser()
    p.add_argument('--build',action='store_true')
    p.add_argument('--summarize',action='store_true')
    p.add_argument('--only')
    p.add_argument('--force',action='store_true')
    args=p.parse_args()
    RAW.mkdir(parents=True,exist_ok=True)
    if args.build:
        subprocess.run(BUILD,check=True)
    if not args.summarize:
        columns=validate_manifest()
        (RAW/'manifest-columns.json').write_text(json.dumps(columns,indent=2)+'\n')
        for job in JOBS:
            if args.only and job[0]!=args.only:
                continue
            path=RAW/('-'.join(job)+'.jsonl')
            status=path.with_suffix('.status.json')
            if not args.force and status.exists() and json.loads(status.read_text())['returncode']==0:
                continue
            command=['flock','/tmp/mini_linq-gpu.lock','nice','-n','19',str(BINARY),*job]
            started=datetime.datetime.now(datetime.timezone.utc).isoformat()
            with path.open('w') as out,path.with_suffix('.stderr').open('w') as err:
                result=subprocess.run(command,stdout=out,stderr=err)
            status.write_text(json.dumps(dict(job=job,command=shlex.join(command),returncode=result.returncode,started_utc=started,ended_utc=datetime.datetime.now(datetime.timezone.utc).isoformat(),stderr=path.with_suffix('.stderr').read_text()))+'\n')
            print(' '.join(job),'exit',result.returncode,flush=True)
            if result.returncode:
                raise SystemExit(result.returncode)
    report=summarize()
    print('Summarized',len(report['results']),'variants;',len(report['failures']),'incomplete jobs',flush=True)

if __name__=='__main__':
    main()
