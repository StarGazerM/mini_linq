fn main() {
    for case in cq_workloads::catalog::all() {
        eprintln!("hypergraph {} begin", case.name);
        let started = std::time::Instant::now();
        let result = cq_workloads::pipelines::Pipeline {
            plan: 2,
            ..Default::default()
        }
        .physical(&case.module().unwrap());
        eprintln!(
            "hypergraph {} {:.3}s {}",
            case.name,
            started.elapsed().as_secs_f64(),
            if result.is_ok() { "ok" } else { "error" }
        );
        result.unwrap();
    }
}
