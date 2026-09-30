```sh
cargo run --manifest-path examples/composition/Cargo.toml --locked
cargo test --manifest-path examples/own-dialect/Cargo.toml --locked
python3 examples/stacked-ir/check_expansions.py
python3 examples/diverged-ra/check.py
cargo run --example cq-inspection --locked
cargo run --example free-variables --locked
```
