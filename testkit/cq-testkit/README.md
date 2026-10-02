# cq-testkit

Public conformance tools for MiniLinq passes and external dialects. This crate
depends on language crates, never compiler passes or the facade. All fixtures
are included in the crate; consumers need no paths into a repository checkout.
Add `cq-testkit` as a **dev-dependency** in a core language, pass, runtime, or
facade crate. External feature crates can use the same API.

## Guarantees and premises

* `corpus` embeds 30 scenarios at CQ, valuation order (VO), relational algebra
  (RA), index demands (IX), Physical, and Structures stages. The IX and
  Structures texts are suffix fragments; their parsed accessors join them to
  the preceding stage. Parsing does not imply language contract validation.
  No policy fixtures are included. `cases` retains the 25 independently authored
  input/answer cases, including raw duplicate inputs. `Scenario::case` maps five
  alternative plans to their original authored cases.
* `values` models finite **Set semantics** over `i32`. `Rows` is a sorted set
  of ordered tuples; output column order matters and duplicate witnesses
  collapse. `Relation` provides named headings, projection, and natural join.
  The equality premise is **Leibniz equality**: equal values are interchangeable
  in every supported observation. Rust `i32` satisfies it. These oracles do not
  model custom `Eq`/`Ord`, bags, NULL, collation, effects, or evaluation counts.
* `cq_semantics` enumerates active-domain valuations and tests the original
  base relations. `semantics` independently interprets the RA DAG with nested
  loops. `valuation_semantics` interprets the written VO schedule and reductions.
  They never call pass compilation, planner, heading-analysis, or key helpers.
  Inputs must satisfy the language contracts, be finite, have the declared
  widths, and provide every declared table. The supported fragment consists of
  positive variable-only atoms, relational integer constraints, and total
  integer comparison guards. VO additionally assumes distinct variables within
  each base atom; repeated positions must be desugared before using that oracle.
  Literals/wildcards/expressions in atom arguments, computed bindings, negation,
  aggregation, arbitrary Rust, and extension clauses are outside this fragment.
  Unsupported expressions or malformed inputs may panic. Constraint parameters,
  when interpreting individual constraints, belong in the external binding map.
* `databases` supplies deterministic binary-domain tables by seed. Its empty
  and full seeds are 0 and 1. Domain enumeration is exponential in input arity
  and is intended for small conformance inputs.
* `harness` takes closures supplied by the caller. It checks compilation against
  an optional fixture, the source/target check, rejection of every supplied
  mutation, and optional execution against authored case answers. It validates
  only the fixtures and mutations supplied, not universal pass correctness.
  Checker panics propagate and are not treated as mutation rejection.
  Refinement helpers compare two executable closures: if the written-order
  reference returns, the target must return the same answer; if the reference
  panics, either target outcome is permitted (`bottom + Set`). They catch Rust
  unwinding panics, not aborts, and do not promise effect or evaluation-count
  equivalence.
* `fuzz` generates bounded abstract queries/data deterministically by seed
  using SplitMix64. Its MiniLinq and SQL printers consume that model separately;
  neither printer parses the other or invokes compilation. `Coverage` counts
  cases carrying each feature, once per feature per observation. Total queries
  have independently printable SQL. Partial Rust terms require executable
  written-order refinement; their SQL text is an explanatory comment.
* The optional `duckdb` feature enables `duckdb` (an in-memory integer SQL
  oracle) and `golden` (strict, byte-for-byte canonical CSV comparison and Git
  fixture inventory). SQL must select the desired set and ordering explicitly.
  `evaluate_fuzz` rejects partial terms and checks DISTINCT. Golden encoding
  requires sorted, distinct rows of the declared width; it rejects CRLF,
  leading zeros, and missing final newlines. Git inventory needs the Git CLI.
  Default builds do not build or link DuckDB. The bundled native engine can
  make the first feature build substantially slower.

## Fixture and oracle use

```rust
use cq_testkit::{CASES, cq_semantics};

for case in CASES {
    let source = case.parse_cq()?;
    cq_testkit::cq::contract::check(&source)?;
    assert_eq!(cq_semantics::evaluate_cq(&source, &case.database()), case.expected_rows());
}
# Ok::<(), syn::Error>(())
```

## Pass harness use

This tiny identity pass illustrates the closure API. Real passes supply their
own independent source/target checker and invalid-target mutations.

```rust
use cq_testkit::{PassCase, harness::PassFailure};

let pass = PassCase {
    source: 3_i32,
    expected_next: Some(3),
    compile: Box::new(|source: &i32| Ok::<_, &'static str>(*source)),
    check: Box::new(|source: &i32, target: &i32| {
        if source == target { Ok(()) } else { Err("not identity") }
    }),
    mutations: vec![Box::new(|target: &i32| target + 1)],
    answers: None,
};
assert_eq!(pass.run(None)?, 3);
# Ok::<(), PassFailure<&'static str, &'static str>>(())
```

`pass.run(Some(case))` uses `case.database()` and `case.expected_rows()` if an
`answers` closure is supplied. Supplying an answer closure without a case is an
error. `harness::check_refinement` and `check_refinement_on` accept executable
reference/target closures, so the compile-and-run driver stays in the caller.

The corpus and oracle implementations were copied from
`test-support/relational`; the abstract fuzz model and DuckDB/golden logic were
copied from `xtask`. The originals remain for a later caller migration.
