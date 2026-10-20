fn main() -> datalog_oracle::Result<()> {
    datalog_parallel_oracle::harness::run(&std::env::args().skip(1).collect::<Vec<_>>())
}
