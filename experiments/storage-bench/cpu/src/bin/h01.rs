use ahash::{AHasher, RandomState};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::hash::{BuildHasher, BuildHasherDefault, Hash, Hasher};
use std::hint::black_box;
use std::sync::LazyLock;
use std::{fs, process::Command, time::Instant};

const DATA: &str = "/data/mini_linq/target/storage-bench/tpch-sf1";
const OUT: &str = "/data/mini_linq/experiments/storage-bench/results/cpu-h01-decomposition-2026-10-29.json";
const CUTOFF: i32 = 10471;
type Row = (i64, i64, String, String, i64, i64, i32);
type CodeRow = (i64, i64, u8, u8, i64, i64, i32);
type Answer = [[i64; 3]; 6];
type Key9<'a> = (&'a i64, &'a String, &'a i64, &'a i64, i64, &'a i64, i64, &'a String, &'a i32);

// Historical pre-fix, counters-disabled wrapper. This is a control, NOT the
// current engine or the counter-instrumented binary (which used SipHash).
struct EngineHasher(AHasher);
impl Default for EngineHasher {
    fn default() -> Self { Self(RandomState::new().build_hasher()) }
}
impl Hasher for EngineHasher {
    fn write(&mut self, b: &[u8]) { self.0.write(b); }
    fn finish(&self) -> u64 { self.0.finish() }
}
// Control: identical wrapper API, but stable per-process keys. Direct uses
// those same keys with AHasher's specialized primitive write methods intact.
static STABLE_STATE: LazyLock<RandomState> = LazyLock::new(|| RandomState::with_seeds(11, 22, 33, 44));
fn stable_state() -> &'static RandomState { &STABLE_STATE }
struct StableWrapper(AHasher);
impl Default for StableWrapper {
    fn default() -> Self { Self(stable_state().build_hasher()) }
}
impl Hasher for StableWrapper {
    fn write(&mut self, b: &[u8]) { self.0.write(b); }
    fn finish(&self) -> u64 { self.0.finish() }
}
#[derive(Clone)]
struct DirectState(RandomState);
impl Default for DirectState {
    fn default() -> Self { Self(stable_state().clone()) }
}
impl BuildHasher for DirectState {
    type Hasher = AHasher;
    fn build_hasher(&self) -> AHasher { self.0.build_hasher() }
}
fn key9(r: &Row) -> Key9<'_> { (&r.1, &r.3, &r.0, &r.5, r.5, &r.4, r.4, &r.2, &r.6) }
fn code(a: u8, b: u8) -> usize {
    let f = match a { b'A' => 0, b'N' => 1, b'R' => 2, _ => panic!("flag") };
    let s = match b { b'F' => 0, b'O' => 1, _ => panic!("status") };
    f * 2 + s
}
fn add(out: &mut Answer, c: usize, q: i64, p: i64) { out[c][0] += q; out[c][1] += p; out[c][2] += 1; }
fn finish<S: BuildHasher>(m: HashMap<(&String, &String), [i64; 3], S>) -> Answer {
    let mut out = [[0; 3]; 6];
    for ((f,s),v) in m { out[code(f.as_bytes()[0],s.as_bytes()[0])] = v; }
    out
}
fn dedup9<S: BuildHasher + Default>(rows: &[Row], reserve: bool) -> Answer {
    let mut seen = HashSet::with_capacity_and_hasher(if reserve { rows.len() } else { 0 }, S::default());
    let mut groups = HashMap::new();
    for r in rows {
        if r.6 <= CUTOFF && seen.insert(key9(r).clone()) {
            let e = groups.entry((&r.2,&r.3)).or_insert([0;3]);
            e[0] += r.4; e[1] += r.5; e[2] += 1;
        }
    }
    finish(groups)
}
fn dedup_row(rows: &[Row]) -> Answer {
    let mut seen = HashSet::<_,DirectState>::default();
    let mut groups = HashMap::new();
    for r in rows {
        if r.6 <= CUTOFF && seen.insert(r) {
            let e = groups.entry((&r.2,&r.3)).or_insert([0;3]);
            e[0] += r.4; e[1] += r.5; e[2] += 1;
        }
    }
    finish(groups)
}
fn owned_control(rows: &[Row]) -> Answer {
    let mut seen = HashSet::<_,DirectState>::default();
    let mut groups = HashMap::new();
    for r in rows {
        if r.6 <= CUTOFF && seen.insert((r.1,r.3.clone(),r.0,r.5,r.5,r.4,r.4,r.2.clone(),r.6)) {
            let e = groups.entry((&r.2,&r.3)).or_insert([0;3]);
            e[0] += r.4; e[1] += r.5; e[2] += 1;
        }
    }
    finish(groups)
}
fn no_dedup<S: BuildHasher + Default>(rows: &[Row], hash_only: bool) -> Answer {
    let mut groups = HashMap::with_hasher(S::default());
    for r in rows {
        if r.6 <= CUTOFF {
            if hash_only { let mut h = stable_state().build_hasher(); key9(r).hash(&mut h); black_box(h.finish()); }
            let e = groups.entry((&r.2,&r.3)).or_insert([0;3]);
            e[0] += r.4; e[1] += r.5; e[2] += 1;
        }
    }
    finish(groups)
}
fn encoded(rows: &[CodeRow], dedup: bool) -> Answer {
    let mut out = [[0;3];6];
    let mut seen = HashSet::<_,DirectState>::default();
    for r in rows {
        if r.6 <= CUTOFF {
            // Match H's nine-field borrowed tuple + two scaled values.
            let key = (&r.1,&r.3,&r.0,&r.5,r.5,&r.4,r.4,&r.2,&r.6);
            if !dedup || seen.insert(key) { add(&mut out, (r.2*2+r.3) as usize, r.4, r.5); }
        }
    }
    out
}
struct Columns { flag: Vec<u8>, status: Vec<u8>, quantity: Vec<i64>, price: Vec<i64>, date: Vec<i32> }
fn columnar(c: &Columns) -> Answer {
    let mut out = [[0;3];6];
    for i in 0..c.date.len() {
        if c.date[i] <= CUTOFF { add(&mut out,(c.flag[i]*2+c.status[i]) as usize,c.quantity[i],c.price[i]); }
    }
    out
}
fn bytes(name: &str) -> Vec<u8> { fs::read(format!("{DATA}/{name}.bin")).unwrap() }
fn longs(name: &str) -> Vec<i64> { bytes(name).chunks_exact(8).map(|b|i64::from_le_bytes(b.try_into().unwrap())).collect() }
fn load(n: usize) -> (Vec<Row>, Vec<CodeRow>, Columns) {
    let keys=longs("l_orderkey"); let numbers=longs("l_linenumber");
    let quantity=longs("l_quantity"); let price=longs("l_extendedprice");
    let flags=bytes("l_returnflag"); let statuses=bytes("l_linestatus");
    let dates:Vec<i32>=bytes("l_shipdate").chunks_exact(4).map(|b|i32::from_le_bytes(b.try_into().unwrap())).collect();
    let n=n.min(keys.len());
    let rows=(0..n).map(|i|(keys[i],numbers[i],(flags[i] as char).to_string(),(statuses[i] as char).to_string(),quantity[i],price[i],dates[i])).collect();
    let codes=(0..n).map(|i|(keys[i],numbers[i],(code(flags[i],b'F')/2) as u8,(code(b'A',statuses[i])%2) as u8,quantity[i],price[i],dates[i])).collect();
    let c=Columns {flag:(0..n).map(|i|(code(flags[i],b'F')/2) as u8).collect(),status:(0..n).map(|i|(code(b'A',statuses[i])%2) as u8).collect(),quantity:quantity[..n].to_vec(),price:price[..n].to_vec(),date:dates[..n].to_vec()};
    (rows,codes,c)
}
fn command(p:&str,a:&[&str])->String { let o=Command::new(p).args(a).output().unwrap();String::from_utf8_lossy(&o.stdout).to_string()+&String::from_utf8_lossy(&o.stderr) }
fn state()->Value {
    let ps=command("ps",&["-eo","pid,pcpu,args","--sort=-pcpu"]);
    let heavy:Vec<_>=ps.lines().skip(1).filter(|line| {let mut f=line.split_whitespace();let pid=f.next().unwrap_or("0").parse::<u32>().unwrap_or(0);let cpu=f.next().unwrap_or("0").parse::<f64>().unwrap_or(0.);let exe=f.next().unwrap_or("");pid!=std::process::id()&&cpu>=20.&&exe!="ps"}).collect();
    json!({"utc":command("date",&["-u","+%FT%TZ"]),"uptime":command("uptime",&[]),"top_cpu_processes":ps.lines().take(6).collect::<Vec<_>>(),"heavy_external_cpu_processes":heavy,"gpu_processes":command("nvidia-smi",&["--query-compute-apps=pid,used_memory","--format=csv,noheader"]),"gpu_clocks":command("nvidia-smi",&["-q","-d","CLOCK"]),"cpu_mhz":fs::read_to_string("/proc/cpuinfo").unwrap().lines().filter(|x|x.starts_with("cpu MHz")).collect::<Vec<_>>()})
}
fn busy(v:&Value)->bool { !v["heavy_external_cpu_processes"].as_array().unwrap().is_empty() || !v["gpu_processes"].as_str().unwrap().trim().is_empty() }
fn random(s:&mut u64)->usize { *s^=*s<<13;*s^=*s>>7;*s^=*s<<17;*s as usize }
fn med(v:&[f64])->f64 { let mut a=v.to_vec();a.sort_by(f64::total_cmp);a[a.len()/2] }
fn bootstrap(a:&[f64],b:Option<&[f64]>)->[f64;2] {
    let mut seed=123456789;let mut out=Vec::new();
    for _ in 0..2000 { let ids:Vec<_>=(0..a.len()).map(|_|random(&mut seed)%a.len()).collect();let aa:Vec<_>=ids.iter().map(|&i|a[i]).collect();let value=if let Some(b)=b {let bb:Vec<_>=ids.iter().map(|&i|b[i]).collect();med(&aa)/med(&bb)}else{med(&aa)};out.push(value); }
    out.sort_by(f64::total_cmp);[out[50],out[1950]]
}
fn save(v:&Value) {fs::write(OUT,serde_json::to_vec_pretty(v).unwrap()).unwrap();}
fn main() {
    let small=std::env::args().any(|s|s=="--small");
    assert!(small||std::env::args().any(|s|s=="--machine-quiet"));
    let (rows,codes,columns)=load(if small{4099}else{6_001_215});
    // Actual dbgen keys are sorted. Verify uniqueness without relying on any
    // experimental hasher; no-dedup variants are legal only after this check.
    assert!(rows.windows(2).all(|r|(r[0].0,r[0].1)<(r[1].0,r[1].1)));
    let names=["A_legacy_fresh_ahash_wrapper","B_H_reserved","C_borrowed_Row","D_codes_dedup_array","E_no_dedup_String_ahash","F_no_dedup_codes_rows","G_columnar_codes","H_engine_direct_stable_ahash","I_stable_wrapper_control","J_owned_clone_control","K_no_dedup_String_SipHash","L_hashonly_direct_ahash_String_SipHash"];
    let run=|v:usize|match v {
        0=>dedup9::<BuildHasherDefault<EngineHasher>>(&rows,false),
        1=>dedup9::<DirectState>(&rows,true),
        2=>dedup_row(&rows),3=>encoded(&codes,true),
        4=>no_dedup::<DirectState>(&rows,false),5=>encoded(&codes,false),6=>columnar(&columns),
        7=>dedup9::<DirectState>(&rows,false),8=>dedup9::<BuildHasherDefault<StableWrapper>>(&rows,false),
        9=>owned_control(&rows),10=>no_dedup::<std::collections::hash_map::RandomState>(&rows,false),
        11=>no_dedup::<std::collections::hash_map::RandomState>(&rows,true),_=>unreachable!()
    };
    let reference=run(7);
    assert_eq!(reference,columnar(&columns));
    let mut report=json!({"experiment":"M6","metadata":{"n":rows.len(),"threads":1,"warmups":5,"samples":21,"bootstrap_resamples":2000,"cutoff_epoch_days":CUTOFF,"input_key_uniqueness_verified":true,"qualifying_rows":reference.iter().map(|x|x[2]).sum::<i64>(),"answer":["sum_quantity_cents","sum_price_cents","count"],"reference":reference,"group_order":["A/F","A/O","N/F","N/O","R/F","R/O"],"flags":"-C target-cpu=native; opt-level=3; lto=thin; codegen-units=1","rustc":command("rustc",&["-Vv"]),"cpu":command("lscpu",&[]),"gpu_driver":command("nvidia-smi",&["--query-gpu=name,driver_version","--format=csv"]),"row_bytes":std::mem::size_of::<Row>(),"code_row_bytes":std::mem::size_of::<CodeRow>(),"borrowed_key9_bytes":std::mem::size_of::<Key9>(),"counting_hasher":"write/finish-only wrapper; Default constructs fresh ahash::RandomState, matching current counters.rs implementation with counters disabled","current_generated_alias":"current.normalized.rs:11 instead names cq_storage::HashSet with direct HashState; H models this direct hasher API with fixed stable keys","group_keys":"borrowed &String pairs, as Rows::Row is Copy; no per-row String clone in A","small":small},"windows":[],"rows":[]});
    report["metadata"]["primary_engine_variant"]=json!(names[7]);
    report["metadata"]["counting_hasher"]=json!("A freezes historical pre-fix counters-disabled fresh RandomState wrapper; not current engine and not actual counters-enabled profile");
    report["metadata"]["current_generated_alias"]=json!("H: direct per-set ahash, matching uninstrumented Dedup alias API; stable fixed seeds for paired measurement");
    report["metadata"]["prior_cancelled_attempt"]=json!("The initial A-based matrix was cancelled without reporting samples after binary-feature provenance established H as the appropriate primary; B/C/D/J and L now use direct stable ahash.");
    if small {for v in 0..names.len(){assert_eq!(run(v),reference,"{}",names[v]);}println!("M6 small exactness and key uniqueness passed (12 variants)");return;}
    // Repeat the matrix if any round boundary sees external CPU/GPU activity.
    // Retain all rejected samples and boundary records.
    let samples=loop {
        let before=state();
        if busy(&before) {report["windows"].as_array_mut().unwrap().push(json!({"before":before,"after":before,"accepted":false,"reason":"busy before matrix"}));save(&report);std::thread::sleep(std::time::Duration::from_secs(15));continue;}
        let mut samples=vec![Vec::new();names.len()];
        let mut round_states=Vec::new();
        let mut contaminated=false;
        for round in 0..26 {
            for j in 0..names.len(){
                let v=(j+round)%names.len();let start=Instant::now();
                let got=black_box(run(v));let ns=start.elapsed().as_nanos() as f64;
                assert_eq!(got,reference,"{} round {round}",names[v]);
                if round>=5{samples[v].push(ns);}
            }
            let observed=state();contaminated=busy(&observed);
            round_states.push(json!({"round":round,"state":observed}));
            eprintln!("M6 round {round} exact");
            if contaminated{break;}
        }
        let after=state();let accepted=!contaminated&&!busy(&after);
        report["windows"].as_array_mut().unwrap().push(json!({"before":before,"after":after,"round_states":round_states,"accepted":accepted,"samples_ns":if accepted{Value::Null}else{json!(samples)}}));
        save(&report);if accepted{break samples;}eprintln!("M6 discarding contaminated matrix and rerunning");
    };
    for v in 0..names.len(){let m=med(&samples[v]);report["rows"].as_array_mut().unwrap().push(json!({"variant":names[v],"samples_ns":samples[v],"median_ns":m,"ci95_ns":bootstrap(&samples[v],None),"ratio_to_H":m/med(&samples[7]),"ratio_to_H_ci95":bootstrap(&samples[v],Some(&samples[7])),"ratio_to_A":m/med(&samples[0]),"ratio_ci95":bootstrap(&samples[v],Some(&samples[0])),"ns_per_input_row":m/rows.len() as f64,"exact":true}));}
    let contrasts=[("growth_including_rehash",7,1),("borrowed9_to_row_reference",7,2),("codes_with_dedup_and_array_groups",7,3),("remove_dedup_keep_String_SipHash",7,10),("grouping_SipHash_to_ahash",10,4),("String_ahash_to_code_array",4,5),("row_to_columnar_codes",5,6),("legacy_wrapper_to_engine_direct",0,7),("fresh_state_to_stable_state_same_wrapper",0,8),("wrapper_API_vs_direct_primitive_writes",8,7),("owned_clone_control_over_borrowed_H",9,7),("standalone_direct_ahash_cost",11,10)];
    let cs:Vec<_>=contrasts.iter().map(|&(name,a,b)|{let differences:Vec<_>=samples[a].iter().zip(&samples[b]).map(|(a,b)|a-b).collect();json!({"name":name,"from":names[a],"to":names[b],"difference_of_medians_ns":med(&samples[a])-med(&samples[b]),"paired_round_difference_median_ns":med(&differences),"paired_difference_ci95_ns":bootstrap(&differences,None),"ratio_to_from":med(&samples[b])/med(&samples[a]),"ratio_ci95":bootstrap(&samples[b],Some(&samples[a]))})}).collect();
    report["contrasts"]=json!(cs);save(&report);println!("M6 complete: {} variants, exact on every sample",names.len());
}
