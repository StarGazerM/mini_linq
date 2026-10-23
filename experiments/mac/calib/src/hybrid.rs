//! CPU x GPU contention: run one CPU work and one GPU work concurrently from a
//! shared start barrier and compare against each one's solo timing.

use crate::measure::{Measurement, Work, run_for, time_work};
use serde_json::json;
use std::sync::Barrier;

/// (cpu op, cpu threads, cpu footprint, gpu op, gpu footprint)
pub struct Pair {
    pub cpu_op: &'static str,
    pub cpu_threads: usize,
    pub cpu_fp: u64,
    pub gpu_op: &'static str,
    pub gpu_fp: u64,
}

const MIB: u64 = 1 << 20;
const GIB: u64 = 1 << 30;

pub const DEFAULT_PAIRS: &[Pair] = &[
    Pair { cpu_op: "seq_read", cpu_threads: 15, cpu_fp: GIB, gpu_op: "seq_read", gpu_fp: GIB },
    Pair { cpu_op: "gather_line", cpu_threads: 15, cpu_fp: GIB, gpu_op: "seq_copy", gpu_fp: GIB },
    Pair { cpu_op: "gather_line", cpu_threads: 15, cpu_fp: 64 * MIB, gpu_op: "seq_copy", gpu_fp: GIB },
    Pair { cpu_op: "gather_line", cpu_threads: 15, cpu_fp: GIB, gpu_op: "merge_path", gpu_fp: GIB },
    Pair { cpu_op: "hashbrown_probe", cpu_threads: 15, cpu_fp: 64 * MIB, gpu_op: "partition_pass", gpu_fp: GIB },
    Pair { cpu_op: "hashbrown_probe", cpu_threads: 15, cpu_fp: GIB, gpu_op: "partition_pass", gpu_fp: GIB },
    Pair { cpu_op: "hashbrown_probe", cpu_threads: 15, cpu_fp: GIB, gpu_op: "merge_path", gpu_fp: GIB },
    Pair { cpu_op: "merge_u32", cpu_threads: 15, cpu_fp: GIB, gpu_op: "gather_line", gpu_fp: GIB },
    Pair { cpu_op: "seq_read", cpu_threads: 5, cpu_fp: GIB, gpu_op: "gather_line", gpu_fp: GIB },
    Pair { cpu_op: "gather_line", cpu_threads: 5, cpu_fp: GIB, gpu_op: "gather_line", gpu_fp: GIB },
    Pair { cpu_op: "gather_line", cpu_threads: 10, cpu_fp: GIB, gpu_op: "gather_line", gpu_fp: GIB },
];

fn footprint(w: &dyn Work) -> u64 {
    w.params().get("footprint_bytes").and_then(|v| v.as_u64()).unwrap_or(0)
}

/// Pick the work with matching op whose footprint, then thread count, is
/// closest to the request (quick runs have smaller footprints / thread lists).
fn find<'a>(works: &'a [Box<dyn Work>], op: &str, threads: usize, fp: u64) -> Option<&'a dyn Work> {
    works
        .iter()
        .filter(|w| w.name() == op)
        .min_by_key(|w| (footprint(w.as_ref()).abs_diff(fp), w.threads().abs_diff(threads)))
        .map(|w| w.as_ref())
}

fn annotate(m: &mut Measurement, partner: &dyn Work, solo: &Measurement) {
    let p = m.params.as_object_mut().expect("params object");
    p.insert("partner".into(), json!(format!("{}:{}:{}", partner.processor(), partner.name(), footprint(partner))));
    p.insert("partner_threads".into(), json!(partner.threads()));
    p.insert("solo_median_ms".into(), json!(solo.median_ms));
    p.insert("solo_gbps".into(), json!(solo.gbps));
    p.insert("slowdown".into(), json!(m.median_ms / solo.median_ms));
}

pub fn run_pairs(cpu: &[Box<dyn Work>], gpu: &[Box<dyn Work>], seconds: f64) -> Vec<Measurement> {
    let mut out = Vec::new();
    for pair in DEFAULT_PAIRS {
        let Some(c) = find(cpu, pair.cpu_op, pair.cpu_threads, pair.cpu_fp) else {
            eprintln!("hybrid: no cpu work {}({}, {} MiB); skipping", pair.cpu_op, pair.cpu_threads, pair.cpu_fp / MIB);
            continue;
        };
        let Some(g) = find(gpu, pair.gpu_op, 1, pair.gpu_fp) else {
            eprintln!("hybrid: no gpu work {}({} MiB); skipping", pair.gpu_op, pair.gpu_fp / MIB);
            continue;
        };
        eprintln!(
            "hybrid: cpu {}({}, {} MiB) x gpu {}({} MiB)",
            c.name(),
            c.threads(),
            footprint(c) / MIB,
            g.name(),
            footprint(g) / MIB
        );
        let solo_c = time_work(c, 2, 7, None);
        let solo_g = time_work(g, 2, 7, None);
        let barrier = Barrier::new(2);
        let (mut mc, mut mg) = std::thread::scope(|s| {
            let h = s.spawn(|| run_for(c, seconds, Some(&barrier)));
            let mg = run_for(g, seconds, Some(&barrier));
            (h.join().expect("cpu partner panicked"), mg)
        });
        annotate(&mut mc, g, &solo_c);
        annotate(&mut mg, c, &solo_g);
        out.push(mc);
        out.push(mg);
    }
    out
}
