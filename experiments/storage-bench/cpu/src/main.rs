use rayon::prelude::*;
use serde::Serialize;
use std::{collections::HashMap, fs, hint::black_box, process::Command, time::Instant};
const B: usize = 1024;
const CHUNK: usize = 64; // fixed 65,536-row tasks, independent of thread count
const DATA: &str = "/data/mini_linq/target/storage-bench/tpch-sf1";
const RESULTS: &str = "/data/mini_linq/experiments/storage-bench/results";
const NAMES: [&str; 5] = ["plain", "bytes", "for", "dict_bits", "dict_bytes"];
fn width(x: u64) -> usize { (64 - x.leading_zeros()) as usize }
fn rng(s: &mut u64) -> u64 { *s ^= *s << 13; *s ^= *s >> 7; *s ^= *s << 17; *s }
#[derive(Clone)]
struct Packed { base: u64, w: usize, words: Vec<u64> }
impl Packed {
    fn new(v: &[u64], subtract: bool) -> Self {
        let base = if subtract { *v.iter().min().unwrap() } else { 0 };
        let w = width(v.iter().max().unwrap() - base);
        let mut p = Self { base, w, words: vec![0; w * 16] };
        p.encode(v); p
    }
    fn encode(&mut self, v: &[u64]) {
        self.words.fill(0);
        if self.w == 0 { return; }
        for (i, &x) in v.iter().enumerate() {
            let bit = (i / 16) * self.w; let lane = i % 16;
            let at = (bit / 64) * 16 + lane; let shift = bit % 64; let x = x - self.base;
            self.words[at] |= x << shift;
            if shift + self.w > 64 { self.words[at + 16] |= x >> (64 - shift); }
        }
    }
    fn get(&self, i: usize) -> u64 {
        if self.w == 0 { return self.base; }
        let bit = (i / 16) * self.w; let at = (bit / 64) * 16 + i % 16; let s = bit % 64;
        let mut x = self.words[at] >> s;
        if s + self.w > 64 { x |= self.words[at + 16] << (64 - s); }
        self.base + (x & (u64::MAX >> (64 - self.w)))
    }
}
// Width is a const generic; the inner lane loop has uniform shifts and contiguous
// loads/stores. LLVM may vectorize it without per-value variable-width dispatch.
fn unpack<const W: usize>(p: &Packed, out: &mut [u64; B]) {
    if W == 0 { out.fill(p.base); return; }
    let mask = u64::MAX >> (64 - W);
    for row in 0..64 {
        let bit = row * W; let at = (bit / 64) * 16; let s = bit % 64;
        for lane in 0..16 {
            let mut x = p.words[at + lane] >> s;
            if s + W > 64 { x |= p.words[at + 16 + lane] << (64 - s); }
            out[row * 16 + lane] = p.base + (x & mask);
        }
    }
}
macro_rules! dispatch { ($p:expr,$o:expr, $($w:literal),*) => { match $p.w { $($w => unpack::<$w>($p,$o),)* _ => unreachable!() } }; }
fn decode(p: &Packed, out: &mut [u64; B]) {
    dispatch!(p,out,0,1,2,3,4,5,6,7,8,9,10,11,12,13,14,15,16,17,18,19,20,21,22,23,24,25,26,27,28,29,30,31,32,33,34,35,36,37,38,39,40,41,42,43,44,45,46,47,48,49,50,51,52,53,54,55,56,57,58,59,60,61,62,63,64);
}
#[derive(Clone)]
struct ByteBlock { base: u64, w: usize, data: Vec<u8> }
impl ByteBlock {
    fn new(v: &[u64], subtract: bool) -> Self {
        let base = if subtract { *v.iter().min().unwrap() } else { 0 };
        let bits = width(v.iter().max().unwrap() - base);
        let w = if bits <= 8 { 1 } else if bits <= 16 { 2 } else if bits <= 32 { 4 } else { 8 };
        let mut data = Vec::with_capacity(v.len()*w);
        for x in v { data.extend_from_slice(&(x-base).to_le_bytes()[..w]); }
        Self { base, w, data }
    }
    fn get(&self, i: usize) -> u64 {
        let p = i*self.w;
        self.base + match self.w { 1 => self.data[p] as u64, 2 => u16::from_le_bytes(self.data[p..p+2].try_into().unwrap()) as u64, 4 => u32::from_le_bytes(self.data[p..p+4].try_into().unwrap()) as u64, 8 => u64::from_le_bytes(self.data[p..p+8].try_into().unwrap()), _=>unreachable!() }
    }
    fn decode(&self, out: &mut [u64;B]) {
        match self.w {
            1 => for (o,&v) in out.iter_mut().zip(&self.data) { *o = self.base + v as u64; },
            2 => for (o,v) in out.iter_mut().zip(self.data.chunks_exact(2)) { *o = self.base + u16::from_le_bytes(v.try_into().unwrap()) as u64; },
            4 => for (o,v) in out.iter_mut().zip(self.data.chunks_exact(4)) { *o = self.base + u32::from_le_bytes(v.try_into().unwrap()) as u64; },
            8 => for (o,v) in out.iter_mut().zip(self.data.chunks_exact(8)) { *o = self.base + u64::from_le_bytes(v.try_into().unwrap()); }, _=>unreachable!()
        }
    }
}
enum Storage { Plain(Vec<u64>), Plain32(Vec<u32>), Bytes(Vec<ByteBlock>), Bits(Vec<Packed>) }
struct Col { n: usize, storage: Storage, dict: Vec<u64> }
impl Col {
    fn new(v: &[u64], variant: usize, native32: bool) -> Self {
        let mut dict = Vec::new(); let codes;
        let source = if variant >= 3 {
            dict = v.to_vec(); dict.sort_unstable(); dict.dedup();
            let index: HashMap<u64,u64> = dict.iter().enumerate().map(|(i,&x)|(x,i as u64)).collect();
            codes = v.iter().map(|x|index[x]).collect::<Vec<_>>(); &codes[..]
        } else { v };
        let storage = match variant {
            0 if native32 => Storage::Plain32(v.iter().map(|&x|x as u32).collect()),
            0 => Storage::Plain(v.to_vec()),
            1|4 => Storage::Bytes(source.chunks(B).map(|x|ByteBlock::new(x,variant==1)).collect()),
            2|3 => Storage::Bits(source.chunks(B).map(|x|Packed::new(x,variant==2)).collect()), _=>unreachable!()
        };
        // Dictionary storage uses one GLOBAL code width, including tail blocks.
        let mut c = Self { n:v.len(), storage, dict };
        if variant == 3 {
            let w = width(c.dict.len().saturating_sub(1) as u64);
            c.storage = Storage::Bits(source.chunks(B).map(|v| { let mut p=Packed{base:0,w,words:vec![0;w*16]};p.encode(v);p }).collect());
        } else if variant == 4 {
            let bits=width(c.dict.len().saturating_sub(1) as u64); let w=if bits<=8{1}else if bits<=16{2}else{4};
            c.storage=Storage::Bytes(source.chunks(B).map(|v|{let mut data=Vec::with_capacity(v.len()*w);for x in v{data.extend_from_slice(&x.to_le_bytes()[..w]);}ByteBlock{base:0,w,data}}).collect());
        }
        c
    }
    fn block<'a>(&'a self, b:usize, out:&'a mut [u64;B]) -> &'a [u64] {
        let n=(self.n-b*B).min(B);
        match &self.storage {
            Storage::Plain(v)=>return &v[b*B..b*B+n],
            Storage::Plain32(v)=>for (o,&x) in out.iter_mut().zip(&v[b*B..b*B+n]){*o=x as u64;},
            Storage::Bytes(v)=>v[b].decode(out), Storage::Bits(v)=>decode(&v[b],out)
        }
        if !self.dict.is_empty() { for x in &mut out[..n] { *x=self.dict[*x as usize]; } }
        &out[..n]
    }
    fn get(&self,i:usize)->u64 {
        let x=match &self.storage{Storage::Plain(v)=>v[i],Storage::Plain32(v)=>v[i] as u64,Storage::Bytes(v)=>v[i/B].get(i%B),Storage::Bits(v)=>v[i/B].get(i%B)};
        if self.dict.is_empty(){x}else{self.dict[x as usize]}
    }
    fn bytes(&self)->usize { self.dict.len()*8+match &self.storage {Storage::Plain(v)=>v.len()*8,Storage::Plain32(v)=>v.len()*4,Storage::Bytes(v)=>v.iter().map(|p|p.data.len()+16).sum(),Storage::Bits(v)=>v.iter().map(|p|p.words.len()*8+16).sum()} }
    fn block_bytes(&self,b:usize)->usize {match &self.storage{Storage::Plain(_)=>(self.n-b*B).min(B)*8,Storage::Plain32(_)=>(self.n-b*B).min(B)*4,Storage::Bytes(v)=>v[b].data.len()+16,Storage::Bits(v)=>v[b].words.len()*8+16}}
}
fn parallel_sum<F:Fn(usize)->u64+Sync>(n:usize,t:usize,f:F)->u64 {
    if t==1 {(0..n).map(f).sum()}else{(0..n.div_ceil(CHUNK)).into_par_iter().map(|chunk|(chunk*CHUNK..((chunk+1)*CHUNK).min(n)).map(&f).sum::<u64>()).sum()}
}
fn count(c:&Col,threshold:u64,t:usize)->u64 {
    match &c.storage {
        Storage::Plain(v)=>if t==1{v.iter().filter(|&&x|x<threshold).count() as u64}else{v.par_chunks(B*CHUNK).map(|x|x.iter().filter(|&&x|x<threshold).count() as u64).sum()},
        Storage::Plain32(v)=>if t==1{v.iter().filter(|&&x|(x as u64)<threshold).count() as u64}else{v.par_chunks(B*CHUNK).map(|x|x.iter().filter(|&&x|(x as u64)<threshold).count() as u64).sum()},
        _=>parallel_sum(c.n.div_ceil(B),t,|b|{let mut a=[0;B];c.block(b,&mut a).iter().filter(|&&x|x<threshold).count() as u64})
    }
}
fn q6(c:&[Col],t:usize)->u64 {
    if let (Storage::Plain32(d),Storage::Plain(discount),Storage::Plain(quantity),Storage::Plain(price))=(&c[0].storage,&c[1].storage,&c[2].storage,&c[3].storage){
        let f=|chunk:usize|{let start=chunk*B*CHUNK;let end=(start+B*CHUNK).min(d.len());let mut sum=0;for i in start..end{if (d[i]>=8766)&(d[i]<9131)&((5..=7).contains(&discount[i]))&(quantity[i]<2400){sum+=price[i]*discount[i];}}sum};
        return if t==1{(0..d.len().div_ceil(B*CHUNK)).map(f).sum()}else{(0..d.len().div_ceil(B*CHUNK)).into_par_iter().map(f).sum()};
    }
    parallel_sum(c[0].n.div_ceil(B),t,|b|{
        let mut d=[0;B];let mut disc=[0;B];let mut q=[0;B];
        let ds=c[0].block(b,&mut d);let discounts=c[1].block(b,&mut disc);let quantities=c[2].block(b,&mut q);
        let mut sum=0;
        for i in 0..ds.len(){if ds[i]>=8766 && ds[i]<9131 && (5..=7).contains(&discounts[i]) && quantities[i]<2400{sum+=c[3].get(b*B+i)*discounts[i];}}
        sum
    })
}
fn load(name:&str,native32:bool,n:usize)->Vec<u64>{let b=fs::read(format!("{DATA}/{name}.bin")).unwrap();let sz=if native32{4}else{8};let base:Vec<u64>=b.chunks_exact(sz).map(|x|if native32{u32::from_le_bytes(x.try_into().unwrap())as u64}else{u64::from_le_bytes(x.try_into().unwrap())}).collect();(0..n).map(|i|base[i%base.len()]).collect()}
fn command(program:&str,args:&[&str])->String{match Command::new(program).args(args).output(){Ok(o)=>String::from_utf8_lossy(&o.stdout).to_string()+&String::from_utf8_lossy(&o.stderr),Err(e)=>e.to_string()}}
fn state()->serde_json::Value {
    let ps=command("ps",&["-eo","pid,pcpu,args","--sort=-pcpu"]);
    let heavy:Vec<&str>=ps.lines().skip(1).filter(|line|{
        let mut fields=line.split_whitespace();
        let pid=fields.next().and_then(|x|x.parse::<u32>().ok()).unwrap_or(0);
        let cpu=fields.next().and_then(|x|x.parse::<f64>().ok()).unwrap_or(0.);
        let executable=fields.next().unwrap_or("");
        pid!=std::process::id() && cpu>=20. && executable!="ps"
    }).collect();
    serde_json::json!({"utc":command("date",&["-u","+%FT%TZ"]),"uptime":command("uptime",&[]),"top_cpu_processes":ps.lines().take(6).collect::<Vec<_>>(),"heavy_external_cpu_processes":heavy,"loadavg":fs::read_to_string("/proc/loadavg").unwrap(),"gpu_processes":command("nvidia-smi",&["--query-compute-apps=pid,used_memory","--format=csv,noheader"]),"gpu_clocks":command("nvidia-smi",&["-q","-d","CLOCK"]),"cpu_mhz":fs::read_to_string("/proc/cpuinfo").unwrap().lines().filter(|s|s.starts_with("cpu MHz")).collect::<Vec<_>>()})
}
fn busy(s:&serde_json::Value)->bool {
    !s["gpu_processes"].as_str().unwrap().trim().is_empty() || !s["heavy_external_cpu_processes"].as_array().unwrap().is_empty()
}
fn quiet_start(report:&mut Report,exp:&str,case:&str,t:usize,small:bool)->serde_json::Value {
    loop {
        let before=state();
        if small || !busy(&before){return before;}
        report.windows.push(serde_json::json!({"experiment":exp,"case":case,"threads":t,"before":before,"after":before,"accepted":false,"reason":"busy before group"}));
        save(report);
        eprintln!("waiting for external CPU/GPU activity before {exp} {case} t{t}");
        std::thread::sleep(std::time::Duration::from_secs(15));
    }
}
fn median(x:&[f64])->f64{let mut x=x.to_vec();x.sort_by(f64::total_cmp);x[x.len()/2]}
fn ci(x:&[f64])->[f64;2]{let mut seed=0x1298347;let mut bs=Vec::with_capacity(2000);let mut a=vec![0.;x.len()];for _ in 0..2000{for v in &mut a{*v=x[rng(&mut seed)as usize%x.len()];}bs.push(median(&a));}bs.sort_by(f64::total_cmp);[bs[50],bs[1950]]}
fn ratio_ci(a:&[f64],b:&[f64])->[f64;2]{let mut seed=0x1938747;let mut bs=Vec::with_capacity(2000);let mut aa=vec![0.;a.len()];let mut bb=aa.clone();for _ in 0..2000{for i in 0..a.len(){let j=rng(&mut seed)as usize%a.len();aa[i]=a[j];bb[i]=b[j];}bs.push(median(&aa)/median(&bb));}bs.sort_by(f64::total_cmp);[bs[50],bs[1950]]}
#[derive(Serialize)]
struct Row {experiment:String,case:String,threads:usize,variant:String,n:usize,logical_read_bytes:usize,resident_bytes:usize,samples_ns:Vec<f64>,median_ns:f64,ci95_ns:[f64;2],ratio_to_plain:f64,ratio_ci95:[f64;2],ns_per_value:f64,effective_gbs:f64,items_per_second:f64,exact:bool}
#[derive(Serialize)]
struct Report{metadata:serde_json::Value,windows:Vec<serde_json::Value>,rows:Vec<Row>}
fn bench<T:PartialEq+std::fmt::Debug,F:FnMut(usize)->T>(report:&mut Report,exp:&str,case:&str,t:usize,n:usize,names:&[&str],bytes:&[(usize,usize)],reference:&T,mut f:F,small:bool){
    let samples=loop {
        let before=quiet_start(report,exp,case,t,small);let rounds=if small{1}else{26};let mut samples=vec![Vec::new();names.len()];
        for round in 0..rounds {for j in 0..names.len(){let v=(j+round)%names.len();let start=Instant::now();let result=black_box(f(v));let elapsed=start.elapsed().as_nanos()as f64;assert_eq!(&result,reference,"{exp}/{case}/t{t}/{}",names[v]);if !small && round>=5{samples[v].push(elapsed);}}}
        let after=state();let accepted=small||!busy(&after);
        report.windows.push(serde_json::json!({"experiment":exp,"case":case,"threads":t,"before":before,"after":after,"accepted":accepted}));
        if accepted{break samples;}
        save(report);
        eprintln!("discarding and rerunning contaminated group {exp} {case} t{t}");
    };
    if small{return;}
    for v in 0..names.len(){let m=median(&samples[v]);report.rows.push(Row{experiment:exp.into(),case:case.into(),threads:t,variant:names[v].into(),n,logical_read_bytes:bytes[v].0,resident_bytes:bytes[v].1,samples_ns:samples[v].clone(),median_ns:m,ci95_ns:ci(&samples[v]),ratio_to_plain:m/median(&samples[0]),ratio_ci95:ratio_ci(&samples[v],&samples[0]),ns_per_value:m/n as f64,effective_gbs:bytes[v].0 as f64/m,items_per_second:n as f64*1e9/m,exact:true});}
    save(report);eprintln!("completed {exp} {case} t{t}");
}
fn save(r:&Report){
    fs::create_dir_all(RESULTS).unwrap();
    let path=std::env::var("CPU_BENCH_OUTPUT").unwrap_or_else(|_|format!("{RESULTS}/cpu-2026-10-28.json"));
    fs::write(path,serde_json::to_vec_pretty(r).unwrap()).unwrap();
}
fn m1(r:&mut Report,small:bool){
    for n in if small{vec![3079]}else{vec![6_001_215,60_012_150]}{
        let raw:Vec<Vec<u64>>=[("l_shipdate",true),("l_discount",false),("l_quantity",false),("l_extendedprice",false)].iter().map(|(s,b)|load(s,*b,n)).collect();
        let cols:Vec<Vec<Col>>=(0..5).map(|v|raw.iter().enumerate().map(|(i,c)|Col::new(c,v,i==0)).collect()).collect();
        let reference=(0..n).filter(|&i|raw[0][i]>=8766&&raw[0][i]<9131&&(5..=7).contains(&raw[1][i])&&raw[2][i]<2400).map(|i|raw[3][i]*raw[1][i]).sum::<u64>();
        let selected:Vec<usize>=(0..n).filter(|&i|raw[0][i]>=8766&&raw[0][i]<9131&&(5..=7).contains(&raw[1][i])&&raw[2][i]<2400).collect();
        // Logical accesses, not DRAM transactions: payload scalar loads + codec
        // metadata and dictionary dereferences, counting repeated scalar accesses.
        let bytes:Vec<(usize,usize)>=cols.iter().map(|cs|{let mut read=0;for c in &cs[..3]{read+=(0..n.div_ceil(B)).map(|b|c.block_bytes(b)).sum::<usize>();if !c.dict.is_empty(){read+=n*8;}}for &i in &selected{read+=match &cs[3].storage{Storage::Plain(_)=>8,Storage::Plain32(_)=>4,Storage::Bytes(v)=>v[i/B].w+16,Storage::Bits(v)=>{let p=&v[i/B];let shift=((i%B)/16*p.w)%64;16+if p.w==0{0}else if shift+p.w>64{16}else{8}}};if !cs[3].dict.is_empty(){read+=8;}}(read,cs.iter().map(Col::bytes).sum())}).collect();
        for t in [1,24]{let pool=rayon::ThreadPoolBuilder::new().num_threads(t).build().unwrap();pool.install(||bench(r,"M1-Q6",&format!("real-n{n}"),t,n,&NAMES,&bytes,&reference,|v|q6(&cols[v],t),small));}
        drop(cols);drop(raw);
        for w in [4,8,12,16,20,24,32]{let mut seed=0x1234567812345678;let mask=(1u64<<w)-1;let v:Vec<u64>=(0..n).map(|_|rng(&mut seed)&mask).collect();let cols:Vec<Col>=(0..5).map(|k|Col::new(&v,k,false)).collect();let bytes:Vec<_>=cols.iter().map(|c|((0..n.div_ceil(B)).map(|b|c.block_bytes(b)).sum::<usize>()+if c.dict.is_empty(){0}else{n*8},c.bytes())).collect();for pct in [1,15,50]{let threshold=((1u64<<w)*pct/100).max(1);let reference=v.iter().filter(|&&x|x<threshold).count()as u64;let case=format!("uniform-w{w}-p{pct}-actual{:.6}-n{n}",reference as f64/n as f64);for t in [1,24]{let pool=rayon::ThreadPoolBuilder::new().num_threads(t).build().unwrap();pool.install(||bench(r,"M1-count",&case,t,n,&NAMES,&bytes,&reference,|i|count(&cols[i],threshold,t),small));}}}
    }
}
struct Run {keys:Vec<u64>,off32:Vec<u32>,off64:Vec<u64>,payload:Vec<Col>,probes:Vec<u64>,reference:u64}
fn make_run(keys:Vec<u64>,values:Vec<u64>,sort:bool,probes:usize)->Run{
    let mut pairs:Vec<_>=keys.into_iter().zip(values).collect();if sort{pairs.sort_unstable_by_key(|x|x.0);}
    let mut keys=Vec::new();let mut offsets=Vec::new();let mut vals=Vec::with_capacity(pairs.len());
    for (k,v) in pairs{if keys.last()!=Some(&k){keys.push(k);offsets.push(vals.len()as u64);}vals.push(v);}offsets.push(vals.len()as u64);
    let max=*keys.last().unwrap();let mut seed=0x12781282;let probes:Vec<_>=(0..probes).map(|i|if i%2==0{keys[rng(&mut seed)as usize%keys.len()]}else{max+1+(rng(&mut seed)%max)}).collect();
    let reference=probes.iter().map(|k|match keys.binary_search(k){Ok(i)=>vals[offsets[i]as usize..offsets[i+1]as usize].iter().sum::<u64>(),Err(_)=>0}).sum();
    Run{keys,off32:offsets.iter().map(|&v|u32::try_from(v).unwrap()).collect(),off64:offsets,payload:(0..3).map(|v|Col::new(&vals,v,false)).collect(),probes,reference}
}
fn lookup(run:&Run,v:usize,offset64:bool,t:usize)->u64{
    let chunk=|ps:&[u64]|ps.iter().map(|k|match run.keys.binary_search(k){Err(_)=>0,Ok(i)=>{let (s,e)=if offset64{(run.off64[i]as usize,run.off64[i+1]as usize)}else{(run.off32[i]as usize,run.off32[i+1]as usize)};(s..e).map(|j|run.payload[v].get(j)).sum::<u64>()}}).sum::<u64>();
    if t==1{chunk(&run.probes)}else{run.probes.par_chunks(1024).map(chunk).sum()}
}
fn m2(r:&mut Report,small:bool){
    let n=if small{3079}else{6_001_215};let probes=if small{257}else{65_536};
    for kind in ["orderkey-positions","partkey-suppkey"]{
        let run=if kind.starts_with("order"){
            make_run(load("l_orderkey",false,n),(0..n as u64).collect(),false,probes)
        }else{make_run(load("l_partkey",false,n),load("l_suppkey",false,n),true,probes)};
        let names=["plain_offset32","bytes_offset32","for_offset32","plain_offset64","bytes_offset64","for_offset64"];
        let bytes:Vec<_>=(0..6).map(|v|{
            let offset_bytes=if v>=3{run.off64.len()*8}else{run.off32.len()*4};
            (0,run.payload[v%3].bytes()+run.keys.len()*8+offset_bytes)
        }).collect();
        for t in [1,24]{
            let pool=rayon::ThreadPoolBuilder::new().num_threads(t).build().unwrap();
            pool.install(||bench(r,"M2",kind,t,probes,&names,&bytes,&run.reference,|v|lookup(&run,v%3,v>=3,t),small));
        }
    }
}
fn corank(k:usize,a:&[u64],b:&[u64])->(usize,usize){let mut lo=k.saturating_sub(b.len());let mut hi=k.min(a.len());loop{let i=(lo+hi)/2;let j=k-i;if i>0&&j<b.len()&&a[i-1]>b[j]{hi=i-1;}else if j>0&&i<a.len()&&b[j-1]>=a[i]{lo=i+1;}else{return(i,j);}}}
fn merge_segment(a:&[u64],b:&[u64],out:&mut[u64]){let(mut i,mut j)=(0,0);for x in out{if i<a.len()&&(j==b.len()||a[i]<=b[j]){*x=a[i];i+=1;}else{*x=b[j];j+=1;}}}
fn merge(a:&[u64],b:&[u64],out:&mut[u64],t:usize){if t==1{merge_segment(a,b,out);}else{out.par_chunks_mut(B*CHUNK).enumerate().for_each(|(chunk,o)|{let start=chunk*B*CHUNK;let(i,j)=corank(start,a,b);let(ii,jj)=corank(start+o.len(),a,b);merge_segment(&a[i..ii],&b[j..jj],o);});}}
fn decompress(c:&Col,out:&mut[u64],t:usize){let f=|(b,o):(usize,&mut[u64])|{let mut tmp=[0;B];o.copy_from_slice(c.block(b,&mut tmp));};if t==1{out.chunks_mut(B).enumerate().for_each(f);}else{out.par_chunks_mut(B).enumerate().for_each(f);}}
fn reencode(out:&[u64],p:&mut[Packed],t:usize){let f=|(b,p):(usize,&mut Packed)|{let v=&out[b*B..((b+1)*B).min(out.len())];p.base=*v.iter().min().unwrap();let w=width(v.iter().max().unwrap()-p.base);assert_eq!(w,p.w);p.encode(v);};if t==1{p.iter_mut().enumerate().for_each(f);}else{p.par_iter_mut().enumerate().for_each(f);}}
fn m3(r:&mut Report,small:bool){
    for n in if small{vec![10001]}else{vec![1_000_000,16_000_000]}{
        let a:Vec<u64>=(0..n as u64).map(|i|i*3).collect();
        let b:Vec<u64>=(0..(n/10)as u64).map(|i|i*29+1).collect();
        let mut reference=a.clone();reference.extend(&b);reference.sort_unstable();
        let ca=Col::new(&a,2,false);let cb=Col::new(&b,2,false);
        for t in [1,24]{
            let mut da=vec![0;a.len()];let mut db=vec![0;b.len()];
            let mut out=vec![0;reference.len()];
            let mut packed:Vec<Packed>=reference.chunks(B).map(|x|Packed::new(x,true)).collect();
            let pool=rayon::ThreadPoolBuilder::new().num_threads(t).build().unwrap();
            let names=["plain","for_decode_merge_encode"];
            let logical=[((a.len()+b.len())*8,(a.len()+b.len())*8),(ca.bytes()+cb.bytes(),ca.bytes()+cb.bytes())];
            let case=format!("sorted-{n}-{}",n/10);
            // Keep the full exact-output comparison outside the timed region.
            let samples=loop {
                let before=quiet_start(r,"M3",&case,t,small);
                let rounds=if small{1}else{26};let mut samples=[Vec::new(),Vec::new()];
                for round in 0..rounds{
                    for j in 0..2{
                        let v=(j+round)%2;let start=Instant::now();
                        pool.install(||{if v==0{merge(&a,&b,&mut out,t);}else{decompress(&ca,&mut da,t);decompress(&cb,&mut db,t);merge(&da,&db,&mut out,t);reencode(&out,&mut packed,t);}});
                        let elapsed=start.elapsed().as_nanos()as f64;black_box(&out);
                        assert_eq!(out,reference);
                        if v==1{
                            for (i,p) in packed.iter().enumerate(){
                                let mut tmp=[0;B];decode(p,&mut tmp);
                                let len=(reference.len()-i*B).min(B);
                                assert_eq!(&tmp[..len],&reference[i*B..i*B+len]);
                            }
                        }
                        if !small&&round>=5{samples[v].push(elapsed);}
                    }
                }
                let after=state();let accepted=small||!busy(&after);
                r.windows.push(serde_json::json!({"experiment":"M3","case":case,"threads":t,"before":before,"after":after,"accepted":accepted}));
                if accepted{break samples;}
                save(r);eprintln!("discarding contaminated M3 group {case} t{t}");
            };
            if !small{
                for v in 0..2{
                    let m=median(&samples[v]);
                    r.rows.push(Row{experiment:"M3".into(),case:case.clone(),threads:t,variant:names[v].into(),n:reference.len(),logical_read_bytes:logical[v].0,resident_bytes:logical[v].1,samples_ns:samples[v].clone(),median_ns:m,ci95_ns:ci(&samples[v]),ratio_to_plain:m/median(&samples[0]),ratio_ci95:ratio_ci(&samples[v],&samples[0]),ns_per_value:m/reference.len()as f64,effective_gbs:logical[v].0 as f64/m,items_per_second:reference.len()as f64*1e9/m,exact:true});
                }
                save(r);eprintln!("completed M3 {case} t{t}");
            }
        }
    }
}
fn group_codes(codes:&[u8],values:&[u64])->[u64;12]{let mut out=[0;12];for (&c,&v)in codes.iter().zip(values){out[c as usize*2]+=1;out[c as usize*2+1]+=v;}out}
fn group_strings(a:&[String],b:&[String],values:&[u64])->[u64;12]{// String-backed keys are borrowed for hashing: no per-row String clone/allocation.
    let mut map:HashMap<(&String,&String),(u64,u64)>=HashMap::with_capacity(6);for ((a,b),&v)in a.iter().zip(b).zip(values){let e=map.entry((a,b)).or_default();e.0+=1;e.1+=v;}
    let mut out=[0;12];for ((a,b),(count,sum))in map{let code=flag_code(a.as_bytes()[0])*2+status_code(b.as_bytes()[0]);out[code as usize*2]=count;out[code as usize*2+1]=sum;}out
}
fn flag_code(x:u8)->u8{match x{b'A'=>0,b'N'=>1,b'R'=>2,_=>panic!("bad returnflag")}}
fn status_code(x:u8)->u8{match x{b'F'=>0,b'O'=>1,_=>panic!("bad linestatus")}}
fn m5(r:&mut Report,small:bool){let n=if small{3079}else{6_001_215};let aa=fs::read(format!("{DATA}/l_returnflag.bin")).unwrap();let bb=fs::read(format!("{DATA}/l_linestatus.bin")).unwrap();let a:Vec<String>=aa[..n].iter().map(|&x|(x as char).to_string()).collect();let b:Vec<String>=bb[..n].iter().map(|&x|(x as char).to_string()).collect();let codes:Vec<u8>=aa[..n].iter().zip(&bb[..n]).map(|(&a,&b)|flag_code(a)*2+status_code(b)).collect();let values=load("l_extendedprice",false,n);let reference=group_codes(&codes,&values);for t in [1,24]{let pool=rayon::ThreadPoolBuilder::new().num_threads(t).build().unwrap();pool.install(||bench(r,"M5","returnflag-linestatus-count-sum",t,n,&["plain_String","u8_codes"],&[(n*(48+2+8),n*(48+2+8)),(n*9,n*9)],&reference,|variant|{let group=|s:usize,e:usize|if variant==0{group_strings(&a[s..e],&b[s..e],&values[s..e])}else{group_codes(&codes[s..e],&values[s..e])};if t==1{group(0,n)}else{(0..n.div_ceil(B*CHUNK)).into_par_iter().map(|chunk|{let s=chunk*B*CHUNK;group(s,(s+B*CHUNK).min(n))}).reduce(||[0;12],|mut a,b|{for i in 0..12{a[i]+=b[i];}a})}},small));}}
fn correctness(){for w in 0..=64{let mut seed=328231;let mask=if w==64{u64::MAX}else{(1u64<<w)-1};let v:Vec<u64>=(0..2079).map(|_|rng(&mut seed)&mask).collect();for variant in 0..5{let c=Col::new(&v,variant,false);for b in 0..v.len().div_ceil(B){let mut out=[0;B];assert_eq!(c.block(b,&mut out),&v[b*B..((b+1)*B).min(v.len())]);}for(i,&x)in v.iter().enumerate(){assert_eq!(c.get(i),x);}}}for na in 0..40{for nb in 0..40{let a:Vec<u64>=(0..na).map(|x|x/2).collect();let b:Vec<u64>=(0..nb).map(|x|x/3).collect();let mut expected=a.clone();expected.extend(&b);expected.sort_unstable();for k in 0..=expected.len(){let(i,j)=corank(k,&a,&b);let mut prefix=vec![0;k];merge_segment(&a[..i],&b[..j],&mut prefix);assert_eq!(prefix,&expected[..k]);}}}eprintln!("codec widths 0..64, tails, dictionaries, and merge partitions exact");}
fn main(){let args:Vec<_>=std::env::args().collect();let small=args.iter().any(|x|x=="--small");let only=args.windows(2).find(|w|w[0]=="--only").map(|w|w[1].clone());if !small&&!args.iter().any(|x|x=="--machine-quiet"){panic!("full timing requires explicit --machine-quiet after parent authorization");}correctness();let mut report=Report{metadata:serde_json::json!({"date":"2026-10-28","cpu":command("lscpu",&[]),"rustc":command("rustc",&["-Vv"]),"flags":"-C target-cpu=native; release opt-level=3 lto=thin codegen-units=1","threads":[1,24],"rayon_fixed_chunk_rows":B*CHUNK,"warmups":5,"samples":21,"bootstrap_resamples":2000,"interleave":"rotate variant starting index every round","packing":"1024 rows/block; 16 interleaved u64 lanes, const-generic width-specialized decode with contiguous lane loop","synthetic":"xorshift64 uniform [0,2^w), seed 0x1234567812345678; threshold floor(2^w*pct/100), min 1; achieved selectivity in case","bytes_metric":"logical encoded reads incl base/width metadata and dictionary dereferences, not measured physical DRAM traffic; repeated random scalar reads count repeatedly; M2 bandwidth unavailable (0); M3 input bytes only (decode intermediates/output excluded)","q6":"shipdate [1994-01-01,1995-01-01), discount cents [5,7], quantity cents <2400; price late materialized scalar access","m3":"preallocated decode/output/reencode buffers; deterministic sorted u64 sequences 3*i and 29*i+1; output full exact each sample; output widths preconfigured, min/max recomputed and checked","m5":"actual dbgen String columns; borrowed String keys HashMap (no per-row cloning), u8 combined dictionary code dense counts/sums; fixed task-local state and reduction","driver":command("nvidia-smi",&["--query-gpu=name,driver_version,pstate,clocks.current.sm,clocks.current.memory","--format=csv"]),"initial_state":state(),"small_correctness_only":small}),windows:Vec::new(),rows:Vec::new()};if only.as_deref().is_none_or(|x|x=="M1"){m1(&mut report,small);}if only.as_deref().is_none_or(|x|x=="M2"){m2(&mut report,small);}if only.as_deref().is_none_or(|x|x=="M3"){m3(&mut report,small);}if only.as_deref().is_none_or(|x|x=="M5"){m5(&mut report,small);}if small{println!("small-size exactness checks passed; no timing samples retained");}else{save(&report);}}
