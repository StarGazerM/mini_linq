## 2172007/002-index_demands

[Raw input](002-index_demands.input.tokens) · [Raw output](002-index_demands.output.tokens)

Actual input:

```rust
struct PullChain; relation L(a : Value); relation R(a : Value, b : Value);
relation S(b : Value, c : Value); answer(a, c) : - L(a), R(a, b), S(b, c);
relational
{
    r0 = rename L { a - > a }; r1 = rename R { a - > a, b - > b }; r2 =
    natural_join r0 with r1; r3 = rename S { b - > b, c - > c }; r4 =
    natural_join r2 with r3; r5 = project r4 keep { a, c }; output r5 as
    answer(a, c);
}
```

Actual output (formatted where Rust syntax permits):

```rust
mini_linq::choose_physical! {
    struct PullChain; relation L(a : Value); relation R(a : Value, b : Value);
    relation S(b : Value, c : Value); answer(a, c) : - L(a), R(a, b), S(b, c);
    relational
    {
        r0 = rename L { a - > a }; r1 = rename R { a - > a, b - > b }; r2 =
        natural_join r0 with r1; r3 = rename S { b - > b, c - > c }; r4 =
        natural_join r2 with r3; r5 = project r4 keep { a, c }; output r5 as
        answer(a, c);
    } indexes { i0 = @ r1 [0]; i1 = @ r3 [0]; } uses
    { r2.right = i0; r4.right = i1; }
}

```
