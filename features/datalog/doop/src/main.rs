fn main() -> doop_shared::harness::Result<()> {
    datalog_doop::harness::main(std::env::args().skip(1))
}
