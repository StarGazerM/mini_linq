fn main() -> oracle_shared::Result<()> {
    datalog_oracle::harness::run(&std::env::args().skip(1).collect::<Vec<_>>())
}
