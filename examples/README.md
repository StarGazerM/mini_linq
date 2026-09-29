# MiniLinq examples

Each example proves one boundary claim of the [design](../doc/DESIGN.md).
`tools/test_regressions.sh` runs all of them.

## composition

A caller-written host composes RA → IX → Physical → Structures → Rust by
binding every macro slot itself: `physical_policy!` appends an empty,
query-independent `policy {}` and invokes `lower_physical!`, and
`select_structures!`, `structures_to_rust!` and a `cq-state-providers`
`distinct_state!` and `index_provider!` providers are imported under the facade's slot names. Host
types provide keyed reads on leading declared columns, so dispatch probes the
caller's `HashMap` and `BTreeMap` in place, builds nothing, and returns
borrowed answers.

```sh
cargo run --manifest-path examples/composition/Cargo.toml --locked
```

## cq-inspection

Parses one query into the typed CQ program (`cq_lang_cq::Program`) and walks
its head and body: the source language is an ordinary Rust value.

```sh
cargo run --example cq-inspection
```

## free-variables

Runs `cq::rust::free_variables` over atom arguments, guards, `let`
initializers and the parsed negation/aggregate syntax, reporting the names
each embedded Rust expression uses, whether it is opaque (macros), and which
logical inputs it depends on.

```sh
cargo run --example free-variables
```

## own-dialect

Swaps the printer. `own-push` consumes only a complete Structures program and
emits nested `for` loops into a caller-bound set sink, using `cq-print` for
row layouts and host/built access, `cq-protocol` for sink requests and
`cq-stage` for its entry; the application never depends on the pull printer.
Tests run the `built` and `generic` queries of `physical_query.rs` against
nested-loop oracles, including empty and duplicate inputs; unsupported
physical forms are diagnosed.

```sh
cargo test --manifest-path examples/own-dialect/Cargo.toml --locked
```

## stacked-ir

Stacks a stage. The caller replaces the Physical → Structures continuation
with Physical → Trace → Physical → Structures: Trace extends the complete
Physical program with an empty `trace {}` annotation, and two ordinary passes
add and erase it through the same `cq-stage::Policed` ABI. The script runs the
facade's `physical_query.rs` through both routes and requires every expanded
`run` function to be byte-identical (needs `cargo expand`).

```sh
python3 examples/stacked-ir/check_expansions.py
```

## diverged-ra

Diverges from the frontend. `ra-theta` parses
`theta_join … with … if …; output …;`, desugars it into RA renames, natural
join, selection and projection, derives its own IX demands and returns a
complete `ix::Program`; the unchanged lowering, selection and realization
passes produce the Rust. Tests compare probe and merge execution with a
nested-loop oracle, including duplicate and empty inputs and 256 subset
pairs. The script also rejects any dependency on `cq-lang-cq`, `cq-lang-vo`,
`cq-order`, `cq-reduce`, `cq-elaborate`, `cq-index-policy` or `mini-linq`,
and checks resolved manifest paths.

```sh
python3 examples/diverged-ra/check.py
```
