#!/usr/bin/env python3
"""Profile the retained no-feature h01 worker, preserving build provenance."""
import hashlib
import json
import os
import pathlib
import subprocess
import time
ROOT=pathlib.Path('/data/mini_linq')
OUT=ROOT/'experiments/storage-bench/results'
PREFIX=OUT/'cpu-h01-perf-2026-10-29'
TAG='2abb2513a75956d0'
BINARY=ROOT/f'features/workloads/target/bench/build/release/deps/tpch_derived_retail_h01-{TAG}'
FINGERPRINT=ROOT/f'features/workloads/target/bench/build/release/.fingerprint/tpch-derived-retail_h01-{TAG}/bin-tpch-derived-retail_h01.json'
COUNTER=ROOT/'target/expanded-diff/build/current/release/tpch-derived-retail_h01'
COUNTER_ARTIFACT=ROOT/'target/expanded-diff/build/current/release/build/tpch-derived-retail_h01/89a0593ebef41215/out/tpch_derived_retail_h01'
COUNTER_FP=ROOT/'target/expanded-diff/build/current/release/build/tpch-derived-retail_h01/89a0593ebef41215/fingerprint/bin-tpch-derived-retail_h01.json'
def capture(args):
    return subprocess.run(args,text=True,stdout=subprocess.PIPE,stderr=subprocess.STDOUT,check=True).stdout

def snapshot():
    ps=capture(['ps','-eo','pid,pcpu,args','--sort=-pcpu'])
    heavy=[]
    for line in ps.splitlines()[1:]:
        fields=line.split(None,2)
        if len(fields)==3 and int(fields[0])!=os.getpid() and float(fields[1])>=20 and not fields[2].startswith('ps '):
            heavy.append(line)
    gpu=capture(['nvidia-smi','--query-compute-apps=pid,used_memory','--format=csv,noheader'])
    return {'utc':capture(['date','-u','+%FT%TZ']).strip(),'uptime':capture(['uptime']),
            'top_cpu_processes':ps.splitlines()[:6],'heavy_external_cpu_processes':heavy,
            'gpu_processes':gpu,'gpu_clocks':capture(['nvidia-smi','-q','-d','CLOCK'])}
def busy(s):return bool(s['heavy_external_cpu_processes'] or s['gpu_processes'].strip())
def sha(path):return hashlib.sha256(path.read_bytes()).hexdigest()
fp=json.loads(FINGERPRINT.read_text());cfp=json.loads(COUNTER_FP.read_text())
assert json.loads(fp['features'])==[]
assert sha(COUNTER)==sha(COUNTER_ARTIFACT)
worker=[str(BINARY),str(ROOT/'features/workloads/target/bench/data/tpch-1/retail_h01'),'default','21',str(PREFIX)+'.answers.json',str(PREFIX)+'.metrics.json',str(ROOT/'features/workloads/target/bench/runs/sf-1/retail_h01/default/oracle.json')]
record=['perf','record','-g','--call-graph','dwarf,16384','-e','cycles:u','-e','page-faults','-F','199','-o',str(PREFIX)+'.data','--']+worker
meta={'binary':str(BINARY),'binary_sha256':sha(BINARY),'fingerprint_path':str(FINGERPRINT),'fingerprint':fp,
      'counter_binary':str(COUNTER),'counter_binary_sha256':sha(COUNTER),'counter_matching_artifact':str(COUNTER_ARTIFACT),'counter_fingerprint':cfp,
      'command':record,'environment':{'RAYON_NUM_THREADS':'1'},'windows':[],
      'scope':'whole-process perf capture; worker reports load separately; callchain analysis separates load from query where symbols permit; one worker warmup plus 21 exact checked runs; profiling run is diagnostic, not M6 timing samples'}
while True:
    before=snapshot()
    if busy(before):
        meta['windows'].append({'before':before,'after':before,'accepted':False,'reason':'busy before profile'})
        pathlib.Path(str(PREFIX)+'.provenance.json').write_text(json.dumps(meta,indent=2)+'\n')
        time.sleep(15);continue
    result=subprocess.run(record,text=True,stdout=subprocess.PIPE,stderr=subprocess.PIPE,env={**os.environ,'RAYON_NUM_THREADS':'1'})
    pathlib.Path(str(PREFIX)+'.stdout').write_text(result.stdout)
    pathlib.Path(str(PREFIX)+'.stderr').write_text(result.stderr)
    after=snapshot();accepted=result.returncode==0 and not busy(after)
    meta['windows'].append({'before':before,'after':after,'accepted':accepted,'returncode':result.returncode})
    pathlib.Path(str(PREFIX)+'.provenance.json').write_text(json.dumps(meta,indent=2)+'\n')
    if result.returncode:
        raise RuntimeError(result.stderr)
    if accepted:break
    print('Discarding contaminated profile and repeating',flush=True)
report=capture(['perf','report','-i',str(PREFIX)+'.data','--stdio','--no-children','--percent-limit','0.2','--sort','symbol,dso'])
pathlib.Path(str(PREFIX)+'.report.txt').write_text(report)
# Keep the original callchains for independently repeatable phase/role analysis.
with pathlib.Path(str(PREFIX)+'.script.txt').open('w') as out:
    subprocess.run(['perf','script','-i',str(PREFIX)+'.data','--ns','-F','comm,pid,tid,time,event,period,ip,sym,dso'],stdout=out,check=True)
print('Profile completed:',PREFIX,flush=True)
