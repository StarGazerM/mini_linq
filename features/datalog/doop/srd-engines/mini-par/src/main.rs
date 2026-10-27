fn main() -> srd_doop_harness::Result<()> {
    let options = srd_doop_harness::Options::parse(true, false)?;
    let threads = options.parallel_threads()?;
    rayon::ThreadPoolBuilder::new().num_threads(threads).build_global()?;
    srd_doop_harness::execute(options, "mini-par", threads, srd_doop_mini_par::run)
}
