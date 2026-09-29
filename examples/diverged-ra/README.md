# Independent theta joins through the downstream compiler

`ra-theta` parses `theta_join … with … if …; output …;`, desugars it into
RA renames, natural join, selection and projection, and derives its own IX
demands. It returns a complete `ix::Program`. The unchanged lowering,
selection and realization passes produce Rust from that program.

`build.rs` writes the generated expression into `OUT_DIR/query.rs`; rustc
compiles it and the tests compare probe and merge execution with a nested-loop
oracle. Coverage includes duplicate and empty inputs and 256 pairs of subsets.

Run `python3 examples/diverged-ra/check.py`.

Allowed internal crates are `cq-syntax`, `cq-lang-ra`, `cq-lang-ix`,
`cq-lang-physical`, `cq-lang-structures`, `cq-physical-lowering`,
`cq-structure-selection`, `cq-physical-realization`, `cq-print`, `cq-storage`,
and `ra-theta`. The checker rejects `cq-lang-cq`, `cq-lang-vo`, `cq-order`,
`cq-reduce`, `cq-elaborate`, `cq-index-policy` and `mini-linq`, and checks
resolved manifest paths. A surface requiring a new operator such as union
would need a corresponding downstream language and implementation.
