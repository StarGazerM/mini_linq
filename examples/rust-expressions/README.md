# Rust expression examples

Run the complete gallery from the repository root after completing the three
basic compiler passes:

```text
cargo run -p mini-linq --example rust-expressions --features compiled-workloads
```

Every example contains its query, input rows, and asserted expected results.
The executable prints each result. It also checks function-call counts and
lazy cursor behavior. The existing `compiled-workloads` gate keeps the homework
starter buildable while its three assigned pass bodies are unfinished.

These are executable acceptance examples: Cargo runs their `main` functions
as part of `cargo test --workspace --features mini-linq/compiled-workloads`.
Assertions remain enabled in release builds.

## Feature map

`R; S` below means scan/match `R` before matching `S`. A computed argument may
depend on its preceding prefix or an earlier argument of the same atom. The
examples use set inputs and return sorted, deduplicated results from `run`.

| File and query | Feature | Required evaluation order | Expected result |
| --- | --- | --- | --- |
| [arguments.rs](arguments.rs): `Constant` | Literal atom argument | Fresh typed `let k = 7`; match `R(k, value)` | `[("yes",)]` |
| [arguments.rs](arguments.rs): `Repeated` | Repeated `String` argument and repeated head | Scan `R`; enforce column equality; project | `[("a", "a")]` |
| [arguments.rs](arguments.rs): `Wildcard` | `_`, unused column, duplicate projection | Scan `R`; project; distinct | `[(5,), (7,)]` |
| [arguments.rs](arguments.rs): `PrefixArgument`, `GenericArgument` | Function call and `add::<i32, i32>(*x, 1)` | `R`; compute key once per prefix tuple; `S` | `[(1, 10), (2, 20)]` |
| [arguments.rs](arguments.rs): `SameAtom` | `R(x, successor(x))` | Bind `x` from `R`; check its second field | `[(1,), (2,)]` |
| [bindings.rs](bindings.rs): `TypedLet` | Typed owned binding; inferred head schema | `R`; Extend with `label`; head | `[("item-1",)]` |
| [bindings.rs](bindings.rs): `Destructure` | Tuple pattern and inferred intermediate types | `R`; bind `(n, label)`; guard on `n` | `[("item-1",), ("item-2",)]` |
| [bindings.rs](bindings.rs): `OwnedJoin` | Method call, owned `String`, repeated bound key | `R`; bind uppercase label; `S`; guard | `[("ALICE", 10), ("ALICE", 20)]` |
| [guards.rs](guards.rs): `Call`, `Compound` | Arbitrary call and compound Boolean condition | `R`; guard | `[(1,), (2,)]` |
| [guards.rs](guards.rs): `Block` | Block, closure with comma-separated parameters, lexical shadowing | `R`; evaluate complete Rust block | `[(1,), (2,)]` |
| [guards.rs](guards.rs): `Matching`, `Conditional` | Rust `match` and Rust `if` expressions | `R`; evaluate condition | `[(1,), (2,), (4,)]` and `[(1,), (2,)]` |
| [guards.rs](guards.rs): `Macro` | Opaque Rust macro condition | `R`; evaluate macro with its environment intact | `[(1,), (2,), (4,)]` |
| [heads.rs](heads.rs): `ComputedHead`, `Wrapped` | Computed output, implicit format capture, DSL macro wrapper | Body first; head with original syntax contexts | `[("item-1",), ("item-2",)]` and `[("wrapped-1",)]` |
| [heads.rs](heads.rs): `Witnesses` | Computed key/head evaluation counts | Key per `R` tuple; head per full join witness; distinct last | `[(1,)]`; 2 key calls, 4 head calls |
| [lazy.rs](lazy.rs): `Lazy` | Deferred clauses, owned value across multiple matches | Build and cursor creation: 0 calls; first `next`: 1; fresh traversal: 2 | One `("item-1",)` per traversal |

The head declaration names public result types; it is not another input to
`run` or `build`. Atom variables borrow their input fields; lets hold Rust
values. A source let must introduce fresh logical variables, while lexical
locals inside a Rust expression may shadow normally. Rules use `;` to avoid
the ambiguity between the legacy rule-ending `.` and Rust float syntax.

## Join schedules

[joins.rs](joins.rs) executes six three-relation examples. Their hand-written
expected results are all `[(1, 10), (2, 20)]`, except `PureGuard`, whose computed
head produces `[("1:10",), ("2:20",)]`.

The following schedules are for the completed planner's `Auto` policy. The
basic lazy pass follows written order after literal normalization. Passing a
guard's variable-dependency check does not authorize changing its evaluation
count: arbitrary Rust can have effects or panic.

| Query | Written body | Expected planned schedule | Why |
| --- | --- | --- | --- |
| `Constant` | `R(7,x), T(y,_), S(x,y)` | Fresh typed constant; `R, S, T` | Normalize the literal; join the connected bridge before `T` |
| `ConstantLet` | `let k:i32=7, R(k,x), T(y,_), S(x,y)` | `let, R, S, T` | Explicit equivalent of literal normalization |
| `DependentLet` | `R(x,_), let k=key(x), T(y,_), S(k,y)` | `R, let, S, T` | The let requires `x`; `S` and `T` may reorder after it; 2 key calls |
| `GuardBoundary` | `R(x,_), T(y,_), if guard(x,y), S(x,y)` | `R, T, guard, S` | The general guard must see all 4 prefix tuples |
| `ArgumentBoundary` | `R(x,_), T(y,_), S(key(x),y)` | `R, T, key, S` | The computed argument is evaluated on all 4 prefix tuples |
| `PureGuard` | `R(x,_), T(y,_), S(x,y), if x > 0` | `R, comparison, S, T, head` | This total primitive comparison can run as soon as `x` is bound |

Constant normalization is inside the existing CQ-to-RelationalPlan pass. It
uses the typed-let lowering and records a `Constant` administrative rule;
there is no additional nominal IR or public macro boundary.

In the completed MiniLinq repository, run the same gallery through the
alternative materialized macro composition:

```text
cargo run -p mini-linq --example rust-expressions-planned --features compiled-workloads
```

That entry point also reads the exact six `query!` inputs from `joins.rs`,
asserts their full administrative schedules and actual RA input orders,
replays each derivation through the contract, and prints peak column arity.
The materialized route evaluates clauses during `build`; the default route's
lazy cursor walkthrough therefore belongs to the first executable only.
The starter exposes the basic route; the alternative planned macro composition
is supplied by the completed repository.

These examples complement the larger
[expression contract and planning matrix](../../doc/RUST-EXPRESSIONS.md).
They do not establish a Yannakakis certificate for general Rust expressions.
For the separate optional syntax, see
[negation and aggregation](../negation-aggregation/README.md).
