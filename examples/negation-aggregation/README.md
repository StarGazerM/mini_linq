# Optional negation and aggregation examples

[main.rs](main.rs) is an executable specification for the optional HW4
extension. Negation and aggregation are **not implemented in the current
basic/reference compiler**. The target is gated by the existing `hw4` feature;
it becomes runnable after that extension is implemented:

```text
cargo run -p mini-linq --example negation-aggregation --features hw4
```

| Query | Case | Required logical order | Expected result |
| --- | --- | --- | --- |
| `Allowed` | Safe negation with a full key | `Person`; anti-semijoin `Blocked` on person; project | `[(1,), (3,)]` |
| `Spend` | Negation with a wildcard; correlated sum | `Person`; anti-semijoin on person; aggregate apply on person; project | `[(1, 10), (3, 0)]` |
| `GlobalSum` | No positive atom; empty correlation | Unit; global anti-semijoin; global aggregate apply; project | `[(10,)]`, empty sum `[(0,)]`, blocked result `[]` |

Negation depends on previously bound correlation variables and introduces no
new variables. Aggregation depends on its correlation variables, binds only
its result outward, and keeps aggregate-local witnesses until the fold.
`Spend` includes duplicate tuples and distinct categories with equal amounts:
set normalization removes the former while both latter rows contribute.
`sum` produces zero on an empty matching input.

The supplied assertions state the target semantics; they are not evidence of
a completed lowering. Follow the [HW4 guide](../../doc/HW4-OPTIONAL.md) for the
IR operators, contracts, overflow checks, and lazy-fold acceptance tests.
Ordinary Rust `if`, `let`, calls, and computed atom/head expressions are already
demonstrated separately in the [runnable expression gallery](../rust-expressions/README.md).
