## 2172002/003-given_physical_bindings

[Raw input](003-given_physical_bindings.input.tokens) · [Raw output](003-given_physical_bindings.output.tokens)

Actual input:

```rust
struct ProjectedChain; relation L(a : Value); relation
R(a : Value, b : Value); relation S(b : Value, c : Value); answer(a, c) : -
L(a), R(a, b), S(b, c); relational
{
    r0 = rename L { a - > a }; r1 = rename R { a - > a, b - > b }; r2 =
    natural_join r0 with r1; r3 = rename S { b - > b, c - > c }; r4 =
    natural_join r2 with r3; r5 = project r4 keep { a, c }; output r5 as
    answer(a, c);
} indexes { i0 = @ r1 [0]; i1 = @ r3 [0]; } uses
{ r2.right = i0; r4.right = i1; } physical
{
    relations
    {
        L = :: cq_example_providers :: slice_rows; R = :: cq_example_providers
        :: hash_relation; S = :: cq_example_providers :: sorted_relation; r0 =
        :: cq_example_providers :: slice_rows; r1 = :: cq_example_providers ::
        hash_relation; r2 = :: cq_example_providers :: borrowed_fields; r3 =
        :: cq_example_providers :: sorted_relation; r4 = ::
        cq_example_providers :: borrowed_fields; r5 = :: cq_example_providers
        :: borrowed_fields;
    } indexes
    {
        i0 = :: cq_example_providers :: hash_relation; i1 = ::
        cq_example_providers :: sorted_relation;
    } lowerings
    {
        r0 = :: cq_example_providers :: rename; r1 = :: cq_example_providers
        :: rename; r2 = :: cq_example_providers :: indexed_join; r3 = ::
        cq_example_providers :: rename; r4 = :: cq_example_providers ::
        indexed_join; r5 = :: cq_example_providers :: project;
    }
}
```

Actual output (formatted where Rust syntax permits):

```rust
mini_linq::realize_physical! {
    struct ProjectedChain; relation L(a : Value); relation
    R(a : Value, b : Value); relation S(b : Value, c : Value); answer(a, c) :
    - L(a), R(a, b), S(b, c); relational
    {
        r0 = rename L { a - > a }; r1 = rename R { a - > a, b - > b }; r2 =
        natural_join r0 with r1; r3 = rename S { b - > b, c - > c }; r4 =
        natural_join r2 with r3; r5 = project r4 keep { a, c }; output r5 as
        answer(a, c);
    } indexes { i0 = @ r1 [0]; i1 = @ r3 [0]; } uses
    { r2.right = i0; r4.right = i1; } physical
    {
        relations
        {
            L = :: cq_example_providers :: slice_rows; R = ::
            cq_example_providers :: hash_relation; S = :: cq_example_providers
            :: sorted_relation; r0 = :: cq_example_providers :: slice_rows; r1
            = :: cq_example_providers :: hash_relation; r2 = ::
            cq_example_providers :: borrowed_fields; r3 = ::
            cq_example_providers :: sorted_relation; r4 = ::
            cq_example_providers :: borrowed_fields; r5 = ::
            cq_example_providers :: borrowed_fields;
        } indexes
        {
            i0 = :: cq_example_providers :: hash_relation; i1 = ::
            cq_example_providers :: sorted_relation;
        } lowerings
        {
            r0 = :: cq_example_providers :: rename; r1 = ::
            cq_example_providers :: rename; r2 = :: cq_example_providers ::
            indexed_join; r3 = :: cq_example_providers :: rename; r4 = ::
            cq_example_providers :: indexed_join; r5 = :: cq_example_providers
            :: project;
        }
    }
}

```
