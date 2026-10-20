fn main() -> datalog_doop::harness::Result<()> {
    datalog_doop::harness::main(std::env::args().skip(1))
}
