# Homework 1: CQ to RelationalPlan

Homework 1 follows the typed CQ-to-RelationalPlan pass chain:

```text
cq::Module -- cq::normal::normalize --> CQ-NF
           -- cq_order::compile (Written, Coarse) --> valuation::Module
           -- cq_elaborate::compile --> relational_plan::Module
```

Start from the immutable `hw1` release and create a submission branch:

```text
git switch hw1
git switch -c submission/hw1
```

Do not start from `main` after a later homework has been announced. The
[branch policy](../BRANCHES.md) keeps every assignment's starter stable.

The current route lives in `crates/cq-ir/src/cq/normal.rs`,
`crates/cq-order/src/lib.rs`, and `crates/cq-elaborate/src/lib.rs`.
The parsers, local contracts, pass correspondence checks, fixtures and lower
stages are supplied in this checkout. A released starter identifies which
parts students complete.
Index selection and physical construction belong to later assignments.
Use the starter branch's corresponding source file when working on that release.

## Read these two artifacts

Compare the complete [CQ](DSL.md#cq-and-rust-leaves) and
[RelationalPlan](DSL.md#named-relational-algebra) Triangle programs.
Ignore the later languages until later homework.

The output carries the complete normalized program and adds a named
relational-algebra fragment. For a variable-only query such as Triangle, the
normalized program is the written source. Each `rN` is a serialized DAG
reference to a logical set-valued result, not an algebra operator, Rust iterator, materialized
collection, or assumed SSA prerequisite.

## Required lowering

CQ arguments are positional against each relation declaration's ordered named
columns. Normalization gives each atom distinct variable arguments, using
`let` and `if` items for the constraints introduced by constants, repeats
and expressions; wildcards receive fresh witness variables. `Written` with
`Coarse` then schedules one full subatom per atom in body order. Elaborate
that valuation plan while maintaining the next unused `rN` number.

For each atom in a variable-only positive body such as Triangle:

1. Emit a fresh `rename` directly from its declared relation name, even when
   that relation appeared earlier.
2. Map every declared source attribute, in declaration order, to the
   occurrence's positional query variable.
3. Use the first Rename as the initial current result.
4. For each later Rename, emit `natural_join current with renamed_occurrence`.
   Do this even when the right heading is wholly shared; a full subatom
   elaborates to NaturalJoin under `Coarse`.
5. Make the NaturalJoin result current.

For Triangle's variable-only head, finish with one `project` keeping the head
attributes, then write separate `output result as head.` metadata. A computed
head instead keeps the whole heading so its expression can read its inputs.
Triangle must become:

```text
r0 = rename R {c0 -> x, c1 -> y};
r1 = rename S {c0 -> y, c1 -> z};
r2 = natural_join r0 with r1;
r3 = rename T {c0 -> z, c1 -> x};
r4 = natural_join r2 with r3;
r5 = project r4 keep {x, y, z};
output r5 as triangle(x, y, z);
```

Ordering and elaboration preserve the normalized program. The supplied
RelationalPlan language includes `unit` to seed a guard or binding before any atom; the
variable-only Triangle plan starts with `rename`.

Homework 1 does **not** choose indexes, collections, join algorithms, Rust
access expressions, or iterator combinators.

## Inspect the edge

These commands inspect syntax and fixtures without invoking CQ lowering:

```text
cargo run --example cq-inspection
cargo test -p cq-ir --test relational_plan
cargo xtask list
cargo xtask explain triangle --through cq
```

After changing the CQ-to-RA stages, inspect the two whole programs side by side:

```text
cargo xtask explain triangle --through relational-plan
```

The files are written under `target/mini-linq-explain/triangle/` as
`01-cq.tokens` and `02-relational-plan.tokens`. In VS Code, run **MiniLinq: Explain
query**; the Homework 1 starter defaults to `relational-plan`.

## Definition of done

Run the normal-form, ordering and elaboration tests and formatting check:

```text
cargo test -p cq-ir --test valuation_language
cargo test -p cq-order --test order
cargo test -p cq-elaborate --test elaborate
cargo fmt --all --check
```

You are done when:

- the CQ-NF, VO and RelationalPlan checks pass, with VO and RA erasing to the
  normalized kernel up to variable renaming;
- every source occurrence has its own complete Rename;
- source attributes map positionally to the occurrence variables;
- `Written`/`Coarse` schedules one full subatom per atom in source order;
- VO and RA retain the normalized program;
- the final Project and output metadata are exact; and
- Homework 2 and Homework 3 remain untouched.

Complete the root [AI-use and verification record](../AI-USE.md). Be prepared
to explain how one unseen CQ maps named source columns to query variables and
changes the inferred heading.

The cumulative [query-lowering handout](R1-START-HERE.md) documents later
passes, but it is not required reading for Homework 1.
