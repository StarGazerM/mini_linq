//! Exact launch/site replay of joint probe and child-address captures.
use super::*;
use std::{collections::{BTreeMap, HashMap}, io::{BufRead, BufReader}, sync::Arc};
use gpu_datalog_runtime::{radix_run::RadixRunView, packed_run::{PackedRun, PackedView, read_support::checksum}};
use cuda_core::LaunchConfig1D;
#[path="real_kernels.rs"] mod device;
#[path="real_legacy.rs"] mod legacy;

#[derive(Clone, Deserialize)]
struct Snapshot { snapshot: u64, runs: Vec<usize>, bytes: usize, bits: u32 }
#[derive(Clone, Deserialize)]
struct Description { snapshot: u64, word_bytes: usize, fields: Vec<Vec<u32>> }
#[derive(Deserialize)]
struct Launch { seq: usize, file: String, kernel: String, rule: String, round: usize, sites: Vec<Value>, scan_rows: Vec<u64>, bindings: Vec<[u64;3]> }

struct Clock { start:cuda_core::CudaEvent, end:cuda_core::CudaEvent }
impl Clock {
    fn new(pool:&Pool)->Result<Self>{
        let flags=Some(cuda_core::sys::CUevent_flags_enum_CU_EVENT_DEFAULT);
        Ok(Self{start:pool.context().new_event(flags)?,end:pool.context().new_event(flags)?})
    }
    fn time(&self,pool:&Pool,call:impl FnOnce()->Result<()>)->Result<(u64,u64)>{
        pool.stream().synchronize()?;
        let begin=Instant::now();self.start.record(pool.stream())?;call()?;self.end.record(pool.stream())?;
        pool.stream().synchronize()?;
        Ok((ns(begin.elapsed()),(self.start.elapsed_ms(&self.end)? as f64*1e6).round() as u64))
    }
}

