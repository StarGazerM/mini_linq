//! Apple-silicon primitive-cost calibration driver.
//!
//! maccal <cpu|gpu|hybrid|all> [--footprints 4MiB,64MiB,1GiB,4GiB] [--threads 1,5,10,15]
//!        [--reps 9] [--seconds 2.0] [--probes 32M] [--seed 42] [--quick]

mod cpu;
mod gpu;
mod hybrid;
mod measure;

use cpu::CpuCfg;
use gpu::{GIB, GpuCfg, MIB};
use measure::{Measurement, Work, time_work};
use std::time::{SystemTime, UNIX_EPOCH};

struct Opts {
    sub: String,
    footprints: Vec<u64>,
    threads: Vec<usize>,
    reps: usize,
    seconds: f64,
    probes: u64,
    seed: u64,
    quick: bool,
    /// Only run works whose name is in this list (empty = all).
    ops: Vec<String>,
    /// Suffix for the results file stem.
    tag: String,
}

fn parse_size(s: &str) -> u64 {
    let s = s.trim();
    let (num, mul) = if let Some(n) = s.strip_suffix("GiB").or(s.strip_suffix("G")) {
        (n, GIB)
    } else if let Some(n) = s.strip_suffix("MiB").or(s.strip_suffix("M")) {
        (n, MIB)
    } else if let Some(n) = s.strip_suffix("KiB").or(s.strip_suffix("K")) {
        (n, 1024)
    } else {
        (s, 1)
    };
    (num.trim().parse::<f64>().unwrap_or_else(|_| panic!("bad size {s}")) * mul as f64) as u64
}

fn parse_args() -> Opts {
    let mut args = std::env::args().skip(1);
    let mut o = Opts {
        sub: String::new(),
        footprints: vec![4 * MIB, 64 * MIB, GIB, 4 * GIB],
        threads: vec![1, 5, 10, 15],
        reps: 9,
        seconds: 2.0,
        probes: 32 << 20,
        seed: 42,
        quick: false,
        ops: Vec::new(),
        tag: String::new(),
    };
    let mut fp_set = false;
    let mut th_set = false;
    let mut reps_set = false;
    let mut sec_set = false;
    let mut probes_set = false;
    while let Some(a) = args.next() {
        let val = |args: &mut dyn Iterator<Item = String>| args.next().unwrap_or_else(|| panic!("{a} needs a value"));
        match a.as_str() {
            "cpu" | "gpu" | "hybrid" | "all" => o.sub = a.clone(),
            "--footprints" => {
                o.footprints = val(&mut args).split(',').map(parse_size).collect();
                fp_set = true;
            }
            "--threads" => {
                o.threads = val(&mut args).split(',').map(|t| t.trim().parse().expect("thread count")).collect();
                th_set = true;
            }
            "--reps" => {
                o.reps = val(&mut args).parse().expect("reps");
                reps_set = true;
            }
            "--seconds" => {
                o.seconds = val(&mut args).parse().expect("seconds");
                sec_set = true;
            }
            "--probes" => {
                o.probes = parse_size(&val(&mut args));
                probes_set = true;
            }
            "--seed" => o.seed = val(&mut args).parse().expect("seed"),
            "--quick" => o.quick = true,
            "--ops" => o.ops = val(&mut args).split(',').map(|s| s.trim().to_string()).collect(),
            "--tag" => o.tag = val(&mut args),
            _ => panic!("unknown argument {a}"),
        }
    }
    if o.sub.is_empty() {
        eprintln!("usage: maccal <cpu|gpu|hybrid|all> [--footprints L] [--threads L] [--reps N] [--seconds S] [--probes N] [--seed N] [--quick]");
        std::process::exit(2);
    }
    if o.quick {
        if !fp_set {
            o.footprints = vec![4 * MIB, 64 * MIB];
        }
        if !th_set {
            o.threads = vec![1, 5];
        }
        if !reps_set {
            o.reps = 3;
        }
        if !sec_set {
            o.seconds = 0.5;
        }
        if !probes_set {
            o.probes = 2 << 20;
        }
    }
    o
}

fn today() -> String {
    // civil date from days since epoch (Howard Hinnant's algorithm)
    let days = (SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs() / 86400) as i64;
    let z = days + 719468;
    let era = z.div_euclid(146097);
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!("{y:04}-{m:02}-{d:02}")
}

fn fmt_size(b: u64) -> String {
    if b == 0 {
        "-".into()
    } else if b >= GIB && b % GIB == 0 {
        format!("{} GiB", b / GIB)
    } else if b >= MIB {
        format!("{} MiB", b / MIB)
    } else {
        format!("{} KiB", b / 1024)
    }
}

fn table(ms: &[Measurement]) -> String {
    let mut s = String::from("| op | proc | thr | footprint | GB/s | ns/item | median ms | gpu ms | notes |\n|---|---|---:|---:|---:|---:|---:|---:|---|\n");
    for m in ms {
        let p = m.params.as_object().cloned().unwrap_or_default();
        let fp = p.get("footprint_bytes").and_then(|v| v.as_u64()).unwrap_or(0);
        let gpu_ms = p.get("gpu_ms").and_then(|v| v.as_f64()).map(|v| format!("{v:.3}")).unwrap_or_else(|| "-".into());
        let notes: Vec<String> = p
            .iter()
            .filter(|(k, _)| !matches!(k.as_str(), "footprint_bytes" | "gpu_ms" | "probes"))
            .map(|(k, v)| match v {
                serde_json::Value::Number(n) if n.as_f64().is_some_and(|f| f.fract() != 0.0) => format!("{k}={:.3}", n.as_f64().unwrap()),
                serde_json::Value::String(t) => format!("{k}={t}"),
                _ => format!("{k}={v}"),
            })
            .collect();
        s += &format!(
            "| {} | {} | {} | {} | {:.1} | {:.2} | {:.3} | {} | {} |\n",
            m.op,
            m.processor,
            m.threads,
            fmt_size(fp),
            m.gbps,
            m.ns_per_item,
            m.median_ms,
            gpu_ms,
            notes.join(" ")
        );
    }
    s
}

