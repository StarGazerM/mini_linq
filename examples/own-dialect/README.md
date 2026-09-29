# Own printer dialect

`own-push` consumes the reference Structures payload and emits nested `for`
loops into a set sink. It uses `cq-print` for row layouts and host/built index
access, `cq-protocol` for its sink requests, and `cq-stage` for its proc-macro
entry. The application chooses the earlier reference passes and supplies
`cq-storage`; it never depends on the reference pull printer.

The tests reuse the `built` and `generic` queries from `physical_query.rs` and
compare their answers with independent nested-loop oracles, including empty
and duplicate inputs. The small dialect supports unit, rename, project,
select, probe and semijoin, and diagnoses unsupported physical forms.

Run `cargo test --manifest-path examples/own-dialect/Cargo.toml --locked`.
