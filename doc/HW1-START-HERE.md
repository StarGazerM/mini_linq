# Homework 1: CQ to RelationalPlan

Homework 1 implements one typed compiler edge:

```text
cq::Module -- cq_query_lowering::written_order::compile --> relational_plan::Module
```

Start from the immutable `hw1` release and create a submission branch:

```text
git switch hw1
git switch -c submission/hw1
```

Do not start from `main` after a later homework has been announced. The
[branch policy](../BRANCHES.md) keeps every assignment's starter stable.

The assigned edge is `cq_query_lowering::written_order::compile`, implemented in this checkout at
`crates/cq-query-lowering/src/written_order/lower.rs`. The parsers, local
contracts, independent edge checker, fixtures and lower stages are supplied.
Index selection and physical construction belong to later assignments.
Use the starter branch's corresponding source file when working on that release.

## Read these two artifacts

Compare the complete [CQ](DSL.md#1-entire-cq) and
[RelationalPlan](DSL.md#2-entire-relationalplan-program) Triangle programs.
Ignore the later four residuals until later homework.

The output preserves the complete source program and adds a named
relational-algebra fragment. Each `rN` is a serialized DAG reference to a
logical set-valued result, not an algebra operator, Rust iterator, materialized
collection, or assumed SSA prerequisite.

## Required lowering

CQ arguments are positional against each relation declaration's ordered named
columns. Walk the positive CQ body from left to right and maintain the next
unused `rN` number.

For every body occurrence:

1. Emit a fresh `rename` directly from its declared relation name, even when
   that relation appeared earlier.
2. Map every declared source attribute, in declaration order, to the
   occurrence's positional query variable.
3. Use the first Rename as the initial current result.
4. For each later Rename, emit `natural_join current with renamed_occurrence`.
   Do this even when the right heading is wholly shared; membership is a later
   physical choice.
5. Make the NaturalJoin result current.

Finally emit exactly one `project` keeping the source-head attributes, then
write separate `output result as head.` metadata. Triangle must become:

```text
r0 = rename R {c0 -> x, c1 -> y};
r1 = rename S {c0 -> y, c1 -> z};
r2 = natural_join r0 with r1;
r3 = rename T {c0 -> z, c1 -> x};
r4 = natural_join r2 with r3;
r5 = project r4 keep {x, y, z};
output r5 as triangle(x, y, z);
```

Construct the typed result with `quote!` and `syn::parse2`; do not build a
string. The supplied RelationalPlan language includes `unit` for a later
extension, but a positive Homework 1 query always starts with `rename` and
must not emit `unit`.

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

After implementing `compile_cq`, inspect the two whole programs side by side:

```text
cargo xtask explain triangle --through relational-plan
```

The files are written under `target/mini-linq-explain/triangle/` as
`01-cq.rs` and `02-relational-plan.rs`. In VS Code, run **MiniLinq: Explain
query**; the Homework 1 starter defaults to `relational-plan`.

## Definition of done

Run the Homework 1 edge test and formatting check:

```text
cargo test -p cq-compose --test edge_contracts cq_to_relational_plan_contract
cargo fmt --all --check
```

You are done when:

- the exact CQ-to-RelationalPlan fixtures pass;
- every source occurrence has its own complete Rename;
- source attributes map positionally to the occurrence variables;
- body occurrences fold through source-order left-deep NaturalJoins;
- the source program is unchanged;
- the final Project and output metadata are exact; and
- Homework 2 and Homework 3 remain untouched.

Complete the root [AI-use and verification record](../AI-USE.md). Be prepared
to explain how one unseen CQ maps named source columns to query variables and
changes the inferred heading.

The cumulative [query-lowering handout](R1-START-HERE.md) documents later
passes, but it is not required reading for Homework 1.