trait Resident {
    fn replay(&self, records: Vec<[u64;16]>, site: &Value, launch: &Launch, repeats: usize) -> Result<Value>;
    fn bytes(&self) -> [usize;3];
}
struct StateRuns<W: Word> {
    pool: Arc<Pool>, spec: Snapshot, description: Description,
    host: Vec<Vec<W>>, inputs: Vec<Buffer<W>>, compressed: Vec<Option<PackedRun<W>>>,
    engine: Buffer<RadixRunView<W,u64>>, plain: Buffer<PackedView<W>>, packed: Buffer<PackedView<W>>,
    module:device::kernels::LoadedModule, clock:Clock,
}
impl<W: Word> StateRuns<W> {
    fn new(root: &Path, pool: Arc<Pool>, spec: Snapshot, description: Description) -> Result<Self> {
        let packing = Packing::new(&description.fields.iter().map(|f|f[1]).collect::<Vec<_>>())?;
        let ablation = super::read_ablation()?;
        if packing.bits()!=spec.bits || description.word_bytes!=std::mem::size_of::<W>() { return Err("snapshot packing mismatch".into()); }
        let mut host=Vec::new(); let mut inputs=Vec::new(); let mut compressed=Vec::new();
        let mut ev=Vec::new(); let mut pv=Vec::new(); let mut cv=Vec::new();
        for (run,&rows) in spec.runs.iter().enumerate() {
            let keys=read_rows::<W>(root,&FileRows{file:format!("snapshot-{}-{run}.bin",spec.snapshot),rows})?;
            let input=Buffer::from_host(pool.clone(),&keys)?;
            ev.push(RadixRunView { keys:input.ptr(), offsets:std::ptr::null(), directory:std::ptr::null(), keys_len:rows as u64, items:rows as u64, radix_bits:0, unit_offsets:1 });
            let view=PackedView { plain:input.ptr(),payload:std::ptr::null(),headers:std::ptr::null(),row_directory:std::ptr::null(),frames:std::ptr::null(),fences:std::ptr::null(),fields:std::ptr::null(),rows:rows as u64,blocks:0,block_rows:256,columns:packing.len() as u32 };
            pv.push(view);
            let packed = if rows > gpu_datalog_runtime::packed_run::SMALL_RUN_ROWS {
                if std::env::var_os("PACKED_READ_LEAVES").is_some() {
                    let mut left = Vec::with_capacity(rows * 2 / 3 + 1);
                    let mut right = Vec::with_capacity(rows / 3 + 1);
                    for (i, &key) in keys.iter().enumerate() {
                        if i % 3 == 2 { right.push(key); } else { left.push(key); }
                    }
                    let a = unsafe { PackedRun::seal(&Buffer::from_host(pool.clone(), &left)?, left.len(), &packing, 256)? };
                    let b = unsafe { PackedRun::seal(&Buffer::from_host(pool.clone(), &right)?, right.len(), &packing, 256)? };
                    let merged = unsafe { a.merge(&b)? };
                    Some(if let Some((fixed, inherit)) = ablation {
                        unsafe { merged.replay_reframe(&input, fixed, inherit)? }
                    } else { merged })
                } else { Some(unsafe { PackedRun::seal(&input, rows, &packing, 256)? }) }
            } else { None };
            cv.push(packed.as_ref().map_or(view,|r|r.view()));
            host.push(keys);inputs.push(input);compressed.push(packed);
        }
        let engine=Buffer::from_host(pool.clone(),&ev)?;
        let plain=Buffer::from_host(pool.clone(),&pv)?;
        let packed=Buffer::from_host(pool.clone(),&cv)?;
        let module=unsafe{device::kernels::load(pool.context())?};let clock=Clock::new(&pool)?;
        Ok(Self{pool,spec,description,host,inputs,compressed,engine,plain,packed,module,clock})
    }
    fn encode(&self, record: &[u64;16]) -> Result<(W,W)> {
        if record[15]>>63 != 0 { return Err("out-of-domain trace requires explicit rejection job".into()); }
        let count=record[15] as usize;
        if count>self.description.fields.len() || count>4 { return Err("invalid bound-field count".into()); }
        let mut lo=W::zero();let mut remaining=self.spec.bits;
        for (field,&value) in self.description.fields.iter().zip(&record[8..12]).take(count) {
            let sign = if field.get(2).copied().unwrap_or(0) != 0 { 1u64 << (field[1]-1) } else { 0 };
            lo.insert(field[0],field[1],value ^ sign);remaining=field[0];
        }
        let mut hi=lo;
        for lane in 0..W::BITS/64 { let bits=remaining.saturating_sub(lane*64).min(64);if bits!=0{hi.set_lane(lane,hi.lane(lane)|(u64::MAX>>(64-bits)));} }
        Ok((lo,hi))
    }
    fn execute(&self,lows:&[W],highs:&[W],jobs:&[[u64;3]],expected:&[[u64;3]],repeats:usize,sequence:usize,scope:&'static std::ffi::CStr)->Result<([u64;3],[u64;3])>{
        let count=jobs.len();
        let low=Buffer::from_host(self.pool.clone(),lows)?;let high=Buffer::from_host(self.pool.clone(),highs)?;
        let jobs=Buffer::from_host(self.pool.clone(),jobs)?;let output=unsafe{Buffer::<[u64;3]>::uninitialized(self.pool.clone(),count)?};
        let module=&self.module;
        let cfg=LaunchConfig1D::new(count.div_ceil(256).max(1).min(65535) as u32,256,0);
        let e=module.prepare_replay::<W,0>(cfg)?;let p=module.prepare_replay::<W,1>(cfg)?;let c=module.prepare_replay::<W,2>(cfg)?;
        // Match the settled PackedSet adapter's all-plain specialization.
        let all_plain=self.compressed.iter().all(Option::is_none);
        let call=|mode:usize|->Result<()>{match mode{
            0=>module.replay::<W,0>(self.pool.stream(),&e,self.engine.ptr(),self.plain.ptr(),low.ptr(),high.ptr(),jobs.ptr(),count as u64,output.ptr())?,
            1=>module.replay::<W,1>(self.pool.stream(),&p,self.engine.ptr(),self.plain.ptr(),low.ptr(),high.ptr(),jobs.ptr(),count as u64,output.ptr())?,
            _ if all_plain=>module.replay::<W,1>(self.pool.stream(),&p,self.engine.ptr(),self.packed.ptr(),low.ptr(),high.ptr(),jobs.ptr(),count as u64,output.ptr())?,
            _=>module.replay::<W,2>(self.pool.stream(),&c,self.engine.ptr(),self.packed.ptr(),low.ptr(),high.ptr(),jobs.ptr(),count as u64,output.ptr())?,
        }Ok(())};
        for mode in 0..3 {call(mode)?;let actual=output.to_host()?;if actual!=expected{let first=actual.iter().zip(expected).position(|(a,b)|a!=b);return Err(format!("GPU oracle mismatch sequence={sequence} mode={mode} first={first:?}").into());}}
        let mut wall=[0;3];let mut gpu=[0;3];let _scope=cq_gpu_runtime::nvtx::range(scope);
        for repeat in 0..repeats {for offset in 0..3 {let mode=(sequence+repeat+offset)%3;
            let _mode=cq_gpu_runtime::nvtx::range(match mode{0=>c"real_engine",1=>c"real_plain",_=>c"real_packed"});
            let (w,g)=self.clock.time(&self.pool,||call(mode))?;wall[mode]+=w;gpu[mode]+=g;
        }}
        Ok((wall,gpu))
    }
}
impl<W: Word> Resident for StateRuns<W> {
    fn bytes(&self)->[usize;3] {
        let plain=self.inputs.iter().map(Buffer::allocated_bytes).sum::<usize>();
        [plain+self.engine.allocated_bytes(),plain+self.plain.allocated_bytes(),self.inputs.iter().zip(&self.compressed).map(|(p,c)|c.as_ref().map_or(p.allocated_bytes(),PackedRun::allocated_bytes)).sum::<usize>()+self.packed.allocated_bytes()]
    }
    fn replay(&self,mut records:Vec<[u64;16]>,site:&Value,launch:&Launch,repeats:usize)->Result<Value>{
        let nested=site["depth"].as_u64().ok_or("missing site depth")?!=0;
        if !nested && site["replay_preordered"]!=true { records.sort_by_key(|r|if r[0]>>63!=0{(1,0,0,0)}else{(0,r[4],r[5],r[1])}); }
        let mut lows=Vec::with_capacity(records.len());let mut highs=Vec::with_capacity(records.len());
        let mut jobs=Vec::with_capacity(records.len());let mut expected=Vec::with_capacity(records.len());
        let mut counts=[0u64;5];let mut scans=0u64;let mut packed_probes=0u64;let mut domain_rejects=0u64;let mut negative_blocks=0u64;
        let mut packed_misses=0u64;let mut empty_run_misses=0u64;
        let mut comparisons=0u64;let mut nondecreasing=0u64;let mut previous=None;
        let bound=(records.iter().find(|r|r[0]>>63==0).ok_or("child group without probes")?[15]&!(1u64<<63)) as usize;
        let suffix=if bound==0{self.spec.bits}else{self.description.fields[bound-1][0]};
        let mut kinds=Vec::with_capacity(records.len());let mut prefix_kinds=BTreeMap::new();
        for r in &records {
            let scan=r[0]>>63!=0;
            let run=r[5] as usize;
            if r[4]!=self.spec.snapshot {return Err("site spans different snapshots".into());}
            if scan {
                let word=*self.host.get(run).and_then(|v|v.get(r[1] as usize)).ok_or("child address outside captured run")?;
                lows.push(W::zero());highs.push(W::zero());jobs.push([run as u64,1,r[1]]);
                expected.push([r[1],r[1]+1,checksum(word,W::BITS)]);scans+=1;
                let mut prefix=word;
                for lane in 0..W::BITS/64 {let bits=suffix.saturating_sub(lane*64).min(64);if bits!=0{prefix.set_lane(lane,prefix.lane(lane)&!(u64::MAX>>(64-bits)));}}
                kinds.push(*prefix_kinds.get(&(run,prefix)).ok_or("child row has no preceding recorded range probe")?);
            } else {
                let (lo,hi)=self.encode(r)?;
                let empty=self.host.is_empty();
                let keys=if empty { &[][..] } else {self.host.get(run).ok_or("invalid probe run")?.as_slice()};
                let a=keys.partition_point(|k|*k<lo);let b=keys.partition_point(|k|*k<=hi);
                if [a as u64,b as u64]!=[r[6],r[7]] {return Err(format!("recorded range mismatch launch={} site={} snapshot={} run={} actual={a}..{b} recorded={}..{}",launch.seq,site["site"],self.spec.snapshot,run,r[6],r[7]).into());}
                lows.push(lo);highs.push(hi);jobs.push([if empty{u64::MAX}else{run as u64},0,0]);expected.push([a as u64,b as u64,(a!=b) as u64]);
                let member=site["membership"]==true;
                let kind=if a==b {if member{0}else{1}}else if b-a==1{2}else if b-a<=32{3}else{4};
                counts[kind]+=1;kinds.push(kind);
                if a!=b && site["scan"]==true {prefix_kinds.insert((run,lo),kind);}
                if keys.len()>32768 {packed_probes+=1;}
                if a==b && keys.len()>32768 {packed_misses+=1;}
                if a==b {
                    if keys.is_empty(){empty_run_misses+=1;}
                    else if keys.first().is_some_and(|k|hi<*k)||keys.last().is_some_and(|k|lo>*k){domain_rejects+=1;}
                    if !keys.is_empty(){let block=a.saturating_sub(1)/256;let first=block*256;let last=(first+256).min(keys.len())-1;if hi<keys[first]||lo>keys[last]{negative_blocks+=1;}}
                }
                if let Some((prev_run,prev))=previous {if prev_run==run{comparisons+=1;nondecreasing+=(prev<=lo) as u64;}}
                previous=Some((run,lo));
            }
        }
        if scans!=launch.scan_rows[site["site"].as_u64().ok_or("site id")? as usize]{return Err("child enumeration count mismatch".into());}
        let (times,gpu)=self.execute(&lows,&highs,&jobs,&expected,repeats,launch.seq,c"real_site")?;
        let mut by_kind=serde_json::Map::new();
        for (kind,name) in ["membership_miss","range_miss","one_row_hit","short_range","long_range"].iter().enumerate(){
            let selected=kinds.iter().filter(|&&k|k==kind).count();
            if selected==0 {continue;}
            let (wall,device)=if selected==jobs.len(){(times,gpu)}else{
                let mut lo=Vec::with_capacity(selected);let mut hi=Vec::with_capacity(selected);let mut work=Vec::with_capacity(selected);let mut oracle=Vec::with_capacity(selected);
                for (i,&k) in kinds.iter().enumerate(){if k==kind{lo.push(lows[i]);hi.push(highs[i]);work.push(jobs[i]);oracle.push(expected[i]);}}
                self.execute(&lo,&hi,&work,&oracle,repeats,launch.seq,c"real_kind")?
            };
            by_kind.insert((*name).into(),json!({"ns":wall,"gpu_ns":device,"probes":counts[kind],"enumerated_rows":selected as u64-counts[kind]}));
        }
        Ok(json!({"launch":launch.seq,"kernel":launch.kernel,"rule":launch.rule,"round":launch.round,"site":site,"snapshot":self.spec.snapshot,
            "counts":{"membership_miss":counts[0],"range_miss":counts[1],"one_row_hit":counts[2],"short_range":counts[3],"long_range":counts[4]},
            "probes":counts.iter().sum::<u64>(),"enumerated_rows":scans,"packed_probes":packed_probes,"state_bytes":self.spec.bytes,"representation_bytes":self.bytes(),
            "resident_rows":self.spec.runs.iter().sum::<usize>(),
            "order":if nested{"nested_completion"}else if comparisons==0{"root_insufficient"}else if nondecreasing as f64/comparisons as f64>=0.95{"root_sorted"}else{"root_unsorted"},
            "order_nondecreasing":nondecreasing,"order_comparisons":comparisons,"run_domain_negative_opportunity":domain_rejects,"block_domain_negative_opportunity":negative_blocks,
            "packed_misses":packed_misses,"empty_run_misses":empty_run_misses,
            "ns":times,"gpu_ns":gpu,"stratified_kind_diagnostics":by_kind,"repeats":repeats,"verified_results":records.len()}))
    }
}