/// Times each work and drops it immediately so lazily-allocated buffers do not accumulate.
fn run_all(works: Vec<Box<dyn Work>>, reps: usize, ops: &[String]) -> Vec<Measurement> {
    let mut out = Vec::with_capacity(works.len());
    for w in works.into_iter().filter(|w| ops.is_empty() || ops.iter().any(|o| *o == w.name())) {
        eprint!("  {} {} ...", w.name(), fmt_size(w.params().get("footprint_bytes").and_then(|v| v.as_u64()).unwrap_or(0)));
        let m = time_work(w.as_ref(), 2, reps, None);
        eprintln!(" {:.1} GB/s {:.2} ns/item ({:.3} ms)", m.gbps, m.ns_per_item, m.median_ms);
        out.push(m);
    }
    out
}

fn cpu_cfg(o: &Opts, footprints: Vec<u64>, threads: Vec<usize>) -> CpuCfg {
    CpuCfg { threads, footprint_bytes: footprints, seed: o.seed }
}

fn gpu_cfg(o: &Opts, footprints: Vec<u64>) -> GpuCfg {
    GpuCfg { footprints, probes: o.probes, seed: o.seed }
}

fn phase_cpu(o: &Opts) -> Vec<Measurement> {
    eprintln!("== cpu threads={:?} footprints={:?}", o.threads, o.footprints.iter().map(|&f| fmt_size(f)).collect::<Vec<_>>());
    let works = cpu::cpu_works(&cpu_cfg(o, o.footprints.clone(), o.threads.clone()));
    run_all(works, o.reps, &o.ops)
}

fn phase_gpu(o: &Opts) -> Vec<Measurement> {
    eprintln!("== gpu footprints={:?} probes={}", o.footprints.iter().map(|&f| fmt_size(f)).collect::<Vec<_>>(), o.probes);
    let works = gpu::gpu_works(&gpu_cfg(o, o.footprints.clone()));
    run_all(works, o.reps, &o.ops)
}

fn phase_hybrid(o: &Opts) -> Vec<Measurement> {
    // exactly the footprints/threads the pair table asks for (quick: capped to the
    // requested max footprint), so both pools stay small and matches are exact
    let cap = o.footprints.iter().copied().max().unwrap_or(GIB);
    let mut fps: Vec<u64> = hybrid::DEFAULT_PAIRS.iter().map(|p| p.cpu_fp.min(cap)).collect();
    fps.sort_unstable();
    fps.dedup();
    let mut gpu_fp: Vec<u64> = hybrid::DEFAULT_PAIRS.iter().map(|p| p.gpu_fp.min(cap)).collect();
    gpu_fp.sort_unstable();
    gpu_fp.dedup();
    let mut threads: Vec<usize> = hybrid::DEFAULT_PAIRS.iter().map(|p| p.cpu_threads).collect();
    threads.sort_unstable();
    threads.dedup();
    eprintln!("== hybrid seconds={} cpu footprints={:?} threads={:?}", o.seconds, fps.iter().map(|&f| fmt_size(f)).collect::<Vec<_>>(), threads);
    let cpu = cpu::cpu_works(&cpu_cfg(o, fps, threads));
    let gpu = gpu::gpu_works(&gpu_cfg(o, gpu_fp));
    hybrid::run_pairs(&cpu, &gpu, o.seconds)
}

fn save(sub: &str, tag: &str, ms: &[Measurement]) {
    std::fs::create_dir_all("results").expect("results dir");
    let stem = if tag.is_empty() { format!("results/{sub}-{}", today()) } else { format!("results/{sub}-{tag}-{}", today()) };
    std::fs::write(format!("{stem}.json"), serde_json::to_string_pretty(ms).unwrap()).expect("write json");
    let md = table(ms);
    std::fs::write(format!("{stem}.md"), &md).expect("write md");
    println!("\n### {sub}\n\n{md}");
    eprintln!("wrote {stem}.json / .md");
}

fn main() {
    let o = parse_args();
    let dev = metal::Device::system_default().expect("no Metal device");
    eprintln!(
        "{} unified={} maxbuf={} MiB wss={} MiB quick={} reps={}",
        dev.name(),
        dev.has_unified_memory(),
        dev.max_buffer_length() >> 20,
        dev.recommended_max_working_set_size() >> 20,
        o.quick,
        o.reps
    );
    match o.sub.as_str() {
        "cpu" => save("cpu", &o.tag, &phase_cpu(&o)),
        "gpu" => save("gpu", &o.tag, &phase_gpu(&o)),
        "hybrid" => save("hybrid", &o.tag, &phase_hybrid(&o)),
        "all" => {
            let c = phase_cpu(&o);
            save("cpu", &o.tag, &c);
            let g = phase_gpu(&o);
            save("gpu", &o.tag, &g);
            let h = phase_hybrid(&o);
            save("hybrid", &o.tag, &h);
        }
        _ => unreachable!(),
    }
}
