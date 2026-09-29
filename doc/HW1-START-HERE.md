# Homework 1: CQ to RA

Homework 1 follows the typed CQ-to-RA pass chain:

```text
cq::Program -- cq::normal::normalize --> CQ-NF
           -- cq_order::compile (Written, Coarse) --> vo::Program
           -- cq_elaborate::compile --> ra::Program
```

Start from the immutable `hw1` release and create a submission branch:

```text
git switch hw1
git switch -c submission/hw1
```

Do not start from `main` after a later homework has been announced. The
[branch policy](../BRANCHES.md) keeps every assignment's starter stable.

The current route lives in `languages/cq-lang-cq/src/normal.rs`,
`passes/cq-order/src/lib.rs`, and `passes/cq-elaborate/src/lib.rs`.
The parsers, local contracts, pass correspondence checks, fixtures and lower
stages are supplied in this checkout. A released starter identifies which
parts students complete.
Demand derivation and physical construction belong to later assignments.
Use the starter branch's corresponding source file when working on that release.

## Read these two artifacts

Compare the complete Triangle
[CQ](../test-support/relational/scenarios/triangle.cq) and
[RA](../test-support/relational/scenarios/triangle.ra) programs; the
[grammars](DSL.md#named-relational-algebra) define both languages. Ignore the
later languages until later homework.

VO retains the normalized CQ. Elaboration produces a standalone RA program
carrying the declarations, the named definitions and the output metadata; it
does not embed the CQ, and `cq_elaborate::check` relates the two by kernel
erasure. Each `rN` is a serialized DAG reference to a logical set-valued
result, not an algebra operator, Rust iterator, materialized collection, or
assumed SSA prerequisite.

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
attributes, then write separate `output result as head;` metadata. A computed
head instead keeps the whole heading so its expression can read its inputs.
Triangle must become:

```text
relational {
    relation R(first: i32, second: i32);
    relation S(first: i32, second: i32);
    relation T(first: i32, second: i32);
    r0 = rename R { first -> a, second -> b };
    r1 = rename S { first -> b, second -> c };
    r2 = natural_join r0 with r1;
    r3 = rename T { first -> c, second -> a };
    r4 = natural_join r2 with r3;
    r5 = project r4 keep { a, b, c };
    output r5 as answer(a, b, c);
}
```

The RA language includes `unit`: a guard or binding before the first atom
seeds the prefix with it, including the bindings normalization introduces for
constants. The variable-only Triangle plan starts with `rename`.

Homework 1 does **not** choose indexes, collections, join algorithms, Rust
access expressions, or iterator combinators.

## Inspect the edge

These commands inspect syntax and fixtures without invoking CQ lowering:

```text
cargo run --example cq-inspection
cargo test -p cq-lang-ra --test relational_program
cargo xtask list
cargo xtask explain triangle --through cq
```

After changing the CQ-to-RA stages, inspect the two whole programs side by side:

```text
cargo xtask explain triangle --through relational-plan
```

The files are written under `target/mini-linq-explain/triangle/` as
`01-cq.tokens` and `02-relational-plan.tokens`. In VS Code, run **MiniLinq: Explain
query** and choose `relational-plan` in the stage picker; this checkout
defaults to `index-requirements`.

## Definition of done

Run the normal-form, ordering and elaboration tests and formatting check:

```text
cargo test -p cq-lang-cq --test normal
cargo test -p cq-order --test order
cargo test -p cq-elaborate --test elaborate
cargo fmt --all --check
```

You are done when:

- the CQ-NF, VO and RA contracts hold, and `cq_order::check` and
  `cq_elaborate::check` accept your output, the RA kernel agreeing with the
  normalized kernel up to variable renaming;
- every source occurrence has its own complete Rename;
- source attributes map positionally to the occurrence variables;
- `Written`/`Coarse` schedules one full subatom per atom in source order;
- VO retains the normalized CQ, and RA carries its declarations and head;
- the final Project and output metadata are exact; and
- Homework 2 and Homework 3 remain untouched.

Complete the root [AI-use and verification record](../AI-USE.md). Be prepared
to explain how one unseen CQ maps named source columns to query variables and
changes the inferred heading.

The cumulative [query-lowering handout](R1-START-HERE.md) documents later
passes, but it is not required reading for Homework 1.