pub(super) fn run(root:&Path,repeats:usize,site_filter:Option<&str>,launch_filter:Option<usize>)->Result<Value>{
    let mut snapshots=BTreeMap::new();let mut descriptions=BTreeMap::new();let mut launches=Vec::new();let mut legacy_data=BTreeMap::new();
    let mut model="one launch per recorded launch/site; source-ordered roots; instrumented completion order nested; independently addressed exact child reads".to_string();
    let mut names=HashMap::new();
    for line in BufReader::new(fs::File::open(root.join("events.jsonl"))?).lines(){let event:Value=serde_json::from_str(&line?)?;match event["event"].as_str(){
        Some("snapshot")=>{let s:Snapshot=serde_json::from_value(event)?;snapshots.insert(s.snapshot,s);},
        Some("snapshot_data")=>{let d:Description=serde_json::from_value(event)?;descriptions.insert(d.snapshot,d);},
        Some("legacy_data")=>{let d:legacy::Legacy=serde_json::from_value(event)?;legacy_data.insert((d.launch,d.site),d);},
        Some("replay_model")=>{model=event["model"].as_str().ok_or("replay model label")?.to_owned();},
        Some("name")=>{names.insert(event["snapshot"].as_u64().ok_or("named snapshot")?,event["relation"].clone());},
        Some("launch")=>launches.push(serde_json::from_value::<Launch>(event)?),_=>{}}}
    if descriptions.is_empty(){return Err("joint trace required: old aggregate-only trace cannot replay actual child addresses".into());}
    launches.sort_by_key(|l|l.seq);
    let pool=Pool::new(0,0,0)?;let before=tenant()?;if foreign(&before){return Err(format!("GPU not quiet: {before}").into());}
    let mut states:HashMap<u64,Box<dyn Resident>>=HashMap::new();let mut reports=Vec::new();
    let mut last_use=HashMap::new();
    for launch in &launches { for binding in &launch.bindings { last_use.insert(binding[1],launch.seq); } }
    for launch in &launches {
        if launch_filter.is_some_and(|seq|seq!=launch.seq){continue;}
        let selected=|site:&Value|site_filter.is_none_or(|s|format!("{}/{}",launch.kernel,site["site"]).contains(s)||format!("{}/{}",launch.rule,site["site"]).contains(s));
        if !launch.sites.iter().any(selected){continue;}
        let bytes=fs::read(root.join(&launch.file))?;
        if bytes.len()%128!=0{return Err("partial probe record".into());}
        let mut groups:BTreeMap<u64,Vec<[u64;16]>>=BTreeMap::new();
        for raw in bytes.chunks_exact(128){let mut r=[0u64;16];for (word,raw) in r.iter_mut().zip(raw.chunks_exact(8)){*word=u64::from_le_bytes(raw.try_into()?);}groups.entry(r[0]&!(1<<63)).or_default().push(r);}
        for site in &launch.sites {
            let id=site["site"].as_u64().ok_or("site id")?;
            if !selected(site){continue;}
            let Some(records)=groups.remove(&id) else{continue;};
            if site.get("whole_source").is_some() {
                reports.push(legacy::whole(&pool,records,site,launch,repeats)?);
                continue;
            }
            if site["nonradix"]==true {
                let metadata=legacy_data.get(&(launch.seq,id)).ok_or("legacy snapshot data absent")?;
                reports.push(legacy::replay(root,&pool,metadata,records,site,launch,repeats)?);
                continue;
            }
            let snapshot=records[0][4];
            if !states.contains_key(&snapshot){
                let spec=snapshots.get(&snapshot).ok_or("snapshot metadata absent")?.clone();let desc=descriptions.get(&snapshot).ok_or("snapshot data absent")?.clone();
                let state:Box<dyn Resident>=match desc.word_bytes{
                    8=>Box::new(StateRuns::<u64>::new(root,pool.clone(),spec,desc)?),
                    16=>Box::new(StateRuns::<u128>::new(root,pool.clone(),spec,desc)?),
                    24=>Box::new(StateRuns::<[u64;3]>::new(root,pool.clone(),spec,desc)?),
                    32=>Box::new(StateRuns::<[u64;4]>::new(root,pool.clone(),spec,desc)?),
                    _=>return Err("unsupported snapshot word size".into())};states.insert(snapshot,state);
            }
            reports.push(states[&snapshot].replay(records,site,launch,repeats)?);
        }
        states.retain(|id,_|last_use.get(id).is_some_and(|&last|last>launch.seq));
    }
    let after=tenant()?;if foreign(&after){return Err(format!("GPU not quiet after replay: {after}").into());}
    for report in &mut reports {if let Some(name)=report["snapshot"].as_u64().and_then(|id|names.get(&id)){report["relation"]=name.clone();}}
    Ok(json!({"model":model, "recorded_launches":launches.len(),"tenant_before":before,"tenant_after":after,"sites":reports,"pool_peak_including_all_representations_and_replay_scratch":pool.memory_stats().peak_bytes}))
}
