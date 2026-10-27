fn main() -> srd_doop_harness::Result<()> {
    let options = srd_doop_harness::Options::parse(false, false)?;
    srd_doop_harness::execute(options, "mini", 1, srd_doop_mini::run)
}
