# Actual proc-macro boundaries

These calls were captured during rustc expansion of the unchanged target.
Raw token files are authoritative. Formatted Rust is a reading aid only.
Sequence numbers are invocation order within each compiler process, not runtime order.
Provider calls selected by --macro-package share that process sequence.
See command.json, compile.log, source-hashes.json, instrumentation.patch
and instrumented-packages.json.

[Print all procedural steps](steps.md)

[Declarative macro input/output trace](declarative.log) was captured in a
second compilation of the same source with target-local `-Ztrace-macros`.
The two logs are separate observations, not one merged event timeline.


## 2172002/001-cq_to_ra_written_order

[Raw input](raw/2172002/001-cq_to_ra_written_order.input.tokens) · [Raw output](raw/2172002/001-cq_to_ra_written_order.output.tokens)

Actual input:

```rust
struct ProjectedChain; relation L(a: Value); relation R(a: Value, b: Value);
relation S(b: Value, c: Value); answer(a, c) :- L(a), R(a, b), S(b, c);
```

Actual output (formatted where Rust syntax permits):

```rust
mini_linq::choose_index_policy! {
    struct ProjectedChain; relation L(a : Value); relation
    R(a : Value, b : Value); relation S(b : Value, c : Value); answer(a, c) :
    - L(a), R(a, b), S(b, c); relational
    {
        r0 = rename L { a - > a }; r1 = rename R { a - > a, b - > b }; r2 =
        natural_join r0 with r1; r3 = rename S { b - > b, c - > c }; r4 =
        natural_join r2 with r3; r5 = project r4 keep { a, c }; output r5 as
        answer(a, c);
    }
}

```

---

## 2172002/002-index_demands

[Raw input](raw/2172002/002-index_demands.input.tokens) · [Raw output](raw/2172002/002-index_demands.output.tokens)

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
}
```

Actual output (formatted where Rust syntax permits):

```rust
mini_linq::choose_physical! {
    struct ProjectedChain; relation L(a : Value); relation
    R(a : Value, b : Value); relation S(b : Value, c : Value); answer(a, c) :
    - L(a), R(a, b), S(b, c); relational
    {
        r0 = rename L { a - > a }; r1 = rename R { a - > a, b - > b }; r2 =
        natural_join r0 with r1; r3 = rename S { b - > b, c - > c }; r4 =
        natural_join r2 with r3; r5 = project r4 keep { a, c }; output r5 as
        answer(a, c);
    } indexes { i0 = @ r1 [0]; i1 = @ r3 [0]; } uses
    { r2.right = i0; r4.right = i1; }
}

```

---

## 2172002/003-given_physical_bindings

[Raw input](raw/2172002/003-given_physical_bindings.input.tokens) · [Raw output](raw/2172002/003-given_physical_bindings.output.tokens)

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

---

## 2172002/004-realize_physical

[Raw input](raw/2172002/004-realize_physical.input.tokens) · [Raw output](raw/2172002/004-realize_physical.output.tokens)

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
::cq_example_providers::slice_rows! {
    reply_to(:: mini_linq :: resume_physical)
    {
        plan
        {
            struct ProjectedChain; relation L(a : Value); relation
            R(a : Value, b : Value); relation S(b : Value, c : Value);
            answer(a, c) : - L(a), R(a, b), S(b, c); relational
            {
                r0 = rename L { a - > a }; r1 = rename R { a - > a, b - > b };
                r2 = natural_join r0 with r1; r3 = rename S
                { b - > b, c - > c }; r4 = natural_join r2 with r3; r5 =
                project r4 keep { a, c }; output r5 as answer(a, c);
            } indexes { i0 = @ r1 [0]; i1 = @ r3 [0]; } uses
            { r2.right = i0; r4.right = i1; } physical
            {
                relations
                {
                    L = :: cq_example_providers :: slice_rows; R = ::
                    cq_example_providers :: hash_relation; S = ::
                    cq_example_providers :: sorted_relation; r0 = ::
                    cq_example_providers :: slice_rows; r1 = ::
                    cq_example_providers :: hash_relation; r2 = ::
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
                    cq_example_providers :: rename; r2 = :: cq_example_providers
                    :: indexed_join; r3 = :: cq_example_providers :: rename; r4
                    = :: cq_example_providers :: indexed_join; r5 = ::
                    cq_example_providers :: project;
                }
            }
        } environment {} emitted {} pending { input L from(L) }
        continuations(:: mini_linq :: resume_physical, mini_linq :: access)
    } plan
    {
        struct ProjectedChain; relation L(a : Value); relation
        R(a : Value, b : Value); relation S(b : Value, c : Value);
        answer(a, c) : - L(a), R(a, b), S(b, c); relational
        {
            r0 = rename L { a - > a }; r1 = rename R { a - > a, b - > b }; r2
            = natural_join r0 with r1; r3 = rename S { b - > b, c - > c }; r4
            = natural_join r2 with r3; r5 = project r4 keep { a, c }; output
            r5 as answer(a, c);
        } indexes { i0 = @ r1 [0]; i1 = @ r3 [0]; } uses
        { r2.right = i0; r4.right = i1; } physical
        {
            relations
            {
                L = :: cq_example_providers :: slice_rows; R = ::
                cq_example_providers :: hash_relation; S = ::
                cq_example_providers :: sorted_relation; r0 = ::
                cq_example_providers :: slice_rows; r1 = ::
                cq_example_providers :: hash_relation; r2 = ::
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
                cq_example_providers :: rename; r2 = :: cq_example_providers
                :: indexed_join; r3 = :: cq_example_providers :: rename; r4 =
                :: cq_example_providers :: indexed_join; r5 = ::
                cq_example_providers :: project;
            }
        }
    } environment {} request { input L from(L) }
}

```

---

## 2172002/005-cq-example-providers::slice_rows

[Raw input](raw/2172002/005-cq-example-providers::slice_rows.input.tokens) · [Raw output](raw/2172002/005-cq-example-providers::slice_rows.output.tokens)

Actual input:

```rust
reply_to(:: mini_linq :: resume_physical)
{
    plan
    {
        struct ProjectedChain; relation L(a : Value); relation
        R(a : Value, b : Value); relation S(b : Value, c : Value);
        answer(a, c) : - L(a), R(a, b), S(b, c); relational
        {
            r0 = rename L { a - > a }; r1 = rename R { a - > a, b - > b }; r2
            = natural_join r0 with r1; r3 = rename S { b - > b, c - > c }; r4
            = natural_join r2 with r3; r5 = project r4 keep { a, c }; output
            r5 as answer(a, c);
        } indexes { i0 = @ r1 [0]; i1 = @ r3 [0]; } uses
        { r2.right = i0; r4.right = i1; } physical
        {
            relations
            {
                L = :: cq_example_providers :: slice_rows; R = ::
                cq_example_providers :: hash_relation; S = ::
                cq_example_providers :: sorted_relation; r0 = ::
                cq_example_providers :: slice_rows; r1 = ::
                cq_example_providers :: hash_relation; r2 = ::
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
                cq_example_providers :: rename; r2 = :: cq_example_providers
                :: indexed_join; r3 = :: cq_example_providers :: rename; r4 =
                :: cq_example_providers :: indexed_join; r5 = ::
                cq_example_providers :: project;
            }
        }
    } environment {} emitted {} pending { input L from(L) }
    continuations(:: mini_linq :: resume_physical, mini_linq :: access)
} plan
{
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
} environment {} request { input L from(L) }
```

Actual output (formatted where Rust syntax permits):

```rust
::mini_linq::resume_physical! {
    resume
    {
        plan
        {
            struct ProjectedChain; relation L(a : Value); relation
            R(a : Value, b : Value); relation S(b : Value, c : Value);
            answer(a, c) : - L(a), R(a, b), S(b, c); relational
            {
                r0 = rename L { a - > a }; r1 = rename R { a - > a, b - > b };
                r2 = natural_join r0 with r1; r3 = rename S
                { b - > b, c - > c }; r4 = natural_join r2 with r3; r5 =
                project r4 keep { a, c }; output r5 as answer(a, c);
            } indexes { i0 = @ r1 [0]; i1 = @ r3 [0]; } uses
            { r2.right = i0; r4.right = i1; } physical
            {
                relations
                {
                    L = :: cq_example_providers :: slice_rows; R = ::
                    cq_example_providers :: hash_relation; S = ::
                    cq_example_providers :: sorted_relation; r0 = ::
                    cq_example_providers :: slice_rows; r1 = ::
                    cq_example_providers :: hash_relation; r2 = ::
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
                    cq_example_providers :: rename; r2 = :: cq_example_providers
                    :: indexed_join; r3 = :: cq_example_providers :: rename; r4
                    = :: cq_example_providers :: indexed_join; r5 = ::
                    cq_example_providers :: project;
                }
            }
        } environment {} emitted {} pending { input L from(L) }
        continuations(:: mini_linq :: resume_physical, mini_linq :: access)
    } emitted { let __cq_store_L : & [Value] = L; } result
    {
        stored L at(__cq_store_L) operations :: cq_example_providers ::
        slice_rows
    }
}

```

---

## 2172002/006-resume_physical

[Raw input](raw/2172002/006-resume_physical.input.tokens) · [Raw output](raw/2172002/006-resume_physical.output.tokens)

Actual input:

```rust
resume
{
    plan
    {
        struct ProjectedChain; relation L(a : Value); relation
        R(a : Value, b : Value); relation S(b : Value, c : Value);
        answer(a, c) : - L(a), R(a, b), S(b, c); relational
        {
            r0 = rename L { a - > a }; r1 = rename R { a - > a, b - > b }; r2
            = natural_join r0 with r1; r3 = rename S { b - > b, c - > c }; r4
            = natural_join r2 with r3; r5 = project r4 keep { a, c }; output
            r5 as answer(a, c);
        } indexes { i0 = @ r1 [0]; i1 = @ r3 [0]; } uses
        { r2.right = i0; r4.right = i1; } physical
        {
            relations
            {
                L = :: cq_example_providers :: slice_rows; R = ::
                cq_example_providers :: hash_relation; S = ::
                cq_example_providers :: sorted_relation; r0 = ::
                cq_example_providers :: slice_rows; r1 = ::
                cq_example_providers :: hash_relation; r2 = ::
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
                cq_example_providers :: rename; r2 = :: cq_example_providers
                :: indexed_join; r3 = :: cq_example_providers :: rename; r4 =
                :: cq_example_providers :: indexed_join; r5 = ::
                cq_example_providers :: project;
            }
        }
    } environment {} emitted {} pending { input L from(L) }
    continuations(:: mini_linq :: resume_physical, mini_linq :: access)
} emitted { let __cq_store_L : & [Value] = L; } result
{ stored L at(__cq_store_L) operations :: cq_example_providers :: slice_rows }
```

Actual output (formatted where Rust syntax permits):

```rust
::cq_example_providers::hash_relation! {
    reply_to(:: mini_linq :: resume_physical)
    {
        plan
        {
            struct ProjectedChain; relation L(a : Value); relation
            R(a : Value, b : Value); relation S(b : Value, c : Value);
            answer(a, c) : - L(a), R(a, b), S(b, c); relational
            {
                r0 = rename L { a - > a }; r1 = rename R { a - > a, b - > b };
                r2 = natural_join r0 with r1; r3 = rename S
                { b - > b, c - > c }; r4 = natural_join r2 with r3; r5 =
                project r4 keep { a, c }; output r5 as answer(a, c);
            } indexes { i0 = @ r1 [0]; i1 = @ r3 [0]; } uses
            { r2.right = i0; r4.right = i1; } physical
            {
                relations
                {
                    L = :: cq_example_providers :: slice_rows; R = ::
                    cq_example_providers :: hash_relation; S = ::
                    cq_example_providers :: sorted_relation; r0 = ::
                    cq_example_providers :: slice_rows; r1 = ::
                    cq_example_providers :: hash_relation; r2 = ::
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
                    cq_example_providers :: rename; r2 = :: cq_example_providers
                    :: indexed_join; r3 = :: cq_example_providers :: rename; r4
                    = :: cq_example_providers :: indexed_join; r5 = ::
                    cq_example_providers :: project;
                }
            }
        } environment
        {
            stored L at(__cq_store_L) operations :: cq_example_providers ::
            slice_rows
        } emitted { let __cq_store_L : & [Value] = L; } pending
        { input R from(R) }
        continuations(:: mini_linq :: resume_physical, mini_linq :: access)
    } plan
    {
        struct ProjectedChain; relation L(a : Value); relation
        R(a : Value, b : Value); relation S(b : Value, c : Value);
        answer(a, c) : - L(a), R(a, b), S(b, c); relational
        {
            r0 = rename L { a - > a }; r1 = rename R { a - > a, b - > b }; r2
            = natural_join r0 with r1; r3 = rename S { b - > b, c - > c }; r4
            = natural_join r2 with r3; r5 = project r4 keep { a, c }; output
            r5 as answer(a, c);
        } indexes { i0 = @ r1 [0]; i1 = @ r3 [0]; } uses
        { r2.right = i0; r4.right = i1; } physical
        {
            relations
            {
                L = :: cq_example_providers :: slice_rows; R = ::
                cq_example_providers :: hash_relation; S = ::
                cq_example_providers :: sorted_relation; r0 = ::
                cq_example_providers :: slice_rows; r1 = ::
                cq_example_providers :: hash_relation; r2 = ::
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
                cq_example_providers :: rename; r2 = :: cq_example_providers
                :: indexed_join; r3 = :: cq_example_providers :: rename; r4 =
                :: cq_example_providers :: indexed_join; r5 = ::
                cq_example_providers :: project;
            }
        }
    } environment
    {
        stored L at(__cq_store_L) operations :: cq_example_providers ::
        slice_rows
    } request { input R from(R) }
}

```

---

## 2172002/007-cq-example-providers::hash_relation

[Raw input](raw/2172002/007-cq-example-providers::hash_relation.input.tokens) · [Raw output](raw/2172002/007-cq-example-providers::hash_relation.output.tokens)

Actual input:

```rust
reply_to(:: mini_linq :: resume_physical)
{
    plan
    {
        struct ProjectedChain; relation L(a : Value); relation
        R(a : Value, b : Value); relation S(b : Value, c : Value);
        answer(a, c) : - L(a), R(a, b), S(b, c); relational
        {
            r0 = rename L { a - > a }; r1 = rename R { a - > a, b - > b }; r2
            = natural_join r0 with r1; r3 = rename S { b - > b, c - > c }; r4
            = natural_join r2 with r3; r5 = project r4 keep { a, c }; output
            r5 as answer(a, c);
        } indexes { i0 = @ r1 [0]; i1 = @ r3 [0]; } uses
        { r2.right = i0; r4.right = i1; } physical
        {
            relations
            {
                L = :: cq_example_providers :: slice_rows; R = ::
                cq_example_providers :: hash_relation; S = ::
                cq_example_providers :: sorted_relation; r0 = ::
                cq_example_providers :: slice_rows; r1 = ::
                cq_example_providers :: hash_relation; r2 = ::
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
                cq_example_providers :: rename; r2 = :: cq_example_providers
                :: indexed_join; r3 = :: cq_example_providers :: rename; r4 =
                :: cq_example_providers :: indexed_join; r5 = ::
                cq_example_providers :: project;
            }
        }
    } environment
    {
        stored L at(__cq_store_L) operations :: cq_example_providers ::
        slice_rows
    } emitted { let __cq_store_L : & [Value] = L; } pending
    { input R from(R) }
    continuations(:: mini_linq :: resume_physical, mini_linq :: access)
} plan
{
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
} environment
{ stored L at(__cq_store_L) operations :: cq_example_providers :: slice_rows }
request { input R from(R) }
```

Actual output (formatted where Rust syntax permits):

```rust
::mini_linq::resume_physical! {
    resume
    {
        plan
        {
            struct ProjectedChain; relation L(a : Value); relation
            R(a : Value, b : Value); relation S(b : Value, c : Value);
            answer(a, c) : - L(a), R(a, b), S(b, c); relational
            {
                r0 = rename L { a - > a }; r1 = rename R { a - > a, b - > b };
                r2 = natural_join r0 with r1; r3 = rename S
                { b - > b, c - > c }; r4 = natural_join r2 with r3; r5 =
                project r4 keep { a, c }; output r5 as answer(a, c);
            } indexes { i0 = @ r1 [0]; i1 = @ r3 [0]; } uses
            { r2.right = i0; r4.right = i1; } physical
            {
                relations
                {
                    L = :: cq_example_providers :: slice_rows; R = ::
                    cq_example_providers :: hash_relation; S = ::
                    cq_example_providers :: sorted_relation; r0 = ::
                    cq_example_providers :: slice_rows; r1 = ::
                    cq_example_providers :: hash_relation; r2 = ::
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
                    cq_example_providers :: rename; r2 = :: cq_example_providers
                    :: indexed_join; r3 = :: cq_example_providers :: rename; r4
                    = :: cq_example_providers :: indexed_join; r5 = ::
                    cq_example_providers :: project;
                }
            }
        } environment
        {
            stored L at(__cq_store_L) operations :: cq_example_providers ::
            slice_rows
        } emitted { let __cq_store_L : & [Value] = L; } pending
        { input R from(R) }
        continuations(:: mini_linq :: resume_physical, mini_linq :: access)
    } emitted
    {
        let __cq_store_R : & :: std :: collections :: HashMap < Value, :: std
        :: vec :: Vec < Value >> = R;
    } result
    {
        stored R at(__cq_store_R) operations :: cq_example_providers ::
        hash_relation
    }
}

```

---

## 2172002/008-resume_physical

[Raw input](raw/2172002/008-resume_physical.input.tokens) · [Raw output](raw/2172002/008-resume_physical.output.tokens)

Actual input:

```rust
resume
{
    plan
    {
        struct ProjectedChain; relation L(a : Value); relation
        R(a : Value, b : Value); relation S(b : Value, c : Value);
        answer(a, c) : - L(a), R(a, b), S(b, c); relational
        {
            r0 = rename L { a - > a }; r1 = rename R { a - > a, b - > b }; r2
            = natural_join r0 with r1; r3 = rename S { b - > b, c - > c }; r4
            = natural_join r2 with r3; r5 = project r4 keep { a, c }; output
            r5 as answer(a, c);
        } indexes { i0 = @ r1 [0]; i1 = @ r3 [0]; } uses
        { r2.right = i0; r4.right = i1; } physical
        {
            relations
            {
                L = :: cq_example_providers :: slice_rows; R = ::
                cq_example_providers :: hash_relation; S = ::
                cq_example_providers :: sorted_relation; r0 = ::
                cq_example_providers :: slice_rows; r1 = ::
                cq_example_providers :: hash_relation; r2 = ::
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
                cq_example_providers :: rename; r2 = :: cq_example_providers
                :: indexed_join; r3 = :: cq_example_providers :: rename; r4 =
                :: cq_example_providers :: indexed_join; r5 = ::
                cq_example_providers :: project;
            }
        }
    } environment
    {
        stored L at(__cq_store_L) operations :: cq_example_providers ::
        slice_rows
    } emitted { let __cq_store_L : & [Value] = L; } pending
    { input R from(R) }
    continuations(:: mini_linq :: resume_physical, mini_linq :: access)
} emitted
{
    let __cq_store_R : & :: std :: collections :: HashMap < Value, :: std ::
    vec :: Vec < Value >> = R;
} result
{
    stored R at(__cq_store_R) operations :: cq_example_providers ::
    hash_relation
}
```

Actual output (formatted where Rust syntax permits):

```rust
::cq_example_providers::sorted_relation! {
    reply_to(:: mini_linq :: resume_physical)
    {
        plan
        {
            struct ProjectedChain; relation L(a : Value); relation
            R(a : Value, b : Value); relation S(b : Value, c : Value);
            answer(a, c) : - L(a), R(a, b), S(b, c); relational
            {
                r0 = rename L { a - > a }; r1 = rename R { a - > a, b - > b };
                r2 = natural_join r0 with r1; r3 = rename S
                { b - > b, c - > c }; r4 = natural_join r2 with r3; r5 =
                project r4 keep { a, c }; output r5 as answer(a, c);
            } indexes { i0 = @ r1 [0]; i1 = @ r3 [0]; } uses
            { r2.right = i0; r4.right = i1; } physical
            {
                relations
                {
                    L = :: cq_example_providers :: slice_rows; R = ::
                    cq_example_providers :: hash_relation; S = ::
                    cq_example_providers :: sorted_relation; r0 = ::
                    cq_example_providers :: slice_rows; r1 = ::
                    cq_example_providers :: hash_relation; r2 = ::
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
                    cq_example_providers :: rename; r2 = :: cq_example_providers
                    :: indexed_join; r3 = :: cq_example_providers :: rename; r4
                    = :: cq_example_providers :: indexed_join; r5 = ::
                    cq_example_providers :: project;
                }
            }
        } environment
        {
            stored L at(__cq_store_L) operations :: cq_example_providers ::
            slice_rows stored R at(__cq_store_R) operations ::
            cq_example_providers :: hash_relation
        } emitted
        {
            let __cq_store_L : & [Value] = L; let __cq_store_R : & :: std ::
            collections :: HashMap < Value, :: std :: vec :: Vec < Value > > =
            R;
        } pending { input S from(S) }
        continuations(:: mini_linq :: resume_physical, mini_linq :: access)
    } plan
    {
        struct ProjectedChain; relation L(a : Value); relation
        R(a : Value, b : Value); relation S(b : Value, c : Value);
        answer(a, c) : - L(a), R(a, b), S(b, c); relational
        {
            r0 = rename L { a - > a }; r1 = rename R { a - > a, b - > b }; r2
            = natural_join r0 with r1; r3 = rename S { b - > b, c - > c }; r4
            = natural_join r2 with r3; r5 = project r4 keep { a, c }; output
            r5 as answer(a, c);
        } indexes { i0 = @ r1 [0]; i1 = @ r3 [0]; } uses
        { r2.right = i0; r4.right = i1; } physical
        {
            relations
            {
                L = :: cq_example_providers :: slice_rows; R = ::
                cq_example_providers :: hash_relation; S = ::
                cq_example_providers :: sorted_relation; r0 = ::
                cq_example_providers :: slice_rows; r1 = ::
                cq_example_providers :: hash_relation; r2 = ::
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
                cq_example_providers :: rename; r2 = :: cq_example_providers
                :: indexed_join; r3 = :: cq_example_providers :: rename; r4 =
                :: cq_example_providers :: indexed_join; r5 = ::
                cq_example_providers :: project;
            }
        }
    } environment
    {
        stored L at(__cq_store_L) operations :: cq_example_providers ::
        slice_rows stored R at(__cq_store_R) operations ::
        cq_example_providers :: hash_relation
    } request { input S from(S) }
}

```

---

## 2172002/009-cq-example-providers::sorted_relation

[Raw input](raw/2172002/009-cq-example-providers::sorted_relation.input.tokens) · [Raw output](raw/2172002/009-cq-example-providers::sorted_relation.output.tokens)

Actual input:

```rust
reply_to(:: mini_linq :: resume_physical)
{
    plan
    {
        struct ProjectedChain; relation L(a : Value); relation
        R(a : Value, b : Value); relation S(b : Value, c : Value);
        answer(a, c) : - L(a), R(a, b), S(b, c); relational
        {
            r0 = rename L { a - > a }; r1 = rename R { a - > a, b - > b }; r2
            = natural_join r0 with r1; r3 = rename S { b - > b, c - > c }; r4
            = natural_join r2 with r3; r5 = project r4 keep { a, c }; output
            r5 as answer(a, c);
        } indexes { i0 = @ r1 [0]; i1 = @ r3 [0]; } uses
        { r2.right = i0; r4.right = i1; } physical
        {
            relations
            {
                L = :: cq_example_providers :: slice_rows; R = ::
                cq_example_providers :: hash_relation; S = ::
                cq_example_providers :: sorted_relation; r0 = ::
                cq_example_providers :: slice_rows; r1 = ::
                cq_example_providers :: hash_relation; r2 = ::
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
                cq_example_providers :: rename; r2 = :: cq_example_providers
                :: indexed_join; r3 = :: cq_example_providers :: rename; r4 =
                :: cq_example_providers :: indexed_join; r5 = ::
                cq_example_providers :: project;
            }
        }
    } environment
    {
        stored L at(__cq_store_L) operations :: cq_example_providers ::
        slice_rows stored R at(__cq_store_R) operations ::
        cq_example_providers :: hash_relation
    } emitted
    {
        let __cq_store_L : & [Value] = L; let __cq_store_R : & :: std ::
        collections :: HashMap < Value, :: std :: vec :: Vec < Value > > = R;
    } pending { input S from(S) }
    continuations(:: mini_linq :: resume_physical, mini_linq :: access)
} plan
{
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
} environment
{
    stored L at(__cq_store_L) operations :: cq_example_providers :: slice_rows
    stored R at(__cq_store_R) operations :: cq_example_providers ::
    hash_relation
} request { input S from(S) }
```

Actual output (formatted where Rust syntax permits):

```rust
::mini_linq::resume_physical! {
    resume
    {
        plan
        {
            struct ProjectedChain; relation L(a : Value); relation
            R(a : Value, b : Value); relation S(b : Value, c : Value);
            answer(a, c) : - L(a), R(a, b), S(b, c); relational
            {
                r0 = rename L { a - > a }; r1 = rename R { a - > a, b - > b };
                r2 = natural_join r0 with r1; r3 = rename S
                { b - > b, c - > c }; r4 = natural_join r2 with r3; r5 =
                project r4 keep { a, c }; output r5 as answer(a, c);
            } indexes { i0 = @ r1 [0]; i1 = @ r3 [0]; } uses
            { r2.right = i0; r4.right = i1; } physical
            {
                relations
                {
                    L = :: cq_example_providers :: slice_rows; R = ::
                    cq_example_providers :: hash_relation; S = ::
                    cq_example_providers :: sorted_relation; r0 = ::
                    cq_example_providers :: slice_rows; r1 = ::
                    cq_example_providers :: hash_relation; r2 = ::
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
                    cq_example_providers :: rename; r2 = :: cq_example_providers
                    :: indexed_join; r3 = :: cq_example_providers :: rename; r4
                    = :: cq_example_providers :: indexed_join; r5 = ::
                    cq_example_providers :: project;
                }
            }
        } environment
        {
            stored L at(__cq_store_L) operations :: cq_example_providers ::
            slice_rows stored R at(__cq_store_R) operations ::
            cq_example_providers :: hash_relation
        } emitted
        {
            let __cq_store_L : & [Value] = L; let __cq_store_R : & :: std ::
            collections :: HashMap < Value, :: std :: vec :: Vec < Value > > =
            R;
        } pending { input S from(S) }
        continuations(:: mini_linq :: resume_physical, mini_linq :: access)
    } emitted
    {
        let __cq_store_S : & :: std :: collections :: BTreeMap < Value, :: std
        :: vec :: Vec < Value >> = S;
    } result
    {
        stored S at(__cq_store_S) operations :: cq_example_providers ::
        sorted_relation
    }
}

```

---

## 2172002/010-resume_physical

[Raw input](raw/2172002/010-resume_physical.input.tokens) · [Raw output](raw/2172002/010-resume_physical.output.tokens)

Actual input:

```rust
resume
{
    plan
    {
        struct ProjectedChain; relation L(a : Value); relation
        R(a : Value, b : Value); relation S(b : Value, c : Value);
        answer(a, c) : - L(a), R(a, b), S(b, c); relational
        {
            r0 = rename L { a - > a }; r1 = rename R { a - > a, b - > b }; r2
            = natural_join r0 with r1; r3 = rename S { b - > b, c - > c }; r4
            = natural_join r2 with r3; r5 = project r4 keep { a, c }; output
            r5 as answer(a, c);
        } indexes { i0 = @ r1 [0]; i1 = @ r3 [0]; } uses
        { r2.right = i0; r4.right = i1; } physical
        {
            relations
            {
                L = :: cq_example_providers :: slice_rows; R = ::
                cq_example_providers :: hash_relation; S = ::
                cq_example_providers :: sorted_relation; r0 = ::
                cq_example_providers :: slice_rows; r1 = ::
                cq_example_providers :: hash_relation; r2 = ::
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
                cq_example_providers :: rename; r2 = :: cq_example_providers
                :: indexed_join; r3 = :: cq_example_providers :: rename; r4 =
                :: cq_example_providers :: indexed_join; r5 = ::
                cq_example_providers :: project;
            }
        }
    } environment
    {
        stored L at(__cq_store_L) operations :: cq_example_providers ::
        slice_rows stored R at(__cq_store_R) operations ::
        cq_example_providers :: hash_relation
    } emitted
    {
        let __cq_store_L : & [Value] = L; let __cq_store_R : & :: std ::
        collections :: HashMap < Value, :: std :: vec :: Vec < Value > > = R;
    } pending { input S from(S) }
    continuations(:: mini_linq :: resume_physical, mini_linq :: access)
} emitted
{
    let __cq_store_S : & :: std :: collections :: BTreeMap < Value, :: std ::
    vec :: Vec < Value >> = S;
} result
{
    stored S at(__cq_store_S) operations :: cq_example_providers ::
    sorted_relation
}
```

Actual output (formatted where Rust syntax permits):

```rust
::cq_example_providers::rename! {
    reply_to(:: mini_linq :: resume_physical)
    {
        plan
        {
            struct ProjectedChain; relation L(a : Value); relation
            R(a : Value, b : Value); relation S(b : Value, c : Value);
            answer(a, c) : - L(a), R(a, b), S(b, c); relational
            {
                r0 = rename L { a - > a }; r1 = rename R { a - > a, b - > b };
                r2 = natural_join r0 with r1; r3 = rename S
                { b - > b, c - > c }; r4 = natural_join r2 with r3; r5 =
                project r4 keep { a, c }; output r5 as answer(a, c);
            } indexes { i0 = @ r1 [0]; i1 = @ r3 [0]; } uses
            { r2.right = i0; r4.right = i1; } physical
            {
                relations
                {
                    L = :: cq_example_providers :: slice_rows; R = ::
                    cq_example_providers :: hash_relation; S = ::
                    cq_example_providers :: sorted_relation; r0 = ::
                    cq_example_providers :: slice_rows; r1 = ::
                    cq_example_providers :: hash_relation; r2 = ::
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
                    cq_example_providers :: rename; r2 = :: cq_example_providers
                    :: indexed_join; r3 = :: cq_example_providers :: rename; r4
                    = :: cq_example_providers :: indexed_join; r5 = ::
                    cq_example_providers :: project;
                }
            }
        } environment
        {
            stored L at(__cq_store_L) operations :: cq_example_providers ::
            slice_rows stored R at(__cq_store_R) operations ::
            cq_example_providers :: hash_relation stored S at(__cq_store_S)
            operations :: cq_example_providers :: sorted_relation
        } emitted
        {
            let __cq_store_L : & [Value] = L; let __cq_store_R : & :: std ::
            collections :: HashMap < Value, :: std :: vec :: Vec < Value > > =
            R; let __cq_store_S : & :: std :: collections :: BTreeMap < Value,
            :: std :: vec :: Vec < Value > > = S;
        } pending { lower r0 }
        continuations(:: mini_linq :: resume_physical, mini_linq :: access)
    } plan
    {
        struct ProjectedChain; relation L(a : Value); relation
        R(a : Value, b : Value); relation S(b : Value, c : Value);
        answer(a, c) : - L(a), R(a, b), S(b, c); relational
        {
            r0 = rename L { a - > a }; r1 = rename R { a - > a, b - > b }; r2
            = natural_join r0 with r1; r3 = rename S { b - > b, c - > c }; r4
            = natural_join r2 with r3; r5 = project r4 keep { a, c }; output
            r5 as answer(a, c);
        } indexes { i0 = @ r1 [0]; i1 = @ r3 [0]; } uses
        { r2.right = i0; r4.right = i1; } physical
        {
            relations
            {
                L = :: cq_example_providers :: slice_rows; R = ::
                cq_example_providers :: hash_relation; S = ::
                cq_example_providers :: sorted_relation; r0 = ::
                cq_example_providers :: slice_rows; r1 = ::
                cq_example_providers :: hash_relation; r2 = ::
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
                cq_example_providers :: rename; r2 = :: cq_example_providers
                :: indexed_join; r3 = :: cq_example_providers :: rename; r4 =
                :: cq_example_providers :: indexed_join; r5 = ::
                cq_example_providers :: project;
            }
        }
    } environment
    {
        stored L at(__cq_store_L) operations :: cq_example_providers ::
        slice_rows stored R at(__cq_store_R) operations ::
        cq_example_providers :: hash_relation stored S at(__cq_store_S)
        operations :: cq_example_providers :: sorted_relation
    } request { lower r0 }
}

```

---

## 2172002/011-cq-example-providers::rename

[Raw input](raw/2172002/011-cq-example-providers::rename.input.tokens) · [Raw output](raw/2172002/011-cq-example-providers::rename.output.tokens)

Actual input:

```rust
reply_to(:: mini_linq :: resume_physical)
{
    plan
    {
        struct ProjectedChain; relation L(a : Value); relation
        R(a : Value, b : Value); relation S(b : Value, c : Value);
        answer(a, c) : - L(a), R(a, b), S(b, c); relational
        {
            r0 = rename L { a - > a }; r1 = rename R { a - > a, b - > b }; r2
            = natural_join r0 with r1; r3 = rename S { b - > b, c - > c }; r4
            = natural_join r2 with r3; r5 = project r4 keep { a, c }; output
            r5 as answer(a, c);
        } indexes { i0 = @ r1 [0]; i1 = @ r3 [0]; } uses
        { r2.right = i0; r4.right = i1; } physical
        {
            relations
            {
                L = :: cq_example_providers :: slice_rows; R = ::
                cq_example_providers :: hash_relation; S = ::
                cq_example_providers :: sorted_relation; r0 = ::
                cq_example_providers :: slice_rows; r1 = ::
                cq_example_providers :: hash_relation; r2 = ::
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
                cq_example_providers :: rename; r2 = :: cq_example_providers
                :: indexed_join; r3 = :: cq_example_providers :: rename; r4 =
                :: cq_example_providers :: indexed_join; r5 = ::
                cq_example_providers :: project;
            }
        }
    } environment
    {
        stored L at(__cq_store_L) operations :: cq_example_providers ::
        slice_rows stored R at(__cq_store_R) operations ::
        cq_example_providers :: hash_relation stored S at(__cq_store_S)
        operations :: cq_example_providers :: sorted_relation
    } emitted
    {
        let __cq_store_L : & [Value] = L; let __cq_store_R : & :: std ::
        collections :: HashMap < Value, :: std :: vec :: Vec < Value > > = R;
        let __cq_store_S : & :: std :: collections :: BTreeMap < Value, :: std
        :: vec :: Vec < Value > > = S;
    } pending { lower r0 }
    continuations(:: mini_linq :: resume_physical, mini_linq :: access)
} plan
{
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
} environment
{
    stored L at(__cq_store_L) operations :: cq_example_providers :: slice_rows
    stored R at(__cq_store_R) operations :: cq_example_providers ::
    hash_relation stored S at(__cq_store_S) operations :: cq_example_providers
    :: sorted_relation
} request { lower r0 }
```

Actual output (formatted where Rust syntax permits):

```rust
::cq_example_providers::slice_rows! {
    reply_to(:: mini_linq :: resume_physical)
    {
        plan
        {
            struct ProjectedChain; relation L(a : Value); relation
            R(a : Value, b : Value); relation S(b : Value, c : Value);
            answer(a, c) : - L(a), R(a, b), S(b, c); relational
            {
                r0 = rename L { a - > a }; r1 = rename R { a - > a, b - > b };
                r2 = natural_join r0 with r1; r3 = rename S
                { b - > b, c - > c }; r4 = natural_join r2 with r3; r5 =
                project r4 keep { a, c }; output r5 as answer(a, c);
            } indexes { i0 = @ r1 [0]; i1 = @ r3 [0]; } uses
            { r2.right = i0; r4.right = i1; } physical
            {
                relations
                {
                    L = :: cq_example_providers :: slice_rows; R = ::
                    cq_example_providers :: hash_relation; S = ::
                    cq_example_providers :: sorted_relation; r0 = ::
                    cq_example_providers :: slice_rows; r1 = ::
                    cq_example_providers :: hash_relation; r2 = ::
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
                    cq_example_providers :: rename; r2 = :: cq_example_providers
                    :: indexed_join; r3 = :: cq_example_providers :: rename; r4
                    = :: cq_example_providers :: indexed_join; r5 = ::
                    cq_example_providers :: project;
                }
            }
        } environment
        {
            stored L at(__cq_store_L) operations :: cq_example_providers ::
            slice_rows stored R at(__cq_store_R) operations ::
            cq_example_providers :: hash_relation stored S at(__cq_store_S)
            operations :: cq_example_providers :: sorted_relation
        } emitted
        {
            let __cq_store_L : & [Value] = L; let __cq_store_R : & :: std ::
            collections :: HashMap < Value, :: std :: vec :: Vec < Value > > =
            R; let __cq_store_S : & :: std :: collections :: BTreeMap < Value,
            :: std :: vec :: Vec < Value > > = S;
        } pending { lower r0 }
        continuations(:: mini_linq :: resume_physical, mini_linq :: access)
    } plan
    {
        struct ProjectedChain; relation L(a : Value); relation
        R(a : Value, b : Value); relation S(b : Value, c : Value);
        answer(a, c) : - L(a), R(a, b), S(b, c); relational
        {
            r0 = rename L { a - > a }; r1 = rename R { a - > a, b - > b }; r2
            = natural_join r0 with r1; r3 = rename S { b - > b, c - > c }; r4
            = natural_join r2 with r3; r5 = project r4 keep { a, c }; output
            r5 as answer(a, c);
        } indexes { i0 = @ r1 [0]; i1 = @ r3 [0]; } uses
        { r2.right = i0; r4.right = i1; } physical
        {
            relations
            {
                L = :: cq_example_providers :: slice_rows; R = ::
                cq_example_providers :: hash_relation; S = ::
                cq_example_providers :: sorted_relation; r0 = ::
                cq_example_providers :: slice_rows; r1 = ::
                cq_example_providers :: hash_relation; r2 = ::
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
                cq_example_providers :: rename; r2 = :: cq_example_providers
                :: indexed_join; r3 = :: cq_example_providers :: rename; r4 =
                :: cq_example_providers :: indexed_join; r5 = ::
                cq_example_providers :: project;
            }
        }
    } environment
    {
        stored L at(__cq_store_L) operations :: cq_example_providers ::
        slice_rows stored R at(__cq_store_R) operations ::
        cq_example_providers :: hash_relation stored S at(__cq_store_S)
        operations :: cq_example_providers :: sorted_relation
    } request { input r0 from(__cq_store_L) }
}

```

---

## 2172002/012-cq-example-providers::slice_rows

[Raw input](raw/2172002/012-cq-example-providers::slice_rows.input.tokens) · [Raw output](raw/2172002/012-cq-example-providers::slice_rows.output.tokens)

Actual input:

```rust
reply_to(:: mini_linq :: resume_physical)
{
    plan
    {
        struct ProjectedChain; relation L(a : Value); relation
        R(a : Value, b : Value); relation S(b : Value, c : Value);
        answer(a, c) : - L(a), R(a, b), S(b, c); relational
        {
            r0 = rename L { a - > a }; r1 = rename R { a - > a, b - > b }; r2
            = natural_join r0 with r1; r3 = rename S { b - > b, c - > c }; r4
            = natural_join r2 with r3; r5 = project r4 keep { a, c }; output
            r5 as answer(a, c);
        } indexes { i0 = @ r1 [0]; i1 = @ r3 [0]; } uses
        { r2.right = i0; r4.right = i1; } physical
        {
            relations
            {
                L = :: cq_example_providers :: slice_rows; R = ::
                cq_example_providers :: hash_relation; S = ::
                cq_example_providers :: sorted_relation; r0 = ::
                cq_example_providers :: slice_rows; r1 = ::
                cq_example_providers :: hash_relation; r2 = ::
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
                cq_example_providers :: rename; r2 = :: cq_example_providers
                :: indexed_join; r3 = :: cq_example_providers :: rename; r4 =
                :: cq_example_providers :: indexed_join; r5 = ::
                cq_example_providers :: project;
            }
        }
    } environment
    {
        stored L at(__cq_store_L) operations :: cq_example_providers ::
        slice_rows stored R at(__cq_store_R) operations ::
        cq_example_providers :: hash_relation stored S at(__cq_store_S)
        operations :: cq_example_providers :: sorted_relation
    } emitted
    {
        let __cq_store_L : & [Value] = L; let __cq_store_R : & :: std ::
        collections :: HashMap < Value, :: std :: vec :: Vec < Value > > = R;
        let __cq_store_S : & :: std :: collections :: BTreeMap < Value, :: std
        :: vec :: Vec < Value > > = S;
    } pending { lower r0 }
    continuations(:: mini_linq :: resume_physical, mini_linq :: access)
} plan
{
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
} environment
{
    stored L at(__cq_store_L) operations :: cq_example_providers :: slice_rows
    stored R at(__cq_store_R) operations :: cq_example_providers ::
    hash_relation stored S at(__cq_store_S) operations :: cq_example_providers
    :: sorted_relation
} request { input r0 from(__cq_store_L) }
```

Actual output (formatted where Rust syntax permits):

```rust
::mini_linq::resume_physical! {
    resume
    {
        plan
        {
            struct ProjectedChain; relation L(a : Value); relation
            R(a : Value, b : Value); relation S(b : Value, c : Value);
            answer(a, c) : - L(a), R(a, b), S(b, c); relational
            {
                r0 = rename L { a - > a }; r1 = rename R { a - > a, b - > b };
                r2 = natural_join r0 with r1; r3 = rename S
                { b - > b, c - > c }; r4 = natural_join r2 with r3; r5 =
                project r4 keep { a, c }; output r5 as answer(a, c);
            } indexes { i0 = @ r1 [0]; i1 = @ r3 [0]; } uses
            { r2.right = i0; r4.right = i1; } physical
            {
                relations
                {
                    L = :: cq_example_providers :: slice_rows; R = ::
                    cq_example_providers :: hash_relation; S = ::
                    cq_example_providers :: sorted_relation; r0 = ::
                    cq_example_providers :: slice_rows; r1 = ::
                    cq_example_providers :: hash_relation; r2 = ::
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
                    cq_example_providers :: rename; r2 = :: cq_example_providers
                    :: indexed_join; r3 = :: cq_example_providers :: rename; r4
                    = :: cq_example_providers :: indexed_join; r5 = ::
                    cq_example_providers :: project;
                }
            }
        } environment
        {
            stored L at(__cq_store_L) operations :: cq_example_providers ::
            slice_rows stored R at(__cq_store_R) operations ::
            cq_example_providers :: hash_relation stored S at(__cq_store_S)
            operations :: cq_example_providers :: sorted_relation
        } emitted
        {
            let __cq_store_L : & [Value] = L; let __cq_store_R : & :: std ::
            collections :: HashMap < Value, :: std :: vec :: Vec < Value > > =
            R; let __cq_store_S : & :: std :: collections :: BTreeMap < Value,
            :: std :: vec :: Vec < Value > > = S;
        } pending { lower r0 }
        continuations(:: mini_linq :: resume_physical, mini_linq :: access)
    } emitted { let __cq_store_r0 : & [Value] = __cq_store_L; } result
    {
        stored r0 at(__cq_store_r0) operations :: cq_example_providers ::
        slice_rows
    }
}

```

---

## 2172002/013-resume_physical

[Raw input](raw/2172002/013-resume_physical.input.tokens) · [Raw output](raw/2172002/013-resume_physical.output.tokens)

Actual input:

```rust
resume
{
    plan
    {
        struct ProjectedChain; relation L(a : Value); relation
        R(a : Value, b : Value); relation S(b : Value, c : Value);
        answer(a, c) : - L(a), R(a, b), S(b, c); relational
        {
            r0 = rename L { a - > a }; r1 = rename R { a - > a, b - > b }; r2
            = natural_join r0 with r1; r3 = rename S { b - > b, c - > c }; r4
            = natural_join r2 with r3; r5 = project r4 keep { a, c }; output
            r5 as answer(a, c);
        } indexes { i0 = @ r1 [0]; i1 = @ r3 [0]; } uses
        { r2.right = i0; r4.right = i1; } physical
        {
            relations
            {
                L = :: cq_example_providers :: slice_rows; R = ::
                cq_example_providers :: hash_relation; S = ::
                cq_example_providers :: sorted_relation; r0 = ::
                cq_example_providers :: slice_rows; r1 = ::
                cq_example_providers :: hash_relation; r2 = ::
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
                cq_example_providers :: rename; r2 = :: cq_example_providers
                :: indexed_join; r3 = :: cq_example_providers :: rename; r4 =
                :: cq_example_providers :: indexed_join; r5 = ::
                cq_example_providers :: project;
            }
        }
    } environment
    {
        stored L at(__cq_store_L) operations :: cq_example_providers ::
        slice_rows stored R at(__cq_store_R) operations ::
        cq_example_providers :: hash_relation stored S at(__cq_store_S)
        operations :: cq_example_providers :: sorted_relation
    } emitted
    {
        let __cq_store_L : & [Value] = L; let __cq_store_R : & :: std ::
        collections :: HashMap < Value, :: std :: vec :: Vec < Value > > = R;
        let __cq_store_S : & :: std :: collections :: BTreeMap < Value, :: std
        :: vec :: Vec < Value > > = S;
    } pending { lower r0 }
    continuations(:: mini_linq :: resume_physical, mini_linq :: access)
} emitted { let __cq_store_r0 : & [Value] = __cq_store_L; } result
{
    stored r0 at(__cq_store_r0) operations :: cq_example_providers ::
    slice_rows
}
```

Actual output (formatted where Rust syntax permits):

```rust
::cq_example_providers::rename! {
    reply_to(:: mini_linq :: resume_physical)
    {
        plan
        {
            struct ProjectedChain; relation L(a : Value); relation
            R(a : Value, b : Value); relation S(b : Value, c : Value);
            answer(a, c) : - L(a), R(a, b), S(b, c); relational
            {
                r0 = rename L { a - > a }; r1 = rename R { a - > a, b - > b };
                r2 = natural_join r0 with r1; r3 = rename S
                { b - > b, c - > c }; r4 = natural_join r2 with r3; r5 =
                project r4 keep { a, c }; output r5 as answer(a, c);
            } indexes { i0 = @ r1 [0]; i1 = @ r3 [0]; } uses
            { r2.right = i0; r4.right = i1; } physical
            {
                relations
                {
                    L = :: cq_example_providers :: slice_rows; R = ::
                    cq_example_providers :: hash_relation; S = ::
                    cq_example_providers :: sorted_relation; r0 = ::
                    cq_example_providers :: slice_rows; r1 = ::
                    cq_example_providers :: hash_relation; r2 = ::
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
                    cq_example_providers :: rename; r2 = :: cq_example_providers
                    :: indexed_join; r3 = :: cq_example_providers :: rename; r4
                    = :: cq_example_providers :: indexed_join; r5 = ::
                    cq_example_providers :: project;
                }
            }
        } environment
        {
            stored L at(__cq_store_L) operations :: cq_example_providers ::
            slice_rows stored R at(__cq_store_R) operations ::
            cq_example_providers :: hash_relation stored S at(__cq_store_S)
            operations :: cq_example_providers :: sorted_relation stored r0
            at(__cq_store_r0) operations :: cq_example_providers :: slice_rows
        } emitted
        {
            let __cq_store_L : & [Value] = L; let __cq_store_R : & :: std ::
            collections :: HashMap < Value, :: std :: vec :: Vec < Value > > =
            R; let __cq_store_S : & :: std :: collections :: BTreeMap < Value,
            :: std :: vec :: Vec < Value > > = S; let __cq_store_r0 : &
            [Value] = __cq_store_L;
        } pending { lower r1 }
        continuations(:: mini_linq :: resume_physical, mini_linq :: access)
    } plan
    {
        struct ProjectedChain; relation L(a : Value); relation
        R(a : Value, b : Value); relation S(b : Value, c : Value);
        answer(a, c) : - L(a), R(a, b), S(b, c); relational
        {
            r0 = rename L { a - > a }; r1 = rename R { a - > a, b - > b }; r2
            = natural_join r0 with r1; r3 = rename S { b - > b, c - > c }; r4
            = natural_join r2 with r3; r5 = project r4 keep { a, c }; output
            r5 as answer(a, c);
        } indexes { i0 = @ r1 [0]; i1 = @ r3 [0]; } uses
        { r2.right = i0; r4.right = i1; } physical
        {
            relations
            {
                L = :: cq_example_providers :: slice_rows; R = ::
                cq_example_providers :: hash_relation; S = ::
                cq_example_providers :: sorted_relation; r0 = ::
                cq_example_providers :: slice_rows; r1 = ::
                cq_example_providers :: hash_relation; r2 = ::
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
                cq_example_providers :: rename; r2 = :: cq_example_providers
                :: indexed_join; r3 = :: cq_example_providers :: rename; r4 =
                :: cq_example_providers :: indexed_join; r5 = ::
                cq_example_providers :: project;
            }
        }
    } environment
    {
        stored L at(__cq_store_L) operations :: cq_example_providers ::
        slice_rows stored R at(__cq_store_R) operations ::
        cq_example_providers :: hash_relation stored S at(__cq_store_S)
        operations :: cq_example_providers :: sorted_relation stored r0
        at(__cq_store_r0) operations :: cq_example_providers :: slice_rows
    } request { lower r1 }
}

```

---

## 2172002/014-cq-example-providers::rename

[Raw input](raw/2172002/014-cq-example-providers::rename.input.tokens) · [Raw output](raw/2172002/014-cq-example-providers::rename.output.tokens)

Actual input:

```rust
reply_to(:: mini_linq :: resume_physical)
{
    plan
    {
        struct ProjectedChain; relation L(a : Value); relation
        R(a : Value, b : Value); relation S(b : Value, c : Value);
        answer(a, c) : - L(a), R(a, b), S(b, c); relational
        {
            r0 = rename L { a - > a }; r1 = rename R { a - > a, b - > b }; r2
            = natural_join r0 with r1; r3 = rename S { b - > b, c - > c }; r4
            = natural_join r2 with r3; r5 = project r4 keep { a, c }; output
            r5 as answer(a, c);
        } indexes { i0 = @ r1 [0]; i1 = @ r3 [0]; } uses
        { r2.right = i0; r4.right = i1; } physical
        {
            relations
            {
                L = :: cq_example_providers :: slice_rows; R = ::
                cq_example_providers :: hash_relation; S = ::
                cq_example_providers :: sorted_relation; r0 = ::
                cq_example_providers :: slice_rows; r1 = ::
                cq_example_providers :: hash_relation; r2 = ::
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
                cq_example_providers :: rename; r2 = :: cq_example_providers
                :: indexed_join; r3 = :: cq_example_providers :: rename; r4 =
                :: cq_example_providers :: indexed_join; r5 = ::
                cq_example_providers :: project;
            }
        }
    } environment
    {
        stored L at(__cq_store_L) operations :: cq_example_providers ::
        slice_rows stored R at(__cq_store_R) operations ::
        cq_example_providers :: hash_relation stored S at(__cq_store_S)
        operations :: cq_example_providers :: sorted_relation stored r0
        at(__cq_store_r0) operations :: cq_example_providers :: slice_rows
    } emitted
    {
        let __cq_store_L : & [Value] = L; let __cq_store_R : & :: std ::
        collections :: HashMap < Value, :: std :: vec :: Vec < Value > > = R;
        let __cq_store_S : & :: std :: collections :: BTreeMap < Value, :: std
        :: vec :: Vec < Value > > = S; let __cq_store_r0 : & [Value] =
        __cq_store_L;
    } pending { lower r1 }
    continuations(:: mini_linq :: resume_physical, mini_linq :: access)
} plan
{
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
} environment
{
    stored L at(__cq_store_L) operations :: cq_example_providers :: slice_rows
    stored R at(__cq_store_R) operations :: cq_example_providers ::
    hash_relation stored S at(__cq_store_S) operations :: cq_example_providers
    :: sorted_relation stored r0 at(__cq_store_r0) operations ::
    cq_example_providers :: slice_rows
} request { lower r1 }
```

Actual output (formatted where Rust syntax permits):

```rust
::cq_example_providers::hash_relation! {
    reply_to(:: mini_linq :: resume_physical)
    {
        plan
        {
            struct ProjectedChain; relation L(a : Value); relation
            R(a : Value, b : Value); relation S(b : Value, c : Value);
            answer(a, c) : - L(a), R(a, b), S(b, c); relational
            {
                r0 = rename L { a - > a }; r1 = rename R { a - > a, b - > b };
                r2 = natural_join r0 with r1; r3 = rename S
                { b - > b, c - > c }; r4 = natural_join r2 with r3; r5 =
                project r4 keep { a, c }; output r5 as answer(a, c);
            } indexes { i0 = @ r1 [0]; i1 = @ r3 [0]; } uses
            { r2.right = i0; r4.right = i1; } physical
            {
                relations
                {
                    L = :: cq_example_providers :: slice_rows; R = ::
                    cq_example_providers :: hash_relation; S = ::
                    cq_example_providers :: sorted_relation; r0 = ::
                    cq_example_providers :: slice_rows; r1 = ::
                    cq_example_providers :: hash_relation; r2 = ::
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
                    cq_example_providers :: rename; r2 = :: cq_example_providers
                    :: indexed_join; r3 = :: cq_example_providers :: rename; r4
                    = :: cq_example_providers :: indexed_join; r5 = ::
                    cq_example_providers :: project;
                }
            }
        } environment
        {
            stored L at(__cq_store_L) operations :: cq_example_providers ::
            slice_rows stored R at(__cq_store_R) operations ::
            cq_example_providers :: hash_relation stored S at(__cq_store_S)
            operations :: cq_example_providers :: sorted_relation stored r0
            at(__cq_store_r0) operations :: cq_example_providers :: slice_rows
        } emitted
        {
            let __cq_store_L : & [Value] = L; let __cq_store_R : & :: std ::
            collections :: HashMap < Value, :: std :: vec :: Vec < Value > > =
            R; let __cq_store_S : & :: std :: collections :: BTreeMap < Value,
            :: std :: vec :: Vec < Value > > = S; let __cq_store_r0 : &
            [Value] = __cq_store_L;
        } pending { lower r1 }
        continuations(:: mini_linq :: resume_physical, mini_linq :: access)
    } plan
    {
        struct ProjectedChain; relation L(a : Value); relation
        R(a : Value, b : Value); relation S(b : Value, c : Value);
        answer(a, c) : - L(a), R(a, b), S(b, c); relational
        {
            r0 = rename L { a - > a }; r1 = rename R { a - > a, b - > b }; r2
            = natural_join r0 with r1; r3 = rename S { b - > b, c - > c }; r4
            = natural_join r2 with r3; r5 = project r4 keep { a, c }; output
            r5 as answer(a, c);
        } indexes { i0 = @ r1 [0]; i1 = @ r3 [0]; } uses
        { r2.right = i0; r4.right = i1; } physical
        {
            relations
            {
                L = :: cq_example_providers :: slice_rows; R = ::
                cq_example_providers :: hash_relation; S = ::
                cq_example_providers :: sorted_relation; r0 = ::
                cq_example_providers :: slice_rows; r1 = ::
                cq_example_providers :: hash_relation; r2 = ::
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
                cq_example_providers :: rename; r2 = :: cq_example_providers
                :: indexed_join; r3 = :: cq_example_providers :: rename; r4 =
                :: cq_example_providers :: indexed_join; r5 = ::
                cq_example_providers :: project;
            }
        }
    } environment
    {
        stored L at(__cq_store_L) operations :: cq_example_providers ::
        slice_rows stored R at(__cq_store_R) operations ::
        cq_example_providers :: hash_relation stored S at(__cq_store_S)
        operations :: cq_example_providers :: sorted_relation stored r0
        at(__cq_store_r0) operations :: cq_example_providers :: slice_rows
    } request { input r1 from(__cq_store_R) }
}

```

---

## 2172002/015-cq-example-providers::hash_relation

[Raw input](raw/2172002/015-cq-example-providers::hash_relation.input.tokens) · [Raw output](raw/2172002/015-cq-example-providers::hash_relation.output.tokens)

Actual input:

```rust
reply_to(:: mini_linq :: resume_physical)
{
    plan
    {
        struct ProjectedChain; relation L(a : Value); relation
        R(a : Value, b : Value); relation S(b : Value, c : Value);
        answer(a, c) : - L(a), R(a, b), S(b, c); relational
        {
            r0 = rename L { a - > a }; r1 = rename R { a - > a, b - > b }; r2
            = natural_join r0 with r1; r3 = rename S { b - > b, c - > c }; r4
            = natural_join r2 with r3; r5 = project r4 keep { a, c }; output
            r5 as answer(a, c);
        } indexes { i0 = @ r1 [0]; i1 = @ r3 [0]; } uses
        { r2.right = i0; r4.right = i1; } physical
        {
            relations
            {
                L = :: cq_example_providers :: slice_rows; R = ::
                cq_example_providers :: hash_relation; S = ::
                cq_example_providers :: sorted_relation; r0 = ::
                cq_example_providers :: slice_rows; r1 = ::
                cq_example_providers :: hash_relation; r2 = ::
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
                cq_example_providers :: rename; r2 = :: cq_example_providers
                :: indexed_join; r3 = :: cq_example_providers :: rename; r4 =
                :: cq_example_providers :: indexed_join; r5 = ::
                cq_example_providers :: project;
            }
        }
    } environment
    {
        stored L at(__cq_store_L) operations :: cq_example_providers ::
        slice_rows stored R at(__cq_store_R) operations ::
        cq_example_providers :: hash_relation stored S at(__cq_store_S)
        operations :: cq_example_providers :: sorted_relation stored r0
        at(__cq_store_r0) operations :: cq_example_providers :: slice_rows
    } emitted
    {
        let __cq_store_L : & [Value] = L; let __cq_store_R : & :: std ::
        collections :: HashMap < Value, :: std :: vec :: Vec < Value > > = R;
        let __cq_store_S : & :: std :: collections :: BTreeMap < Value, :: std
        :: vec :: Vec < Value > > = S; let __cq_store_r0 : & [Value] =
        __cq_store_L;
    } pending { lower r1 }
    continuations(:: mini_linq :: resume_physical, mini_linq :: access)
} plan
{
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
} environment
{
    stored L at(__cq_store_L) operations :: cq_example_providers :: slice_rows
    stored R at(__cq_store_R) operations :: cq_example_providers ::
    hash_relation stored S at(__cq_store_S) operations :: cq_example_providers
    :: sorted_relation stored r0 at(__cq_store_r0) operations ::
    cq_example_providers :: slice_rows
} request { input r1 from(__cq_store_R) }
```

Actual output (formatted where Rust syntax permits):

```rust
::mini_linq::resume_physical! {
    resume
    {
        plan
        {
            struct ProjectedChain; relation L(a : Value); relation
            R(a : Value, b : Value); relation S(b : Value, c : Value);
            answer(a, c) : - L(a), R(a, b), S(b, c); relational
            {
                r0 = rename L { a - > a }; r1 = rename R { a - > a, b - > b };
                r2 = natural_join r0 with r1; r3 = rename S
                { b - > b, c - > c }; r4 = natural_join r2 with r3; r5 =
                project r4 keep { a, c }; output r5 as answer(a, c);
            } indexes { i0 = @ r1 [0]; i1 = @ r3 [0]; } uses
            { r2.right = i0; r4.right = i1; } physical
            {
                relations
                {
                    L = :: cq_example_providers :: slice_rows; R = ::
                    cq_example_providers :: hash_relation; S = ::
                    cq_example_providers :: sorted_relation; r0 = ::
                    cq_example_providers :: slice_rows; r1 = ::
                    cq_example_providers :: hash_relation; r2 = ::
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
                    cq_example_providers :: rename; r2 = :: cq_example_providers
                    :: indexed_join; r3 = :: cq_example_providers :: rename; r4
                    = :: cq_example_providers :: indexed_join; r5 = ::
                    cq_example_providers :: project;
                }
            }
        } environment
        {
            stored L at(__cq_store_L) operations :: cq_example_providers ::
            slice_rows stored R at(__cq_store_R) operations ::
            cq_example_providers :: hash_relation stored S at(__cq_store_S)
            operations :: cq_example_providers :: sorted_relation stored r0
            at(__cq_store_r0) operations :: cq_example_providers :: slice_rows
        } emitted
        {
            let __cq_store_L : & [Value] = L; let __cq_store_R : & :: std ::
            collections :: HashMap < Value, :: std :: vec :: Vec < Value > > =
            R; let __cq_store_S : & :: std :: collections :: BTreeMap < Value,
            :: std :: vec :: Vec < Value > > = S; let __cq_store_r0 : &
            [Value] = __cq_store_L;
        } pending { lower r1 }
        continuations(:: mini_linq :: resume_physical, mini_linq :: access)
    } emitted
    {
        let __cq_store_r1 : & :: std :: collections :: HashMap < Value, :: std
        :: vec :: Vec < Value >> = __cq_store_R;
    } result
    {
        stored r1 at(__cq_store_r1) operations :: cq_example_providers ::
        hash_relation
    }
}

```

---

## 2172002/016-resume_physical

[Raw input](raw/2172002/016-resume_physical.input.tokens) · [Raw output](raw/2172002/016-resume_physical.output.tokens)

Actual input:

```rust
resume
{
    plan
    {
        struct ProjectedChain; relation L(a : Value); relation
        R(a : Value, b : Value); relation S(b : Value, c : Value);
        answer(a, c) : - L(a), R(a, b), S(b, c); relational
        {
            r0 = rename L { a - > a }; r1 = rename R { a - > a, b - > b }; r2
            = natural_join r0 with r1; r3 = rename S { b - > b, c - > c }; r4
            = natural_join r2 with r3; r5 = project r4 keep { a, c }; output
            r5 as answer(a, c);
        } indexes { i0 = @ r1 [0]; i1 = @ r3 [0]; } uses
        { r2.right = i0; r4.right = i1; } physical
        {
            relations
            {
                L = :: cq_example_providers :: slice_rows; R = ::
                cq_example_providers :: hash_relation; S = ::
                cq_example_providers :: sorted_relation; r0 = ::
                cq_example_providers :: slice_rows; r1 = ::
                cq_example_providers :: hash_relation; r2 = ::
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
                cq_example_providers :: rename; r2 = :: cq_example_providers
                :: indexed_join; r3 = :: cq_example_providers :: rename; r4 =
                :: cq_example_providers :: indexed_join; r5 = ::
                cq_example_providers :: project;
            }
        }
    } environment
    {
        stored L at(__cq_store_L) operations :: cq_example_providers ::
        slice_rows stored R at(__cq_store_R) operations ::
        cq_example_providers :: hash_relation stored S at(__cq_store_S)
        operations :: cq_example_providers :: sorted_relation stored r0
        at(__cq_store_r0) operations :: cq_example_providers :: slice_rows
    } emitted
    {
        let __cq_store_L : & [Value] = L; let __cq_store_R : & :: std ::
        collections :: HashMap < Value, :: std :: vec :: Vec < Value > > = R;
        let __cq_store_S : & :: std :: collections :: BTreeMap < Value, :: std
        :: vec :: Vec < Value > > = S; let __cq_store_r0 : & [Value] =
        __cq_store_L;
    } pending { lower r1 }
    continuations(:: mini_linq :: resume_physical, mini_linq :: access)
} emitted
{
    let __cq_store_r1 : & :: std :: collections :: HashMap < Value, :: std ::
    vec :: Vec < Value >> = __cq_store_R;
} result
{
    stored r1 at(__cq_store_r1) operations :: cq_example_providers ::
    hash_relation
}
```

Actual output (formatted where Rust syntax permits):

```rust
::cq_example_providers::hash_relation! {
    reply_to(:: mini_linq :: resume_physical)
    {
        plan
        {
            struct ProjectedChain; relation L(a : Value); relation
            R(a : Value, b : Value); relation S(b : Value, c : Value);
            answer(a, c) : - L(a), R(a, b), S(b, c); relational
            {
                r0 = rename L { a - > a }; r1 = rename R { a - > a, b - > b };
                r2 = natural_join r0 with r1; r3 = rename S
                { b - > b, c - > c }; r4 = natural_join r2 with r3; r5 =
                project r4 keep { a, c }; output r5 as answer(a, c);
            } indexes { i0 = @ r1 [0]; i1 = @ r3 [0]; } uses
            { r2.right = i0; r4.right = i1; } physical
            {
                relations
                {
                    L = :: cq_example_providers :: slice_rows; R = ::
                    cq_example_providers :: hash_relation; S = ::
                    cq_example_providers :: sorted_relation; r0 = ::
                    cq_example_providers :: slice_rows; r1 = ::
                    cq_example_providers :: hash_relation; r2 = ::
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
                    cq_example_providers :: rename; r2 = :: cq_example_providers
                    :: indexed_join; r3 = :: cq_example_providers :: rename; r4
                    = :: cq_example_providers :: indexed_join; r5 = ::
                    cq_example_providers :: project;
                }
            }
        } environment
        {
            stored L at(__cq_store_L) operations :: cq_example_providers ::
            slice_rows stored R at(__cq_store_R) operations ::
            cq_example_providers :: hash_relation stored S at(__cq_store_S)
            operations :: cq_example_providers :: sorted_relation stored r0
            at(__cq_store_r0) operations :: cq_example_providers :: slice_rows
            stored r1 at(__cq_store_r1) operations :: cq_example_providers ::
            hash_relation
        } emitted
        {
            let __cq_store_L : & [Value] = L; let __cq_store_R : & :: std ::
            collections :: HashMap < Value, :: std :: vec :: Vec < Value > > =
            R; let __cq_store_S : & :: std :: collections :: BTreeMap < Value,
            :: std :: vec :: Vec < Value > > = S; let __cq_store_r0 : &
            [Value] = __cq_store_L; let __cq_store_r1 : & :: std ::
            collections :: HashMap < Value, :: std :: vec :: Vec < Value > > =
            __cq_store_R;
        } pending { index i0 }
        continuations(:: mini_linq :: resume_physical, mini_linq :: access)
    } plan
    {
        struct ProjectedChain; relation L(a : Value); relation
        R(a : Value, b : Value); relation S(b : Value, c : Value);
        answer(a, c) : - L(a), R(a, b), S(b, c); relational
        {
            r0 = rename L { a - > a }; r1 = rename R { a - > a, b - > b }; r2
            = natural_join r0 with r1; r3 = rename S { b - > b, c - > c }; r4
            = natural_join r2 with r3; r5 = project r4 keep { a, c }; output
            r5 as answer(a, c);
        } indexes { i0 = @ r1 [0]; i1 = @ r3 [0]; } uses
        { r2.right = i0; r4.right = i1; } physical
        {
            relations
            {
                L = :: cq_example_providers :: slice_rows; R = ::
                cq_example_providers :: hash_relation; S = ::
                cq_example_providers :: sorted_relation; r0 = ::
                cq_example_providers :: slice_rows; r1 = ::
                cq_example_providers :: hash_relation; r2 = ::
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
                cq_example_providers :: rename; r2 = :: cq_example_providers
                :: indexed_join; r3 = :: cq_example_providers :: rename; r4 =
                :: cq_example_providers :: indexed_join; r5 = ::
                cq_example_providers :: project;
            }
        }
    } environment
    {
        stored L at(__cq_store_L) operations :: cq_example_providers ::
        slice_rows stored R at(__cq_store_R) operations ::
        cq_example_providers :: hash_relation stored S at(__cq_store_S)
        operations :: cq_example_providers :: sorted_relation stored r0
        at(__cq_store_r0) operations :: cq_example_providers :: slice_rows
        stored r1 at(__cq_store_r1) operations :: cq_example_providers ::
        hash_relation
    } request { index i0 }
}

```

---

## 2172002/017-cq-example-providers::hash_relation

[Raw input](raw/2172002/017-cq-example-providers::hash_relation.input.tokens) · [Raw output](raw/2172002/017-cq-example-providers::hash_relation.output.tokens)

Actual input:

```rust
reply_to(:: mini_linq :: resume_physical)
{
    plan
    {
        struct ProjectedChain; relation L(a : Value); relation
        R(a : Value, b : Value); relation S(b : Value, c : Value);
        answer(a, c) : - L(a), R(a, b), S(b, c); relational
        {
            r0 = rename L { a - > a }; r1 = rename R { a - > a, b - > b }; r2
            = natural_join r0 with r1; r3 = rename S { b - > b, c - > c }; r4
            = natural_join r2 with r3; r5 = project r4 keep { a, c }; output
            r5 as answer(a, c);
        } indexes { i0 = @ r1 [0]; i1 = @ r3 [0]; } uses
        { r2.right = i0; r4.right = i1; } physical
        {
            relations
            {
                L = :: cq_example_providers :: slice_rows; R = ::
                cq_example_providers :: hash_relation; S = ::
                cq_example_providers :: sorted_relation; r0 = ::
                cq_example_providers :: slice_rows; r1 = ::
                cq_example_providers :: hash_relation; r2 = ::
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
                cq_example_providers :: rename; r2 = :: cq_example_providers
                :: indexed_join; r3 = :: cq_example_providers :: rename; r4 =
                :: cq_example_providers :: indexed_join; r5 = ::
                cq_example_providers :: project;
            }
        }
    } environment
    {
        stored L at(__cq_store_L) operations :: cq_example_providers ::
        slice_rows stored R at(__cq_store_R) operations ::
        cq_example_providers :: hash_relation stored S at(__cq_store_S)
        operations :: cq_example_providers :: sorted_relation stored r0
        at(__cq_store_r0) operations :: cq_example_providers :: slice_rows
        stored r1 at(__cq_store_r1) operations :: cq_example_providers ::
        hash_relation
    } emitted
    {
        let __cq_store_L : & [Value] = L; let __cq_store_R : & :: std ::
        collections :: HashMap < Value, :: std :: vec :: Vec < Value > > = R;
        let __cq_store_S : & :: std :: collections :: BTreeMap < Value, :: std
        :: vec :: Vec < Value > > = S; let __cq_store_r0 : & [Value] =
        __cq_store_L; let __cq_store_r1 : & :: std :: collections :: HashMap <
        Value, :: std :: vec :: Vec < Value > > = __cq_store_R;
    } pending { index i0 }
    continuations(:: mini_linq :: resume_physical, mini_linq :: access)
} plan
{
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
} environment
{
    stored L at(__cq_store_L) operations :: cq_example_providers :: slice_rows
    stored R at(__cq_store_R) operations :: cq_example_providers ::
    hash_relation stored S at(__cq_store_S) operations :: cq_example_providers
    :: sorted_relation stored r0 at(__cq_store_r0) operations ::
    cq_example_providers :: slice_rows stored r1 at(__cq_store_r1) operations
    :: cq_example_providers :: hash_relation
} request { index i0 }
```

Actual output (formatted where Rust syntax permits):

```rust
::mini_linq::resume_physical! {
    resume
    {
        plan
        {
            struct ProjectedChain; relation L(a : Value); relation
            R(a : Value, b : Value); relation S(b : Value, c : Value);
            answer(a, c) : - L(a), R(a, b), S(b, c); relational
            {
                r0 = rename L { a - > a }; r1 = rename R { a - > a, b - > b };
                r2 = natural_join r0 with r1; r3 = rename S
                { b - > b, c - > c }; r4 = natural_join r2 with r3; r5 =
                project r4 keep { a, c }; output r5 as answer(a, c);
            } indexes { i0 = @ r1 [0]; i1 = @ r3 [0]; } uses
            { r2.right = i0; r4.right = i1; } physical
            {
                relations
                {
                    L = :: cq_example_providers :: slice_rows; R = ::
                    cq_example_providers :: hash_relation; S = ::
                    cq_example_providers :: sorted_relation; r0 = ::
                    cq_example_providers :: slice_rows; r1 = ::
                    cq_example_providers :: hash_relation; r2 = ::
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
                    cq_example_providers :: rename; r2 = :: cq_example_providers
                    :: indexed_join; r3 = :: cq_example_providers :: rename; r4
                    = :: cq_example_providers :: indexed_join; r5 = ::
                    cq_example_providers :: project;
                }
            }
        } environment
        {
            stored L at(__cq_store_L) operations :: cq_example_providers ::
            slice_rows stored R at(__cq_store_R) operations ::
            cq_example_providers :: hash_relation stored S at(__cq_store_S)
            operations :: cq_example_providers :: sorted_relation stored r0
            at(__cq_store_r0) operations :: cq_example_providers :: slice_rows
            stored r1 at(__cq_store_r1) operations :: cq_example_providers ::
            hash_relation
        } emitted
        {
            let __cq_store_L : & [Value] = L; let __cq_store_R : & :: std ::
            collections :: HashMap < Value, :: std :: vec :: Vec < Value > > =
            R; let __cq_store_S : & :: std :: collections :: BTreeMap < Value,
            :: std :: vec :: Vec < Value > > = S; let __cq_store_r0 : &
            [Value] = __cq_store_L; let __cq_store_r1 : & :: std ::
            collections :: HashMap < Value, :: std :: vec :: Vec < Value > > =
            __cq_store_R;
        } pending { index i0 }
        continuations(:: mini_linq :: resume_physical, mini_linq :: access)
    } emitted
    {
        let __cq_index_i0 : & :: std :: collections :: HashMap < Value, :: std
        :: vec :: Vec < Value >> = __cq_store_r1;
    } result
    {
        indexed i0 at(__cq_index_i0) operations :: cq_example_providers ::
        hash_relation
    }
}

```

---

## 2172002/018-resume_physical

[Raw input](raw/2172002/018-resume_physical.input.tokens) · [Raw output](raw/2172002/018-resume_physical.output.tokens)

Actual input:

```rust
resume
{
    plan
    {
        struct ProjectedChain; relation L(a : Value); relation
        R(a : Value, b : Value); relation S(b : Value, c : Value);
        answer(a, c) : - L(a), R(a, b), S(b, c); relational
        {
            r0 = rename L { a - > a }; r1 = rename R { a - > a, b - > b }; r2
            = natural_join r0 with r1; r3 = rename S { b - > b, c - > c }; r4
            = natural_join r2 with r3; r5 = project r4 keep { a, c }; output
            r5 as answer(a, c);
        } indexes { i0 = @ r1 [0]; i1 = @ r3 [0]; } uses
        { r2.right = i0; r4.right = i1; } physical
        {
            relations
            {
                L = :: cq_example_providers :: slice_rows; R = ::
                cq_example_providers :: hash_relation; S = ::
                cq_example_providers :: sorted_relation; r0 = ::
                cq_example_providers :: slice_rows; r1 = ::
                cq_example_providers :: hash_relation; r2 = ::
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
                cq_example_providers :: rename; r2 = :: cq_example_providers
                :: indexed_join; r3 = :: cq_example_providers :: rename; r4 =
                :: cq_example_providers :: indexed_join; r5 = ::
                cq_example_providers :: project;
            }
        }
    } environment
    {
        stored L at(__cq_store_L) operations :: cq_example_providers ::
        slice_rows stored R at(__cq_store_R) operations ::
        cq_example_providers :: hash_relation stored S at(__cq_store_S)
        operations :: cq_example_providers :: sorted_relation stored r0
        at(__cq_store_r0) operations :: cq_example_providers :: slice_rows
        stored r1 at(__cq_store_r1) operations :: cq_example_providers ::
        hash_relation
    } emitted
    {
        let __cq_store_L : & [Value] = L; let __cq_store_R : & :: std ::
        collections :: HashMap < Value, :: std :: vec :: Vec < Value > > = R;
        let __cq_store_S : & :: std :: collections :: BTreeMap < Value, :: std
        :: vec :: Vec < Value > > = S; let __cq_store_r0 : & [Value] =
        __cq_store_L; let __cq_store_r1 : & :: std :: collections :: HashMap <
        Value, :: std :: vec :: Vec < Value > > = __cq_store_R;
    } pending { index i0 }
    continuations(:: mini_linq :: resume_physical, mini_linq :: access)
} emitted
{
    let __cq_index_i0 : & :: std :: collections :: HashMap < Value, :: std ::
    vec :: Vec < Value >> = __cq_store_r1;
} result
{
    indexed i0 at(__cq_index_i0) operations :: cq_example_providers ::
    hash_relation
}
```

Actual output (formatted where Rust syntax permits):

```rust
::cq_example_providers::indexed_join! {
    reply_to(:: mini_linq :: resume_physical)
    {
        plan
        {
            struct ProjectedChain; relation L(a : Value); relation
            R(a : Value, b : Value); relation S(b : Value, c : Value);
            answer(a, c) : - L(a), R(a, b), S(b, c); relational
            {
                r0 = rename L { a - > a }; r1 = rename R { a - > a, b - > b };
                r2 = natural_join r0 with r1; r3 = rename S
                { b - > b, c - > c }; r4 = natural_join r2 with r3; r5 =
                project r4 keep { a, c }; output r5 as answer(a, c);
            } indexes { i0 = @ r1 [0]; i1 = @ r3 [0]; } uses
            { r2.right = i0; r4.right = i1; } physical
            {
                relations
                {
                    L = :: cq_example_providers :: slice_rows; R = ::
                    cq_example_providers :: hash_relation; S = ::
                    cq_example_providers :: sorted_relation; r0 = ::
                    cq_example_providers :: slice_rows; r1 = ::
                    cq_example_providers :: hash_relation; r2 = ::
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
                    cq_example_providers :: rename; r2 = :: cq_example_providers
                    :: indexed_join; r3 = :: cq_example_providers :: rename; r4
                    = :: cq_example_providers :: indexed_join; r5 = ::
                    cq_example_providers :: project;
                }
            }
        } environment
        {
            stored L at(__cq_store_L) operations :: cq_example_providers ::
            slice_rows stored R at(__cq_store_R) operations ::
            cq_example_providers :: hash_relation stored S at(__cq_store_S)
            operations :: cq_example_providers :: sorted_relation stored r0
            at(__cq_store_r0) operations :: cq_example_providers :: slice_rows
            stored r1 at(__cq_store_r1) operations :: cq_example_providers ::
            hash_relation indexed i0 at(__cq_index_i0) operations ::
            cq_example_providers :: hash_relation
        } emitted
        {
            let __cq_store_L : & [Value] = L; let __cq_store_R : & :: std ::
            collections :: HashMap < Value, :: std :: vec :: Vec < Value > > =
            R; let __cq_store_S : & :: std :: collections :: BTreeMap < Value,
            :: std :: vec :: Vec < Value > > = S; let __cq_store_r0 : &
            [Value] = __cq_store_L; let __cq_store_r1 : & :: std ::
            collections :: HashMap < Value, :: std :: vec :: Vec < Value > > =
            __cq_store_R; let __cq_index_i0 : & :: std :: collections ::
            HashMap < Value, :: std :: vec :: Vec < Value > > = __cq_store_r1;
        } pending { lower r2 }
        continuations(:: mini_linq :: resume_physical, mini_linq :: access)
    } plan
    {
        struct ProjectedChain; relation L(a : Value); relation
        R(a : Value, b : Value); relation S(b : Value, c : Value);
        answer(a, c) : - L(a), R(a, b), S(b, c); relational
        {
            r0 = rename L { a - > a }; r1 = rename R { a - > a, b - > b }; r2
            = natural_join r0 with r1; r3 = rename S { b - > b, c - > c }; r4
            = natural_join r2 with r3; r5 = project r4 keep { a, c }; output
            r5 as answer(a, c);
        } indexes { i0 = @ r1 [0]; i1 = @ r3 [0]; } uses
        { r2.right = i0; r4.right = i1; } physical
        {
            relations
            {
                L = :: cq_example_providers :: slice_rows; R = ::
                cq_example_providers :: hash_relation; S = ::
                cq_example_providers :: sorted_relation; r0 = ::
                cq_example_providers :: slice_rows; r1 = ::
                cq_example_providers :: hash_relation; r2 = ::
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
                cq_example_providers :: rename; r2 = :: cq_example_providers
                :: indexed_join; r3 = :: cq_example_providers :: rename; r4 =
                :: cq_example_providers :: indexed_join; r5 = ::
                cq_example_providers :: project;
            }
        }
    } environment
    {
        stored L at(__cq_store_L) operations :: cq_example_providers ::
        slice_rows stored R at(__cq_store_R) operations ::
        cq_example_providers :: hash_relation stored S at(__cq_store_S)
        operations :: cq_example_providers :: sorted_relation stored r0
        at(__cq_store_r0) operations :: cq_example_providers :: slice_rows
        stored r1 at(__cq_store_r1) operations :: cq_example_providers ::
        hash_relation indexed i0 at(__cq_index_i0) operations ::
        cq_example_providers :: hash_relation
    } request { lower r2 }
}

```

---

## 2172002/019-cq-example-providers::indexed_join

[Raw input](raw/2172002/019-cq-example-providers::indexed_join.input.tokens) · [Raw output](raw/2172002/019-cq-example-providers::indexed_join.output.tokens)

Actual input:

```rust
reply_to(:: mini_linq :: resume_physical)
{
    plan
    {
        struct ProjectedChain; relation L(a : Value); relation
        R(a : Value, b : Value); relation S(b : Value, c : Value);
        answer(a, c) : - L(a), R(a, b), S(b, c); relational
        {
            r0 = rename L { a - > a }; r1 = rename R { a - > a, b - > b }; r2
            = natural_join r0 with r1; r3 = rename S { b - > b, c - > c }; r4
            = natural_join r2 with r3; r5 = project r4 keep { a, c }; output
            r5 as answer(a, c);
        } indexes { i0 = @ r1 [0]; i1 = @ r3 [0]; } uses
        { r2.right = i0; r4.right = i1; } physical
        {
            relations
            {
                L = :: cq_example_providers :: slice_rows; R = ::
                cq_example_providers :: hash_relation; S = ::
                cq_example_providers :: sorted_relation; r0 = ::
                cq_example_providers :: slice_rows; r1 = ::
                cq_example_providers :: hash_relation; r2 = ::
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
                cq_example_providers :: rename; r2 = :: cq_example_providers
                :: indexed_join; r3 = :: cq_example_providers :: rename; r4 =
                :: cq_example_providers :: indexed_join; r5 = ::
                cq_example_providers :: project;
            }
        }
    } environment
    {
        stored L at(__cq_store_L) operations :: cq_example_providers ::
        slice_rows stored R at(__cq_store_R) operations ::
        cq_example_providers :: hash_relation stored S at(__cq_store_S)
        operations :: cq_example_providers :: sorted_relation stored r0
        at(__cq_store_r0) operations :: cq_example_providers :: slice_rows
        stored r1 at(__cq_store_r1) operations :: cq_example_providers ::
        hash_relation indexed i0 at(__cq_index_i0) operations ::
        cq_example_providers :: hash_relation
    } emitted
    {
        let __cq_store_L : & [Value] = L; let __cq_store_R : & :: std ::
        collections :: HashMap < Value, :: std :: vec :: Vec < Value > > = R;
        let __cq_store_S : & :: std :: collections :: BTreeMap < Value, :: std
        :: vec :: Vec < Value > > = S; let __cq_store_r0 : & [Value] =
        __cq_store_L; let __cq_store_r1 : & :: std :: collections :: HashMap <
        Value, :: std :: vec :: Vec < Value > > = __cq_store_R; let
        __cq_index_i0 : & :: std :: collections :: HashMap < Value, :: std ::
        vec :: Vec < Value > > = __cq_store_r1;
    } pending { lower r2 }
    continuations(:: mini_linq :: resume_physical, mini_linq :: access)
} plan
{
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
} environment
{
    stored L at(__cq_store_L) operations :: cq_example_providers :: slice_rows
    stored R at(__cq_store_R) operations :: cq_example_providers ::
    hash_relation stored S at(__cq_store_S) operations :: cq_example_providers
    :: sorted_relation stored r0 at(__cq_store_r0) operations ::
    cq_example_providers :: slice_rows stored r1 at(__cq_store_r1) operations
    :: cq_example_providers :: hash_relation indexed i0 at(__cq_index_i0)
    operations :: cq_example_providers :: hash_relation
} request { lower r2 }
```

Actual output (formatted where Rust syntax permits):

```rust
::cq_example_providers::slice_rows! {
    reply_to(:: cq_example_providers :: indexed_join_rows)
    {
        reply_to(:: mini_linq :: resume_physical)
        {
            plan
            {
                struct ProjectedChain; relation L(a : Value); relation
                R(a : Value, b : Value); relation S(b : Value, c : Value);
                answer(a, c) : - L(a), R(a, b), S(b, c); relational
                {
                    r0 = rename L { a - > a }; r1 = rename R
                    { a - > a, b - > b }; r2 = natural_join r0 with r1; r3 =
                    rename S { b - > b, c - > c }; r4 = natural_join r2 with r3;
                    r5 = project r4 keep { a, c }; output r5 as answer(a, c);
                } indexes { i0 = @ r1 [0]; i1 = @ r3 [0]; } uses
                { r2.right = i0; r4.right = i1; } physical
                {
                    relations
                    {
                        L = :: cq_example_providers :: slice_rows; R = ::
                        cq_example_providers :: hash_relation; S = ::
                        cq_example_providers :: sorted_relation; r0 = ::
                        cq_example_providers :: slice_rows; r1 = ::
                        cq_example_providers :: hash_relation; r2 = ::
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
                        cq_example_providers :: rename; r2 = :: cq_example_providers
                        :: indexed_join; r3 = :: cq_example_providers :: rename; r4
                        = :: cq_example_providers :: indexed_join; r5 = ::
                        cq_example_providers :: project;
                    }
                }
            } environment
            {
                stored L at(__cq_store_L) operations :: cq_example_providers
                :: slice_rows stored R at(__cq_store_R) operations ::
                cq_example_providers :: hash_relation stored S
                at(__cq_store_S) operations :: cq_example_providers ::
                sorted_relation stored r0 at(__cq_store_r0) operations ::
                cq_example_providers :: slice_rows stored r1 at(__cq_store_r1)
                operations :: cq_example_providers :: hash_relation indexed i0
                at(__cq_index_i0) operations :: cq_example_providers ::
                hash_relation
            } emitted
            {
                let __cq_store_L : & [Value] = L; let __cq_store_R : & :: std
                :: collections :: HashMap < Value, :: std :: vec :: Vec <
                Value > > = R; let __cq_store_S : & :: std :: collections ::
                BTreeMap < Value, :: std :: vec :: Vec < Value > > = S; let
                __cq_store_r0 : & [Value] = __cq_store_L; let __cq_store_r1 :
                & :: std :: collections :: HashMap < Value, :: std :: vec ::
                Vec < Value > > = __cq_store_R; let __cq_index_i0 : & :: std
                :: collections :: HashMap < Value, :: std :: vec :: Vec <
                Value > > = __cq_store_r1;
            } pending { lower r2 }
            continuations(:: mini_linq :: resume_physical, mini_linq ::
            access)
        } plan
        {
            struct ProjectedChain; relation L(a : Value); relation
            R(a : Value, b : Value); relation S(b : Value, c : Value);
            answer(a, c) : - L(a), R(a, b), S(b, c); relational
            {
                r0 = rename L { a - > a }; r1 = rename R { a - > a, b - > b };
                r2 = natural_join r0 with r1; r3 = rename S
                { b - > b, c - > c }; r4 = natural_join r2 with r3; r5 =
                project r4 keep { a, c }; output r5 as answer(a, c);
            } indexes { i0 = @ r1 [0]; i1 = @ r3 [0]; } uses
            { r2.right = i0; r4.right = i1; } physical
            {
                relations
                {
                    L = :: cq_example_providers :: slice_rows; R = ::
                    cq_example_providers :: hash_relation; S = ::
                    cq_example_providers :: sorted_relation; r0 = ::
                    cq_example_providers :: slice_rows; r1 = ::
                    cq_example_providers :: hash_relation; r2 = ::
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
                    cq_example_providers :: rename; r2 = :: cq_example_providers
                    :: indexed_join; r3 = :: cq_example_providers :: rename; r4
                    = :: cq_example_providers :: indexed_join; r5 = ::
                    cq_example_providers :: project;
                }
            }
        } environment
        {
            stored L at(__cq_store_L) operations :: cq_example_providers ::
            slice_rows stored R at(__cq_store_R) operations ::
            cq_example_providers :: hash_relation stored S at(__cq_store_S)
            operations :: cq_example_providers :: sorted_relation stored r0
            at(__cq_store_r0) operations :: cq_example_providers :: slice_rows
            stored r1 at(__cq_store_r1) operations :: cq_example_providers ::
            hash_relation indexed i0 at(__cq_index_i0) operations ::
            cq_example_providers :: hash_relation
        } request { lower r2 }
    } plan
    {
        struct ProjectedChain; relation L(a : Value); relation
        R(a : Value, b : Value); relation S(b : Value, c : Value);
        answer(a, c) : - L(a), R(a, b), S(b, c); relational
        {
            r0 = rename L { a - > a }; r1 = rename R { a - > a, b - > b }; r2
            = natural_join r0 with r1; r3 = rename S { b - > b, c - > c }; r4
            = natural_join r2 with r3; r5 = project r4 keep { a, c }; output
            r5 as answer(a, c);
        } indexes { i0 = @ r1 [0]; i1 = @ r3 [0]; } uses
        { r2.right = i0; r4.right = i1; } physical
        {
            relations
            {
                L = :: cq_example_providers :: slice_rows; R = ::
                cq_example_providers :: hash_relation; S = ::
                cq_example_providers :: sorted_relation; r0 = ::
                cq_example_providers :: slice_rows; r1 = ::
                cq_example_providers :: hash_relation; r2 = ::
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
                cq_example_providers :: rename; r2 = :: cq_example_providers
                :: indexed_join; r3 = :: cq_example_providers :: rename; r4 =
                :: cq_example_providers :: indexed_join; r5 = ::
                cq_example_providers :: project;
            }
        }
    } environment
    {
        stored L at(__cq_store_L) operations :: cq_example_providers ::
        slice_rows stored R at(__cq_store_R) operations ::
        cq_example_providers :: hash_relation stored S at(__cq_store_S)
        operations :: cq_example_providers :: sorted_relation stored r0
        at(__cq_store_r0) operations :: cq_example_providers :: slice_rows
        stored r1 at(__cq_store_r1) operations :: cq_example_providers ::
        hash_relation indexed i0 at(__cq_index_i0) operations ::
        cq_example_providers :: hash_relation
    } request { rows r0 }
}

```

---

## 2172002/020-cq-example-providers::slice_rows

[Raw input](raw/2172002/020-cq-example-providers::slice_rows.input.tokens) · [Raw output](raw/2172002/020-cq-example-providers::slice_rows.output.tokens)

Actual input:

```rust
reply_to(:: cq_example_providers :: indexed_join_rows)
{
    reply_to(:: mini_linq :: resume_physical)
    {
        plan
        {
            struct ProjectedChain; relation L(a : Value); relation
            R(a : Value, b : Value); relation S(b : Value, c : Value);
            answer(a, c) : - L(a), R(a, b), S(b, c); relational
            {
                r0 = rename L { a - > a }; r1 = rename R { a - > a, b - > b };
                r2 = natural_join r0 with r1; r3 = rename S
                { b - > b, c - > c }; r4 = natural_join r2 with r3; r5 =
                project r4 keep { a, c }; output r5 as answer(a, c);
            } indexes { i0 = @ r1 [0]; i1 = @ r3 [0]; } uses
            { r2.right = i0; r4.right = i1; } physical
            {
                relations
                {
                    L = :: cq_example_providers :: slice_rows; R = ::
                    cq_example_providers :: hash_relation; S = ::
                    cq_example_providers :: sorted_relation; r0 = ::
                    cq_example_providers :: slice_rows; r1 = ::
                    cq_example_providers :: hash_relation; r2 = ::
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
                    cq_example_providers :: rename; r2 = :: cq_example_providers
                    :: indexed_join; r3 = :: cq_example_providers :: rename; r4
                    = :: cq_example_providers :: indexed_join; r5 = ::
                    cq_example_providers :: project;
                }
            }
        } environment
        {
            stored L at(__cq_store_L) operations :: cq_example_providers ::
            slice_rows stored R at(__cq_store_R) operations ::
            cq_example_providers :: hash_relation stored S at(__cq_store_S)
            operations :: cq_example_providers :: sorted_relation stored r0
            at(__cq_store_r0) operations :: cq_example_providers :: slice_rows
            stored r1 at(__cq_store_r1) operations :: cq_example_providers ::
            hash_relation indexed i0 at(__cq_index_i0) operations ::
            cq_example_providers :: hash_relation
        } emitted
        {
            let __cq_store_L : & [Value] = L; let __cq_store_R : & :: std ::
            collections :: HashMap < Value, :: std :: vec :: Vec < Value > > =
            R; let __cq_store_S : & :: std :: collections :: BTreeMap < Value,
            :: std :: vec :: Vec < Value > > = S; let __cq_store_r0 : &
            [Value] = __cq_store_L; let __cq_store_r1 : & :: std ::
            collections :: HashMap < Value, :: std :: vec :: Vec < Value > > =
            __cq_store_R; let __cq_index_i0 : & :: std :: collections ::
            HashMap < Value, :: std :: vec :: Vec < Value > > = __cq_store_r1;
        } pending { lower r2 }
        continuations(:: mini_linq :: resume_physical, mini_linq :: access)
    } plan
    {
        struct ProjectedChain; relation L(a : Value); relation
        R(a : Value, b : Value); relation S(b : Value, c : Value);
        answer(a, c) : - L(a), R(a, b), S(b, c); relational
        {
            r0 = rename L { a - > a }; r1 = rename R { a - > a, b - > b }; r2
            = natural_join r0 with r1; r3 = rename S { b - > b, c - > c }; r4
            = natural_join r2 with r3; r5 = project r4 keep { a, c }; output
            r5 as answer(a, c);
        } indexes { i0 = @ r1 [0]; i1 = @ r3 [0]; } uses
        { r2.right = i0; r4.right = i1; } physical
        {
            relations
            {
                L = :: cq_example_providers :: slice_rows; R = ::
                cq_example_providers :: hash_relation; S = ::
                cq_example_providers :: sorted_relation; r0 = ::
                cq_example_providers :: slice_rows; r1 = ::
                cq_example_providers :: hash_relation; r2 = ::
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
                cq_example_providers :: rename; r2 = :: cq_example_providers
                :: indexed_join; r3 = :: cq_example_providers :: rename; r4 =
                :: cq_example_providers :: indexed_join; r5 = ::
                cq_example_providers :: project;
            }
        }
    } environment
    {
        stored L at(__cq_store_L) operations :: cq_example_providers ::
        slice_rows stored R at(__cq_store_R) operations ::
        cq_example_providers :: hash_relation stored S at(__cq_store_S)
        operations :: cq_example_providers :: sorted_relation stored r0
        at(__cq_store_r0) operations :: cq_example_providers :: slice_rows
        stored r1 at(__cq_store_r1) operations :: cq_example_providers ::
        hash_relation indexed i0 at(__cq_index_i0) operations ::
        cq_example_providers :: hash_relation
    } request { lower r2 }
} plan
{
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
} environment
{
    stored L at(__cq_store_L) operations :: cq_example_providers :: slice_rows
    stored R at(__cq_store_R) operations :: cq_example_providers ::
    hash_relation stored S at(__cq_store_S) operations :: cq_example_providers
    :: sorted_relation stored r0 at(__cq_store_r0) operations ::
    cq_example_providers :: slice_rows stored r1 at(__cq_store_r1) operations
    :: cq_example_providers :: hash_relation indexed i0 at(__cq_index_i0)
    operations :: cq_example_providers :: hash_relation
} request { rows r0 }
```

Actual output (formatted where Rust syntax permits):

```rust
::cq_example_providers::indexed_join_rows! {
    resume
    {
        reply_to(:: mini_linq :: resume_physical)
        {
            plan
            {
                struct ProjectedChain; relation L(a : Value); relation
                R(a : Value, b : Value); relation S(b : Value, c : Value);
                answer(a, c) : - L(a), R(a, b), S(b, c); relational
                {
                    r0 = rename L { a - > a }; r1 = rename R
                    { a - > a, b - > b }; r2 = natural_join r0 with r1; r3 =
                    rename S { b - > b, c - > c }; r4 = natural_join r2 with r3;
                    r5 = project r4 keep { a, c }; output r5 as answer(a, c);
                } indexes { i0 = @ r1 [0]; i1 = @ r3 [0]; } uses
                { r2.right = i0; r4.right = i1; } physical
                {
                    relations
                    {
                        L = :: cq_example_providers :: slice_rows; R = ::
                        cq_example_providers :: hash_relation; S = ::
                        cq_example_providers :: sorted_relation; r0 = ::
                        cq_example_providers :: slice_rows; r1 = ::
                        cq_example_providers :: hash_relation; r2 = ::
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
                        cq_example_providers :: rename; r2 = :: cq_example_providers
                        :: indexed_join; r3 = :: cq_example_providers :: rename; r4
                        = :: cq_example_providers :: indexed_join; r5 = ::
                        cq_example_providers :: project;
                    }
                }
            } environment
            {
                stored L at(__cq_store_L) operations :: cq_example_providers
                :: slice_rows stored R at(__cq_store_R) operations ::
                cq_example_providers :: hash_relation stored S
                at(__cq_store_S) operations :: cq_example_providers ::
                sorted_relation stored r0 at(__cq_store_r0) operations ::
                cq_example_providers :: slice_rows stored r1 at(__cq_store_r1)
                operations :: cq_example_providers :: hash_relation indexed i0
                at(__cq_index_i0) operations :: cq_example_providers ::
                hash_relation
            } emitted
            {
                let __cq_store_L : & [Value] = L; let __cq_store_R : & :: std
                :: collections :: HashMap < Value, :: std :: vec :: Vec <
                Value > > = R; let __cq_store_S : & :: std :: collections ::
                BTreeMap < Value, :: std :: vec :: Vec < Value > > = S; let
                __cq_store_r0 : & [Value] = __cq_store_L; let __cq_store_r1 :
                & :: std :: collections :: HashMap < Value, :: std :: vec ::
                Vec < Value > > = __cq_store_R; let __cq_index_i0 : & :: std
                :: collections :: HashMap < Value, :: std :: vec :: Vec <
                Value > > = __cq_store_r1;
            } pending { lower r2 }
            continuations(:: mini_linq :: resume_physical, mini_linq ::
            access)
        } plan
        {
            struct ProjectedChain; relation L(a : Value); relation
            R(a : Value, b : Value); relation S(b : Value, c : Value);
            answer(a, c) : - L(a), R(a, b), S(b, c); relational
            {
                r0 = rename L { a - > a }; r1 = rename R { a - > a, b - > b };
                r2 = natural_join r0 with r1; r3 = rename S
                { b - > b, c - > c }; r4 = natural_join r2 with r3; r5 =
                project r4 keep { a, c }; output r5 as answer(a, c);
            } indexes { i0 = @ r1 [0]; i1 = @ r3 [0]; } uses
            { r2.right = i0; r4.right = i1; } physical
            {
                relations
                {
                    L = :: cq_example_providers :: slice_rows; R = ::
                    cq_example_providers :: hash_relation; S = ::
                    cq_example_providers :: sorted_relation; r0 = ::
                    cq_example_providers :: slice_rows; r1 = ::
                    cq_example_providers :: hash_relation; r2 = ::
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
                    cq_example_providers :: rename; r2 = :: cq_example_providers
                    :: indexed_join; r3 = :: cq_example_providers :: rename; r4
                    = :: cq_example_providers :: indexed_join; r5 = ::
                    cq_example_providers :: project;
                }
            }
        } environment
        {
            stored L at(__cq_store_L) operations :: cq_example_providers ::
            slice_rows stored R at(__cq_store_R) operations ::
            cq_example_providers :: hash_relation stored S at(__cq_store_S)
            operations :: cq_example_providers :: sorted_relation stored r0
            at(__cq_store_r0) operations :: cq_example_providers :: slice_rows
            stored r1 at(__cq_store_r1) operations :: cq_example_providers ::
            hash_relation indexed i0 at(__cq_index_i0) operations ::
            cq_example_providers :: hash_relation
        } request { lower r2 }
    } source((__cq_store_r0).iter()) item(a) witness
    { row(a) fields { 0 = (a); } }
}

```

---

## 2172002/021-cq-example-providers::indexed_join_rows

[Raw input](raw/2172002/021-cq-example-providers::indexed_join_rows.input.tokens) · [Raw output](raw/2172002/021-cq-example-providers::indexed_join_rows.output.tokens)

Actual input:

```rust
resume
{
    reply_to(:: mini_linq :: resume_physical)
    {
        plan
        {
            struct ProjectedChain; relation L(a : Value); relation
            R(a : Value, b : Value); relation S(b : Value, c : Value);
            answer(a, c) : - L(a), R(a, b), S(b, c); relational
            {
                r0 = rename L { a - > a }; r1 = rename R { a - > a, b - > b };
                r2 = natural_join r0 with r1; r3 = rename S
                { b - > b, c - > c }; r4 = natural_join r2 with r3; r5 =
                project r4 keep { a, c }; output r5 as answer(a, c);
            } indexes { i0 = @ r1 [0]; i1 = @ r3 [0]; } uses
            { r2.right = i0; r4.right = i1; } physical
            {
                relations
                {
                    L = :: cq_example_providers :: slice_rows; R = ::
                    cq_example_providers :: hash_relation; S = ::
                    cq_example_providers :: sorted_relation; r0 = ::
                    cq_example_providers :: slice_rows; r1 = ::
                    cq_example_providers :: hash_relation; r2 = ::
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
                    cq_example_providers :: rename; r2 = :: cq_example_providers
                    :: indexed_join; r3 = :: cq_example_providers :: rename; r4
                    = :: cq_example_providers :: indexed_join; r5 = ::
                    cq_example_providers :: project;
                }
            }
        } environment
        {
            stored L at(__cq_store_L) operations :: cq_example_providers ::
            slice_rows stored R at(__cq_store_R) operations ::
            cq_example_providers :: hash_relation stored S at(__cq_store_S)
            operations :: cq_example_providers :: sorted_relation stored r0
            at(__cq_store_r0) operations :: cq_example_providers :: slice_rows
            stored r1 at(__cq_store_r1) operations :: cq_example_providers ::
            hash_relation indexed i0 at(__cq_index_i0) operations ::
            cq_example_providers :: hash_relation
        } emitted
        {
            let __cq_store_L : & [Value] = L; let __cq_store_R : & :: std ::
            collections :: HashMap < Value, :: std :: vec :: Vec < Value > > =
            R; let __cq_store_S : & :: std :: collections :: BTreeMap < Value,
            :: std :: vec :: Vec < Value > > = S; let __cq_store_r0 : &
            [Value] = __cq_store_L; let __cq_store_r1 : & :: std ::
            collections :: HashMap < Value, :: std :: vec :: Vec < Value > > =
            __cq_store_R; let __cq_index_i0 : & :: std :: collections ::
            HashMap < Value, :: std :: vec :: Vec < Value > > = __cq_store_r1;
        } pending { lower r2 }
        continuations(:: mini_linq :: resume_physical, mini_linq :: access)
    } plan
    {
        struct ProjectedChain; relation L(a : Value); relation
        R(a : Value, b : Value); relation S(b : Value, c : Value);
        answer(a, c) : - L(a), R(a, b), S(b, c); relational
        {
            r0 = rename L { a - > a }; r1 = rename R { a - > a, b - > b }; r2
            = natural_join r0 with r1; r3 = rename S { b - > b, c - > c }; r4
            = natural_join r2 with r3; r5 = project r4 keep { a, c }; output
            r5 as answer(a, c);
        } indexes { i0 = @ r1 [0]; i1 = @ r3 [0]; } uses
        { r2.right = i0; r4.right = i1; } physical
        {
            relations
            {
                L = :: cq_example_providers :: slice_rows; R = ::
                cq_example_providers :: hash_relation; S = ::
                cq_example_providers :: sorted_relation; r0 = ::
                cq_example_providers :: slice_rows; r1 = ::
                cq_example_providers :: hash_relation; r2 = ::
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
                cq_example_providers :: rename; r2 = :: cq_example_providers
                :: indexed_join; r3 = :: cq_example_providers :: rename; r4 =
                :: cq_example_providers :: indexed_join; r5 = ::
                cq_example_providers :: project;
            }
        }
    } environment
    {
        stored L at(__cq_store_L) operations :: cq_example_providers ::
        slice_rows stored R at(__cq_store_R) operations ::
        cq_example_providers :: hash_relation stored S at(__cq_store_S)
        operations :: cq_example_providers :: sorted_relation stored r0
        at(__cq_store_r0) operations :: cq_example_providers :: slice_rows
        stored r1 at(__cq_store_r1) operations :: cq_example_providers ::
        hash_relation indexed i0 at(__cq_index_i0) operations ::
        cq_example_providers :: hash_relation
    } request { lower r2 }
} source((__cq_store_r0).iter()) item(a) witness
{ row(a) fields { 0 = (a); } }
```

Actual output (formatted where Rust syntax permits):

```rust
::cq_example_providers::hash_relation! {
    reply_to(:: cq_example_providers :: indexed_join_matches)
    {
        request
        {
            reply_to(:: mini_linq :: resume_physical)
            {
                plan
                {
                    struct ProjectedChain; relation L(a : Value); relation
                    R(a : Value, b : Value); relation S(b : Value, c : Value);
                    answer(a, c) : - L(a), R(a, b), S(b, c); relational
                    {
                        r0 = rename L { a - > a }; r1 = rename R
                        { a - > a, b - > b }; r2 = natural_join r0 with r1; r3 =
                        rename S { b - > b, c - > c }; r4 = natural_join r2 with r3;
                        r5 = project r4 keep { a, c }; output r5 as answer(a, c);
                    } indexes { i0 = @ r1 [0]; i1 = @ r3 [0]; } uses
                    { r2.right = i0; r4.right = i1; } physical
                    {
                        relations
                        {
                            L = :: cq_example_providers :: slice_rows; R = ::
                            cq_example_providers :: hash_relation; S = ::
                            cq_example_providers :: sorted_relation; r0 = ::
                            cq_example_providers :: slice_rows; r1 = ::
                            cq_example_providers :: hash_relation; r2 = ::
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
                            cq_example_providers :: rename; r2 = :: cq_example_providers
                            :: indexed_join; r3 = :: cq_example_providers :: rename; r4
                            = :: cq_example_providers :: indexed_join; r5 = ::
                            cq_example_providers :: project;
                        }
                    }
                } environment
                {
                    stored L at(__cq_store_L) operations :: cq_example_providers
                    :: slice_rows stored R at(__cq_store_R) operations ::
                    cq_example_providers :: hash_relation stored S
                    at(__cq_store_S) operations :: cq_example_providers ::
                    sorted_relation stored r0 at(__cq_store_r0) operations ::
                    cq_example_providers :: slice_rows stored r1
                    at(__cq_store_r1) operations :: cq_example_providers ::
                    hash_relation indexed i0 at(__cq_index_i0) operations ::
                    cq_example_providers :: hash_relation
                } emitted
                {
                    let __cq_store_L : & [Value] = L; let __cq_store_R : & ::
                    std :: collections :: HashMap < Value, :: std :: vec :: Vec
                    < Value > > = R; let __cq_store_S : & :: std :: collections
                    :: BTreeMap < Value, :: std :: vec :: Vec < Value > > = S;
                    let __cq_store_r0 : & [Value] = __cq_store_L; let
                    __cq_store_r1 : & :: std :: collections :: HashMap < Value,
                    :: std :: vec :: Vec < Value > > = __cq_store_R; let
                    __cq_index_i0 : & :: std :: collections :: HashMap < Value,
                    :: std :: vec :: Vec < Value > > = __cq_store_r1;
                } pending { lower r2 }
                continuations(:: mini_linq :: resume_physical, mini_linq ::
                access)
            } plan
            {
                struct ProjectedChain; relation L(a : Value); relation
                R(a : Value, b : Value); relation S(b : Value, c : Value);
                answer(a, c) : - L(a), R(a, b), S(b, c); relational
                {
                    r0 = rename L { a - > a }; r1 = rename R
                    { a - > a, b - > b }; r2 = natural_join r0 with r1; r3 =
                    rename S { b - > b, c - > c }; r4 = natural_join r2 with r3;
                    r5 = project r4 keep { a, c }; output r5 as answer(a, c);
                } indexes { i0 = @ r1 [0]; i1 = @ r3 [0]; } uses
                { r2.right = i0; r4.right = i1; } physical
                {
                    relations
                    {
                        L = :: cq_example_providers :: slice_rows; R = ::
                        cq_example_providers :: hash_relation; S = ::
                        cq_example_providers :: sorted_relation; r0 = ::
                        cq_example_providers :: slice_rows; r1 = ::
                        cq_example_providers :: hash_relation; r2 = ::
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
                        cq_example_providers :: rename; r2 = :: cq_example_providers
                        :: indexed_join; r3 = :: cq_example_providers :: rename; r4
                        = :: cq_example_providers :: indexed_join; r5 = ::
                        cq_example_providers :: project;
                    }
                }
            } environment
            {
                stored L at(__cq_store_L) operations :: cq_example_providers
                :: slice_rows stored R at(__cq_store_R) operations ::
                cq_example_providers :: hash_relation stored S
                at(__cq_store_S) operations :: cq_example_providers ::
                sorted_relation stored r0 at(__cq_store_r0) operations ::
                cq_example_providers :: slice_rows stored r1 at(__cq_store_r1)
                operations :: cq_example_providers :: hash_relation indexed i0
                at(__cq_index_i0) operations :: cq_example_providers ::
                hash_relation
            } request { lower r2 }
        } access { r0(a) => bind a <- ((__cq_store_r0).iter()) } witness
        { row(a) fields { 0 = (a); } }
    } plan
    {
        struct ProjectedChain; relation L(a : Value); relation
        R(a : Value, b : Value); relation S(b : Value, c : Value);
        answer(a, c) : - L(a), R(a, b), S(b, c); relational
        {
            r0 = rename L { a - > a }; r1 = rename R { a - > a, b - > b }; r2
            = natural_join r0 with r1; r3 = rename S { b - > b, c - > c }; r4
            = natural_join r2 with r3; r5 = project r4 keep { a, c }; output
            r5 as answer(a, c);
        } indexes { i0 = @ r1 [0]; i1 = @ r3 [0]; } uses
        { r2.right = i0; r4.right = i1; } physical
        {
            relations
            {
                L = :: cq_example_providers :: slice_rows; R = ::
                cq_example_providers :: hash_relation; S = ::
                cq_example_providers :: sorted_relation; r0 = ::
                cq_example_providers :: slice_rows; r1 = ::
                cq_example_providers :: hash_relation; r2 = ::
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
                cq_example_providers :: rename; r2 = :: cq_example_providers
                :: indexed_join; r3 = :: cq_example_providers :: rename; r4 =
                :: cq_example_providers :: indexed_join; r5 = ::
                cq_example_providers :: project;
            }
        }
    } environment
    {
        stored L at(__cq_store_L) operations :: cq_example_providers ::
        slice_rows stored R at(__cq_store_R) operations ::
        cq_example_providers :: hash_relation stored S at(__cq_store_S)
        operations :: cq_example_providers :: sorted_relation stored r0
        at(__cq_store_r0) operations :: cq_example_providers :: slice_rows
        stored r1 at(__cq_store_r1) operations :: cq_example_providers ::
        hash_relation indexed i0 at(__cq_index_i0) operations ::
        cq_example_providers :: hash_relation
    } request { matches i0 values [(a)] }
}

```

---

## 2172002/022-cq-example-providers::hash_relation

[Raw input](raw/2172002/022-cq-example-providers::hash_relation.input.tokens) · [Raw output](raw/2172002/022-cq-example-providers::hash_relation.output.tokens)

Actual input:

```rust
reply_to(:: cq_example_providers :: indexed_join_matches)
{
    request
    {
        reply_to(:: mini_linq :: resume_physical)
        {
            plan
            {
                struct ProjectedChain; relation L(a : Value); relation
                R(a : Value, b : Value); relation S(b : Value, c : Value);
                answer(a, c) : - L(a), R(a, b), S(b, c); relational
                {
                    r0 = rename L { a - > a }; r1 = rename R
                    { a - > a, b - > b }; r2 = natural_join r0 with r1; r3 =
                    rename S { b - > b, c - > c }; r4 = natural_join r2 with r3;
                    r5 = project r4 keep { a, c }; output r5 as answer(a, c);
                } indexes { i0 = @ r1 [0]; i1 = @ r3 [0]; } uses
                { r2.right = i0; r4.right = i1; } physical
                {
                    relations
                    {
                        L = :: cq_example_providers :: slice_rows; R = ::
                        cq_example_providers :: hash_relation; S = ::
                        cq_example_providers :: sorted_relation; r0 = ::
                        cq_example_providers :: slice_rows; r1 = ::
                        cq_example_providers :: hash_relation; r2 = ::
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
                        cq_example_providers :: rename; r2 = :: cq_example_providers
                        :: indexed_join; r3 = :: cq_example_providers :: rename; r4
                        = :: cq_example_providers :: indexed_join; r5 = ::
                        cq_example_providers :: project;
                    }
                }
            } environment
            {
                stored L at(__cq_store_L) operations :: cq_example_providers
                :: slice_rows stored R at(__cq_store_R) operations ::
                cq_example_providers :: hash_relation stored S
                at(__cq_store_S) operations :: cq_example_providers ::
                sorted_relation stored r0 at(__cq_store_r0) operations ::
                cq_example_providers :: slice_rows stored r1 at(__cq_store_r1)
                operations :: cq_example_providers :: hash_relation indexed i0
                at(__cq_index_i0) operations :: cq_example_providers ::
                hash_relation
            } emitted
            {
                let __cq_store_L : & [Value] = L; let __cq_store_R : & :: std
                :: collections :: HashMap < Value, :: std :: vec :: Vec <
                Value > > = R; let __cq_store_S : & :: std :: collections ::
                BTreeMap < Value, :: std :: vec :: Vec < Value > > = S; let
                __cq_store_r0 : & [Value] = __cq_store_L; let __cq_store_r1 :
                & :: std :: collections :: HashMap < Value, :: std :: vec ::
                Vec < Value > > = __cq_store_R; let __cq_index_i0 : & :: std
                :: collections :: HashMap < Value, :: std :: vec :: Vec <
                Value > > = __cq_store_r1;
            } pending { lower r2 }
            continuations(:: mini_linq :: resume_physical, mini_linq ::
            access)
        } plan
        {
            struct ProjectedChain; relation L(a : Value); relation
            R(a : Value, b : Value); relation S(b : Value, c : Value);
            answer(a, c) : - L(a), R(a, b), S(b, c); relational
            {
                r0 = rename L { a - > a }; r1 = rename R { a - > a, b - > b };
                r2 = natural_join r0 with r1; r3 = rename S
                { b - > b, c - > c }; r4 = natural_join r2 with r3; r5 =
                project r4 keep { a, c }; output r5 as answer(a, c);
            } indexes { i0 = @ r1 [0]; i1 = @ r3 [0]; } uses
            { r2.right = i0; r4.right = i1; } physical
            {
                relations
                {
                    L = :: cq_example_providers :: slice_rows; R = ::
                    cq_example_providers :: hash_relation; S = ::
                    cq_example_providers :: sorted_relation; r0 = ::
                    cq_example_providers :: slice_rows; r1 = ::
                    cq_example_providers :: hash_relation; r2 = ::
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
                    cq_example_providers :: rename; r2 = :: cq_example_providers
                    :: indexed_join; r3 = :: cq_example_providers :: rename; r4
                    = :: cq_example_providers :: indexed_join; r5 = ::
                    cq_example_providers :: project;
                }
            }
        } environment
        {
            stored L at(__cq_store_L) operations :: cq_example_providers ::
            slice_rows stored R at(__cq_store_R) operations ::
            cq_example_providers :: hash_relation stored S at(__cq_store_S)
            operations :: cq_example_providers :: sorted_relation stored r0
            at(__cq_store_r0) operations :: cq_example_providers :: slice_rows
            stored r1 at(__cq_store_r1) operations :: cq_example_providers ::
            hash_relation indexed i0 at(__cq_index_i0) operations ::
            cq_example_providers :: hash_relation
        } request { lower r2 }
    } access { r0(a) => bind a <- ((__cq_store_r0).iter()) } witness
    { row(a) fields { 0 = (a); } }
} plan
{
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
} environment
{
    stored L at(__cq_store_L) operations :: cq_example_providers :: slice_rows
    stored R at(__cq_store_R) operations :: cq_example_providers ::
    hash_relation stored S at(__cq_store_S) operations :: cq_example_providers
    :: sorted_relation stored r0 at(__cq_store_r0) operations ::
    cq_example_providers :: slice_rows stored r1 at(__cq_store_r1) operations
    :: cq_example_providers :: hash_relation indexed i0 at(__cq_index_i0)
    operations :: cq_example_providers :: hash_relation
} request { matches i0 values [(a)] }
```

Actual output (formatted where Rust syntax permits):

```rust
::cq_example_providers::indexed_join_matches! {
    resume
    {
        request
        {
            reply_to(:: mini_linq :: resume_physical)
            {
                plan
                {
                    struct ProjectedChain; relation L(a : Value); relation
                    R(a : Value, b : Value); relation S(b : Value, c : Value);
                    answer(a, c) : - L(a), R(a, b), S(b, c); relational
                    {
                        r0 = rename L { a - > a }; r1 = rename R
                        { a - > a, b - > b }; r2 = natural_join r0 with r1; r3 =
                        rename S { b - > b, c - > c }; r4 = natural_join r2 with r3;
                        r5 = project r4 keep { a, c }; output r5 as answer(a, c);
                    } indexes { i0 = @ r1 [0]; i1 = @ r3 [0]; } uses
                    { r2.right = i0; r4.right = i1; } physical
                    {
                        relations
                        {
                            L = :: cq_example_providers :: slice_rows; R = ::
                            cq_example_providers :: hash_relation; S = ::
                            cq_example_providers :: sorted_relation; r0 = ::
                            cq_example_providers :: slice_rows; r1 = ::
                            cq_example_providers :: hash_relation; r2 = ::
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
                            cq_example_providers :: rename; r2 = :: cq_example_providers
                            :: indexed_join; r3 = :: cq_example_providers :: rename; r4
                            = :: cq_example_providers :: indexed_join; r5 = ::
                            cq_example_providers :: project;
                        }
                    }
                } environment
                {
                    stored L at(__cq_store_L) operations :: cq_example_providers
                    :: slice_rows stored R at(__cq_store_R) operations ::
                    cq_example_providers :: hash_relation stored S
                    at(__cq_store_S) operations :: cq_example_providers ::
                    sorted_relation stored r0 at(__cq_store_r0) operations ::
                    cq_example_providers :: slice_rows stored r1
                    at(__cq_store_r1) operations :: cq_example_providers ::
                    hash_relation indexed i0 at(__cq_index_i0) operations ::
                    cq_example_providers :: hash_relation
                } emitted
                {
                    let __cq_store_L : & [Value] = L; let __cq_store_R : & ::
                    std :: collections :: HashMap < Value, :: std :: vec :: Vec
                    < Value > > = R; let __cq_store_S : & :: std :: collections
                    :: BTreeMap < Value, :: std :: vec :: Vec < Value > > = S;
                    let __cq_store_r0 : & [Value] = __cq_store_L; let
                    __cq_store_r1 : & :: std :: collections :: HashMap < Value,
                    :: std :: vec :: Vec < Value > > = __cq_store_R; let
                    __cq_index_i0 : & :: std :: collections :: HashMap < Value,
                    :: std :: vec :: Vec < Value > > = __cq_store_r1;
                } pending { lower r2 }
                continuations(:: mini_linq :: resume_physical, mini_linq ::
                access)
            } plan
            {
                struct ProjectedChain; relation L(a : Value); relation
                R(a : Value, b : Value); relation S(b : Value, c : Value);
                answer(a, c) : - L(a), R(a, b), S(b, c); relational
                {
                    r0 = rename L { a - > a }; r1 = rename R
                    { a - > a, b - > b }; r2 = natural_join r0 with r1; r3 =
                    rename S { b - > b, c - > c }; r4 = natural_join r2 with r3;
                    r5 = project r4 keep { a, c }; output r5 as answer(a, c);
                } indexes { i0 = @ r1 [0]; i1 = @ r3 [0]; } uses
                { r2.right = i0; r4.right = i1; } physical
                {
                    relations
                    {
                        L = :: cq_example_providers :: slice_rows; R = ::
                        cq_example_providers :: hash_relation; S = ::
                        cq_example_providers :: sorted_relation; r0 = ::
                        cq_example_providers :: slice_rows; r1 = ::
                        cq_example_providers :: hash_relation; r2 = ::
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
                        cq_example_providers :: rename; r2 = :: cq_example_providers
                        :: indexed_join; r3 = :: cq_example_providers :: rename; r4
                        = :: cq_example_providers :: indexed_join; r5 = ::
                        cq_example_providers :: project;
                    }
                }
            } environment
            {
                stored L at(__cq_store_L) operations :: cq_example_providers
                :: slice_rows stored R at(__cq_store_R) operations ::
                cq_example_providers :: hash_relation stored S
                at(__cq_store_S) operations :: cq_example_providers ::
                sorted_relation stored r0 at(__cq_store_r0) operations ::
                cq_example_providers :: slice_rows stored r1 at(__cq_store_r1)
                operations :: cq_example_providers :: hash_relation indexed i0
                at(__cq_index_i0) operations :: cq_example_providers ::
                hash_relation
            } request { lower r2 }
        } access { r0(a) => bind a <- ((__cq_store_r0).iter()) } witness
        { row(a) fields { 0 = (a); } }
    } source((__cq_index_i0).get(a).into_iter().flatten()) item(b) witness
    { row((a, b,)) fields { 0 = (a); 1 = (b); } }
}

```

---

## 2172002/023-cq-example-providers::indexed_join_matches

[Raw input](raw/2172002/023-cq-example-providers::indexed_join_matches.input.tokens) · [Raw output](raw/2172002/023-cq-example-providers::indexed_join_matches.output.tokens)

Actual input:

```rust
resume
{
    request
    {
        reply_to(:: mini_linq :: resume_physical)
        {
            plan
            {
                struct ProjectedChain; relation L(a : Value); relation
                R(a : Value, b : Value); relation S(b : Value, c : Value);
                answer(a, c) : - L(a), R(a, b), S(b, c); relational
                {
                    r0 = rename L { a - > a }; r1 = rename R
                    { a - > a, b - > b }; r2 = natural_join r0 with r1; r3 =
                    rename S { b - > b, c - > c }; r4 = natural_join r2 with r3;
                    r5 = project r4 keep { a, c }; output r5 as answer(a, c);
                } indexes { i0 = @ r1 [0]; i1 = @ r3 [0]; } uses
                { r2.right = i0; r4.right = i1; } physical
                {
                    relations
                    {
                        L = :: cq_example_providers :: slice_rows; R = ::
                        cq_example_providers :: hash_relation; S = ::
                        cq_example_providers :: sorted_relation; r0 = ::
                        cq_example_providers :: slice_rows; r1 = ::
                        cq_example_providers :: hash_relation; r2 = ::
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
                        cq_example_providers :: rename; r2 = :: cq_example_providers
                        :: indexed_join; r3 = :: cq_example_providers :: rename; r4
                        = :: cq_example_providers :: indexed_join; r5 = ::
                        cq_example_providers :: project;
                    }
                }
            } environment
            {
                stored L at(__cq_store_L) operations :: cq_example_providers
                :: slice_rows stored R at(__cq_store_R) operations ::
                cq_example_providers :: hash_relation stored S
                at(__cq_store_S) operations :: cq_example_providers ::
                sorted_relation stored r0 at(__cq_store_r0) operations ::
                cq_example_providers :: slice_rows stored r1 at(__cq_store_r1)
                operations :: cq_example_providers :: hash_relation indexed i0
                at(__cq_index_i0) operations :: cq_example_providers ::
                hash_relation
            } emitted
            {
                let __cq_store_L : & [Value] = L; let __cq_store_R : & :: std
                :: collections :: HashMap < Value, :: std :: vec :: Vec <
                Value > > = R; let __cq_store_S : & :: std :: collections ::
                BTreeMap < Value, :: std :: vec :: Vec < Value > > = S; let
                __cq_store_r0 : & [Value] = __cq_store_L; let __cq_store_r1 :
                & :: std :: collections :: HashMap < Value, :: std :: vec ::
                Vec < Value > > = __cq_store_R; let __cq_index_i0 : & :: std
                :: collections :: HashMap < Value, :: std :: vec :: Vec <
                Value > > = __cq_store_r1;
            } pending { lower r2 }
            continuations(:: mini_linq :: resume_physical, mini_linq ::
            access)
        } plan
        {
            struct ProjectedChain; relation L(a : Value); relation
            R(a : Value, b : Value); relation S(b : Value, c : Value);
            answer(a, c) : - L(a), R(a, b), S(b, c); relational
            {
                r0 = rename L { a - > a }; r1 = rename R { a - > a, b - > b };
                r2 = natural_join r0 with r1; r3 = rename S
                { b - > b, c - > c }; r4 = natural_join r2 with r3; r5 =
                project r4 keep { a, c }; output r5 as answer(a, c);
            } indexes { i0 = @ r1 [0]; i1 = @ r3 [0]; } uses
            { r2.right = i0; r4.right = i1; } physical
            {
                relations
                {
                    L = :: cq_example_providers :: slice_rows; R = ::
                    cq_example_providers :: hash_relation; S = ::
                    cq_example_providers :: sorted_relation; r0 = ::
                    cq_example_providers :: slice_rows; r1 = ::
                    cq_example_providers :: hash_relation; r2 = ::
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
                    cq_example_providers :: rename; r2 = :: cq_example_providers
                    :: indexed_join; r3 = :: cq_example_providers :: rename; r4
                    = :: cq_example_providers :: indexed_join; r5 = ::
                    cq_example_providers :: project;
                }
            }
        } environment
        {
            stored L at(__cq_store_L) operations :: cq_example_providers ::
            slice_rows stored R at(__cq_store_R) operations ::
            cq_example_providers :: hash_relation stored S at(__cq_store_S)
            operations :: cq_example_providers :: sorted_relation stored r0
            at(__cq_store_r0) operations :: cq_example_providers :: slice_rows
            stored r1 at(__cq_store_r1) operations :: cq_example_providers ::
            hash_relation indexed i0 at(__cq_index_i0) operations ::
            cq_example_providers :: hash_relation
        } request { lower r2 }
    } access { r0(a) => bind a <- ((__cq_store_r0).iter()) } witness
    { row(a) fields { 0 = (a); } }
} source((__cq_index_i0).get(a).into_iter().flatten()) item(b) witness
{ row((a, b,)) fields { 0 = (a); 1 = (b); } }
```

Actual output (formatted where Rust syntax permits):

```rust
::cq_example_providers::borrowed_fields! {
    reply_to(:: cq_example_providers :: indexed_join_output)
    {
        request
        {
            reply_to(:: mini_linq :: resume_physical)
            {
                plan
                {
                    struct ProjectedChain; relation L(a : Value); relation
                    R(a : Value, b : Value); relation S(b : Value, c : Value);
                    answer(a, c) : - L(a), R(a, b), S(b, c); relational
                    {
                        r0 = rename L { a - > a }; r1 = rename R
                        { a - > a, b - > b }; r2 = natural_join r0 with r1; r3 =
                        rename S { b - > b, c - > c }; r4 = natural_join r2 with r3;
                        r5 = project r4 keep { a, c }; output r5 as answer(a, c);
                    } indexes { i0 = @ r1 [0]; i1 = @ r3 [0]; } uses
                    { r2.right = i0; r4.right = i1; } physical
                    {
                        relations
                        {
                            L = :: cq_example_providers :: slice_rows; R = ::
                            cq_example_providers :: hash_relation; S = ::
                            cq_example_providers :: sorted_relation; r0 = ::
                            cq_example_providers :: slice_rows; r1 = ::
                            cq_example_providers :: hash_relation; r2 = ::
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
                            cq_example_providers :: rename; r2 = :: cq_example_providers
                            :: indexed_join; r3 = :: cq_example_providers :: rename; r4
                            = :: cq_example_providers :: indexed_join; r5 = ::
                            cq_example_providers :: project;
                        }
                    }
                } environment
                {
                    stored L at(__cq_store_L) operations :: cq_example_providers
                    :: slice_rows stored R at(__cq_store_R) operations ::
                    cq_example_providers :: hash_relation stored S
                    at(__cq_store_S) operations :: cq_example_providers ::
                    sorted_relation stored r0 at(__cq_store_r0) operations ::
                    cq_example_providers :: slice_rows stored r1
                    at(__cq_store_r1) operations :: cq_example_providers ::
                    hash_relation indexed i0 at(__cq_index_i0) operations ::
                    cq_example_providers :: hash_relation
                } emitted
                {
                    let __cq_store_L : & [Value] = L; let __cq_store_R : & ::
                    std :: collections :: HashMap < Value, :: std :: vec :: Vec
                    < Value > > = R; let __cq_store_S : & :: std :: collections
                    :: BTreeMap < Value, :: std :: vec :: Vec < Value > > = S;
                    let __cq_store_r0 : & [Value] = __cq_store_L; let
                    __cq_store_r1 : & :: std :: collections :: HashMap < Value,
                    :: std :: vec :: Vec < Value > > = __cq_store_R; let
                    __cq_index_i0 : & :: std :: collections :: HashMap < Value,
                    :: std :: vec :: Vec < Value > > = __cq_store_r1;
                } pending { lower r2 }
                continuations(:: mini_linq :: resume_physical, mini_linq ::
                access)
            } plan
            {
                struct ProjectedChain; relation L(a : Value); relation
                R(a : Value, b : Value); relation S(b : Value, c : Value);
                answer(a, c) : - L(a), R(a, b), S(b, c); relational
                {
                    r0 = rename L { a - > a }; r1 = rename R
                    { a - > a, b - > b }; r2 = natural_join r0 with r1; r3 =
                    rename S { b - > b, c - > c }; r4 = natural_join r2 with r3;
                    r5 = project r4 keep { a, c }; output r5 as answer(a, c);
                } indexes { i0 = @ r1 [0]; i1 = @ r3 [0]; } uses
                { r2.right = i0; r4.right = i1; } physical
                {
                    relations
                    {
                        L = :: cq_example_providers :: slice_rows; R = ::
                        cq_example_providers :: hash_relation; S = ::
                        cq_example_providers :: sorted_relation; r0 = ::
                        cq_example_providers :: slice_rows; r1 = ::
                        cq_example_providers :: hash_relation; r2 = ::
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
                        cq_example_providers :: rename; r2 = :: cq_example_providers
                        :: indexed_join; r3 = :: cq_example_providers :: rename; r4
                        = :: cq_example_providers :: indexed_join; r5 = ::
                        cq_example_providers :: project;
                    }
                }
            } environment
            {
                stored L at(__cq_store_L) operations :: cq_example_providers
                :: slice_rows stored R at(__cq_store_R) operations ::
                cq_example_providers :: hash_relation stored S
                at(__cq_store_S) operations :: cq_example_providers ::
                sorted_relation stored r0 at(__cq_store_r0) operations ::
                cq_example_providers :: slice_rows stored r1 at(__cq_store_r1)
                operations :: cq_example_providers :: hash_relation indexed i0
                at(__cq_index_i0) operations :: cq_example_providers ::
                hash_relation
            } request { lower r2 }
        } access
        {
            r0(a) => bind a <- ((__cq_store_r0).iter()), r1(a, b) => bind b <-
            ((__cq_index_i0).get(a).into_iter().flatten())
        } witness { row(a) fields { 0 = (a); } }
    } plan
    {
        struct ProjectedChain; relation L(a : Value); relation
        R(a : Value, b : Value); relation S(b : Value, c : Value);
        answer(a, c) : - L(a), R(a, b), S(b, c); relational
        {
            r0 = rename L { a - > a }; r1 = rename R { a - > a, b - > b }; r2
            = natural_join r0 with r1; r3 = rename S { b - > b, c - > c }; r4
            = natural_join r2 with r3; r5 = project r4 keep { a, c }; output
            r5 as answer(a, c);
        } indexes { i0 = @ r1 [0]; i1 = @ r3 [0]; } uses
        { r2.right = i0; r4.right = i1; } physical
        {
            relations
            {
                L = :: cq_example_providers :: slice_rows; R = ::
                cq_example_providers :: hash_relation; S = ::
                cq_example_providers :: sorted_relation; r0 = ::
                cq_example_providers :: slice_rows; r1 = ::
                cq_example_providers :: hash_relation; r2 = ::
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
                cq_example_providers :: rename; r2 = :: cq_example_providers
                :: indexed_join; r3 = :: cq_example_providers :: rename; r4 =
                :: cq_example_providers :: indexed_join; r5 = ::
                cq_example_providers :: project;
            }
        }
    } environment
    {
        stored L at(__cq_store_L) operations :: cq_example_providers ::
        slice_rows stored R at(__cq_store_R) operations ::
        cq_example_providers :: hash_relation stored S at(__cq_store_S)
        operations :: cq_example_providers :: sorted_relation stored r0
        at(__cq_store_r0) operations :: cq_example_providers :: slice_rows
        stored r1 at(__cq_store_r1) operations :: cq_example_providers ::
        hash_relation indexed i0 at(__cq_index_i0) operations ::
        cq_example_providers :: hash_relation
    } request
    {
        output r2 fields { 0 = (a); 1 = (b); } whole
        { r0 = (a); r1 = ((a, b,)); }
    }
}

```

---

## 2172002/024-cq-example-providers::borrowed_fields

[Raw input](raw/2172002/024-cq-example-providers::borrowed_fields.input.tokens) · [Raw output](raw/2172002/024-cq-example-providers::borrowed_fields.output.tokens)

Actual input:

```rust
reply_to(:: cq_example_providers :: indexed_join_output)
{
    request
    {
        reply_to(:: mini_linq :: resume_physical)
        {
            plan
            {
                struct ProjectedChain; relation L(a : Value); relation
                R(a : Value, b : Value); relation S(b : Value, c : Value);
                answer(a, c) : - L(a), R(a, b), S(b, c); relational
                {
                    r0 = rename L { a - > a }; r1 = rename R
                    { a - > a, b - > b }; r2 = natural_join r0 with r1; r3 =
                    rename S { b - > b, c - > c }; r4 = natural_join r2 with r3;
                    r5 = project r4 keep { a, c }; output r5 as answer(a, c);
                } indexes { i0 = @ r1 [0]; i1 = @ r3 [0]; } uses
                { r2.right = i0; r4.right = i1; } physical
                {
                    relations
                    {
                        L = :: cq_example_providers :: slice_rows; R = ::
                        cq_example_providers :: hash_relation; S = ::
                        cq_example_providers :: sorted_relation; r0 = ::
                        cq_example_providers :: slice_rows; r1 = ::
                        cq_example_providers :: hash_relation; r2 = ::
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
                        cq_example_providers :: rename; r2 = :: cq_example_providers
                        :: indexed_join; r3 = :: cq_example_providers :: rename; r4
                        = :: cq_example_providers :: indexed_join; r5 = ::
                        cq_example_providers :: project;
                    }
                }
            } environment
            {
                stored L at(__cq_store_L) operations :: cq_example_providers
                :: slice_rows stored R at(__cq_store_R) operations ::
                cq_example_providers :: hash_relation stored S
                at(__cq_store_S) operations :: cq_example_providers ::
                sorted_relation stored r0 at(__cq_store_r0) operations ::
                cq_example_providers :: slice_rows stored r1 at(__cq_store_r1)
                operations :: cq_example_providers :: hash_relation indexed i0
                at(__cq_index_i0) operations :: cq_example_providers ::
                hash_relation
            } emitted
            {
                let __cq_store_L : & [Value] = L; let __cq_store_R : & :: std
                :: collections :: HashMap < Value, :: std :: vec :: Vec <
                Value > > = R; let __cq_store_S : & :: std :: collections ::
                BTreeMap < Value, :: std :: vec :: Vec < Value > > = S; let
                __cq_store_r0 : & [Value] = __cq_store_L; let __cq_store_r1 :
                & :: std :: collections :: HashMap < Value, :: std :: vec ::
                Vec < Value > > = __cq_store_R; let __cq_index_i0 : & :: std
                :: collections :: HashMap < Value, :: std :: vec :: Vec <
                Value > > = __cq_store_r1;
            } pending { lower r2 }
            continuations(:: mini_linq :: resume_physical, mini_linq ::
            access)
        } plan
        {
            struct ProjectedChain; relation L(a : Value); relation
            R(a : Value, b : Value); relation S(b : Value, c : Value);
            answer(a, c) : - L(a), R(a, b), S(b, c); relational
            {
                r0 = rename L { a - > a }; r1 = rename R { a - > a, b - > b };
                r2 = natural_join r0 with r1; r3 = rename S
                { b - > b, c - > c }; r4 = natural_join r2 with r3; r5 =
                project r4 keep { a, c }; output r5 as answer(a, c);
            } indexes { i0 = @ r1 [0]; i1 = @ r3 [0]; } uses
            { r2.right = i0; r4.right = i1; } physical
            {
                relations
                {
                    L = :: cq_example_providers :: slice_rows; R = ::
                    cq_example_providers :: hash_relation; S = ::
                    cq_example_providers :: sorted_relation; r0 = ::
                    cq_example_providers :: slice_rows; r1 = ::
                    cq_example_providers :: hash_relation; r2 = ::
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
                    cq_example_providers :: rename; r2 = :: cq_example_providers
                    :: indexed_join; r3 = :: cq_example_providers :: rename; r4
                    = :: cq_example_providers :: indexed_join; r5 = ::
                    cq_example_providers :: project;
                }
            }
        } environment
        {
            stored L at(__cq_store_L) operations :: cq_example_providers ::
            slice_rows stored R at(__cq_store_R) operations ::
            cq_example_providers :: hash_relation stored S at(__cq_store_S)
            operations :: cq_example_providers :: sorted_relation stored r0
            at(__cq_store_r0) operations :: cq_example_providers :: slice_rows
            stored r1 at(__cq_store_r1) operations :: cq_example_providers ::
            hash_relation indexed i0 at(__cq_index_i0) operations ::
            cq_example_providers :: hash_relation
        } request { lower r2 }
    } access
    {
        r0(a) => bind a <- ((__cq_store_r0).iter()), r1(a, b) => bind b <-
        ((__cq_index_i0).get(a).into_iter().flatten())
    } witness { row(a) fields { 0 = (a); } }
} plan
{
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
} environment
{
    stored L at(__cq_store_L) operations :: cq_example_providers :: slice_rows
    stored R at(__cq_store_R) operations :: cq_example_providers ::
    hash_relation stored S at(__cq_store_S) operations :: cq_example_providers
    :: sorted_relation stored r0 at(__cq_store_r0) operations ::
    cq_example_providers :: slice_rows stored r1 at(__cq_store_r1) operations
    :: cq_example_providers :: hash_relation indexed i0 at(__cq_index_i0)
    operations :: cq_example_providers :: hash_relation
} request
{ output r2 fields { 0 = (a); 1 = (b); } whole { r0 = (a); r1 = ((a, b,)); } }
```

Actual output (formatted where Rust syntax permits):

```rust
::cq_example_providers::indexed_join_output! {
    resume
    {
        request
        {
            reply_to(:: mini_linq :: resume_physical)
            {
                plan
                {
                    struct ProjectedChain; relation L(a : Value); relation
                    R(a : Value, b : Value); relation S(b : Value, c : Value);
                    answer(a, c) : - L(a), R(a, b), S(b, c); relational
                    {
                        r0 = rename L { a - > a }; r1 = rename R
                        { a - > a, b - > b }; r2 = natural_join r0 with r1; r3 =
                        rename S { b - > b, c - > c }; r4 = natural_join r2 with r3;
                        r5 = project r4 keep { a, c }; output r5 as answer(a, c);
                    } indexes { i0 = @ r1 [0]; i1 = @ r3 [0]; } uses
                    { r2.right = i0; r4.right = i1; } physical
                    {
                        relations
                        {
                            L = :: cq_example_providers :: slice_rows; R = ::
                            cq_example_providers :: hash_relation; S = ::
                            cq_example_providers :: sorted_relation; r0 = ::
                            cq_example_providers :: slice_rows; r1 = ::
                            cq_example_providers :: hash_relation; r2 = ::
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
                            cq_example_providers :: rename; r2 = :: cq_example_providers
                            :: indexed_join; r3 = :: cq_example_providers :: rename; r4
                            = :: cq_example_providers :: indexed_join; r5 = ::
                            cq_example_providers :: project;
                        }
                    }
                } environment
                {
                    stored L at(__cq_store_L) operations :: cq_example_providers
                    :: slice_rows stored R at(__cq_store_R) operations ::
                    cq_example_providers :: hash_relation stored S
                    at(__cq_store_S) operations :: cq_example_providers ::
                    sorted_relation stored r0 at(__cq_store_r0) operations ::
                    cq_example_providers :: slice_rows stored r1
                    at(__cq_store_r1) operations :: cq_example_providers ::
                    hash_relation indexed i0 at(__cq_index_i0) operations ::
                    cq_example_providers :: hash_relation
                } emitted
                {
                    let __cq_store_L : & [Value] = L; let __cq_store_R : & ::
                    std :: collections :: HashMap < Value, :: std :: vec :: Vec
                    < Value > > = R; let __cq_store_S : & :: std :: collections
                    :: BTreeMap < Value, :: std :: vec :: Vec < Value > > = S;
                    let __cq_store_r0 : & [Value] = __cq_store_L; let
                    __cq_store_r1 : & :: std :: collections :: HashMap < Value,
                    :: std :: vec :: Vec < Value > > = __cq_store_R; let
                    __cq_index_i0 : & :: std :: collections :: HashMap < Value,
                    :: std :: vec :: Vec < Value > > = __cq_store_r1;
                } pending { lower r2 }
                continuations(:: mini_linq :: resume_physical, mini_linq ::
                access)
            } plan
            {
                struct ProjectedChain; relation L(a : Value); relation
                R(a : Value, b : Value); relation S(b : Value, c : Value);
                answer(a, c) : - L(a), R(a, b), S(b, c); relational
                {
                    r0 = rename L { a - > a }; r1 = rename R
                    { a - > a, b - > b }; r2 = natural_join r0 with r1; r3 =
                    rename S { b - > b, c - > c }; r4 = natural_join r2 with r3;
                    r5 = project r4 keep { a, c }; output r5 as answer(a, c);
                } indexes { i0 = @ r1 [0]; i1 = @ r3 [0]; } uses
                { r2.right = i0; r4.right = i1; } physical
                {
                    relations
                    {
                        L = :: cq_example_providers :: slice_rows; R = ::
                        cq_example_providers :: hash_relation; S = ::
                        cq_example_providers :: sorted_relation; r0 = ::
                        cq_example_providers :: slice_rows; r1 = ::
                        cq_example_providers :: hash_relation; r2 = ::
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
                        cq_example_providers :: rename; r2 = :: cq_example_providers
                        :: indexed_join; r3 = :: cq_example_providers :: rename; r4
                        = :: cq_example_providers :: indexed_join; r5 = ::
                        cq_example_providers :: project;
                    }
                }
            } environment
            {
                stored L at(__cq_store_L) operations :: cq_example_providers
                :: slice_rows stored R at(__cq_store_R) operations ::
                cq_example_providers :: hash_relation stored S
                at(__cq_store_S) operations :: cq_example_providers ::
                sorted_relation stored r0 at(__cq_store_r0) operations ::
                cq_example_providers :: slice_rows stored r1 at(__cq_store_r1)
                operations :: cq_example_providers :: hash_relation indexed i0
                at(__cq_index_i0) operations :: cq_example_providers ::
                hash_relation
            } request { lower r2 }
        } access
        {
            r0(a) => bind a <- ((__cq_store_r0).iter()), r1(a, b) => bind b <-
            ((__cq_index_i0).get(a).into_iter().flatten())
        } witness { row(a) fields { 0 = (a); } }
    } witness { row((a, b,)) fields { 0 = (a); 1 = (b); } }
}

```

---

## 2172002/025-cq-example-providers::indexed_join_output

[Raw input](raw/2172002/025-cq-example-providers::indexed_join_output.input.tokens) · [Raw output](raw/2172002/025-cq-example-providers::indexed_join_output.output.tokens)

Actual input:

```rust
resume
{
    request
    {
        reply_to(:: mini_linq :: resume_physical)
        {
            plan
            {
                struct ProjectedChain; relation L(a : Value); relation
                R(a : Value, b : Value); relation S(b : Value, c : Value);
                answer(a, c) : - L(a), R(a, b), S(b, c); relational
                {
                    r0 = rename L { a - > a }; r1 = rename R
                    { a - > a, b - > b }; r2 = natural_join r0 with r1; r3 =
                    rename S { b - > b, c - > c }; r4 = natural_join r2 with r3;
                    r5 = project r4 keep { a, c }; output r5 as answer(a, c);
                } indexes { i0 = @ r1 [0]; i1 = @ r3 [0]; } uses
                { r2.right = i0; r4.right = i1; } physical
                {
                    relations
                    {
                        L = :: cq_example_providers :: slice_rows; R = ::
                        cq_example_providers :: hash_relation; S = ::
                        cq_example_providers :: sorted_relation; r0 = ::
                        cq_example_providers :: slice_rows; r1 = ::
                        cq_example_providers :: hash_relation; r2 = ::
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
                        cq_example_providers :: rename; r2 = :: cq_example_providers
                        :: indexed_join; r3 = :: cq_example_providers :: rename; r4
                        = :: cq_example_providers :: indexed_join; r5 = ::
                        cq_example_providers :: project;
                    }
                }
            } environment
            {
                stored L at(__cq_store_L) operations :: cq_example_providers
                :: slice_rows stored R at(__cq_store_R) operations ::
                cq_example_providers :: hash_relation stored S
                at(__cq_store_S) operations :: cq_example_providers ::
                sorted_relation stored r0 at(__cq_store_r0) operations ::
                cq_example_providers :: slice_rows stored r1 at(__cq_store_r1)
                operations :: cq_example_providers :: hash_relation indexed i0
                at(__cq_index_i0) operations :: cq_example_providers ::
                hash_relation
            } emitted
            {
                let __cq_store_L : & [Value] = L; let __cq_store_R : & :: std
                :: collections :: HashMap < Value, :: std :: vec :: Vec <
                Value > > = R; let __cq_store_S : & :: std :: collections ::
                BTreeMap < Value, :: std :: vec :: Vec < Value > > = S; let
                __cq_store_r0 : & [Value] = __cq_store_L; let __cq_store_r1 :
                & :: std :: collections :: HashMap < Value, :: std :: vec ::
                Vec < Value > > = __cq_store_R; let __cq_index_i0 : & :: std
                :: collections :: HashMap < Value, :: std :: vec :: Vec <
                Value > > = __cq_store_r1;
            } pending { lower r2 }
            continuations(:: mini_linq :: resume_physical, mini_linq ::
            access)
        } plan
        {
            struct ProjectedChain; relation L(a : Value); relation
            R(a : Value, b : Value); relation S(b : Value, c : Value);
            answer(a, c) : - L(a), R(a, b), S(b, c); relational
            {
                r0 = rename L { a - > a }; r1 = rename R { a - > a, b - > b };
                r2 = natural_join r0 with r1; r3 = rename S
                { b - > b, c - > c }; r4 = natural_join r2 with r3; r5 =
                project r4 keep { a, c }; output r5 as answer(a, c);
            } indexes { i0 = @ r1 [0]; i1 = @ r3 [0]; } uses
            { r2.right = i0; r4.right = i1; } physical
            {
                relations
                {
                    L = :: cq_example_providers :: slice_rows; R = ::
                    cq_example_providers :: hash_relation; S = ::
                    cq_example_providers :: sorted_relation; r0 = ::
                    cq_example_providers :: slice_rows; r1 = ::
                    cq_example_providers :: hash_relation; r2 = ::
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
                    cq_example_providers :: rename; r2 = :: cq_example_providers
                    :: indexed_join; r3 = :: cq_example_providers :: rename; r4
                    = :: cq_example_providers :: indexed_join; r5 = ::
                    cq_example_providers :: project;
                }
            }
        } environment
        {
            stored L at(__cq_store_L) operations :: cq_example_providers ::
            slice_rows stored R at(__cq_store_R) operations ::
            cq_example_providers :: hash_relation stored S at(__cq_store_S)
            operations :: cq_example_providers :: sorted_relation stored r0
            at(__cq_store_r0) operations :: cq_example_providers :: slice_rows
            stored r1 at(__cq_store_r1) operations :: cq_example_providers ::
            hash_relation indexed i0 at(__cq_index_i0) operations ::
            cq_example_providers :: hash_relation
        } request { lower r2 }
    } access
    {
        r0(a) => bind a <- ((__cq_store_r0).iter()), r1(a, b) => bind b <-
        ((__cq_index_i0).get(a).into_iter().flatten())
    } witness { row(a) fields { 0 = (a); } }
} witness { row((a, b,)) fields { 0 = (a); 1 = (b); } }
```

Actual output (formatted where Rust syntax permits):

```rust
::mini_linq::resume_physical! {
    resume
    {
        plan
        {
            struct ProjectedChain; relation L(a : Value); relation
            R(a : Value, b : Value); relation S(b : Value, c : Value);
            answer(a, c) : - L(a), R(a, b), S(b, c); relational
            {
                r0 = rename L { a - > a }; r1 = rename R { a - > a, b - > b };
                r2 = natural_join r0 with r1; r3 = rename S
                { b - > b, c - > c }; r4 = natural_join r2 with r3; r5 =
                project r4 keep { a, c }; output r5 as answer(a, c);
            } indexes { i0 = @ r1 [0]; i1 = @ r3 [0]; } uses
            { r2.right = i0; r4.right = i1; } physical
            {
                relations
                {
                    L = :: cq_example_providers :: slice_rows; R = ::
                    cq_example_providers :: hash_relation; S = ::
                    cq_example_providers :: sorted_relation; r0 = ::
                    cq_example_providers :: slice_rows; r1 = ::
                    cq_example_providers :: hash_relation; r2 = ::
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
                    cq_example_providers :: rename; r2 = :: cq_example_providers
                    :: indexed_join; r3 = :: cq_example_providers :: rename; r4
                    = :: cq_example_providers :: indexed_join; r5 = ::
                    cq_example_providers :: project;
                }
            }
        } environment
        {
            stored L at(__cq_store_L) operations :: cq_example_providers ::
            slice_rows stored R at(__cq_store_R) operations ::
            cq_example_providers :: hash_relation stored S at(__cq_store_S)
            operations :: cq_example_providers :: sorted_relation stored r0
            at(__cq_store_r0) operations :: cq_example_providers :: slice_rows
            stored r1 at(__cq_store_r1) operations :: cq_example_providers ::
            hash_relation indexed i0 at(__cq_index_i0) operations ::
            cq_example_providers :: hash_relation
        } emitted
        {
            let __cq_store_L : & [Value] = L; let __cq_store_R : & :: std ::
            collections :: HashMap < Value, :: std :: vec :: Vec < Value > > =
            R; let __cq_store_S : & :: std :: collections :: BTreeMap < Value,
            :: std :: vec :: Vec < Value > > = S; let __cq_store_r0 : &
            [Value] = __cq_store_L; let __cq_store_r1 : & :: std ::
            collections :: HashMap < Value, :: std :: vec :: Vec < Value > > =
            __cq_store_R; let __cq_index_i0 : & :: std :: collections ::
            HashMap < Value, :: std :: vec :: Vec < Value > > = __cq_store_r1;
        } pending { lower r2 }
        continuations(:: mini_linq :: resume_physical, mini_linq :: access)
    } emitted {} result
    {
        region r2
        {
            r2(a, b) => ((a, b,)) : - r0(a) => bind a <-
            ((__cq_store_r0).iter()), r1(a, b) => bind b <-
            ((__cq_index_i0).get(a).into_iter().flatten());
        }
    }
}

```

---

## 2172002/026-resume_physical

[Raw input](raw/2172002/026-resume_physical.input.tokens) · [Raw output](raw/2172002/026-resume_physical.output.tokens)

Actual input:

```rust
resume
{
    plan
    {
        struct ProjectedChain; relation L(a : Value); relation
        R(a : Value, b : Value); relation S(b : Value, c : Value);
        answer(a, c) : - L(a), R(a, b), S(b, c); relational
        {
            r0 = rename L { a - > a }; r1 = rename R { a - > a, b - > b }; r2
            = natural_join r0 with r1; r3 = rename S { b - > b, c - > c }; r4
            = natural_join r2 with r3; r5 = project r4 keep { a, c }; output
            r5 as answer(a, c);
        } indexes { i0 = @ r1 [0]; i1 = @ r3 [0]; } uses
        { r2.right = i0; r4.right = i1; } physical
        {
            relations
            {
                L = :: cq_example_providers :: slice_rows; R = ::
                cq_example_providers :: hash_relation; S = ::
                cq_example_providers :: sorted_relation; r0 = ::
                cq_example_providers :: slice_rows; r1 = ::
                cq_example_providers :: hash_relation; r2 = ::
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
                cq_example_providers :: rename; r2 = :: cq_example_providers
                :: indexed_join; r3 = :: cq_example_providers :: rename; r4 =
                :: cq_example_providers :: indexed_join; r5 = ::
                cq_example_providers :: project;
            }
        }
    } environment
    {
        stored L at(__cq_store_L) operations :: cq_example_providers ::
        slice_rows stored R at(__cq_store_R) operations ::
        cq_example_providers :: hash_relation stored S at(__cq_store_S)
        operations :: cq_example_providers :: sorted_relation stored r0
        at(__cq_store_r0) operations :: cq_example_providers :: slice_rows
        stored r1 at(__cq_store_r1) operations :: cq_example_providers ::
        hash_relation indexed i0 at(__cq_index_i0) operations ::
        cq_example_providers :: hash_relation
    } emitted
    {
        let __cq_store_L : & [Value] = L; let __cq_store_R : & :: std ::
        collections :: HashMap < Value, :: std :: vec :: Vec < Value > > = R;
        let __cq_store_S : & :: std :: collections :: BTreeMap < Value, :: std
        :: vec :: Vec < Value > > = S; let __cq_store_r0 : & [Value] =
        __cq_store_L; let __cq_store_r1 : & :: std :: collections :: HashMap <
        Value, :: std :: vec :: Vec < Value > > = __cq_store_R; let
        __cq_index_i0 : & :: std :: collections :: HashMap < Value, :: std ::
        vec :: Vec < Value > > = __cq_store_r1;
    } pending { lower r2 }
    continuations(:: mini_linq :: resume_physical, mini_linq :: access)
} emitted {} result
{
    region r2
    {
        r2(a, b) => ((a, b,)) : - r0(a) => bind a <- ((__cq_store_r0).iter()),
        r1(a, b) => bind b <- ((__cq_index_i0).get(a).into_iter().flatten());
    }
}
```

Actual output (formatted where Rust syntax permits):

```rust
::cq_example_providers::rename! {
    reply_to(:: mini_linq :: resume_physical)
    {
        plan
        {
            struct ProjectedChain; relation L(a : Value); relation
            R(a : Value, b : Value); relation S(b : Value, c : Value);
            answer(a, c) : - L(a), R(a, b), S(b, c); relational
            {
                r0 = rename L { a - > a }; r1 = rename R { a - > a, b - > b };
                r2 = natural_join r0 with r1; r3 = rename S
                { b - > b, c - > c }; r4 = natural_join r2 with r3; r5 =
                project r4 keep { a, c }; output r5 as answer(a, c);
            } indexes { i0 = @ r1 [0]; i1 = @ r3 [0]; } uses
            { r2.right = i0; r4.right = i1; } physical
            {
                relations
                {
                    L = :: cq_example_providers :: slice_rows; R = ::
                    cq_example_providers :: hash_relation; S = ::
                    cq_example_providers :: sorted_relation; r0 = ::
                    cq_example_providers :: slice_rows; r1 = ::
                    cq_example_providers :: hash_relation; r2 = ::
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
                    cq_example_providers :: rename; r2 = :: cq_example_providers
                    :: indexed_join; r3 = :: cq_example_providers :: rename; r4
                    = :: cq_example_providers :: indexed_join; r5 = ::
                    cq_example_providers :: project;
                }
            }
        } environment
        {
            stored L at(__cq_store_L) operations :: cq_example_providers ::
            slice_rows stored R at(__cq_store_R) operations ::
            cq_example_providers :: hash_relation stored S at(__cq_store_S)
            operations :: cq_example_providers :: sorted_relation stored r0
            at(__cq_store_r0) operations :: cq_example_providers :: slice_rows
            stored r1 at(__cq_store_r1) operations :: cq_example_providers ::
            hash_relation indexed i0 at(__cq_index_i0) operations ::
            cq_example_providers :: hash_relation region r2
            {
                r2(a, b) => ((a, b,)) : - r0(a) => bind a <-
                ((__cq_store_r0).iter()), r1(a, b) => bind b <-
                ((__cq_index_i0).get(a).into_iter().flatten());
            }
        } emitted
        {
            let __cq_store_L : & [Value] = L; let __cq_store_R : & :: std ::
            collections :: HashMap < Value, :: std :: vec :: Vec < Value > > =
            R; let __cq_store_S : & :: std :: collections :: BTreeMap < Value,
            :: std :: vec :: Vec < Value > > = S; let __cq_store_r0 : &
            [Value] = __cq_store_L; let __cq_store_r1 : & :: std ::
            collections :: HashMap < Value, :: std :: vec :: Vec < Value > > =
            __cq_store_R; let __cq_index_i0 : & :: std :: collections ::
            HashMap < Value, :: std :: vec :: Vec < Value > > = __cq_store_r1;
        } pending { lower r3 }
        continuations(:: mini_linq :: resume_physical, mini_linq :: access)
    } plan
    {
        struct ProjectedChain; relation L(a : Value); relation
        R(a : Value, b : Value); relation S(b : Value, c : Value);
        answer(a, c) : - L(a), R(a, b), S(b, c); relational
        {
            r0 = rename L { a - > a }; r1 = rename R { a - > a, b - > b }; r2
            = natural_join r0 with r1; r3 = rename S { b - > b, c - > c }; r4
            = natural_join r2 with r3; r5 = project r4 keep { a, c }; output
            r5 as answer(a, c);
        } indexes { i0 = @ r1 [0]; i1 = @ r3 [0]; } uses
        { r2.right = i0; r4.right = i1; } physical
        {
            relations
            {
                L = :: cq_example_providers :: slice_rows; R = ::
                cq_example_providers :: hash_relation; S = ::
                cq_example_providers :: sorted_relation; r0 = ::
                cq_example_providers :: slice_rows; r1 = ::
                cq_example_providers :: hash_relation; r2 = ::
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
                cq_example_providers :: rename; r2 = :: cq_example_providers
                :: indexed_join; r3 = :: cq_example_providers :: rename; r4 =
                :: cq_example_providers :: indexed_join; r5 = ::
                cq_example_providers :: project;
            }
        }
    } environment
    {
        stored L at(__cq_store_L) operations :: cq_example_providers ::
        slice_rows stored R at(__cq_store_R) operations ::
        cq_example_providers :: hash_relation stored S at(__cq_store_S)
        operations :: cq_example_providers :: sorted_relation stored r0
        at(__cq_store_r0) operations :: cq_example_providers :: slice_rows
        stored r1 at(__cq_store_r1) operations :: cq_example_providers ::
        hash_relation indexed i0 at(__cq_index_i0) operations ::
        cq_example_providers :: hash_relation region r2
        {
            r2(a, b) => ((a, b,)) : - r0(a) => bind a <-
            ((__cq_store_r0).iter()), r1(a, b) => bind b <-
            ((__cq_index_i0).get(a).into_iter().flatten());
        }
    } request { lower r3 }
}

```

---

## 2172002/027-cq-example-providers::rename

[Raw input](raw/2172002/027-cq-example-providers::rename.input.tokens) · [Raw output](raw/2172002/027-cq-example-providers::rename.output.tokens)

Actual input:

```rust
reply_to(:: mini_linq :: resume_physical)
{
    plan
    {
        struct ProjectedChain; relation L(a : Value); relation
        R(a : Value, b : Value); relation S(b : Value, c : Value);
        answer(a, c) : - L(a), R(a, b), S(b, c); relational
        {
            r0 = rename L { a - > a }; r1 = rename R { a - > a, b - > b }; r2
            = natural_join r0 with r1; r3 = rename S { b - > b, c - > c }; r4
            = natural_join r2 with r3; r5 = project r4 keep { a, c }; output
            r5 as answer(a, c);
        } indexes { i0 = @ r1 [0]; i1 = @ r3 [0]; } uses
        { r2.right = i0; r4.right = i1; } physical
        {
            relations
            {
                L = :: cq_example_providers :: slice_rows; R = ::
                cq_example_providers :: hash_relation; S = ::
                cq_example_providers :: sorted_relation; r0 = ::
                cq_example_providers :: slice_rows; r1 = ::
                cq_example_providers :: hash_relation; r2 = ::
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
                cq_example_providers :: rename; r2 = :: cq_example_providers
                :: indexed_join; r3 = :: cq_example_providers :: rename; r4 =
                :: cq_example_providers :: indexed_join; r5 = ::
                cq_example_providers :: project;
            }
        }
    } environment
    {
        stored L at(__cq_store_L) operations :: cq_example_providers ::
        slice_rows stored R at(__cq_store_R) operations ::
        cq_example_providers :: hash_relation stored S at(__cq_store_S)
        operations :: cq_example_providers :: sorted_relation stored r0
        at(__cq_store_r0) operations :: cq_example_providers :: slice_rows
        stored r1 at(__cq_store_r1) operations :: cq_example_providers ::
        hash_relation indexed i0 at(__cq_index_i0) operations ::
        cq_example_providers :: hash_relation region r2
        {
            r2(a, b) => ((a, b,)) : - r0(a) => bind a <-
            ((__cq_store_r0).iter()), r1(a, b) => bind b <-
            ((__cq_index_i0).get(a).into_iter().flatten());
        }
    } emitted
    {
        let __cq_store_L : & [Value] = L; let __cq_store_R : & :: std ::
        collections :: HashMap < Value, :: std :: vec :: Vec < Value > > = R;
        let __cq_store_S : & :: std :: collections :: BTreeMap < Value, :: std
        :: vec :: Vec < Value > > = S; let __cq_store_r0 : & [Value] =
        __cq_store_L; let __cq_store_r1 : & :: std :: collections :: HashMap <
        Value, :: std :: vec :: Vec < Value > > = __cq_store_R; let
        __cq_index_i0 : & :: std :: collections :: HashMap < Value, :: std ::
        vec :: Vec < Value > > = __cq_store_r1;
    } pending { lower r3 }
    continuations(:: mini_linq :: resume_physical, mini_linq :: access)
} plan
{
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
} environment
{
    stored L at(__cq_store_L) operations :: cq_example_providers :: slice_rows
    stored R at(__cq_store_R) operations :: cq_example_providers ::
    hash_relation stored S at(__cq_store_S) operations :: cq_example_providers
    :: sorted_relation stored r0 at(__cq_store_r0) operations ::
    cq_example_providers :: slice_rows stored r1 at(__cq_store_r1) operations
    :: cq_example_providers :: hash_relation indexed i0 at(__cq_index_i0)
    operations :: cq_example_providers :: hash_relation region r2
    {
        r2(a, b) => ((a, b,)) : - r0(a) => bind a <- ((__cq_store_r0).iter()),
        r1(a, b) => bind b <- ((__cq_index_i0).get(a).into_iter().flatten());
    }
} request { lower r3 }
```

Actual output (formatted where Rust syntax permits):

```rust
::cq_example_providers::sorted_relation! {
    reply_to(:: mini_linq :: resume_physical)
    {
        plan
        {
            struct ProjectedChain; relation L(a : Value); relation
            R(a : Value, b : Value); relation S(b : Value, c : Value);
            answer(a, c) : - L(a), R(a, b), S(b, c); relational
            {
                r0 = rename L { a - > a }; r1 = rename R { a - > a, b - > b };
                r2 = natural_join r0 with r1; r3 = rename S
                { b - > b, c - > c }; r4 = natural_join r2 with r3; r5 =
                project r4 keep { a, c }; output r5 as answer(a, c);
            } indexes { i0 = @ r1 [0]; i1 = @ r3 [0]; } uses
            { r2.right = i0; r4.right = i1; } physical
            {
                relations
                {
                    L = :: cq_example_providers :: slice_rows; R = ::
                    cq_example_providers :: hash_relation; S = ::
                    cq_example_providers :: sorted_relation; r0 = ::
                    cq_example_providers :: slice_rows; r1 = ::
                    cq_example_providers :: hash_relation; r2 = ::
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
                    cq_example_providers :: rename; r2 = :: cq_example_providers
                    :: indexed_join; r3 = :: cq_example_providers :: rename; r4
                    = :: cq_example_providers :: indexed_join; r5 = ::
                    cq_example_providers :: project;
                }
            }
        } environment
        {
            stored L at(__cq_store_L) operations :: cq_example_providers ::
            slice_rows stored R at(__cq_store_R) operations ::
            cq_example_providers :: hash_relation stored S at(__cq_store_S)
            operations :: cq_example_providers :: sorted_relation stored r0
            at(__cq_store_r0) operations :: cq_example_providers :: slice_rows
            stored r1 at(__cq_store_r1) operations :: cq_example_providers ::
            hash_relation indexed i0 at(__cq_index_i0) operations ::
            cq_example_providers :: hash_relation region r2
            {
                r2(a, b) => ((a, b,)) : - r0(a) => bind a <-
                ((__cq_store_r0).iter()), r1(a, b) => bind b <-
                ((__cq_index_i0).get(a).into_iter().flatten());
            }
        } emitted
        {
            let __cq_store_L : & [Value] = L; let __cq_store_R : & :: std ::
            collections :: HashMap < Value, :: std :: vec :: Vec < Value > > =
            R; let __cq_store_S : & :: std :: collections :: BTreeMap < Value,
            :: std :: vec :: Vec < Value > > = S; let __cq_store_r0 : &
            [Value] = __cq_store_L; let __cq_store_r1 : & :: std ::
            collections :: HashMap < Value, :: std :: vec :: Vec < Value > > =
            __cq_store_R; let __cq_index_i0 : & :: std :: collections ::
            HashMap < Value, :: std :: vec :: Vec < Value > > = __cq_store_r1;
        } pending { lower r3 }
        continuations(:: mini_linq :: resume_physical, mini_linq :: access)
    } plan
    {
        struct ProjectedChain; relation L(a : Value); relation
        R(a : Value, b : Value); relation S(b : Value, c : Value);
        answer(a, c) : - L(a), R(a, b), S(b, c); relational
        {
            r0 = rename L { a - > a }; r1 = rename R { a - > a, b - > b }; r2
            = natural_join r0 with r1; r3 = rename S { b - > b, c - > c }; r4
            = natural_join r2 with r3; r5 = project r4 keep { a, c }; output
            r5 as answer(a, c);
        } indexes { i0 = @ r1 [0]; i1 = @ r3 [0]; } uses
        { r2.right = i0; r4.right = i1; } physical
        {
            relations
            {
                L = :: cq_example_providers :: slice_rows; R = ::
                cq_example_providers :: hash_relation; S = ::
                cq_example_providers :: sorted_relation; r0 = ::
                cq_example_providers :: slice_rows; r1 = ::
                cq_example_providers :: hash_relation; r2 = ::
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
                cq_example_providers :: rename; r2 = :: cq_example_providers
                :: indexed_join; r3 = :: cq_example_providers :: rename; r4 =
                :: cq_example_providers :: indexed_join; r5 = ::
                cq_example_providers :: project;
            }
        }
    } environment
    {
        stored L at(__cq_store_L) operations :: cq_example_providers ::
        slice_rows stored R at(__cq_store_R) operations ::
        cq_example_providers :: hash_relation stored S at(__cq_store_S)
        operations :: cq_example_providers :: sorted_relation stored r0
        at(__cq_store_r0) operations :: cq_example_providers :: slice_rows
        stored r1 at(__cq_store_r1) operations :: cq_example_providers ::
        hash_relation indexed i0 at(__cq_index_i0) operations ::
        cq_example_providers :: hash_relation region r2
        {
            r2(a, b) => ((a, b,)) : - r0(a) => bind a <-
            ((__cq_store_r0).iter()), r1(a, b) => bind b <-
            ((__cq_index_i0).get(a).into_iter().flatten());
        }
    } request { input r3 from(__cq_store_S) }
}

```

---

## 2172002/028-cq-example-providers::sorted_relation

[Raw input](raw/2172002/028-cq-example-providers::sorted_relation.input.tokens) · [Raw output](raw/2172002/028-cq-example-providers::sorted_relation.output.tokens)

Actual input:

```rust
reply_to(:: mini_linq :: resume_physical)
{
    plan
    {
        struct ProjectedChain; relation L(a : Value); relation
        R(a : Value, b : Value); relation S(b : Value, c : Value);
        answer(a, c) : - L(a), R(a, b), S(b, c); relational
        {
            r0 = rename L { a - > a }; r1 = rename R { a - > a, b - > b }; r2
            = natural_join r0 with r1; r3 = rename S { b - > b, c - > c }; r4
            = natural_join r2 with r3; r5 = project r4 keep { a, c }; output
            r5 as answer(a, c);
        } indexes { i0 = @ r1 [0]; i1 = @ r3 [0]; } uses
        { r2.right = i0; r4.right = i1; } physical
        {
            relations
            {
                L = :: cq_example_providers :: slice_rows; R = ::
                cq_example_providers :: hash_relation; S = ::
                cq_example_providers :: sorted_relation; r0 = ::
                cq_example_providers :: slice_rows; r1 = ::
                cq_example_providers :: hash_relation; r2 = ::
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
                cq_example_providers :: rename; r2 = :: cq_example_providers
                :: indexed_join; r3 = :: cq_example_providers :: rename; r4 =
                :: cq_example_providers :: indexed_join; r5 = ::
                cq_example_providers :: project;
            }
        }
    } environment
    {
        stored L at(__cq_store_L) operations :: cq_example_providers ::
        slice_rows stored R at(__cq_store_R) operations ::
        cq_example_providers :: hash_relation stored S at(__cq_store_S)
        operations :: cq_example_providers :: sorted_relation stored r0
        at(__cq_store_r0) operations :: cq_example_providers :: slice_rows
        stored r1 at(__cq_store_r1) operations :: cq_example_providers ::
        hash_relation indexed i0 at(__cq_index_i0) operations ::
        cq_example_providers :: hash_relation region r2
        {
            r2(a, b) => ((a, b,)) : - r0(a) => bind a <-
            ((__cq_store_r0).iter()), r1(a, b) => bind b <-
            ((__cq_index_i0).get(a).into_iter().flatten());
        }
    } emitted
    {
        let __cq_store_L : & [Value] = L; let __cq_store_R : & :: std ::
        collections :: HashMap < Value, :: std :: vec :: Vec < Value > > = R;
        let __cq_store_S : & :: std :: collections :: BTreeMap < Value, :: std
        :: vec :: Vec < Value > > = S; let __cq_store_r0 : & [Value] =
        __cq_store_L; let __cq_store_r1 : & :: std :: collections :: HashMap <
        Value, :: std :: vec :: Vec < Value > > = __cq_store_R; let
        __cq_index_i0 : & :: std :: collections :: HashMap < Value, :: std ::
        vec :: Vec < Value > > = __cq_store_r1;
    } pending { lower r3 }
    continuations(:: mini_linq :: resume_physical, mini_linq :: access)
} plan
{
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
} environment
{
    stored L at(__cq_store_L) operations :: cq_example_providers :: slice_rows
    stored R at(__cq_store_R) operations :: cq_example_providers ::
    hash_relation stored S at(__cq_store_S) operations :: cq_example_providers
    :: sorted_relation stored r0 at(__cq_store_r0) operations ::
    cq_example_providers :: slice_rows stored r1 at(__cq_store_r1) operations
    :: cq_example_providers :: hash_relation indexed i0 at(__cq_index_i0)
    operations :: cq_example_providers :: hash_relation region r2
    {
        r2(a, b) => ((a, b,)) : - r0(a) => bind a <- ((__cq_store_r0).iter()),
        r1(a, b) => bind b <- ((__cq_index_i0).get(a).into_iter().flatten());
    }
} request { input r3 from(__cq_store_S) }
```

Actual output (formatted where Rust syntax permits):

```rust
::mini_linq::resume_physical! {
    resume
    {
        plan
        {
            struct ProjectedChain; relation L(a : Value); relation
            R(a : Value, b : Value); relation S(b : Value, c : Value);
            answer(a, c) : - L(a), R(a, b), S(b, c); relational
            {
                r0 = rename L { a - > a }; r1 = rename R { a - > a, b - > b };
                r2 = natural_join r0 with r1; r3 = rename S
                { b - > b, c - > c }; r4 = natural_join r2 with r3; r5 =
                project r4 keep { a, c }; output r5 as answer(a, c);
            } indexes { i0 = @ r1 [0]; i1 = @ r3 [0]; } uses
            { r2.right = i0; r4.right = i1; } physical
            {
                relations
                {
                    L = :: cq_example_providers :: slice_rows; R = ::
                    cq_example_providers :: hash_relation; S = ::
                    cq_example_providers :: sorted_relation; r0 = ::
                    cq_example_providers :: slice_rows; r1 = ::
                    cq_example_providers :: hash_relation; r2 = ::
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
                    cq_example_providers :: rename; r2 = :: cq_example_providers
                    :: indexed_join; r3 = :: cq_example_providers :: rename; r4
                    = :: cq_example_providers :: indexed_join; r5 = ::
                    cq_example_providers :: project;
                }
            }
        } environment
        {
            stored L at(__cq_store_L) operations :: cq_example_providers ::
            slice_rows stored R at(__cq_store_R) operations ::
            cq_example_providers :: hash_relation stored S at(__cq_store_S)
            operations :: cq_example_providers :: sorted_relation stored r0
            at(__cq_store_r0) operations :: cq_example_providers :: slice_rows
            stored r1 at(__cq_store_r1) operations :: cq_example_providers ::
            hash_relation indexed i0 at(__cq_index_i0) operations ::
            cq_example_providers :: hash_relation region r2
            {
                r2(a, b) => ((a, b,)) : - r0(a) => bind a <-
                ((__cq_store_r0).iter()), r1(a, b) => bind b <-
                ((__cq_index_i0).get(a).into_iter().flatten());
            }
        } emitted
        {
            let __cq_store_L : & [Value] = L; let __cq_store_R : & :: std ::
            collections :: HashMap < Value, :: std :: vec :: Vec < Value > > =
            R; let __cq_store_S : & :: std :: collections :: BTreeMap < Value,
            :: std :: vec :: Vec < Value > > = S; let __cq_store_r0 : &
            [Value] = __cq_store_L; let __cq_store_r1 : & :: std ::
            collections :: HashMap < Value, :: std :: vec :: Vec < Value > > =
            __cq_store_R; let __cq_index_i0 : & :: std :: collections ::
            HashMap < Value, :: std :: vec :: Vec < Value > > = __cq_store_r1;
        } pending { lower r3 }
        continuations(:: mini_linq :: resume_physical, mini_linq :: access)
    } emitted
    {
        let __cq_store_r3 : & :: std :: collections :: BTreeMap < Value, ::
        std :: vec :: Vec < Value >> = __cq_store_S;
    } result
    {
        stored r3 at(__cq_store_r3) operations :: cq_example_providers ::
        sorted_relation
    }
}

```

---

## 2172002/029-resume_physical

[Raw input](raw/2172002/029-resume_physical.input.tokens) · [Raw output](raw/2172002/029-resume_physical.output.tokens)

Actual input:

```rust
resume
{
    plan
    {
        struct ProjectedChain; relation L(a : Value); relation
        R(a : Value, b : Value); relation S(b : Value, c : Value);
        answer(a, c) : - L(a), R(a, b), S(b, c); relational
        {
            r0 = rename L { a - > a }; r1 = rename R { a - > a, b - > b }; r2
            = natural_join r0 with r1; r3 = rename S { b - > b, c - > c }; r4
            = natural_join r2 with r3; r5 = project r4 keep { a, c }; output
            r5 as answer(a, c);
        } indexes { i0 = @ r1 [0]; i1 = @ r3 [0]; } uses
        { r2.right = i0; r4.right = i1; } physical
        {
            relations
            {
                L = :: cq_example_providers :: slice_rows; R = ::
                cq_example_providers :: hash_relation; S = ::
                cq_example_providers :: sorted_relation; r0 = ::
                cq_example_providers :: slice_rows; r1 = ::
                cq_example_providers :: hash_relation; r2 = ::
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
                cq_example_providers :: rename; r2 = :: cq_example_providers
                :: indexed_join; r3 = :: cq_example_providers :: rename; r4 =
                :: cq_example_providers :: indexed_join; r5 = ::
                cq_example_providers :: project;
            }
        }
    } environment
    {
        stored L at(__cq_store_L) operations :: cq_example_providers ::
        slice_rows stored R at(__cq_store_R) operations ::
        cq_example_providers :: hash_relation stored S at(__cq_store_S)
        operations :: cq_example_providers :: sorted_relation stored r0
        at(__cq_store_r0) operations :: cq_example_providers :: slice_rows
        stored r1 at(__cq_store_r1) operations :: cq_example_providers ::
        hash_relation indexed i0 at(__cq_index_i0) operations ::
        cq_example_providers :: hash_relation region r2
        {
            r2(a, b) => ((a, b,)) : - r0(a) => bind a <-
            ((__cq_store_r0).iter()), r1(a, b) => bind b <-
            ((__cq_index_i0).get(a).into_iter().flatten());
        }
    } emitted
    {
        let __cq_store_L : & [Value] = L; let __cq_store_R : & :: std ::
        collections :: HashMap < Value, :: std :: vec :: Vec < Value > > = R;
        let __cq_store_S : & :: std :: collections :: BTreeMap < Value, :: std
        :: vec :: Vec < Value > > = S; let __cq_store_r0 : & [Value] =
        __cq_store_L; let __cq_store_r1 : & :: std :: collections :: HashMap <
        Value, :: std :: vec :: Vec < Value > > = __cq_store_R; let
        __cq_index_i0 : & :: std :: collections :: HashMap < Value, :: std ::
        vec :: Vec < Value > > = __cq_store_r1;
    } pending { lower r3 }
    continuations(:: mini_linq :: resume_physical, mini_linq :: access)
} emitted
{
    let __cq_store_r3 : & :: std :: collections :: BTreeMap < Value, :: std ::
    vec :: Vec < Value >> = __cq_store_S;
} result
{
    stored r3 at(__cq_store_r3) operations :: cq_example_providers ::
    sorted_relation
}
```

Actual output (formatted where Rust syntax permits):

```rust
::cq_example_providers::sorted_relation! {
    reply_to(:: mini_linq :: resume_physical)
    {
        plan
        {
            struct ProjectedChain; relation L(a : Value); relation
            R(a : Value, b : Value); relation S(b : Value, c : Value);
            answer(a, c) : - L(a), R(a, b), S(b, c); relational
            {
                r0 = rename L { a - > a }; r1 = rename R { a - > a, b - > b };
                r2 = natural_join r0 with r1; r3 = rename S
                { b - > b, c - > c }; r4 = natural_join r2 with r3; r5 =
                project r4 keep { a, c }; output r5 as answer(a, c);
            } indexes { i0 = @ r1 [0]; i1 = @ r3 [0]; } uses
            { r2.right = i0; r4.right = i1; } physical
            {
                relations
                {
                    L = :: cq_example_providers :: slice_rows; R = ::
                    cq_example_providers :: hash_relation; S = ::
                    cq_example_providers :: sorted_relation; r0 = ::
                    cq_example_providers :: slice_rows; r1 = ::
                    cq_example_providers :: hash_relation; r2 = ::
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
                    cq_example_providers :: rename; r2 = :: cq_example_providers
                    :: indexed_join; r3 = :: cq_example_providers :: rename; r4
                    = :: cq_example_providers :: indexed_join; r5 = ::
                    cq_example_providers :: project;
                }
            }
        } environment
        {
            stored L at(__cq_store_L) operations :: cq_example_providers ::
            slice_rows stored R at(__cq_store_R) operations ::
            cq_example_providers :: hash_relation stored S at(__cq_store_S)
            operations :: cq_example_providers :: sorted_relation stored r0
            at(__cq_store_r0) operations :: cq_example_providers :: slice_rows
            stored r1 at(__cq_store_r1) operations :: cq_example_providers ::
            hash_relation indexed i0 at(__cq_index_i0) operations ::
            cq_example_providers :: hash_relation region r2
            {
                r2(a, b) => ((a, b,)) : - r0(a) => bind a <-
                ((__cq_store_r0).iter()), r1(a, b) => bind b <-
                ((__cq_index_i0).get(a).into_iter().flatten());
            } stored r3 at(__cq_store_r3) operations :: cq_example_providers
            :: sorted_relation
        } emitted
        {
            let __cq_store_L : & [Value] = L; let __cq_store_R : & :: std ::
            collections :: HashMap < Value, :: std :: vec :: Vec < Value > > =
            R; let __cq_store_S : & :: std :: collections :: BTreeMap < Value,
            :: std :: vec :: Vec < Value > > = S; let __cq_store_r0 : &
            [Value] = __cq_store_L; let __cq_store_r1 : & :: std ::
            collections :: HashMap < Value, :: std :: vec :: Vec < Value > > =
            __cq_store_R; let __cq_index_i0 : & :: std :: collections ::
            HashMap < Value, :: std :: vec :: Vec < Value > > = __cq_store_r1;
            let __cq_store_r3 : & :: std :: collections :: BTreeMap < Value,
            :: std :: vec :: Vec < Value > > = __cq_store_S;
        } pending { index i1 }
        continuations(:: mini_linq :: resume_physical, mini_linq :: access)
    } plan
    {
        struct ProjectedChain; relation L(a : Value); relation
        R(a : Value, b : Value); relation S(b : Value, c : Value);
        answer(a, c) : - L(a), R(a, b), S(b, c); relational
        {
            r0 = rename L { a - > a }; r1 = rename R { a - > a, b - > b }; r2
            = natural_join r0 with r1; r3 = rename S { b - > b, c - > c }; r4
            = natural_join r2 with r3; r5 = project r4 keep { a, c }; output
            r5 as answer(a, c);
        } indexes { i0 = @ r1 [0]; i1 = @ r3 [0]; } uses
        { r2.right = i0; r4.right = i1; } physical
        {
            relations
            {
                L = :: cq_example_providers :: slice_rows; R = ::
                cq_example_providers :: hash_relation; S = ::
                cq_example_providers :: sorted_relation; r0 = ::
                cq_example_providers :: slice_rows; r1 = ::
                cq_example_providers :: hash_relation; r2 = ::
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
                cq_example_providers :: rename; r2 = :: cq_example_providers
                :: indexed_join; r3 = :: cq_example_providers :: rename; r4 =
                :: cq_example_providers :: indexed_join; r5 = ::
                cq_example_providers :: project;
            }
        }
    } environment
    {
        stored L at(__cq_store_L) operations :: cq_example_providers ::
        slice_rows stored R at(__cq_store_R) operations ::
        cq_example_providers :: hash_relation stored S at(__cq_store_S)
        operations :: cq_example_providers :: sorted_relation stored r0
        at(__cq_store_r0) operations :: cq_example_providers :: slice_rows
        stored r1 at(__cq_store_r1) operations :: cq_example_providers ::
        hash_relation indexed i0 at(__cq_index_i0) operations ::
        cq_example_providers :: hash_relation region r2
        {
            r2(a, b) => ((a, b,)) : - r0(a) => bind a <-
            ((__cq_store_r0).iter()), r1(a, b) => bind b <-
            ((__cq_index_i0).get(a).into_iter().flatten());
        } stored r3 at(__cq_store_r3) operations :: cq_example_providers ::
        sorted_relation
    } request { index i1 }
}

```

---

## 2172002/030-cq-example-providers::sorted_relation

[Raw input](raw/2172002/030-cq-example-providers::sorted_relation.input.tokens) · [Raw output](raw/2172002/030-cq-example-providers::sorted_relation.output.tokens)

Actual input:

```rust
reply_to(:: mini_linq :: resume_physical)
{
    plan
    {
        struct ProjectedChain; relation L(a : Value); relation
        R(a : Value, b : Value); relation S(b : Value, c : Value);
        answer(a, c) : - L(a), R(a, b), S(b, c); relational
        {
            r0 = rename L { a - > a }; r1 = rename R { a - > a, b - > b }; r2
            = natural_join r0 with r1; r3 = rename S { b - > b, c - > c }; r4
            = natural_join r2 with r3; r5 = project r4 keep { a, c }; output
            r5 as answer(a, c);
        } indexes { i0 = @ r1 [0]; i1 = @ r3 [0]; } uses
        { r2.right = i0; r4.right = i1; } physical
        {
            relations
            {
                L = :: cq_example_providers :: slice_rows; R = ::
                cq_example_providers :: hash_relation; S = ::
                cq_example_providers :: sorted_relation; r0 = ::
                cq_example_providers :: slice_rows; r1 = ::
                cq_example_providers :: hash_relation; r2 = ::
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
                cq_example_providers :: rename; r2 = :: cq_example_providers
                :: indexed_join; r3 = :: cq_example_providers :: rename; r4 =
                :: cq_example_providers :: indexed_join; r5 = ::
                cq_example_providers :: project;
            }
        }
    } environment
    {
        stored L at(__cq_store_L) operations :: cq_example_providers ::
        slice_rows stored R at(__cq_store_R) operations ::
        cq_example_providers :: hash_relation stored S at(__cq_store_S)
        operations :: cq_example_providers :: sorted_relation stored r0
        at(__cq_store_r0) operations :: cq_example_providers :: slice_rows
        stored r1 at(__cq_store_r1) operations :: cq_example_providers ::
        hash_relation indexed i0 at(__cq_index_i0) operations ::
        cq_example_providers :: hash_relation region r2
        {
            r2(a, b) => ((a, b,)) : - r0(a) => bind a <-
            ((__cq_store_r0).iter()), r1(a, b) => bind b <-
            ((__cq_index_i0).get(a).into_iter().flatten());
        } stored r3 at(__cq_store_r3) operations :: cq_example_providers ::
        sorted_relation
    } emitted
    {
        let __cq_store_L : & [Value] = L; let __cq_store_R : & :: std ::
        collections :: HashMap < Value, :: std :: vec :: Vec < Value > > = R;
        let __cq_store_S : & :: std :: collections :: BTreeMap < Value, :: std
        :: vec :: Vec < Value > > = S; let __cq_store_r0 : & [Value] =
        __cq_store_L; let __cq_store_r1 : & :: std :: collections :: HashMap <
        Value, :: std :: vec :: Vec < Value > > = __cq_store_R; let
        __cq_index_i0 : & :: std :: collections :: HashMap < Value, :: std ::
        vec :: Vec < Value > > = __cq_store_r1; let __cq_store_r3 : & :: std
        :: collections :: BTreeMap < Value, :: std :: vec :: Vec < Value > > =
        __cq_store_S;
    } pending { index i1 }
    continuations(:: mini_linq :: resume_physical, mini_linq :: access)
} plan
{
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
} environment
{
    stored L at(__cq_store_L) operations :: cq_example_providers :: slice_rows
    stored R at(__cq_store_R) operations :: cq_example_providers ::
    hash_relation stored S at(__cq_store_S) operations :: cq_example_providers
    :: sorted_relation stored r0 at(__cq_store_r0) operations ::
    cq_example_providers :: slice_rows stored r1 at(__cq_store_r1) operations
    :: cq_example_providers :: hash_relation indexed i0 at(__cq_index_i0)
    operations :: cq_example_providers :: hash_relation region r2
    {
        r2(a, b) => ((a, b,)) : - r0(a) => bind a <- ((__cq_store_r0).iter()),
        r1(a, b) => bind b <- ((__cq_index_i0).get(a).into_iter().flatten());
    } stored r3 at(__cq_store_r3) operations :: cq_example_providers ::
    sorted_relation
} request { index i1 }
```

Actual output (formatted where Rust syntax permits):

```rust
::mini_linq::resume_physical! {
    resume
    {
        plan
        {
            struct ProjectedChain; relation L(a : Value); relation
            R(a : Value, b : Value); relation S(b : Value, c : Value);
            answer(a, c) : - L(a), R(a, b), S(b, c); relational
            {
                r0 = rename L { a - > a }; r1 = rename R { a - > a, b - > b };
                r2 = natural_join r0 with r1; r3 = rename S
                { b - > b, c - > c }; r4 = natural_join r2 with r3; r5 =
                project r4 keep { a, c }; output r5 as answer(a, c);
            } indexes { i0 = @ r1 [0]; i1 = @ r3 [0]; } uses
            { r2.right = i0; r4.right = i1; } physical
            {
                relations
                {
                    L = :: cq_example_providers :: slice_rows; R = ::
                    cq_example_providers :: hash_relation; S = ::
                    cq_example_providers :: sorted_relation; r0 = ::
                    cq_example_providers :: slice_rows; r1 = ::
                    cq_example_providers :: hash_relation; r2 = ::
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
                    cq_example_providers :: rename; r2 = :: cq_example_providers
                    :: indexed_join; r3 = :: cq_example_providers :: rename; r4
                    = :: cq_example_providers :: indexed_join; r5 = ::
                    cq_example_providers :: project;
                }
            }
        } environment
        {
            stored L at(__cq_store_L) operations :: cq_example_providers ::
            slice_rows stored R at(__cq_store_R) operations ::
            cq_example_providers :: hash_relation stored S at(__cq_store_S)
            operations :: cq_example_providers :: sorted_relation stored r0
            at(__cq_store_r0) operations :: cq_example_providers :: slice_rows
            stored r1 at(__cq_store_r1) operations :: cq_example_providers ::
            hash_relation indexed i0 at(__cq_index_i0) operations ::
            cq_example_providers :: hash_relation region r2
            {
                r2(a, b) => ((a, b,)) : - r0(a) => bind a <-
                ((__cq_store_r0).iter()), r1(a, b) => bind b <-
                ((__cq_index_i0).get(a).into_iter().flatten());
            } stored r3 at(__cq_store_r3) operations :: cq_example_providers
            :: sorted_relation
        } emitted
        {
            let __cq_store_L : & [Value] = L; let __cq_store_R : & :: std ::
            collections :: HashMap < Value, :: std :: vec :: Vec < Value > > =
            R; let __cq_store_S : & :: std :: collections :: BTreeMap < Value,
            :: std :: vec :: Vec < Value > > = S; let __cq_store_r0 : &
            [Value] = __cq_store_L; let __cq_store_r1 : & :: std ::
            collections :: HashMap < Value, :: std :: vec :: Vec < Value > > =
            __cq_store_R; let __cq_index_i0 : & :: std :: collections ::
            HashMap < Value, :: std :: vec :: Vec < Value > > = __cq_store_r1;
            let __cq_store_r3 : & :: std :: collections :: BTreeMap < Value,
            :: std :: vec :: Vec < Value > > = __cq_store_S;
        } pending { index i1 }
        continuations(:: mini_linq :: resume_physical, mini_linq :: access)
    } emitted
    {
        let __cq_index_i1 : & :: std :: collections :: BTreeMap < Value, ::
        std :: vec :: Vec < Value >> = __cq_store_r3;
    } result
    {
        indexed i1 at(__cq_index_i1) operations :: cq_example_providers ::
        sorted_relation
    }
}

```

---

## 2172002/031-resume_physical

[Raw input](raw/2172002/031-resume_physical.input.tokens) · [Raw output](raw/2172002/031-resume_physical.output.tokens)

Actual input:

```rust
resume
{
    plan
    {
        struct ProjectedChain; relation L(a : Value); relation
        R(a : Value, b : Value); relation S(b : Value, c : Value);
        answer(a, c) : - L(a), R(a, b), S(b, c); relational
        {
            r0 = rename L { a - > a }; r1 = rename R { a - > a, b - > b }; r2
            = natural_join r0 with r1; r3 = rename S { b - > b, c - > c }; r4
            = natural_join r2 with r3; r5 = project r4 keep { a, c }; output
            r5 as answer(a, c);
        } indexes { i0 = @ r1 [0]; i1 = @ r3 [0]; } uses
        { r2.right = i0; r4.right = i1; } physical
        {
            relations
            {
                L = :: cq_example_providers :: slice_rows; R = ::
                cq_example_providers :: hash_relation; S = ::
                cq_example_providers :: sorted_relation; r0 = ::
                cq_example_providers :: slice_rows; r1 = ::
                cq_example_providers :: hash_relation; r2 = ::
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
                cq_example_providers :: rename; r2 = :: cq_example_providers
                :: indexed_join; r3 = :: cq_example_providers :: rename; r4 =
                :: cq_example_providers :: indexed_join; r5 = ::
                cq_example_providers :: project;
            }
        }
    } environment
    {
        stored L at(__cq_store_L) operations :: cq_example_providers ::
        slice_rows stored R at(__cq_store_R) operations ::
        cq_example_providers :: hash_relation stored S at(__cq_store_S)
        operations :: cq_example_providers :: sorted_relation stored r0
        at(__cq_store_r0) operations :: cq_example_providers :: slice_rows
        stored r1 at(__cq_store_r1) operations :: cq_example_providers ::
        hash_relation indexed i0 at(__cq_index_i0) operations ::
        cq_example_providers :: hash_relation region r2
        {
            r2(a, b) => ((a, b,)) : - r0(a) => bind a <-
            ((__cq_store_r0).iter()), r1(a, b) => bind b <-
            ((__cq_index_i0).get(a).into_iter().flatten());
        } stored r3 at(__cq_store_r3) operations :: cq_example_providers ::
        sorted_relation
    } emitted
    {
        let __cq_store_L : & [Value] = L; let __cq_store_R : & :: std ::
        collections :: HashMap < Value, :: std :: vec :: Vec < Value > > = R;
        let __cq_store_S : & :: std :: collections :: BTreeMap < Value, :: std
        :: vec :: Vec < Value > > = S; let __cq_store_r0 : & [Value] =
        __cq_store_L; let __cq_store_r1 : & :: std :: collections :: HashMap <
        Value, :: std :: vec :: Vec < Value > > = __cq_store_R; let
        __cq_index_i0 : & :: std :: collections :: HashMap < Value, :: std ::
        vec :: Vec < Value > > = __cq_store_r1; let __cq_store_r3 : & :: std
        :: collections :: BTreeMap < Value, :: std :: vec :: Vec < Value > > =
        __cq_store_S;
    } pending { index i1 }
    continuations(:: mini_linq :: resume_physical, mini_linq :: access)
} emitted
{
    let __cq_index_i1 : & :: std :: collections :: BTreeMap < Value, :: std ::
    vec :: Vec < Value >> = __cq_store_r3;
} result
{
    indexed i1 at(__cq_index_i1) operations :: cq_example_providers ::
    sorted_relation
}
```

Actual output (formatted where Rust syntax permits):

```rust
::cq_example_providers::indexed_join! {
    reply_to(:: mini_linq :: resume_physical)
    {
        plan
        {
            struct ProjectedChain; relation L(a : Value); relation
            R(a : Value, b : Value); relation S(b : Value, c : Value);
            answer(a, c) : - L(a), R(a, b), S(b, c); relational
            {
                r0 = rename L { a - > a }; r1 = rename R { a - > a, b - > b };
                r2 = natural_join r0 with r1; r3 = rename S
                { b - > b, c - > c }; r4 = natural_join r2 with r3; r5 =
                project r4 keep { a, c }; output r5 as answer(a, c);
            } indexes { i0 = @ r1 [0]; i1 = @ r3 [0]; } uses
            { r2.right = i0; r4.right = i1; } physical
            {
                relations
                {
                    L = :: cq_example_providers :: slice_rows; R = ::
                    cq_example_providers :: hash_relation; S = ::
                    cq_example_providers :: sorted_relation; r0 = ::
                    cq_example_providers :: slice_rows; r1 = ::
                    cq_example_providers :: hash_relation; r2 = ::
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
                    cq_example_providers :: rename; r2 = :: cq_example_providers
                    :: indexed_join; r3 = :: cq_example_providers :: rename; r4
                    = :: cq_example_providers :: indexed_join; r5 = ::
                    cq_example_providers :: project;
                }
            }
        } environment
        {
            stored L at(__cq_store_L) operations :: cq_example_providers ::
            slice_rows stored R at(__cq_store_R) operations ::
            cq_example_providers :: hash_relation stored S at(__cq_store_S)
            operations :: cq_example_providers :: sorted_relation stored r0
            at(__cq_store_r0) operations :: cq_example_providers :: slice_rows
            stored r1 at(__cq_store_r1) operations :: cq_example_providers ::
            hash_relation indexed i0 at(__cq_index_i0) operations ::
            cq_example_providers :: hash_relation region r2
            {
                r2(a, b) => ((a, b,)) : - r0(a) => bind a <-
                ((__cq_store_r0).iter()), r1(a, b) => bind b <-
                ((__cq_index_i0).get(a).into_iter().flatten());
            } stored r3 at(__cq_store_r3) operations :: cq_example_providers
            :: sorted_relation indexed i1 at(__cq_index_i1) operations ::
            cq_example_providers :: sorted_relation
        } emitted
        {
            let __cq_store_L : & [Value] = L; let __cq_store_R : & :: std ::
            collections :: HashMap < Value, :: std :: vec :: Vec < Value > > =
            R; let __cq_store_S : & :: std :: collections :: BTreeMap < Value,
            :: std :: vec :: Vec < Value > > = S; let __cq_store_r0 : &
            [Value] = __cq_store_L; let __cq_store_r1 : & :: std ::
            collections :: HashMap < Value, :: std :: vec :: Vec < Value > > =
            __cq_store_R; let __cq_index_i0 : & :: std :: collections ::
            HashMap < Value, :: std :: vec :: Vec < Value > > = __cq_store_r1;
            let __cq_store_r3 : & :: std :: collections :: BTreeMap < Value,
            :: std :: vec :: Vec < Value > > = __cq_store_S; let __cq_index_i1
            : & :: std :: collections :: BTreeMap < Value, :: std :: vec ::
            Vec < Value > > = __cq_store_r3;
        } pending { lower r4 }
        continuations(:: mini_linq :: resume_physical, mini_linq :: access)
    } plan
    {
        struct ProjectedChain; relation L(a : Value); relation
        R(a : Value, b : Value); relation S(b : Value, c : Value);
        answer(a, c) : - L(a), R(a, b), S(b, c); relational
        {
            r0 = rename L { a - > a }; r1 = rename R { a - > a, b - > b }; r2
            = natural_join r0 with r1; r3 = rename S { b - > b, c - > c }; r4
            = natural_join r2 with r3; r5 = project r4 keep { a, c }; output
            r5 as answer(a, c);
        } indexes { i0 = @ r1 [0]; i1 = @ r3 [0]; } uses
        { r2.right = i0; r4.right = i1; } physical
        {
            relations
            {
                L = :: cq_example_providers :: slice_rows; R = ::
                cq_example_providers :: hash_relation; S = ::
                cq_example_providers :: sorted_relation; r0 = ::
                cq_example_providers :: slice_rows; r1 = ::
                cq_example_providers :: hash_relation; r2 = ::
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
                cq_example_providers :: rename; r2 = :: cq_example_providers
                :: indexed_join; r3 = :: cq_example_providers :: rename; r4 =
                :: cq_example_providers :: indexed_join; r5 = ::
                cq_example_providers :: project;
            }
        }
    } environment
    {
        stored L at(__cq_store_L) operations :: cq_example_providers ::
        slice_rows stored R at(__cq_store_R) operations ::
        cq_example_providers :: hash_relation stored S at(__cq_store_S)
        operations :: cq_example_providers :: sorted_relation stored r0
        at(__cq_store_r0) operations :: cq_example_providers :: slice_rows
        stored r1 at(__cq_store_r1) operations :: cq_example_providers ::
        hash_relation indexed i0 at(__cq_index_i0) operations ::
        cq_example_providers :: hash_relation region r2
        {
            r2(a, b) => ((a, b,)) : - r0(a) => bind a <-
            ((__cq_store_r0).iter()), r1(a, b) => bind b <-
            ((__cq_index_i0).get(a).into_iter().flatten());
        } stored r3 at(__cq_store_r3) operations :: cq_example_providers ::
        sorted_relation indexed i1 at(__cq_index_i1) operations ::
        cq_example_providers :: sorted_relation
    } request { lower r4 }
}

```

---

## 2172002/032-cq-example-providers::indexed_join

[Raw input](raw/2172002/032-cq-example-providers::indexed_join.input.tokens) · [Raw output](raw/2172002/032-cq-example-providers::indexed_join.output.tokens)

Actual input:

```rust
reply_to(:: mini_linq :: resume_physical)
{
    plan
    {
        struct ProjectedChain; relation L(a : Value); relation
        R(a : Value, b : Value); relation S(b : Value, c : Value);
        answer(a, c) : - L(a), R(a, b), S(b, c); relational
        {
            r0 = rename L { a - > a }; r1 = rename R { a - > a, b - > b }; r2
            = natural_join r0 with r1; r3 = rename S { b - > b, c - > c }; r4
            = natural_join r2 with r3; r5 = project r4 keep { a, c }; output
            r5 as answer(a, c);
        } indexes { i0 = @ r1 [0]; i1 = @ r3 [0]; } uses
        { r2.right = i0; r4.right = i1; } physical
        {
            relations
            {
                L = :: cq_example_providers :: slice_rows; R = ::
                cq_example_providers :: hash_relation; S = ::
                cq_example_providers :: sorted_relation; r0 = ::
                cq_example_providers :: slice_rows; r1 = ::
                cq_example_providers :: hash_relation; r2 = ::
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
                cq_example_providers :: rename; r2 = :: cq_example_providers
                :: indexed_join; r3 = :: cq_example_providers :: rename; r4 =
                :: cq_example_providers :: indexed_join; r5 = ::
                cq_example_providers :: project;
            }
        }
    } environment
    {
        stored L at(__cq_store_L) operations :: cq_example_providers ::
        slice_rows stored R at(__cq_store_R) operations ::
        cq_example_providers :: hash_relation stored S at(__cq_store_S)
        operations :: cq_example_providers :: sorted_relation stored r0
        at(__cq_store_r0) operations :: cq_example_providers :: slice_rows
        stored r1 at(__cq_store_r1) operations :: cq_example_providers ::
        hash_relation indexed i0 at(__cq_index_i0) operations ::
        cq_example_providers :: hash_relation region r2
        {
            r2(a, b) => ((a, b,)) : - r0(a) => bind a <-
            ((__cq_store_r0).iter()), r1(a, b) => bind b <-
            ((__cq_index_i0).get(a).into_iter().flatten());
        } stored r3 at(__cq_store_r3) operations :: cq_example_providers ::
        sorted_relation indexed i1 at(__cq_index_i1) operations ::
        cq_example_providers :: sorted_relation
    } emitted
    {
        let __cq_store_L : & [Value] = L; let __cq_store_R : & :: std ::
        collections :: HashMap < Value, :: std :: vec :: Vec < Value > > = R;
        let __cq_store_S : & :: std :: collections :: BTreeMap < Value, :: std
        :: vec :: Vec < Value > > = S; let __cq_store_r0 : & [Value] =
        __cq_store_L; let __cq_store_r1 : & :: std :: collections :: HashMap <
        Value, :: std :: vec :: Vec < Value > > = __cq_store_R; let
        __cq_index_i0 : & :: std :: collections :: HashMap < Value, :: std ::
        vec :: Vec < Value > > = __cq_store_r1; let __cq_store_r3 : & :: std
        :: collections :: BTreeMap < Value, :: std :: vec :: Vec < Value > > =
        __cq_store_S; let __cq_index_i1 : & :: std :: collections :: BTreeMap
        < Value, :: std :: vec :: Vec < Value > > = __cq_store_r3;
    } pending { lower r4 }
    continuations(:: mini_linq :: resume_physical, mini_linq :: access)
} plan
{
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
} environment
{
    stored L at(__cq_store_L) operations :: cq_example_providers :: slice_rows
    stored R at(__cq_store_R) operations :: cq_example_providers ::
    hash_relation stored S at(__cq_store_S) operations :: cq_example_providers
    :: sorted_relation stored r0 at(__cq_store_r0) operations ::
    cq_example_providers :: slice_rows stored r1 at(__cq_store_r1) operations
    :: cq_example_providers :: hash_relation indexed i0 at(__cq_index_i0)
    operations :: cq_example_providers :: hash_relation region r2
    {
        r2(a, b) => ((a, b,)) : - r0(a) => bind a <- ((__cq_store_r0).iter()),
        r1(a, b) => bind b <- ((__cq_index_i0).get(a).into_iter().flatten());
    } stored r3 at(__cq_store_r3) operations :: cq_example_providers ::
    sorted_relation indexed i1 at(__cq_index_i1) operations ::
    cq_example_providers :: sorted_relation
} request { lower r4 }
```

Actual output (formatted where Rust syntax permits):

```rust
::cq_example_providers::sorted_relation! {
    reply_to(:: cq_example_providers :: indexed_join_matches)
    {
        request
        {
            reply_to(:: mini_linq :: resume_physical)
            {
                plan
                {
                    struct ProjectedChain; relation L(a : Value); relation
                    R(a : Value, b : Value); relation S(b : Value, c : Value);
                    answer(a, c) : - L(a), R(a, b), S(b, c); relational
                    {
                        r0 = rename L { a - > a }; r1 = rename R
                        { a - > a, b - > b }; r2 = natural_join r0 with r1; r3 =
                        rename S { b - > b, c - > c }; r4 = natural_join r2 with r3;
                        r5 = project r4 keep { a, c }; output r5 as answer(a, c);
                    } indexes { i0 = @ r1 [0]; i1 = @ r3 [0]; } uses
                    { r2.right = i0; r4.right = i1; } physical
                    {
                        relations
                        {
                            L = :: cq_example_providers :: slice_rows; R = ::
                            cq_example_providers :: hash_relation; S = ::
                            cq_example_providers :: sorted_relation; r0 = ::
                            cq_example_providers :: slice_rows; r1 = ::
                            cq_example_providers :: hash_relation; r2 = ::
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
                            cq_example_providers :: rename; r2 = :: cq_example_providers
                            :: indexed_join; r3 = :: cq_example_providers :: rename; r4
                            = :: cq_example_providers :: indexed_join; r5 = ::
                            cq_example_providers :: project;
                        }
                    }
                } environment
                {
                    stored L at(__cq_store_L) operations :: cq_example_providers
                    :: slice_rows stored R at(__cq_store_R) operations ::
                    cq_example_providers :: hash_relation stored S
                    at(__cq_store_S) operations :: cq_example_providers ::
                    sorted_relation stored r0 at(__cq_store_r0) operations ::
                    cq_example_providers :: slice_rows stored r1
                    at(__cq_store_r1) operations :: cq_example_providers ::
                    hash_relation indexed i0 at(__cq_index_i0) operations ::
                    cq_example_providers :: hash_relation region r2
                    {
                        r2(a, b) => ((a, b,)) : - r0(a) => bind a <-
                        ((__cq_store_r0).iter()), r1(a, b) => bind b <-
                        ((__cq_index_i0).get(a).into_iter().flatten());
                    } stored r3 at(__cq_store_r3) operations ::
                    cq_example_providers :: sorted_relation indexed i1
                    at(__cq_index_i1) operations :: cq_example_providers ::
                    sorted_relation
                } emitted
                {
                    let __cq_store_L : & [Value] = L; let __cq_store_R : & ::
                    std :: collections :: HashMap < Value, :: std :: vec :: Vec
                    < Value > > = R; let __cq_store_S : & :: std :: collections
                    :: BTreeMap < Value, :: std :: vec :: Vec < Value > > = S;
                    let __cq_store_r0 : & [Value] = __cq_store_L; let
                    __cq_store_r1 : & :: std :: collections :: HashMap < Value,
                    :: std :: vec :: Vec < Value > > = __cq_store_R; let
                    __cq_index_i0 : & :: std :: collections :: HashMap < Value,
                    :: std :: vec :: Vec < Value > > = __cq_store_r1; let
                    __cq_store_r3 : & :: std :: collections :: BTreeMap < Value,
                    :: std :: vec :: Vec < Value > > = __cq_store_S; let
                    __cq_index_i1 : & :: std :: collections :: BTreeMap < Value,
                    :: std :: vec :: Vec < Value > > = __cq_store_r3;
                } pending { lower r4 }
                continuations(:: mini_linq :: resume_physical, mini_linq ::
                access)
            } plan
            {
                struct ProjectedChain; relation L(a : Value); relation
                R(a : Value, b : Value); relation S(b : Value, c : Value);
                answer(a, c) : - L(a), R(a, b), S(b, c); relational
                {
                    r0 = rename L { a - > a }; r1 = rename R
                    { a - > a, b - > b }; r2 = natural_join r0 with r1; r3 =
                    rename S { b - > b, c - > c }; r4 = natural_join r2 with r3;
                    r5 = project r4 keep { a, c }; output r5 as answer(a, c);
                } indexes { i0 = @ r1 [0]; i1 = @ r3 [0]; } uses
                { r2.right = i0; r4.right = i1; } physical
                {
                    relations
                    {
                        L = :: cq_example_providers :: slice_rows; R = ::
                        cq_example_providers :: hash_relation; S = ::
                        cq_example_providers :: sorted_relation; r0 = ::
                        cq_example_providers :: slice_rows; r1 = ::
                        cq_example_providers :: hash_relation; r2 = ::
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
                        cq_example_providers :: rename; r2 = :: cq_example_providers
                        :: indexed_join; r3 = :: cq_example_providers :: rename; r4
                        = :: cq_example_providers :: indexed_join; r5 = ::
                        cq_example_providers :: project;
                    }
                }
            } environment
            {
                stored L at(__cq_store_L) operations :: cq_example_providers
                :: slice_rows stored R at(__cq_store_R) operations ::
                cq_example_providers :: hash_relation stored S
                at(__cq_store_S) operations :: cq_example_providers ::
                sorted_relation stored r0 at(__cq_store_r0) operations ::
                cq_example_providers :: slice_rows stored r1 at(__cq_store_r1)
                operations :: cq_example_providers :: hash_relation indexed i0
                at(__cq_index_i0) operations :: cq_example_providers ::
                hash_relation region r2
                {
                    r2(a, b) => ((a, b,)) : - r0(a) => bind a <-
                    ((__cq_store_r0).iter()), r1(a, b) => bind b <-
                    ((__cq_index_i0).get(a).into_iter().flatten());
                } stored r3 at(__cq_store_r3) operations ::
                cq_example_providers :: sorted_relation indexed i1
                at(__cq_index_i1) operations :: cq_example_providers ::
                sorted_relation
            } request { lower r4 }
        } access
        {
            r0(a) => bind a <- ((__cq_store_r0).iter()), r1(a, b) => bind b <-
            ((__cq_index_i0).get(a).into_iter().flatten())
        } witness { row((a, b,)) fields { 0 = (a); 1 = (b); } }
    } plan
    {
        struct ProjectedChain; relation L(a : Value); relation
        R(a : Value, b : Value); relation S(b : Value, c : Value);
        answer(a, c) : - L(a), R(a, b), S(b, c); relational
        {
            r0 = rename L { a - > a }; r1 = rename R { a - > a, b - > b }; r2
            = natural_join r0 with r1; r3 = rename S { b - > b, c - > c }; r4
            = natural_join r2 with r3; r5 = project r4 keep { a, c }; output
            r5 as answer(a, c);
        } indexes { i0 = @ r1 [0]; i1 = @ r3 [0]; } uses
        { r2.right = i0; r4.right = i1; } physical
        {
            relations
            {
                L = :: cq_example_providers :: slice_rows; R = ::
                cq_example_providers :: hash_relation; S = ::
                cq_example_providers :: sorted_relation; r0 = ::
                cq_example_providers :: slice_rows; r1 = ::
                cq_example_providers :: hash_relation; r2 = ::
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
                cq_example_providers :: rename; r2 = :: cq_example_providers
                :: indexed_join; r3 = :: cq_example_providers :: rename; r4 =
                :: cq_example_providers :: indexed_join; r5 = ::
                cq_example_providers :: project;
            }
        }
    } environment
    {
        stored L at(__cq_store_L) operations :: cq_example_providers ::
        slice_rows stored R at(__cq_store_R) operations ::
        cq_example_providers :: hash_relation stored S at(__cq_store_S)
        operations :: cq_example_providers :: sorted_relation stored r0
        at(__cq_store_r0) operations :: cq_example_providers :: slice_rows
        stored r1 at(__cq_store_r1) operations :: cq_example_providers ::
        hash_relation indexed i0 at(__cq_index_i0) operations ::
        cq_example_providers :: hash_relation region r2
        {
            r2(a, b) => ((a, b,)) : - r0(a) => bind a <-
            ((__cq_store_r0).iter()), r1(a, b) => bind b <-
            ((__cq_index_i0).get(a).into_iter().flatten());
        } stored r3 at(__cq_store_r3) operations :: cq_example_providers ::
        sorted_relation indexed i1 at(__cq_index_i1) operations ::
        cq_example_providers :: sorted_relation
    } request { matches i1 values [(b)] }
}

```

---

## 2172002/033-cq-example-providers::sorted_relation

[Raw input](raw/2172002/033-cq-example-providers::sorted_relation.input.tokens) · [Raw output](raw/2172002/033-cq-example-providers::sorted_relation.output.tokens)

Actual input:

```rust
reply_to(:: cq_example_providers :: indexed_join_matches)
{
    request
    {
        reply_to(:: mini_linq :: resume_physical)
        {
            plan
            {
                struct ProjectedChain; relation L(a : Value); relation
                R(a : Value, b : Value); relation S(b : Value, c : Value);
                answer(a, c) : - L(a), R(a, b), S(b, c); relational
                {
                    r0 = rename L { a - > a }; r1 = rename R
                    { a - > a, b - > b }; r2 = natural_join r0 with r1; r3 =
                    rename S { b - > b, c - > c }; r4 = natural_join r2 with r3;
                    r5 = project r4 keep { a, c }; output r5 as answer(a, c);
                } indexes { i0 = @ r1 [0]; i1 = @ r3 [0]; } uses
                { r2.right = i0; r4.right = i1; } physical
                {
                    relations
                    {
                        L = :: cq_example_providers :: slice_rows; R = ::
                        cq_example_providers :: hash_relation; S = ::
                        cq_example_providers :: sorted_relation; r0 = ::
                        cq_example_providers :: slice_rows; r1 = ::
                        cq_example_providers :: hash_relation; r2 = ::
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
                        cq_example_providers :: rename; r2 = :: cq_example_providers
                        :: indexed_join; r3 = :: cq_example_providers :: rename; r4
                        = :: cq_example_providers :: indexed_join; r5 = ::
                        cq_example_providers :: project;
                    }
                }
            } environment
            {
                stored L at(__cq_store_L) operations :: cq_example_providers
                :: slice_rows stored R at(__cq_store_R) operations ::
                cq_example_providers :: hash_relation stored S
                at(__cq_store_S) operations :: cq_example_providers ::
                sorted_relation stored r0 at(__cq_store_r0) operations ::
                cq_example_providers :: slice_rows stored r1 at(__cq_store_r1)
                operations :: cq_example_providers :: hash_relation indexed i0
                at(__cq_index_i0) operations :: cq_example_providers ::
                hash_relation region r2
                {
                    r2(a, b) => ((a, b,)) : - r0(a) => bind a <-
                    ((__cq_store_r0).iter()), r1(a, b) => bind b <-
                    ((__cq_index_i0).get(a).into_iter().flatten());
                } stored r3 at(__cq_store_r3) operations ::
                cq_example_providers :: sorted_relation indexed i1
                at(__cq_index_i1) operations :: cq_example_providers ::
                sorted_relation
            } emitted
            {
                let __cq_store_L : & [Value] = L; let __cq_store_R : & :: std
                :: collections :: HashMap < Value, :: std :: vec :: Vec <
                Value > > = R; let __cq_store_S : & :: std :: collections ::
                BTreeMap < Value, :: std :: vec :: Vec < Value > > = S; let
                __cq_store_r0 : & [Value] = __cq_store_L; let __cq_store_r1 :
                & :: std :: collections :: HashMap < Value, :: std :: vec ::
                Vec < Value > > = __cq_store_R; let __cq_index_i0 : & :: std
                :: collections :: HashMap < Value, :: std :: vec :: Vec <
                Value > > = __cq_store_r1; let __cq_store_r3 : & :: std ::
                collections :: BTreeMap < Value, :: std :: vec :: Vec < Value
                > > = __cq_store_S; let __cq_index_i1 : & :: std ::
                collections :: BTreeMap < Value, :: std :: vec :: Vec < Value
                > > = __cq_store_r3;
            } pending { lower r4 }
            continuations(:: mini_linq :: resume_physical, mini_linq ::
            access)
        } plan
        {
            struct ProjectedChain; relation L(a : Value); relation
            R(a : Value, b : Value); relation S(b : Value, c : Value);
            answer(a, c) : - L(a), R(a, b), S(b, c); relational
            {
                r0 = rename L { a - > a }; r1 = rename R { a - > a, b - > b };
                r2 = natural_join r0 with r1; r3 = rename S
                { b - > b, c - > c }; r4 = natural_join r2 with r3; r5 =
                project r4 keep { a, c }; output r5 as answer(a, c);
            } indexes { i0 = @ r1 [0]; i1 = @ r3 [0]; } uses
            { r2.right = i0; r4.right = i1; } physical
            {
                relations
                {
                    L = :: cq_example_providers :: slice_rows; R = ::
                    cq_example_providers :: hash_relation; S = ::
                    cq_example_providers :: sorted_relation; r0 = ::
                    cq_example_providers :: slice_rows; r1 = ::
                    cq_example_providers :: hash_relation; r2 = ::
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
                    cq_example_providers :: rename; r2 = :: cq_example_providers
                    :: indexed_join; r3 = :: cq_example_providers :: rename; r4
                    = :: cq_example_providers :: indexed_join; r5 = ::
                    cq_example_providers :: project;
                }
            }
        } environment
        {
            stored L at(__cq_store_L) operations :: cq_example_providers ::
            slice_rows stored R at(__cq_store_R) operations ::
            cq_example_providers :: hash_relation stored S at(__cq_store_S)
            operations :: cq_example_providers :: sorted_relation stored r0
            at(__cq_store_r0) operations :: cq_example_providers :: slice_rows
            stored r1 at(__cq_store_r1) operations :: cq_example_providers ::
            hash_relation indexed i0 at(__cq_index_i0) operations ::
            cq_example_providers :: hash_relation region r2
            {
                r2(a, b) => ((a, b,)) : - r0(a) => bind a <-
                ((__cq_store_r0).iter()), r1(a, b) => bind b <-
                ((__cq_index_i0).get(a).into_iter().flatten());
            } stored r3 at(__cq_store_r3) operations :: cq_example_providers
            :: sorted_relation indexed i1 at(__cq_index_i1) operations ::
            cq_example_providers :: sorted_relation
        } request { lower r4 }
    } access
    {
        r0(a) => bind a <- ((__cq_store_r0).iter()), r1(a, b) => bind b <-
        ((__cq_index_i0).get(a).into_iter().flatten())
    } witness { row((a, b,)) fields { 0 = (a); 1 = (b); } }
} plan
{
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
} environment
{
    stored L at(__cq_store_L) operations :: cq_example_providers :: slice_rows
    stored R at(__cq_store_R) operations :: cq_example_providers ::
    hash_relation stored S at(__cq_store_S) operations :: cq_example_providers
    :: sorted_relation stored r0 at(__cq_store_r0) operations ::
    cq_example_providers :: slice_rows stored r1 at(__cq_store_r1) operations
    :: cq_example_providers :: hash_relation indexed i0 at(__cq_index_i0)
    operations :: cq_example_providers :: hash_relation region r2
    {
        r2(a, b) => ((a, b,)) : - r0(a) => bind a <- ((__cq_store_r0).iter()),
        r1(a, b) => bind b <- ((__cq_index_i0).get(a).into_iter().flatten());
    } stored r3 at(__cq_store_r3) operations :: cq_example_providers ::
    sorted_relation indexed i1 at(__cq_index_i1) operations ::
    cq_example_providers :: sorted_relation
} request { matches i1 values [(b)] }
```

Actual output (formatted where Rust syntax permits):

```rust
::cq_example_providers::indexed_join_matches! {
    resume
    {
        request
        {
            reply_to(:: mini_linq :: resume_physical)
            {
                plan
                {
                    struct ProjectedChain; relation L(a : Value); relation
                    R(a : Value, b : Value); relation S(b : Value, c : Value);
                    answer(a, c) : - L(a), R(a, b), S(b, c); relational
                    {
                        r0 = rename L { a - > a }; r1 = rename R
                        { a - > a, b - > b }; r2 = natural_join r0 with r1; r3 =
                        rename S { b - > b, c - > c }; r4 = natural_join r2 with r3;
                        r5 = project r4 keep { a, c }; output r5 as answer(a, c);
                    } indexes { i0 = @ r1 [0]; i1 = @ r3 [0]; } uses
                    { r2.right = i0; r4.right = i1; } physical
                    {
                        relations
                        {
                            L = :: cq_example_providers :: slice_rows; R = ::
                            cq_example_providers :: hash_relation; S = ::
                            cq_example_providers :: sorted_relation; r0 = ::
                            cq_example_providers :: slice_rows; r1 = ::
                            cq_example_providers :: hash_relation; r2 = ::
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
                            cq_example_providers :: rename; r2 = :: cq_example_providers
                            :: indexed_join; r3 = :: cq_example_providers :: rename; r4
                            = :: cq_example_providers :: indexed_join; r5 = ::
                            cq_example_providers :: project;
                        }
                    }
                } environment
                {
                    stored L at(__cq_store_L) operations :: cq_example_providers
                    :: slice_rows stored R at(__cq_store_R) operations ::
                    cq_example_providers :: hash_relation stored S
                    at(__cq_store_S) operations :: cq_example_providers ::
                    sorted_relation stored r0 at(__cq_store_r0) operations ::
                    cq_example_providers :: slice_rows stored r1
                    at(__cq_store_r1) operations :: cq_example_providers ::
                    hash_relation indexed i0 at(__cq_index_i0) operations ::
                    cq_example_providers :: hash_relation region r2
                    {
                        r2(a, b) => ((a, b,)) : - r0(a) => bind a <-
                        ((__cq_store_r0).iter()), r1(a, b) => bind b <-
                        ((__cq_index_i0).get(a).into_iter().flatten());
                    } stored r3 at(__cq_store_r3) operations ::
                    cq_example_providers :: sorted_relation indexed i1
                    at(__cq_index_i1) operations :: cq_example_providers ::
                    sorted_relation
                } emitted
                {
                    let __cq_store_L : & [Value] = L; let __cq_store_R : & ::
                    std :: collections :: HashMap < Value, :: std :: vec :: Vec
                    < Value > > = R; let __cq_store_S : & :: std :: collections
                    :: BTreeMap < Value, :: std :: vec :: Vec < Value > > = S;
                    let __cq_store_r0 : & [Value] = __cq_store_L; let
                    __cq_store_r1 : & :: std :: collections :: HashMap < Value,
                    :: std :: vec :: Vec < Value > > = __cq_store_R; let
                    __cq_index_i0 : & :: std :: collections :: HashMap < Value,
                    :: std :: vec :: Vec < Value > > = __cq_store_r1; let
                    __cq_store_r3 : & :: std :: collections :: BTreeMap < Value,
                    :: std :: vec :: Vec < Value > > = __cq_store_S; let
                    __cq_index_i1 : & :: std :: collections :: BTreeMap < Value,
                    :: std :: vec :: Vec < Value > > = __cq_store_r3;
                } pending { lower r4 }
                continuations(:: mini_linq :: resume_physical, mini_linq ::
                access)
            } plan
            {
                struct ProjectedChain; relation L(a : Value); relation
                R(a : Value, b : Value); relation S(b : Value, c : Value);
                answer(a, c) : - L(a), R(a, b), S(b, c); relational
                {
                    r0 = rename L { a - > a }; r1 = rename R
                    { a - > a, b - > b }; r2 = natural_join r0 with r1; r3 =
                    rename S { b - > b, c - > c }; r4 = natural_join r2 with r3;
                    r5 = project r4 keep { a, c }; output r5 as answer(a, c);
                } indexes { i0 = @ r1 [0]; i1 = @ r3 [0]; } uses
                { r2.right = i0; r4.right = i1; } physical
                {
                    relations
                    {
                        L = :: cq_example_providers :: slice_rows; R = ::
                        cq_example_providers :: hash_relation; S = ::
                        cq_example_providers :: sorted_relation; r0 = ::
                        cq_example_providers :: slice_rows; r1 = ::
                        cq_example_providers :: hash_relation; r2 = ::
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
                        cq_example_providers :: rename; r2 = :: cq_example_providers
                        :: indexed_join; r3 = :: cq_example_providers :: rename; r4
                        = :: cq_example_providers :: indexed_join; r5 = ::
                        cq_example_providers :: project;
                    }
                }
            } environment
            {
                stored L at(__cq_store_L) operations :: cq_example_providers
                :: slice_rows stored R at(__cq_store_R) operations ::
                cq_example_providers :: hash_relation stored S
                at(__cq_store_S) operations :: cq_example_providers ::
                sorted_relation stored r0 at(__cq_store_r0) operations ::
                cq_example_providers :: slice_rows stored r1 at(__cq_store_r1)
                operations :: cq_example_providers :: hash_relation indexed i0
                at(__cq_index_i0) operations :: cq_example_providers ::
                hash_relation region r2
                {
                    r2(a, b) => ((a, b,)) : - r0(a) => bind a <-
                    ((__cq_store_r0).iter()), r1(a, b) => bind b <-
                    ((__cq_index_i0).get(a).into_iter().flatten());
                } stored r3 at(__cq_store_r3) operations ::
                cq_example_providers :: sorted_relation indexed i1
                at(__cq_index_i1) operations :: cq_example_providers ::
                sorted_relation
            } request { lower r4 }
        } access
        {
            r0(a) => bind a <- ((__cq_store_r0).iter()), r1(a, b) => bind b <-
            ((__cq_index_i0).get(a).into_iter().flatten())
        } witness { row((a, b,)) fields { 0 = (a); 1 = (b); } }
    } source((__cq_index_i1).get(b).into_iter().flatten()) item(c) witness
    { row((b, c,)) fields { 0 = (b); 1 = (c); } }
}

```

---

## 2172002/034-cq-example-providers::indexed_join_matches

[Raw input](raw/2172002/034-cq-example-providers::indexed_join_matches.input.tokens) · [Raw output](raw/2172002/034-cq-example-providers::indexed_join_matches.output.tokens)

Actual input:

```rust
resume
{
    request
    {
        reply_to(:: mini_linq :: resume_physical)
        {
            plan
            {
                struct ProjectedChain; relation L(a : Value); relation
                R(a : Value, b : Value); relation S(b : Value, c : Value);
                answer(a, c) : - L(a), R(a, b), S(b, c); relational
                {
                    r0 = rename L { a - > a }; r1 = rename R
                    { a - > a, b - > b }; r2 = natural_join r0 with r1; r3 =
                    rename S { b - > b, c - > c }; r4 = natural_join r2 with r3;
                    r5 = project r4 keep { a, c }; output r5 as answer(a, c);
                } indexes { i0 = @ r1 [0]; i1 = @ r3 [0]; } uses
                { r2.right = i0; r4.right = i1; } physical
                {
                    relations
                    {
                        L = :: cq_example_providers :: slice_rows; R = ::
                        cq_example_providers :: hash_relation; S = ::
                        cq_example_providers :: sorted_relation; r0 = ::
                        cq_example_providers :: slice_rows; r1 = ::
                        cq_example_providers :: hash_relation; r2 = ::
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
                        cq_example_providers :: rename; r2 = :: cq_example_providers
                        :: indexed_join; r3 = :: cq_example_providers :: rename; r4
                        = :: cq_example_providers :: indexed_join; r5 = ::
                        cq_example_providers :: project;
                    }
                }
            } environment
            {
                stored L at(__cq_store_L) operations :: cq_example_providers
                :: slice_rows stored R at(__cq_store_R) operations ::
                cq_example_providers :: hash_relation stored S
                at(__cq_store_S) operations :: cq_example_providers ::
                sorted_relation stored r0 at(__cq_store_r0) operations ::
                cq_example_providers :: slice_rows stored r1 at(__cq_store_r1)
                operations :: cq_example_providers :: hash_relation indexed i0
                at(__cq_index_i0) operations :: cq_example_providers ::
                hash_relation region r2
                {
                    r2(a, b) => ((a, b,)) : - r0(a) => bind a <-
                    ((__cq_store_r0).iter()), r1(a, b) => bind b <-
                    ((__cq_index_i0).get(a).into_iter().flatten());
                } stored r3 at(__cq_store_r3) operations ::
                cq_example_providers :: sorted_relation indexed i1
                at(__cq_index_i1) operations :: cq_example_providers ::
                sorted_relation
            } emitted
            {
                let __cq_store_L : & [Value] = L; let __cq_store_R : & :: std
                :: collections :: HashMap < Value, :: std :: vec :: Vec <
                Value > > = R; let __cq_store_S : & :: std :: collections ::
                BTreeMap < Value, :: std :: vec :: Vec < Value > > = S; let
                __cq_store_r0 : & [Value] = __cq_store_L; let __cq_store_r1 :
                & :: std :: collections :: HashMap < Value, :: std :: vec ::
                Vec < Value > > = __cq_store_R; let __cq_index_i0 : & :: std
                :: collections :: HashMap < Value, :: std :: vec :: Vec <
                Value > > = __cq_store_r1; let __cq_store_r3 : & :: std ::
                collections :: BTreeMap < Value, :: std :: vec :: Vec < Value
                > > = __cq_store_S; let __cq_index_i1 : & :: std ::
                collections :: BTreeMap < Value, :: std :: vec :: Vec < Value
                > > = __cq_store_r3;
            } pending { lower r4 }
            continuations(:: mini_linq :: resume_physical, mini_linq ::
            access)
        } plan
        {
            struct ProjectedChain; relation L(a : Value); relation
            R(a : Value, b : Value); relation S(b : Value, c : Value);
            answer(a, c) : - L(a), R(a, b), S(b, c); relational
            {
                r0 = rename L { a - > a }; r1 = rename R { a - > a, b - > b };
                r2 = natural_join r0 with r1; r3 = rename S
                { b - > b, c - > c }; r4 = natural_join r2 with r3; r5 =
                project r4 keep { a, c }; output r5 as answer(a, c);
            } indexes { i0 = @ r1 [0]; i1 = @ r3 [0]; } uses
            { r2.right = i0; r4.right = i1; } physical
            {
                relations
                {
                    L = :: cq_example_providers :: slice_rows; R = ::
                    cq_example_providers :: hash_relation; S = ::
                    cq_example_providers :: sorted_relation; r0 = ::
                    cq_example_providers :: slice_rows; r1 = ::
                    cq_example_providers :: hash_relation; r2 = ::
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
                    cq_example_providers :: rename; r2 = :: cq_example_providers
                    :: indexed_join; r3 = :: cq_example_providers :: rename; r4
                    = :: cq_example_providers :: indexed_join; r5 = ::
                    cq_example_providers :: project;
                }
            }
        } environment
        {
            stored L at(__cq_store_L) operations :: cq_example_providers ::
            slice_rows stored R at(__cq_store_R) operations ::
            cq_example_providers :: hash_relation stored S at(__cq_store_S)
            operations :: cq_example_providers :: sorted_relation stored r0
            at(__cq_store_r0) operations :: cq_example_providers :: slice_rows
            stored r1 at(__cq_store_r1) operations :: cq_example_providers ::
            hash_relation indexed i0 at(__cq_index_i0) operations ::
            cq_example_providers :: hash_relation region r2
            {
                r2(a, b) => ((a, b,)) : - r0(a) => bind a <-
                ((__cq_store_r0).iter()), r1(a, b) => bind b <-
                ((__cq_index_i0).get(a).into_iter().flatten());
            } stored r3 at(__cq_store_r3) operations :: cq_example_providers
            :: sorted_relation indexed i1 at(__cq_index_i1) operations ::
            cq_example_providers :: sorted_relation
        } request { lower r4 }
    } access
    {
        r0(a) => bind a <- ((__cq_store_r0).iter()), r1(a, b) => bind b <-
        ((__cq_index_i0).get(a).into_iter().flatten())
    } witness { row((a, b,)) fields { 0 = (a); 1 = (b); } }
} source((__cq_index_i1).get(b).into_iter().flatten()) item(c) witness
{ row((b, c,)) fields { 0 = (b); 1 = (c); } }
```

Actual output (formatted where Rust syntax permits):

```rust
::cq_example_providers::borrowed_fields! {
    reply_to(:: cq_example_providers :: indexed_join_output)
    {
        request
        {
            reply_to(:: mini_linq :: resume_physical)
            {
                plan
                {
                    struct ProjectedChain; relation L(a : Value); relation
                    R(a : Value, b : Value); relation S(b : Value, c : Value);
                    answer(a, c) : - L(a), R(a, b), S(b, c); relational
                    {
                        r0 = rename L { a - > a }; r1 = rename R
                        { a - > a, b - > b }; r2 = natural_join r0 with r1; r3 =
                        rename S { b - > b, c - > c }; r4 = natural_join r2 with r3;
                        r5 = project r4 keep { a, c }; output r5 as answer(a, c);
                    } indexes { i0 = @ r1 [0]; i1 = @ r3 [0]; } uses
                    { r2.right = i0; r4.right = i1; } physical
                    {
                        relations
                        {
                            L = :: cq_example_providers :: slice_rows; R = ::
                            cq_example_providers :: hash_relation; S = ::
                            cq_example_providers :: sorted_relation; r0 = ::
                            cq_example_providers :: slice_rows; r1 = ::
                            cq_example_providers :: hash_relation; r2 = ::
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
                            cq_example_providers :: rename; r2 = :: cq_example_providers
                            :: indexed_join; r3 = :: cq_example_providers :: rename; r4
                            = :: cq_example_providers :: indexed_join; r5 = ::
                            cq_example_providers :: project;
                        }
                    }
                } environment
                {
                    stored L at(__cq_store_L) operations :: cq_example_providers
                    :: slice_rows stored R at(__cq_store_R) operations ::
                    cq_example_providers :: hash_relation stored S
                    at(__cq_store_S) operations :: cq_example_providers ::
                    sorted_relation stored r0 at(__cq_store_r0) operations ::
                    cq_example_providers :: slice_rows stored r1
                    at(__cq_store_r1) operations :: cq_example_providers ::
                    hash_relation indexed i0 at(__cq_index_i0) operations ::
                    cq_example_providers :: hash_relation region r2
                    {
                        r2(a, b) => ((a, b,)) : - r0(a) => bind a <-
                        ((__cq_store_r0).iter()), r1(a, b) => bind b <-
                        ((__cq_index_i0).get(a).into_iter().flatten());
                    } stored r3 at(__cq_store_r3) operations ::
                    cq_example_providers :: sorted_relation indexed i1
                    at(__cq_index_i1) operations :: cq_example_providers ::
                    sorted_relation
                } emitted
                {
                    let __cq_store_L : & [Value] = L; let __cq_store_R : & ::
                    std :: collections :: HashMap < Value, :: std :: vec :: Vec
                    < Value > > = R; let __cq_store_S : & :: std :: collections
                    :: BTreeMap < Value, :: std :: vec :: Vec < Value > > = S;
                    let __cq_store_r0 : & [Value] = __cq_store_L; let
                    __cq_store_r1 : & :: std :: collections :: HashMap < Value,
                    :: std :: vec :: Vec < Value > > = __cq_store_R; let
                    __cq_index_i0 : & :: std :: collections :: HashMap < Value,
                    :: std :: vec :: Vec < Value > > = __cq_store_r1; let
                    __cq_store_r3 : & :: std :: collections :: BTreeMap < Value,
                    :: std :: vec :: Vec < Value > > = __cq_store_S; let
                    __cq_index_i1 : & :: std :: collections :: BTreeMap < Value,
                    :: std :: vec :: Vec < Value > > = __cq_store_r3;
                } pending { lower r4 }
                continuations(:: mini_linq :: resume_physical, mini_linq ::
                access)
            } plan
            {
                struct ProjectedChain; relation L(a : Value); relation
                R(a : Value, b : Value); relation S(b : Value, c : Value);
                answer(a, c) : - L(a), R(a, b), S(b, c); relational
                {
                    r0 = rename L { a - > a }; r1 = rename R
                    { a - > a, b - > b }; r2 = natural_join r0 with r1; r3 =
                    rename S { b - > b, c - > c }; r4 = natural_join r2 with r3;
                    r5 = project r4 keep { a, c }; output r5 as answer(a, c);
                } indexes { i0 = @ r1 [0]; i1 = @ r3 [0]; } uses
                { r2.right = i0; r4.right = i1; } physical
                {
                    relations
                    {
                        L = :: cq_example_providers :: slice_rows; R = ::
                        cq_example_providers :: hash_relation; S = ::
                        cq_example_providers :: sorted_relation; r0 = ::
                        cq_example_providers :: slice_rows; r1 = ::
                        cq_example_providers :: hash_relation; r2 = ::
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
                        cq_example_providers :: rename; r2 = :: cq_example_providers
                        :: indexed_join; r3 = :: cq_example_providers :: rename; r4
                        = :: cq_example_providers :: indexed_join; r5 = ::
                        cq_example_providers :: project;
                    }
                }
            } environment
            {
                stored L at(__cq_store_L) operations :: cq_example_providers
                :: slice_rows stored R at(__cq_store_R) operations ::
                cq_example_providers :: hash_relation stored S
                at(__cq_store_S) operations :: cq_example_providers ::
                sorted_relation stored r0 at(__cq_store_r0) operations ::
                cq_example_providers :: slice_rows stored r1 at(__cq_store_r1)
                operations :: cq_example_providers :: hash_relation indexed i0
                at(__cq_index_i0) operations :: cq_example_providers ::
                hash_relation region r2
                {
                    r2(a, b) => ((a, b,)) : - r0(a) => bind a <-
                    ((__cq_store_r0).iter()), r1(a, b) => bind b <-
                    ((__cq_index_i0).get(a).into_iter().flatten());
                } stored r3 at(__cq_store_r3) operations ::
                cq_example_providers :: sorted_relation indexed i1
                at(__cq_index_i1) operations :: cq_example_providers ::
                sorted_relation
            } request { lower r4 }
        } access
        {
            r0(a) => bind a <- ((__cq_store_r0).iter()), r1(a, b) => bind b <-
            ((__cq_index_i0).get(a).into_iter().flatten()), r3(b, c) => bind c
            <- ((__cq_index_i1).get(b).into_iter().flatten())
        } witness { row((a, b,)) fields { 0 = (a); 1 = (b); } }
    } plan
    {
        struct ProjectedChain; relation L(a : Value); relation
        R(a : Value, b : Value); relation S(b : Value, c : Value);
        answer(a, c) : - L(a), R(a, b), S(b, c); relational
        {
            r0 = rename L { a - > a }; r1 = rename R { a - > a, b - > b }; r2
            = natural_join r0 with r1; r3 = rename S { b - > b, c - > c }; r4
            = natural_join r2 with r3; r5 = project r4 keep { a, c }; output
            r5 as answer(a, c);
        } indexes { i0 = @ r1 [0]; i1 = @ r3 [0]; } uses
        { r2.right = i0; r4.right = i1; } physical
        {
            relations
            {
                L = :: cq_example_providers :: slice_rows; R = ::
                cq_example_providers :: hash_relation; S = ::
                cq_example_providers :: sorted_relation; r0 = ::
                cq_example_providers :: slice_rows; r1 = ::
                cq_example_providers :: hash_relation; r2 = ::
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
                cq_example_providers :: rename; r2 = :: cq_example_providers
                :: indexed_join; r3 = :: cq_example_providers :: rename; r4 =
                :: cq_example_providers :: indexed_join; r5 = ::
                cq_example_providers :: project;
            }
        }
    } environment
    {
        stored L at(__cq_store_L) operations :: cq_example_providers ::
        slice_rows stored R at(__cq_store_R) operations ::
        cq_example_providers :: hash_relation stored S at(__cq_store_S)
        operations :: cq_example_providers :: sorted_relation stored r0
        at(__cq_store_r0) operations :: cq_example_providers :: slice_rows
        stored r1 at(__cq_store_r1) operations :: cq_example_providers ::
        hash_relation indexed i0 at(__cq_index_i0) operations ::
        cq_example_providers :: hash_relation region r2
        {
            r2(a, b) => ((a, b,)) : - r0(a) => bind a <-
            ((__cq_store_r0).iter()), r1(a, b) => bind b <-
            ((__cq_index_i0).get(a).into_iter().flatten());
        } stored r3 at(__cq_store_r3) operations :: cq_example_providers ::
        sorted_relation indexed i1 at(__cq_index_i1) operations ::
        cq_example_providers :: sorted_relation
    } request
    {
        output r4 fields { 0 = (a); 1 = (b); 2 = (c); } whole
        { r2 = ((a, b,)); r3 = ((b, c,)); }
    }
}

```

---

## 2172002/035-cq-example-providers::borrowed_fields

[Raw input](raw/2172002/035-cq-example-providers::borrowed_fields.input.tokens) · [Raw output](raw/2172002/035-cq-example-providers::borrowed_fields.output.tokens)

Actual input:

```rust
reply_to(:: cq_example_providers :: indexed_join_output)
{
    request
    {
        reply_to(:: mini_linq :: resume_physical)
        {
            plan
            {
                struct ProjectedChain; relation L(a : Value); relation
                R(a : Value, b : Value); relation S(b : Value, c : Value);
                answer(a, c) : - L(a), R(a, b), S(b, c); relational
                {
                    r0 = rename L { a - > a }; r1 = rename R
                    { a - > a, b - > b }; r2 = natural_join r0 with r1; r3 =
                    rename S { b - > b, c - > c }; r4 = natural_join r2 with r3;
                    r5 = project r4 keep { a, c }; output r5 as answer(a, c);
                } indexes { i0 = @ r1 [0]; i1 = @ r3 [0]; } uses
                { r2.right = i0; r4.right = i1; } physical
                {
                    relations
                    {
                        L = :: cq_example_providers :: slice_rows; R = ::
                        cq_example_providers :: hash_relation; S = ::
                        cq_example_providers :: sorted_relation; r0 = ::
                        cq_example_providers :: slice_rows; r1 = ::
                        cq_example_providers :: hash_relation; r2 = ::
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
                        cq_example_providers :: rename; r2 = :: cq_example_providers
                        :: indexed_join; r3 = :: cq_example_providers :: rename; r4
                        = :: cq_example_providers :: indexed_join; r5 = ::
                        cq_example_providers :: project;
                    }
                }
            } environment
            {
                stored L at(__cq_store_L) operations :: cq_example_providers
                :: slice_rows stored R at(__cq_store_R) operations ::
                cq_example_providers :: hash_relation stored S
                at(__cq_store_S) operations :: cq_example_providers ::
                sorted_relation stored r0 at(__cq_store_r0) operations ::
                cq_example_providers :: slice_rows stored r1 at(__cq_store_r1)
                operations :: cq_example_providers :: hash_relation indexed i0
                at(__cq_index_i0) operations :: cq_example_providers ::
                hash_relation region r2
                {
                    r2(a, b) => ((a, b,)) : - r0(a) => bind a <-
                    ((__cq_store_r0).iter()), r1(a, b) => bind b <-
                    ((__cq_index_i0).get(a).into_iter().flatten());
                } stored r3 at(__cq_store_r3) operations ::
                cq_example_providers :: sorted_relation indexed i1
                at(__cq_index_i1) operations :: cq_example_providers ::
                sorted_relation
            } emitted
            {
                let __cq_store_L : & [Value] = L; let __cq_store_R : & :: std
                :: collections :: HashMap < Value, :: std :: vec :: Vec <
                Value > > = R; let __cq_store_S : & :: std :: collections ::
                BTreeMap < Value, :: std :: vec :: Vec < Value > > = S; let
                __cq_store_r0 : & [Value] = __cq_store_L; let __cq_store_r1 :
                & :: std :: collections :: HashMap < Value, :: std :: vec ::
                Vec < Value > > = __cq_store_R; let __cq_index_i0 : & :: std
                :: collections :: HashMap < Value, :: std :: vec :: Vec <
                Value > > = __cq_store_r1; let __cq_store_r3 : & :: std ::
                collections :: BTreeMap < Value, :: std :: vec :: Vec < Value
                > > = __cq_store_S; let __cq_index_i1 : & :: std ::
                collections :: BTreeMap < Value, :: std :: vec :: Vec < Value
                > > = __cq_store_r3;
            } pending { lower r4 }
            continuations(:: mini_linq :: resume_physical, mini_linq ::
            access)
        } plan
        {
            struct ProjectedChain; relation L(a : Value); relation
            R(a : Value, b : Value); relation S(b : Value, c : Value);
            answer(a, c) : - L(a), R(a, b), S(b, c); relational
            {
                r0 = rename L { a - > a }; r1 = rename R { a - > a, b - > b };
                r2 = natural_join r0 with r1; r3 = rename S
                { b - > b, c - > c }; r4 = natural_join r2 with r3; r5 =
                project r4 keep { a, c }; output r5 as answer(a, c);
            } indexes { i0 = @ r1 [0]; i1 = @ r3 [0]; } uses
            { r2.right = i0; r4.right = i1; } physical
            {
                relations
                {
                    L = :: cq_example_providers :: slice_rows; R = ::
                    cq_example_providers :: hash_relation; S = ::
                    cq_example_providers :: sorted_relation; r0 = ::
                    cq_example_providers :: slice_rows; r1 = ::
                    cq_example_providers :: hash_relation; r2 = ::
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
                    cq_example_providers :: rename; r2 = :: cq_example_providers
                    :: indexed_join; r3 = :: cq_example_providers :: rename; r4
                    = :: cq_example_providers :: indexed_join; r5 = ::
                    cq_example_providers :: project;
                }
            }
        } environment
        {
            stored L at(__cq_store_L) operations :: cq_example_providers ::
            slice_rows stored R at(__cq_store_R) operations ::
            cq_example_providers :: hash_relation stored S at(__cq_store_S)
            operations :: cq_example_providers :: sorted_relation stored r0
            at(__cq_store_r0) operations :: cq_example_providers :: slice_rows
            stored r1 at(__cq_store_r1) operations :: cq_example_providers ::
            hash_relation indexed i0 at(__cq_index_i0) operations ::
            cq_example_providers :: hash_relation region r2
            {
                r2(a, b) => ((a, b,)) : - r0(a) => bind a <-
                ((__cq_store_r0).iter()), r1(a, b) => bind b <-
                ((__cq_index_i0).get(a).into_iter().flatten());
            } stored r3 at(__cq_store_r3) operations :: cq_example_providers
            :: sorted_relation indexed i1 at(__cq_index_i1) operations ::
            cq_example_providers :: sorted_relation
        } request { lower r4 }
    } access
    {
        r0(a) => bind a <- ((__cq_store_r0).iter()), r1(a, b) => bind b <-
        ((__cq_index_i0).get(a).into_iter().flatten()), r3(b, c) => bind c <-
        ((__cq_index_i1).get(b).into_iter().flatten())
    } witness { row((a, b,)) fields { 0 = (a); 1 = (b); } }
} plan
{
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
} environment
{
    stored L at(__cq_store_L) operations :: cq_example_providers :: slice_rows
    stored R at(__cq_store_R) operations :: cq_example_providers ::
    hash_relation stored S at(__cq_store_S) operations :: cq_example_providers
    :: sorted_relation stored r0 at(__cq_store_r0) operations ::
    cq_example_providers :: slice_rows stored r1 at(__cq_store_r1) operations
    :: cq_example_providers :: hash_relation indexed i0 at(__cq_index_i0)
    operations :: cq_example_providers :: hash_relation region r2
    {
        r2(a, b) => ((a, b,)) : - r0(a) => bind a <- ((__cq_store_r0).iter()),
        r1(a, b) => bind b <- ((__cq_index_i0).get(a).into_iter().flatten());
    } stored r3 at(__cq_store_r3) operations :: cq_example_providers ::
    sorted_relation indexed i1 at(__cq_index_i1) operations ::
    cq_example_providers :: sorted_relation
} request
{
    output r4 fields { 0 = (a); 1 = (b); 2 = (c); } whole
    { r2 = ((a, b,)); r3 = ((b, c,)); }
}
```

Actual output (formatted where Rust syntax permits):

```rust
::cq_example_providers::indexed_join_output! {
    resume
    {
        request
        {
            reply_to(:: mini_linq :: resume_physical)
            {
                plan
                {
                    struct ProjectedChain; relation L(a : Value); relation
                    R(a : Value, b : Value); relation S(b : Value, c : Value);
                    answer(a, c) : - L(a), R(a, b), S(b, c); relational
                    {
                        r0 = rename L { a - > a }; r1 = rename R
                        { a - > a, b - > b }; r2 = natural_join r0 with r1; r3 =
                        rename S { b - > b, c - > c }; r4 = natural_join r2 with r3;
                        r5 = project r4 keep { a, c }; output r5 as answer(a, c);
                    } indexes { i0 = @ r1 [0]; i1 = @ r3 [0]; } uses
                    { r2.right = i0; r4.right = i1; } physical
                    {
                        relations
                        {
                            L = :: cq_example_providers :: slice_rows; R = ::
                            cq_example_providers :: hash_relation; S = ::
                            cq_example_providers :: sorted_relation; r0 = ::
                            cq_example_providers :: slice_rows; r1 = ::
                            cq_example_providers :: hash_relation; r2 = ::
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
                            cq_example_providers :: rename; r2 = :: cq_example_providers
                            :: indexed_join; r3 = :: cq_example_providers :: rename; r4
                            = :: cq_example_providers :: indexed_join; r5 = ::
                            cq_example_providers :: project;
                        }
                    }
                } environment
                {
                    stored L at(__cq_store_L) operations :: cq_example_providers
                    :: slice_rows stored R at(__cq_store_R) operations ::
                    cq_example_providers :: hash_relation stored S
                    at(__cq_store_S) operations :: cq_example_providers ::
                    sorted_relation stored r0 at(__cq_store_r0) operations ::
                    cq_example_providers :: slice_rows stored r1
                    at(__cq_store_r1) operations :: cq_example_providers ::
                    hash_relation indexed i0 at(__cq_index_i0) operations ::
                    cq_example_providers :: hash_relation region r2
                    {
                        r2(a, b) => ((a, b,)) : - r0(a) => bind a <-
                        ((__cq_store_r0).iter()), r1(a, b) => bind b <-
                        ((__cq_index_i0).get(a).into_iter().flatten());
                    } stored r3 at(__cq_store_r3) operations ::
                    cq_example_providers :: sorted_relation indexed i1
                    at(__cq_index_i1) operations :: cq_example_providers ::
                    sorted_relation
                } emitted
                {
                    let __cq_store_L : & [Value] = L; let __cq_store_R : & ::
                    std :: collections :: HashMap < Value, :: std :: vec :: Vec
                    < Value > > = R; let __cq_store_S : & :: std :: collections
                    :: BTreeMap < Value, :: std :: vec :: Vec < Value > > = S;
                    let __cq_store_r0 : & [Value] = __cq_store_L; let
                    __cq_store_r1 : & :: std :: collections :: HashMap < Value,
                    :: std :: vec :: Vec < Value > > = __cq_store_R; let
                    __cq_index_i0 : & :: std :: collections :: HashMap < Value,
                    :: std :: vec :: Vec < Value > > = __cq_store_r1; let
                    __cq_store_r3 : & :: std :: collections :: BTreeMap < Value,
                    :: std :: vec :: Vec < Value > > = __cq_store_S; let
                    __cq_index_i1 : & :: std :: collections :: BTreeMap < Value,
                    :: std :: vec :: Vec < Value > > = __cq_store_r3;
                } pending { lower r4 }
                continuations(:: mini_linq :: resume_physical, mini_linq ::
                access)
            } plan
            {
                struct ProjectedChain; relation L(a : Value); relation
                R(a : Value, b : Value); relation S(b : Value, c : Value);
                answer(a, c) : - L(a), R(a, b), S(b, c); relational
                {
                    r0 = rename L { a - > a }; r1 = rename R
                    { a - > a, b - > b }; r2 = natural_join r0 with r1; r3 =
                    rename S { b - > b, c - > c }; r4 = natural_join r2 with r3;
                    r5 = project r4 keep { a, c }; output r5 as answer(a, c);
                } indexes { i0 = @ r1 [0]; i1 = @ r3 [0]; } uses
                { r2.right = i0; r4.right = i1; } physical
                {
                    relations
                    {
                        L = :: cq_example_providers :: slice_rows; R = ::
                        cq_example_providers :: hash_relation; S = ::
                        cq_example_providers :: sorted_relation; r0 = ::
                        cq_example_providers :: slice_rows; r1 = ::
                        cq_example_providers :: hash_relation; r2 = ::
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
                        cq_example_providers :: rename; r2 = :: cq_example_providers
                        :: indexed_join; r3 = :: cq_example_providers :: rename; r4
                        = :: cq_example_providers :: indexed_join; r5 = ::
                        cq_example_providers :: project;
                    }
                }
            } environment
            {
                stored L at(__cq_store_L) operations :: cq_example_providers
                :: slice_rows stored R at(__cq_store_R) operations ::
                cq_example_providers :: hash_relation stored S
                at(__cq_store_S) operations :: cq_example_providers ::
                sorted_relation stored r0 at(__cq_store_r0) operations ::
                cq_example_providers :: slice_rows stored r1 at(__cq_store_r1)
                operations :: cq_example_providers :: hash_relation indexed i0
                at(__cq_index_i0) operations :: cq_example_providers ::
                hash_relation region r2
                {
                    r2(a, b) => ((a, b,)) : - r0(a) => bind a <-
                    ((__cq_store_r0).iter()), r1(a, b) => bind b <-
                    ((__cq_index_i0).get(a).into_iter().flatten());
                } stored r3 at(__cq_store_r3) operations ::
                cq_example_providers :: sorted_relation indexed i1
                at(__cq_index_i1) operations :: cq_example_providers ::
                sorted_relation
            } request { lower r4 }
        } access
        {
            r0(a) => bind a <- ((__cq_store_r0).iter()), r1(a, b) => bind b <-
            ((__cq_index_i0).get(a).into_iter().flatten()), r3(b, c) => bind c
            <- ((__cq_index_i1).get(b).into_iter().flatten())
        } witness { row((a, b,)) fields { 0 = (a); 1 = (b); } }
    } witness { row((a, b, c,)) fields { 0 = (a); 1 = (b); 2 = (c); } }
}

```

---

## 2172002/036-cq-example-providers::indexed_join_output

[Raw input](raw/2172002/036-cq-example-providers::indexed_join_output.input.tokens) · [Raw output](raw/2172002/036-cq-example-providers::indexed_join_output.output.tokens)

Actual input:

```rust
resume
{
    request
    {
        reply_to(:: mini_linq :: resume_physical)
        {
            plan
            {
                struct ProjectedChain; relation L(a : Value); relation
                R(a : Value, b : Value); relation S(b : Value, c : Value);
                answer(a, c) : - L(a), R(a, b), S(b, c); relational
                {
                    r0 = rename L { a - > a }; r1 = rename R
                    { a - > a, b - > b }; r2 = natural_join r0 with r1; r3 =
                    rename S { b - > b, c - > c }; r4 = natural_join r2 with r3;
                    r5 = project r4 keep { a, c }; output r5 as answer(a, c);
                } indexes { i0 = @ r1 [0]; i1 = @ r3 [0]; } uses
                { r2.right = i0; r4.right = i1; } physical
                {
                    relations
                    {
                        L = :: cq_example_providers :: slice_rows; R = ::
                        cq_example_providers :: hash_relation; S = ::
                        cq_example_providers :: sorted_relation; r0 = ::
                        cq_example_providers :: slice_rows; r1 = ::
                        cq_example_providers :: hash_relation; r2 = ::
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
                        cq_example_providers :: rename; r2 = :: cq_example_providers
                        :: indexed_join; r3 = :: cq_example_providers :: rename; r4
                        = :: cq_example_providers :: indexed_join; r5 = ::
                        cq_example_providers :: project;
                    }
                }
            } environment
            {
                stored L at(__cq_store_L) operations :: cq_example_providers
                :: slice_rows stored R at(__cq_store_R) operations ::
                cq_example_providers :: hash_relation stored S
                at(__cq_store_S) operations :: cq_example_providers ::
                sorted_relation stored r0 at(__cq_store_r0) operations ::
                cq_example_providers :: slice_rows stored r1 at(__cq_store_r1)
                operations :: cq_example_providers :: hash_relation indexed i0
                at(__cq_index_i0) operations :: cq_example_providers ::
                hash_relation region r2
                {
                    r2(a, b) => ((a, b,)) : - r0(a) => bind a <-
                    ((__cq_store_r0).iter()), r1(a, b) => bind b <-
                    ((__cq_index_i0).get(a).into_iter().flatten());
                } stored r3 at(__cq_store_r3) operations ::
                cq_example_providers :: sorted_relation indexed i1
                at(__cq_index_i1) operations :: cq_example_providers ::
                sorted_relation
            } emitted
            {
                let __cq_store_L : & [Value] = L; let __cq_store_R : & :: std
                :: collections :: HashMap < Value, :: std :: vec :: Vec <
                Value > > = R; let __cq_store_S : & :: std :: collections ::
                BTreeMap < Value, :: std :: vec :: Vec < Value > > = S; let
                __cq_store_r0 : & [Value] = __cq_store_L; let __cq_store_r1 :
                & :: std :: collections :: HashMap < Value, :: std :: vec ::
                Vec < Value > > = __cq_store_R; let __cq_index_i0 : & :: std
                :: collections :: HashMap < Value, :: std :: vec :: Vec <
                Value > > = __cq_store_r1; let __cq_store_r3 : & :: std ::
                collections :: BTreeMap < Value, :: std :: vec :: Vec < Value
                > > = __cq_store_S; let __cq_index_i1 : & :: std ::
                collections :: BTreeMap < Value, :: std :: vec :: Vec < Value
                > > = __cq_store_r3;
            } pending { lower r4 }
            continuations(:: mini_linq :: resume_physical, mini_linq ::
            access)
        } plan
        {
            struct ProjectedChain; relation L(a : Value); relation
            R(a : Value, b : Value); relation S(b : Value, c : Value);
            answer(a, c) : - L(a), R(a, b), S(b, c); relational
            {
                r0 = rename L { a - > a }; r1 = rename R { a - > a, b - > b };
                r2 = natural_join r0 with r1; r3 = rename S
                { b - > b, c - > c }; r4 = natural_join r2 with r3; r5 =
                project r4 keep { a, c }; output r5 as answer(a, c);
            } indexes { i0 = @ r1 [0]; i1 = @ r3 [0]; } uses
            { r2.right = i0; r4.right = i1; } physical
            {
                relations
                {
                    L = :: cq_example_providers :: slice_rows; R = ::
                    cq_example_providers :: hash_relation; S = ::
                    cq_example_providers :: sorted_relation; r0 = ::
                    cq_example_providers :: slice_rows; r1 = ::
                    cq_example_providers :: hash_relation; r2 = ::
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
                    cq_example_providers :: rename; r2 = :: cq_example_providers
                    :: indexed_join; r3 = :: cq_example_providers :: rename; r4
                    = :: cq_example_providers :: indexed_join; r5 = ::
                    cq_example_providers :: project;
                }
            }
        } environment
        {
            stored L at(__cq_store_L) operations :: cq_example_providers ::
            slice_rows stored R at(__cq_store_R) operations ::
            cq_example_providers :: hash_relation stored S at(__cq_store_S)
            operations :: cq_example_providers :: sorted_relation stored r0
            at(__cq_store_r0) operations :: cq_example_providers :: slice_rows
            stored r1 at(__cq_store_r1) operations :: cq_example_providers ::
            hash_relation indexed i0 at(__cq_index_i0) operations ::
            cq_example_providers :: hash_relation region r2
            {
                r2(a, b) => ((a, b,)) : - r0(a) => bind a <-
                ((__cq_store_r0).iter()), r1(a, b) => bind b <-
                ((__cq_index_i0).get(a).into_iter().flatten());
            } stored r3 at(__cq_store_r3) operations :: cq_example_providers
            :: sorted_relation indexed i1 at(__cq_index_i1) operations ::
            cq_example_providers :: sorted_relation
        } request { lower r4 }
    } access
    {
        r0(a) => bind a <- ((__cq_store_r0).iter()), r1(a, b) => bind b <-
        ((__cq_index_i0).get(a).into_iter().flatten()), r3(b, c) => bind c <-
        ((__cq_index_i1).get(b).into_iter().flatten())
    } witness { row((a, b,)) fields { 0 = (a); 1 = (b); } }
} witness { row((a, b, c,)) fields { 0 = (a); 1 = (b); 2 = (c); } }
```

Actual output (formatted where Rust syntax permits):

```rust
::mini_linq::resume_physical! {
    resume
    {
        plan
        {
            struct ProjectedChain; relation L(a : Value); relation
            R(a : Value, b : Value); relation S(b : Value, c : Value);
            answer(a, c) : - L(a), R(a, b), S(b, c); relational
            {
                r0 = rename L { a - > a }; r1 = rename R { a - > a, b - > b };
                r2 = natural_join r0 with r1; r3 = rename S
                { b - > b, c - > c }; r4 = natural_join r2 with r3; r5 =
                project r4 keep { a, c }; output r5 as answer(a, c);
            } indexes { i0 = @ r1 [0]; i1 = @ r3 [0]; } uses
            { r2.right = i0; r4.right = i1; } physical
            {
                relations
                {
                    L = :: cq_example_providers :: slice_rows; R = ::
                    cq_example_providers :: hash_relation; S = ::
                    cq_example_providers :: sorted_relation; r0 = ::
                    cq_example_providers :: slice_rows; r1 = ::
                    cq_example_providers :: hash_relation; r2 = ::
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
                    cq_example_providers :: rename; r2 = :: cq_example_providers
                    :: indexed_join; r3 = :: cq_example_providers :: rename; r4
                    = :: cq_example_providers :: indexed_join; r5 = ::
                    cq_example_providers :: project;
                }
            }
        } environment
        {
            stored L at(__cq_store_L) operations :: cq_example_providers ::
            slice_rows stored R at(__cq_store_R) operations ::
            cq_example_providers :: hash_relation stored S at(__cq_store_S)
            operations :: cq_example_providers :: sorted_relation stored r0
            at(__cq_store_r0) operations :: cq_example_providers :: slice_rows
            stored r1 at(__cq_store_r1) operations :: cq_example_providers ::
            hash_relation indexed i0 at(__cq_index_i0) operations ::
            cq_example_providers :: hash_relation region r2
            {
                r2(a, b) => ((a, b,)) : - r0(a) => bind a <-
                ((__cq_store_r0).iter()), r1(a, b) => bind b <-
                ((__cq_index_i0).get(a).into_iter().flatten());
            } stored r3 at(__cq_store_r3) operations :: cq_example_providers
            :: sorted_relation indexed i1 at(__cq_index_i1) operations ::
            cq_example_providers :: sorted_relation
        } emitted
        {
            let __cq_store_L : & [Value] = L; let __cq_store_R : & :: std ::
            collections :: HashMap < Value, :: std :: vec :: Vec < Value > > =
            R; let __cq_store_S : & :: std :: collections :: BTreeMap < Value,
            :: std :: vec :: Vec < Value > > = S; let __cq_store_r0 : &
            [Value] = __cq_store_L; let __cq_store_r1 : & :: std ::
            collections :: HashMap < Value, :: std :: vec :: Vec < Value > > =
            __cq_store_R; let __cq_index_i0 : & :: std :: collections ::
            HashMap < Value, :: std :: vec :: Vec < Value > > = __cq_store_r1;
            let __cq_store_r3 : & :: std :: collections :: BTreeMap < Value,
            :: std :: vec :: Vec < Value > > = __cq_store_S; let __cq_index_i1
            : & :: std :: collections :: BTreeMap < Value, :: std :: vec ::
            Vec < Value > > = __cq_store_r3;
        } pending { lower r4 }
        continuations(:: mini_linq :: resume_physical, mini_linq :: access)
    } emitted {} result
    {
        region r4
        {
            r4(a, b, c) => ((a, b, c,)) : - r0(a) => bind a <-
            ((__cq_store_r0).iter()), r1(a, b) => bind b <-
            ((__cq_index_i0).get(a).into_iter().flatten()), r3(b, c) => bind c
            <- ((__cq_index_i1).get(b).into_iter().flatten());
        }
    }
}

```

---

## 2172002/037-resume_physical

[Raw input](raw/2172002/037-resume_physical.input.tokens) · [Raw output](raw/2172002/037-resume_physical.output.tokens)

Actual input:

```rust
resume
{
    plan
    {
        struct ProjectedChain; relation L(a : Value); relation
        R(a : Value, b : Value); relation S(b : Value, c : Value);
        answer(a, c) : - L(a), R(a, b), S(b, c); relational
        {
            r0 = rename L { a - > a }; r1 = rename R { a - > a, b - > b }; r2
            = natural_join r0 with r1; r3 = rename S { b - > b, c - > c }; r4
            = natural_join r2 with r3; r5 = project r4 keep { a, c }; output
            r5 as answer(a, c);
        } indexes { i0 = @ r1 [0]; i1 = @ r3 [0]; } uses
        { r2.right = i0; r4.right = i1; } physical
        {
            relations
            {
                L = :: cq_example_providers :: slice_rows; R = ::
                cq_example_providers :: hash_relation; S = ::
                cq_example_providers :: sorted_relation; r0 = ::
                cq_example_providers :: slice_rows; r1 = ::
                cq_example_providers :: hash_relation; r2 = ::
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
                cq_example_providers :: rename; r2 = :: cq_example_providers
                :: indexed_join; r3 = :: cq_example_providers :: rename; r4 =
                :: cq_example_providers :: indexed_join; r5 = ::
                cq_example_providers :: project;
            }
        }
    } environment
    {
        stored L at(__cq_store_L) operations :: cq_example_providers ::
        slice_rows stored R at(__cq_store_R) operations ::
        cq_example_providers :: hash_relation stored S at(__cq_store_S)
        operations :: cq_example_providers :: sorted_relation stored r0
        at(__cq_store_r0) operations :: cq_example_providers :: slice_rows
        stored r1 at(__cq_store_r1) operations :: cq_example_providers ::
        hash_relation indexed i0 at(__cq_index_i0) operations ::
        cq_example_providers :: hash_relation region r2
        {
            r2(a, b) => ((a, b,)) : - r0(a) => bind a <-
            ((__cq_store_r0).iter()), r1(a, b) => bind b <-
            ((__cq_index_i0).get(a).into_iter().flatten());
        } stored r3 at(__cq_store_r3) operations :: cq_example_providers ::
        sorted_relation indexed i1 at(__cq_index_i1) operations ::
        cq_example_providers :: sorted_relation
    } emitted
    {
        let __cq_store_L : & [Value] = L; let __cq_store_R : & :: std ::
        collections :: HashMap < Value, :: std :: vec :: Vec < Value > > = R;
        let __cq_store_S : & :: std :: collections :: BTreeMap < Value, :: std
        :: vec :: Vec < Value > > = S; let __cq_store_r0 : & [Value] =
        __cq_store_L; let __cq_store_r1 : & :: std :: collections :: HashMap <
        Value, :: std :: vec :: Vec < Value > > = __cq_store_R; let
        __cq_index_i0 : & :: std :: collections :: HashMap < Value, :: std ::
        vec :: Vec < Value > > = __cq_store_r1; let __cq_store_r3 : & :: std
        :: collections :: BTreeMap < Value, :: std :: vec :: Vec < Value > > =
        __cq_store_S; let __cq_index_i1 : & :: std :: collections :: BTreeMap
        < Value, :: std :: vec :: Vec < Value > > = __cq_store_r3;
    } pending { lower r4 }
    continuations(:: mini_linq :: resume_physical, mini_linq :: access)
} emitted {} result
{
    region r4
    {
        r4(a, b, c) => ((a, b, c,)) : - r0(a) => bind a <-
        ((__cq_store_r0).iter()), r1(a, b) => bind b <-
        ((__cq_index_i0).get(a).into_iter().flatten()), r3(b, c) => bind c <-
        ((__cq_index_i1).get(b).into_iter().flatten());
    }
}
```

Actual output (formatted where Rust syntax permits):

```rust
::cq_example_providers::project! {
    reply_to(:: mini_linq :: resume_physical)
    {
        plan
        {
            struct ProjectedChain; relation L(a : Value); relation
            R(a : Value, b : Value); relation S(b : Value, c : Value);
            answer(a, c) : - L(a), R(a, b), S(b, c); relational
            {
                r0 = rename L { a - > a }; r1 = rename R { a - > a, b - > b };
                r2 = natural_join r0 with r1; r3 = rename S
                { b - > b, c - > c }; r4 = natural_join r2 with r3; r5 =
                project r4 keep { a, c }; output r5 as answer(a, c);
            } indexes { i0 = @ r1 [0]; i1 = @ r3 [0]; } uses
            { r2.right = i0; r4.right = i1; } physical
            {
                relations
                {
                    L = :: cq_example_providers :: slice_rows; R = ::
                    cq_example_providers :: hash_relation; S = ::
                    cq_example_providers :: sorted_relation; r0 = ::
                    cq_example_providers :: slice_rows; r1 = ::
                    cq_example_providers :: hash_relation; r2 = ::
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
                    cq_example_providers :: rename; r2 = :: cq_example_providers
                    :: indexed_join; r3 = :: cq_example_providers :: rename; r4
                    = :: cq_example_providers :: indexed_join; r5 = ::
                    cq_example_providers :: project;
                }
            }
        } environment
        {
            stored L at(__cq_store_L) operations :: cq_example_providers ::
            slice_rows stored R at(__cq_store_R) operations ::
            cq_example_providers :: hash_relation stored S at(__cq_store_S)
            operations :: cq_example_providers :: sorted_relation stored r0
            at(__cq_store_r0) operations :: cq_example_providers :: slice_rows
            stored r1 at(__cq_store_r1) operations :: cq_example_providers ::
            hash_relation indexed i0 at(__cq_index_i0) operations ::
            cq_example_providers :: hash_relation region r2
            {
                r2(a, b) => ((a, b,)) : - r0(a) => bind a <-
                ((__cq_store_r0).iter()), r1(a, b) => bind b <-
                ((__cq_index_i0).get(a).into_iter().flatten());
            } stored r3 at(__cq_store_r3) operations :: cq_example_providers
            :: sorted_relation indexed i1 at(__cq_index_i1) operations ::
            cq_example_providers :: sorted_relation region r4
            {
                r4(a, b, c) => ((a, b, c,)) : - r0(a) => bind a <-
                ((__cq_store_r0).iter()), r1(a, b) => bind b <-
                ((__cq_index_i0).get(a).into_iter().flatten()), r3(b, c) =>
                bind c <- ((__cq_index_i1).get(b).into_iter().flatten());
            }
        } emitted
        {
            let __cq_store_L : & [Value] = L; let __cq_store_R : & :: std ::
            collections :: HashMap < Value, :: std :: vec :: Vec < Value > > =
            R; let __cq_store_S : & :: std :: collections :: BTreeMap < Value,
            :: std :: vec :: Vec < Value > > = S; let __cq_store_r0 : &
            [Value] = __cq_store_L; let __cq_store_r1 : & :: std ::
            collections :: HashMap < Value, :: std :: vec :: Vec < Value > > =
            __cq_store_R; let __cq_index_i0 : & :: std :: collections ::
            HashMap < Value, :: std :: vec :: Vec < Value > > = __cq_store_r1;
            let __cq_store_r3 : & :: std :: collections :: BTreeMap < Value,
            :: std :: vec :: Vec < Value > > = __cq_store_S; let __cq_index_i1
            : & :: std :: collections :: BTreeMap < Value, :: std :: vec ::
            Vec < Value > > = __cq_store_r3;
        } pending { lower r5 }
        continuations(:: mini_linq :: resume_physical, mini_linq :: access)
    } plan
    {
        struct ProjectedChain; relation L(a : Value); relation
        R(a : Value, b : Value); relation S(b : Value, c : Value);
        answer(a, c) : - L(a), R(a, b), S(b, c); relational
        {
            r0 = rename L { a - > a }; r1 = rename R { a - > a, b - > b }; r2
            = natural_join r0 with r1; r3 = rename S { b - > b, c - > c }; r4
            = natural_join r2 with r3; r5 = project r4 keep { a, c }; output
            r5 as answer(a, c);
        } indexes { i0 = @ r1 [0]; i1 = @ r3 [0]; } uses
        { r2.right = i0; r4.right = i1; } physical
        {
            relations
            {
                L = :: cq_example_providers :: slice_rows; R = ::
                cq_example_providers :: hash_relation; S = ::
                cq_example_providers :: sorted_relation; r0 = ::
                cq_example_providers :: slice_rows; r1 = ::
                cq_example_providers :: hash_relation; r2 = ::
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
                cq_example_providers :: rename; r2 = :: cq_example_providers
                :: indexed_join; r3 = :: cq_example_providers :: rename; r4 =
                :: cq_example_providers :: indexed_join; r5 = ::
                cq_example_providers :: project;
            }
        }
    } environment
    {
        stored L at(__cq_store_L) operations :: cq_example_providers ::
        slice_rows stored R at(__cq_store_R) operations ::
        cq_example_providers :: hash_relation stored S at(__cq_store_S)
        operations :: cq_example_providers :: sorted_relation stored r0
        at(__cq_store_r0) operations :: cq_example_providers :: slice_rows
        stored r1 at(__cq_store_r1) operations :: cq_example_providers ::
        hash_relation indexed i0 at(__cq_index_i0) operations ::
        cq_example_providers :: hash_relation region r2
        {
            r2(a, b) => ((a, b,)) : - r0(a) => bind a <-
            ((__cq_store_r0).iter()), r1(a, b) => bind b <-
            ((__cq_index_i0).get(a).into_iter().flatten());
        } stored r3 at(__cq_store_r3) operations :: cq_example_providers ::
        sorted_relation indexed i1 at(__cq_index_i1) operations ::
        cq_example_providers :: sorted_relation region r4
        {
            r4(a, b, c) => ((a, b, c,)) : - r0(a) => bind a <-
            ((__cq_store_r0).iter()), r1(a, b) => bind b <-
            ((__cq_index_i0).get(a).into_iter().flatten()), r3(b, c) => bind c
            <- ((__cq_index_i1).get(b).into_iter().flatten());
        }
    } request { lower r5 }
}

```

---

## 2172002/038-cq-example-providers::project

[Raw input](raw/2172002/038-cq-example-providers::project.input.tokens) · [Raw output](raw/2172002/038-cq-example-providers::project.output.tokens)

Actual input:

```rust
reply_to(:: mini_linq :: resume_physical)
{
    plan
    {
        struct ProjectedChain; relation L(a : Value); relation
        R(a : Value, b : Value); relation S(b : Value, c : Value);
        answer(a, c) : - L(a), R(a, b), S(b, c); relational
        {
            r0 = rename L { a - > a }; r1 = rename R { a - > a, b - > b }; r2
            = natural_join r0 with r1; r3 = rename S { b - > b, c - > c }; r4
            = natural_join r2 with r3; r5 = project r4 keep { a, c }; output
            r5 as answer(a, c);
        } indexes { i0 = @ r1 [0]; i1 = @ r3 [0]; } uses
        { r2.right = i0; r4.right = i1; } physical
        {
            relations
            {
                L = :: cq_example_providers :: slice_rows; R = ::
                cq_example_providers :: hash_relation; S = ::
                cq_example_providers :: sorted_relation; r0 = ::
                cq_example_providers :: slice_rows; r1 = ::
                cq_example_providers :: hash_relation; r2 = ::
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
                cq_example_providers :: rename; r2 = :: cq_example_providers
                :: indexed_join; r3 = :: cq_example_providers :: rename; r4 =
                :: cq_example_providers :: indexed_join; r5 = ::
                cq_example_providers :: project;
            }
        }
    } environment
    {
        stored L at(__cq_store_L) operations :: cq_example_providers ::
        slice_rows stored R at(__cq_store_R) operations ::
        cq_example_providers :: hash_relation stored S at(__cq_store_S)
        operations :: cq_example_providers :: sorted_relation stored r0
        at(__cq_store_r0) operations :: cq_example_providers :: slice_rows
        stored r1 at(__cq_store_r1) operations :: cq_example_providers ::
        hash_relation indexed i0 at(__cq_index_i0) operations ::
        cq_example_providers :: hash_relation region r2
        {
            r2(a, b) => ((a, b,)) : - r0(a) => bind a <-
            ((__cq_store_r0).iter()), r1(a, b) => bind b <-
            ((__cq_index_i0).get(a).into_iter().flatten());
        } stored r3 at(__cq_store_r3) operations :: cq_example_providers ::
        sorted_relation indexed i1 at(__cq_index_i1) operations ::
        cq_example_providers :: sorted_relation region r4
        {
            r4(a, b, c) => ((a, b, c,)) : - r0(a) => bind a <-
            ((__cq_store_r0).iter()), r1(a, b) => bind b <-
            ((__cq_index_i0).get(a).into_iter().flatten()), r3(b, c) => bind c
            <- ((__cq_index_i1).get(b).into_iter().flatten());
        }
    } emitted
    {
        let __cq_store_L : & [Value] = L; let __cq_store_R : & :: std ::
        collections :: HashMap < Value, :: std :: vec :: Vec < Value > > = R;
        let __cq_store_S : & :: std :: collections :: BTreeMap < Value, :: std
        :: vec :: Vec < Value > > = S; let __cq_store_r0 : & [Value] =
        __cq_store_L; let __cq_store_r1 : & :: std :: collections :: HashMap <
        Value, :: std :: vec :: Vec < Value > > = __cq_store_R; let
        __cq_index_i0 : & :: std :: collections :: HashMap < Value, :: std ::
        vec :: Vec < Value > > = __cq_store_r1; let __cq_store_r3 : & :: std
        :: collections :: BTreeMap < Value, :: std :: vec :: Vec < Value > > =
        __cq_store_S; let __cq_index_i1 : & :: std :: collections :: BTreeMap
        < Value, :: std :: vec :: Vec < Value > > = __cq_store_r3;
    } pending { lower r5 }
    continuations(:: mini_linq :: resume_physical, mini_linq :: access)
} plan
{
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
} environment
{
    stored L at(__cq_store_L) operations :: cq_example_providers :: slice_rows
    stored R at(__cq_store_R) operations :: cq_example_providers ::
    hash_relation stored S at(__cq_store_S) operations :: cq_example_providers
    :: sorted_relation stored r0 at(__cq_store_r0) operations ::
    cq_example_providers :: slice_rows stored r1 at(__cq_store_r1) operations
    :: cq_example_providers :: hash_relation indexed i0 at(__cq_index_i0)
    operations :: cq_example_providers :: hash_relation region r2
    {
        r2(a, b) => ((a, b,)) : - r0(a) => bind a <- ((__cq_store_r0).iter()),
        r1(a, b) => bind b <- ((__cq_index_i0).get(a).into_iter().flatten());
    } stored r3 at(__cq_store_r3) operations :: cq_example_providers ::
    sorted_relation indexed i1 at(__cq_index_i1) operations ::
    cq_example_providers :: sorted_relation region r4
    {
        r4(a, b, c) => ((a, b, c,)) : - r0(a) => bind a <-
        ((__cq_store_r0).iter()), r1(a, b) => bind b <-
        ((__cq_index_i0).get(a).into_iter().flatten()), r3(b, c) => bind c <-
        ((__cq_index_i1).get(b).into_iter().flatten());
    }
} request { lower r5 }
```

Actual output (formatted where Rust syntax permits):

```rust
::cq_example_providers::borrowed_fields! {
    reply_to(:: cq_example_providers :: project_output)
    {
        request
        {
            reply_to(:: mini_linq :: resume_physical)
            {
                plan
                {
                    struct ProjectedChain; relation L(a : Value); relation
                    R(a : Value, b : Value); relation S(b : Value, c : Value);
                    answer(a, c) : - L(a), R(a, b), S(b, c); relational
                    {
                        r0 = rename L { a - > a }; r1 = rename R
                        { a - > a, b - > b }; r2 = natural_join r0 with r1; r3 =
                        rename S { b - > b, c - > c }; r4 = natural_join r2 with r3;
                        r5 = project r4 keep { a, c }; output r5 as answer(a, c);
                    } indexes { i0 = @ r1 [0]; i1 = @ r3 [0]; } uses
                    { r2.right = i0; r4.right = i1; } physical
                    {
                        relations
                        {
                            L = :: cq_example_providers :: slice_rows; R = ::
                            cq_example_providers :: hash_relation; S = ::
                            cq_example_providers :: sorted_relation; r0 = ::
                            cq_example_providers :: slice_rows; r1 = ::
                            cq_example_providers :: hash_relation; r2 = ::
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
                            cq_example_providers :: rename; r2 = :: cq_example_providers
                            :: indexed_join; r3 = :: cq_example_providers :: rename; r4
                            = :: cq_example_providers :: indexed_join; r5 = ::
                            cq_example_providers :: project;
                        }
                    }
                } environment
                {
                    stored L at(__cq_store_L) operations :: cq_example_providers
                    :: slice_rows stored R at(__cq_store_R) operations ::
                    cq_example_providers :: hash_relation stored S
                    at(__cq_store_S) operations :: cq_example_providers ::
                    sorted_relation stored r0 at(__cq_store_r0) operations ::
                    cq_example_providers :: slice_rows stored r1
                    at(__cq_store_r1) operations :: cq_example_providers ::
                    hash_relation indexed i0 at(__cq_index_i0) operations ::
                    cq_example_providers :: hash_relation region r2
                    {
                        r2(a, b) => ((a, b,)) : - r0(a) => bind a <-
                        ((__cq_store_r0).iter()), r1(a, b) => bind b <-
                        ((__cq_index_i0).get(a).into_iter().flatten());
                    } stored r3 at(__cq_store_r3) operations ::
                    cq_example_providers :: sorted_relation indexed i1
                    at(__cq_index_i1) operations :: cq_example_providers ::
                    sorted_relation region r4
                    {
                        r4(a, b, c) => ((a, b, c,)) : - r0(a) => bind a <-
                        ((__cq_store_r0).iter()), r1(a, b) => bind b <-
                        ((__cq_index_i0).get(a).into_iter().flatten()), r3(b, c) =>
                        bind c <- ((__cq_index_i1).get(b).into_iter().flatten());
                    }
                } emitted
                {
                    let __cq_store_L : & [Value] = L; let __cq_store_R : & ::
                    std :: collections :: HashMap < Value, :: std :: vec :: Vec
                    < Value > > = R; let __cq_store_S : & :: std :: collections
                    :: BTreeMap < Value, :: std :: vec :: Vec < Value > > = S;
                    let __cq_store_r0 : & [Value] = __cq_store_L; let
                    __cq_store_r1 : & :: std :: collections :: HashMap < Value,
                    :: std :: vec :: Vec < Value > > = __cq_store_R; let
                    __cq_index_i0 : & :: std :: collections :: HashMap < Value,
                    :: std :: vec :: Vec < Value > > = __cq_store_r1; let
                    __cq_store_r3 : & :: std :: collections :: BTreeMap < Value,
                    :: std :: vec :: Vec < Value > > = __cq_store_S; let
                    __cq_index_i1 : & :: std :: collections :: BTreeMap < Value,
                    :: std :: vec :: Vec < Value > > = __cq_store_r3;
                } pending { lower r5 }
                continuations(:: mini_linq :: resume_physical, mini_linq ::
                access)
            } plan
            {
                struct ProjectedChain; relation L(a : Value); relation
                R(a : Value, b : Value); relation S(b : Value, c : Value);
                answer(a, c) : - L(a), R(a, b), S(b, c); relational
                {
                    r0 = rename L { a - > a }; r1 = rename R
                    { a - > a, b - > b }; r2 = natural_join r0 with r1; r3 =
                    rename S { b - > b, c - > c }; r4 = natural_join r2 with r3;
                    r5 = project r4 keep { a, c }; output r5 as answer(a, c);
                } indexes { i0 = @ r1 [0]; i1 = @ r3 [0]; } uses
                { r2.right = i0; r4.right = i1; } physical
                {
                    relations
                    {
                        L = :: cq_example_providers :: slice_rows; R = ::
                        cq_example_providers :: hash_relation; S = ::
                        cq_example_providers :: sorted_relation; r0 = ::
                        cq_example_providers :: slice_rows; r1 = ::
                        cq_example_providers :: hash_relation; r2 = ::
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
                        cq_example_providers :: rename; r2 = :: cq_example_providers
                        :: indexed_join; r3 = :: cq_example_providers :: rename; r4
                        = :: cq_example_providers :: indexed_join; r5 = ::
                        cq_example_providers :: project;
                    }
                }
            } environment
            {
                stored L at(__cq_store_L) operations :: cq_example_providers
                :: slice_rows stored R at(__cq_store_R) operations ::
                cq_example_providers :: hash_relation stored S
                at(__cq_store_S) operations :: cq_example_providers ::
                sorted_relation stored r0 at(__cq_store_r0) operations ::
                cq_example_providers :: slice_rows stored r1 at(__cq_store_r1)
                operations :: cq_example_providers :: hash_relation indexed i0
                at(__cq_index_i0) operations :: cq_example_providers ::
                hash_relation region r2
                {
                    r2(a, b) => ((a, b,)) : - r0(a) => bind a <-
                    ((__cq_store_r0).iter()), r1(a, b) => bind b <-
                    ((__cq_index_i0).get(a).into_iter().flatten());
                } stored r3 at(__cq_store_r3) operations ::
                cq_example_providers :: sorted_relation indexed i1
                at(__cq_index_i1) operations :: cq_example_providers ::
                sorted_relation region r4
                {
                    r4(a, b, c) => ((a, b, c,)) : - r0(a) => bind a <-
                    ((__cq_store_r0).iter()), r1(a, b) => bind b <-
                    ((__cq_index_i0).get(a).into_iter().flatten()), r3(b, c) =>
                    bind c <- ((__cq_index_i1).get(b).into_iter().flatten());
                }
            } request { lower r5 }
        } access
        {
            r0(a) => bind a <- ((__cq_store_r0).iter()), r1(a, b) => bind b <-
            ((__cq_index_i0).get(a).into_iter().flatten()), r3(b, c) => bind c
            <- ((__cq_index_i1).get(b).into_iter().flatten())
        } witness { row((a, b, c,)) fields { 0 = (a); 1 = (b); 2 = (c); } }
    } plan
    {
        struct ProjectedChain; relation L(a : Value); relation
        R(a : Value, b : Value); relation S(b : Value, c : Value);
        answer(a, c) : - L(a), R(a, b), S(b, c); relational
        {
            r0 = rename L { a - > a }; r1 = rename R { a - > a, b - > b }; r2
            = natural_join r0 with r1; r3 = rename S { b - > b, c - > c }; r4
            = natural_join r2 with r3; r5 = project r4 keep { a, c }; output
            r5 as answer(a, c);
        } indexes { i0 = @ r1 [0]; i1 = @ r3 [0]; } uses
        { r2.right = i0; r4.right = i1; } physical
        {
            relations
            {
                L = :: cq_example_providers :: slice_rows; R = ::
                cq_example_providers :: hash_relation; S = ::
                cq_example_providers :: sorted_relation; r0 = ::
                cq_example_providers :: slice_rows; r1 = ::
                cq_example_providers :: hash_relation; r2 = ::
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
                cq_example_providers :: rename; r2 = :: cq_example_providers
                :: indexed_join; r3 = :: cq_example_providers :: rename; r4 =
                :: cq_example_providers :: indexed_join; r5 = ::
                cq_example_providers :: project;
            }
        }
    } environment
    {
        stored L at(__cq_store_L) operations :: cq_example_providers ::
        slice_rows stored R at(__cq_store_R) operations ::
        cq_example_providers :: hash_relation stored S at(__cq_store_S)
        operations :: cq_example_providers :: sorted_relation stored r0
        at(__cq_store_r0) operations :: cq_example_providers :: slice_rows
        stored r1 at(__cq_store_r1) operations :: cq_example_providers ::
        hash_relation indexed i0 at(__cq_index_i0) operations ::
        cq_example_providers :: hash_relation region r2
        {
            r2(a, b) => ((a, b,)) : - r0(a) => bind a <-
            ((__cq_store_r0).iter()), r1(a, b) => bind b <-
            ((__cq_index_i0).get(a).into_iter().flatten());
        } stored r3 at(__cq_store_r3) operations :: cq_example_providers ::
        sorted_relation indexed i1 at(__cq_index_i1) operations ::
        cq_example_providers :: sorted_relation region r4
        {
            r4(a, b, c) => ((a, b, c,)) : - r0(a) => bind a <-
            ((__cq_store_r0).iter()), r1(a, b) => bind b <-
            ((__cq_index_i0).get(a).into_iter().flatten()), r3(b, c) => bind c
            <- ((__cq_index_i1).get(b).into_iter().flatten());
        }
    } request
    { output r5 fields { 0 = (a); 1 = (c); } whole { r4 = ((a, b, c,)); } }
}

```

---

## 2172002/039-cq-example-providers::borrowed_fields

[Raw input](raw/2172002/039-cq-example-providers::borrowed_fields.input.tokens) · [Raw output](raw/2172002/039-cq-example-providers::borrowed_fields.output.tokens)

Actual input:

```rust
reply_to(:: cq_example_providers :: project_output)
{
    request
    {
        reply_to(:: mini_linq :: resume_physical)
        {
            plan
            {
                struct ProjectedChain; relation L(a : Value); relation
                R(a : Value, b : Value); relation S(b : Value, c : Value);
                answer(a, c) : - L(a), R(a, b), S(b, c); relational
                {
                    r0 = rename L { a - > a }; r1 = rename R
                    { a - > a, b - > b }; r2 = natural_join r0 with r1; r3 =
                    rename S { b - > b, c - > c }; r4 = natural_join r2 with r3;
                    r5 = project r4 keep { a, c }; output r5 as answer(a, c);
                } indexes { i0 = @ r1 [0]; i1 = @ r3 [0]; } uses
                { r2.right = i0; r4.right = i1; } physical
                {
                    relations
                    {
                        L = :: cq_example_providers :: slice_rows; R = ::
                        cq_example_providers :: hash_relation; S = ::
                        cq_example_providers :: sorted_relation; r0 = ::
                        cq_example_providers :: slice_rows; r1 = ::
                        cq_example_providers :: hash_relation; r2 = ::
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
                        cq_example_providers :: rename; r2 = :: cq_example_providers
                        :: indexed_join; r3 = :: cq_example_providers :: rename; r4
                        = :: cq_example_providers :: indexed_join; r5 = ::
                        cq_example_providers :: project;
                    }
                }
            } environment
            {
                stored L at(__cq_store_L) operations :: cq_example_providers
                :: slice_rows stored R at(__cq_store_R) operations ::
                cq_example_providers :: hash_relation stored S
                at(__cq_store_S) operations :: cq_example_providers ::
                sorted_relation stored r0 at(__cq_store_r0) operations ::
                cq_example_providers :: slice_rows stored r1 at(__cq_store_r1)
                operations :: cq_example_providers :: hash_relation indexed i0
                at(__cq_index_i0) operations :: cq_example_providers ::
                hash_relation region r2
                {
                    r2(a, b) => ((a, b,)) : - r0(a) => bind a <-
                    ((__cq_store_r0).iter()), r1(a, b) => bind b <-
                    ((__cq_index_i0).get(a).into_iter().flatten());
                } stored r3 at(__cq_store_r3) operations ::
                cq_example_providers :: sorted_relation indexed i1
                at(__cq_index_i1) operations :: cq_example_providers ::
                sorted_relation region r4
                {
                    r4(a, b, c) => ((a, b, c,)) : - r0(a) => bind a <-
                    ((__cq_store_r0).iter()), r1(a, b) => bind b <-
                    ((__cq_index_i0).get(a).into_iter().flatten()), r3(b, c) =>
                    bind c <- ((__cq_index_i1).get(b).into_iter().flatten());
                }
            } emitted
            {
                let __cq_store_L : & [Value] = L; let __cq_store_R : & :: std
                :: collections :: HashMap < Value, :: std :: vec :: Vec <
                Value > > = R; let __cq_store_S : & :: std :: collections ::
                BTreeMap < Value, :: std :: vec :: Vec < Value > > = S; let
                __cq_store_r0 : & [Value] = __cq_store_L; let __cq_store_r1 :
                & :: std :: collections :: HashMap < Value, :: std :: vec ::
                Vec < Value > > = __cq_store_R; let __cq_index_i0 : & :: std
                :: collections :: HashMap < Value, :: std :: vec :: Vec <
                Value > > = __cq_store_r1; let __cq_store_r3 : & :: std ::
                collections :: BTreeMap < Value, :: std :: vec :: Vec < Value
                > > = __cq_store_S; let __cq_index_i1 : & :: std ::
                collections :: BTreeMap < Value, :: std :: vec :: Vec < Value
                > > = __cq_store_r3;
            } pending { lower r5 }
            continuations(:: mini_linq :: resume_physical, mini_linq ::
            access)
        } plan
        {
            struct ProjectedChain; relation L(a : Value); relation
            R(a : Value, b : Value); relation S(b : Value, c : Value);
            answer(a, c) : - L(a), R(a, b), S(b, c); relational
            {
                r0 = rename L { a - > a }; r1 = rename R { a - > a, b - > b };
                r2 = natural_join r0 with r1; r3 = rename S
                { b - > b, c - > c }; r4 = natural_join r2 with r3; r5 =
                project r4 keep { a, c }; output r5 as answer(a, c);
            } indexes { i0 = @ r1 [0]; i1 = @ r3 [0]; } uses
            { r2.right = i0; r4.right = i1; } physical
            {
                relations
                {
                    L = :: cq_example_providers :: slice_rows; R = ::
                    cq_example_providers :: hash_relation; S = ::
                    cq_example_providers :: sorted_relation; r0 = ::
                    cq_example_providers :: slice_rows; r1 = ::
                    cq_example_providers :: hash_relation; r2 = ::
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
                    cq_example_providers :: rename; r2 = :: cq_example_providers
                    :: indexed_join; r3 = :: cq_example_providers :: rename; r4
                    = :: cq_example_providers :: indexed_join; r5 = ::
                    cq_example_providers :: project;
                }
            }
        } environment
        {
            stored L at(__cq_store_L) operations :: cq_example_providers ::
            slice_rows stored R at(__cq_store_R) operations ::
            cq_example_providers :: hash_relation stored S at(__cq_store_S)
            operations :: cq_example_providers :: sorted_relation stored r0
            at(__cq_store_r0) operations :: cq_example_providers :: slice_rows
            stored r1 at(__cq_store_r1) operations :: cq_example_providers ::
            hash_relation indexed i0 at(__cq_index_i0) operations ::
            cq_example_providers :: hash_relation region r2
            {
                r2(a, b) => ((a, b,)) : - r0(a) => bind a <-
                ((__cq_store_r0).iter()), r1(a, b) => bind b <-
                ((__cq_index_i0).get(a).into_iter().flatten());
            } stored r3 at(__cq_store_r3) operations :: cq_example_providers
            :: sorted_relation indexed i1 at(__cq_index_i1) operations ::
            cq_example_providers :: sorted_relation region r4
            {
                r4(a, b, c) => ((a, b, c,)) : - r0(a) => bind a <-
                ((__cq_store_r0).iter()), r1(a, b) => bind b <-
                ((__cq_index_i0).get(a).into_iter().flatten()), r3(b, c) =>
                bind c <- ((__cq_index_i1).get(b).into_iter().flatten());
            }
        } request { lower r5 }
    } access
    {
        r0(a) => bind a <- ((__cq_store_r0).iter()), r1(a, b) => bind b <-
        ((__cq_index_i0).get(a).into_iter().flatten()), r3(b, c) => bind c <-
        ((__cq_index_i1).get(b).into_iter().flatten())
    } witness { row((a, b, c,)) fields { 0 = (a); 1 = (b); 2 = (c); } }
} plan
{
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
} environment
{
    stored L at(__cq_store_L) operations :: cq_example_providers :: slice_rows
    stored R at(__cq_store_R) operations :: cq_example_providers ::
    hash_relation stored S at(__cq_store_S) operations :: cq_example_providers
    :: sorted_relation stored r0 at(__cq_store_r0) operations ::
    cq_example_providers :: slice_rows stored r1 at(__cq_store_r1) operations
    :: cq_example_providers :: hash_relation indexed i0 at(__cq_index_i0)
    operations :: cq_example_providers :: hash_relation region r2
    {
        r2(a, b) => ((a, b,)) : - r0(a) => bind a <- ((__cq_store_r0).iter()),
        r1(a, b) => bind b <- ((__cq_index_i0).get(a).into_iter().flatten());
    } stored r3 at(__cq_store_r3) operations :: cq_example_providers ::
    sorted_relation indexed i1 at(__cq_index_i1) operations ::
    cq_example_providers :: sorted_relation region r4
    {
        r4(a, b, c) => ((a, b, c,)) : - r0(a) => bind a <-
        ((__cq_store_r0).iter()), r1(a, b) => bind b <-
        ((__cq_index_i0).get(a).into_iter().flatten()), r3(b, c) => bind c <-
        ((__cq_index_i1).get(b).into_iter().flatten());
    }
} request
{ output r5 fields { 0 = (a); 1 = (c); } whole { r4 = ((a, b, c,)); } }
```

Actual output (formatted where Rust syntax permits):

```rust
::cq_example_providers::project_output! {
    resume
    {
        request
        {
            reply_to(:: mini_linq :: resume_physical)
            {
                plan
                {
                    struct ProjectedChain; relation L(a : Value); relation
                    R(a : Value, b : Value); relation S(b : Value, c : Value);
                    answer(a, c) : - L(a), R(a, b), S(b, c); relational
                    {
                        r0 = rename L { a - > a }; r1 = rename R
                        { a - > a, b - > b }; r2 = natural_join r0 with r1; r3 =
                        rename S { b - > b, c - > c }; r4 = natural_join r2 with r3;
                        r5 = project r4 keep { a, c }; output r5 as answer(a, c);
                    } indexes { i0 = @ r1 [0]; i1 = @ r3 [0]; } uses
                    { r2.right = i0; r4.right = i1; } physical
                    {
                        relations
                        {
                            L = :: cq_example_providers :: slice_rows; R = ::
                            cq_example_providers :: hash_relation; S = ::
                            cq_example_providers :: sorted_relation; r0 = ::
                            cq_example_providers :: slice_rows; r1 = ::
                            cq_example_providers :: hash_relation; r2 = ::
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
                            cq_example_providers :: rename; r2 = :: cq_example_providers
                            :: indexed_join; r3 = :: cq_example_providers :: rename; r4
                            = :: cq_example_providers :: indexed_join; r5 = ::
                            cq_example_providers :: project;
                        }
                    }
                } environment
                {
                    stored L at(__cq_store_L) operations :: cq_example_providers
                    :: slice_rows stored R at(__cq_store_R) operations ::
                    cq_example_providers :: hash_relation stored S
                    at(__cq_store_S) operations :: cq_example_providers ::
                    sorted_relation stored r0 at(__cq_store_r0) operations ::
                    cq_example_providers :: slice_rows stored r1
                    at(__cq_store_r1) operations :: cq_example_providers ::
                    hash_relation indexed i0 at(__cq_index_i0) operations ::
                    cq_example_providers :: hash_relation region r2
                    {
                        r2(a, b) => ((a, b,)) : - r0(a) => bind a <-
                        ((__cq_store_r0).iter()), r1(a, b) => bind b <-
                        ((__cq_index_i0).get(a).into_iter().flatten());
                    } stored r3 at(__cq_store_r3) operations ::
                    cq_example_providers :: sorted_relation indexed i1
                    at(__cq_index_i1) operations :: cq_example_providers ::
                    sorted_relation region r4
                    {
                        r4(a, b, c) => ((a, b, c,)) : - r0(a) => bind a <-
                        ((__cq_store_r0).iter()), r1(a, b) => bind b <-
                        ((__cq_index_i0).get(a).into_iter().flatten()), r3(b, c) =>
                        bind c <- ((__cq_index_i1).get(b).into_iter().flatten());
                    }
                } emitted
                {
                    let __cq_store_L : & [Value] = L; let __cq_store_R : & ::
                    std :: collections :: HashMap < Value, :: std :: vec :: Vec
                    < Value > > = R; let __cq_store_S : & :: std :: collections
                    :: BTreeMap < Value, :: std :: vec :: Vec < Value > > = S;
                    let __cq_store_r0 : & [Value] = __cq_store_L; let
                    __cq_store_r1 : & :: std :: collections :: HashMap < Value,
                    :: std :: vec :: Vec < Value > > = __cq_store_R; let
                    __cq_index_i0 : & :: std :: collections :: HashMap < Value,
                    :: std :: vec :: Vec < Value > > = __cq_store_r1; let
                    __cq_store_r3 : & :: std :: collections :: BTreeMap < Value,
                    :: std :: vec :: Vec < Value > > = __cq_store_S; let
                    __cq_index_i1 : & :: std :: collections :: BTreeMap < Value,
                    :: std :: vec :: Vec < Value > > = __cq_store_r3;
                } pending { lower r5 }
                continuations(:: mini_linq :: resume_physical, mini_linq ::
                access)
            } plan
            {
                struct ProjectedChain; relation L(a : Value); relation
                R(a : Value, b : Value); relation S(b : Value, c : Value);
                answer(a, c) : - L(a), R(a, b), S(b, c); relational
                {
                    r0 = rename L { a - > a }; r1 = rename R
                    { a - > a, b - > b }; r2 = natural_join r0 with r1; r3 =
                    rename S { b - > b, c - > c }; r4 = natural_join r2 with r3;
                    r5 = project r4 keep { a, c }; output r5 as answer(a, c);
                } indexes { i0 = @ r1 [0]; i1 = @ r3 [0]; } uses
                { r2.right = i0; r4.right = i1; } physical
                {
                    relations
                    {
                        L = :: cq_example_providers :: slice_rows; R = ::
                        cq_example_providers :: hash_relation; S = ::
                        cq_example_providers :: sorted_relation; r0 = ::
                        cq_example_providers :: slice_rows; r1 = ::
                        cq_example_providers :: hash_relation; r2 = ::
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
                        cq_example_providers :: rename; r2 = :: cq_example_providers
                        :: indexed_join; r3 = :: cq_example_providers :: rename; r4
                        = :: cq_example_providers :: indexed_join; r5 = ::
                        cq_example_providers :: project;
                    }
                }
            } environment
            {
                stored L at(__cq_store_L) operations :: cq_example_providers
                :: slice_rows stored R at(__cq_store_R) operations ::
                cq_example_providers :: hash_relation stored S
                at(__cq_store_S) operations :: cq_example_providers ::
                sorted_relation stored r0 at(__cq_store_r0) operations ::
                cq_example_providers :: slice_rows stored r1 at(__cq_store_r1)
                operations :: cq_example_providers :: hash_relation indexed i0
                at(__cq_index_i0) operations :: cq_example_providers ::
                hash_relation region r2
                {
                    r2(a, b) => ((a, b,)) : - r0(a) => bind a <-
                    ((__cq_store_r0).iter()), r1(a, b) => bind b <-
                    ((__cq_index_i0).get(a).into_iter().flatten());
                } stored r3 at(__cq_store_r3) operations ::
                cq_example_providers :: sorted_relation indexed i1
                at(__cq_index_i1) operations :: cq_example_providers ::
                sorted_relation region r4
                {
                    r4(a, b, c) => ((a, b, c,)) : - r0(a) => bind a <-
                    ((__cq_store_r0).iter()), r1(a, b) => bind b <-
                    ((__cq_index_i0).get(a).into_iter().flatten()), r3(b, c) =>
                    bind c <- ((__cq_index_i1).get(b).into_iter().flatten());
                }
            } request { lower r5 }
        } access
        {
            r0(a) => bind a <- ((__cq_store_r0).iter()), r1(a, b) => bind b <-
            ((__cq_index_i0).get(a).into_iter().flatten()), r3(b, c) => bind c
            <- ((__cq_index_i1).get(b).into_iter().flatten())
        } witness { row((a, b, c,)) fields { 0 = (a); 1 = (b); 2 = (c); } }
    } witness { row((a, c,)) fields { 0 = (a); 1 = (c); } }
}

```

---

## 2172002/040-cq-example-providers::project_output

[Raw input](raw/2172002/040-cq-example-providers::project_output.input.tokens) · [Raw output](raw/2172002/040-cq-example-providers::project_output.output.tokens)

Actual input:

```rust
resume
{
    request
    {
        reply_to(:: mini_linq :: resume_physical)
        {
            plan
            {
                struct ProjectedChain; relation L(a : Value); relation
                R(a : Value, b : Value); relation S(b : Value, c : Value);
                answer(a, c) : - L(a), R(a, b), S(b, c); relational
                {
                    r0 = rename L { a - > a }; r1 = rename R
                    { a - > a, b - > b }; r2 = natural_join r0 with r1; r3 =
                    rename S { b - > b, c - > c }; r4 = natural_join r2 with r3;
                    r5 = project r4 keep { a, c }; output r5 as answer(a, c);
                } indexes { i0 = @ r1 [0]; i1 = @ r3 [0]; } uses
                { r2.right = i0; r4.right = i1; } physical
                {
                    relations
                    {
                        L = :: cq_example_providers :: slice_rows; R = ::
                        cq_example_providers :: hash_relation; S = ::
                        cq_example_providers :: sorted_relation; r0 = ::
                        cq_example_providers :: slice_rows; r1 = ::
                        cq_example_providers :: hash_relation; r2 = ::
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
                        cq_example_providers :: rename; r2 = :: cq_example_providers
                        :: indexed_join; r3 = :: cq_example_providers :: rename; r4
                        = :: cq_example_providers :: indexed_join; r5 = ::
                        cq_example_providers :: project;
                    }
                }
            } environment
            {
                stored L at(__cq_store_L) operations :: cq_example_providers
                :: slice_rows stored R at(__cq_store_R) operations ::
                cq_example_providers :: hash_relation stored S
                at(__cq_store_S) operations :: cq_example_providers ::
                sorted_relation stored r0 at(__cq_store_r0) operations ::
                cq_example_providers :: slice_rows stored r1 at(__cq_store_r1)
                operations :: cq_example_providers :: hash_relation indexed i0
                at(__cq_index_i0) operations :: cq_example_providers ::
                hash_relation region r2
                {
                    r2(a, b) => ((a, b,)) : - r0(a) => bind a <-
                    ((__cq_store_r0).iter()), r1(a, b) => bind b <-
                    ((__cq_index_i0).get(a).into_iter().flatten());
                } stored r3 at(__cq_store_r3) operations ::
                cq_example_providers :: sorted_relation indexed i1
                at(__cq_index_i1) operations :: cq_example_providers ::
                sorted_relation region r4
                {
                    r4(a, b, c) => ((a, b, c,)) : - r0(a) => bind a <-
                    ((__cq_store_r0).iter()), r1(a, b) => bind b <-
                    ((__cq_index_i0).get(a).into_iter().flatten()), r3(b, c) =>
                    bind c <- ((__cq_index_i1).get(b).into_iter().flatten());
                }
            } emitted
            {
                let __cq_store_L : & [Value] = L; let __cq_store_R : & :: std
                :: collections :: HashMap < Value, :: std :: vec :: Vec <
                Value > > = R; let __cq_store_S : & :: std :: collections ::
                BTreeMap < Value, :: std :: vec :: Vec < Value > > = S; let
                __cq_store_r0 : & [Value] = __cq_store_L; let __cq_store_r1 :
                & :: std :: collections :: HashMap < Value, :: std :: vec ::
                Vec < Value > > = __cq_store_R; let __cq_index_i0 : & :: std
                :: collections :: HashMap < Value, :: std :: vec :: Vec <
                Value > > = __cq_store_r1; let __cq_store_r3 : & :: std ::
                collections :: BTreeMap < Value, :: std :: vec :: Vec < Value
                > > = __cq_store_S; let __cq_index_i1 : & :: std ::
                collections :: BTreeMap < Value, :: std :: vec :: Vec < Value
                > > = __cq_store_r3;
            } pending { lower r5 }
            continuations(:: mini_linq :: resume_physical, mini_linq ::
            access)
        } plan
        {
            struct ProjectedChain; relation L(a : Value); relation
            R(a : Value, b : Value); relation S(b : Value, c : Value);
            answer(a, c) : - L(a), R(a, b), S(b, c); relational
            {
                r0 = rename L { a - > a }; r1 = rename R { a - > a, b - > b };
                r2 = natural_join r0 with r1; r3 = rename S
                { b - > b, c - > c }; r4 = natural_join r2 with r3; r5 =
                project r4 keep { a, c }; output r5 as answer(a, c);
            } indexes { i0 = @ r1 [0]; i1 = @ r3 [0]; } uses
            { r2.right = i0; r4.right = i1; } physical
            {
                relations
                {
                    L = :: cq_example_providers :: slice_rows; R = ::
                    cq_example_providers :: hash_relation; S = ::
                    cq_example_providers :: sorted_relation; r0 = ::
                    cq_example_providers :: slice_rows; r1 = ::
                    cq_example_providers :: hash_relation; r2 = ::
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
                    cq_example_providers :: rename; r2 = :: cq_example_providers
                    :: indexed_join; r3 = :: cq_example_providers :: rename; r4
                    = :: cq_example_providers :: indexed_join; r5 = ::
                    cq_example_providers :: project;
                }
            }
        } environment
        {
            stored L at(__cq_store_L) operations :: cq_example_providers ::
            slice_rows stored R at(__cq_store_R) operations ::
            cq_example_providers :: hash_relation stored S at(__cq_store_S)
            operations :: cq_example_providers :: sorted_relation stored r0
            at(__cq_store_r0) operations :: cq_example_providers :: slice_rows
            stored r1 at(__cq_store_r1) operations :: cq_example_providers ::
            hash_relation indexed i0 at(__cq_index_i0) operations ::
            cq_example_providers :: hash_relation region r2
            {
                r2(a, b) => ((a, b,)) : - r0(a) => bind a <-
                ((__cq_store_r0).iter()), r1(a, b) => bind b <-
                ((__cq_index_i0).get(a).into_iter().flatten());
            } stored r3 at(__cq_store_r3) operations :: cq_example_providers
            :: sorted_relation indexed i1 at(__cq_index_i1) operations ::
            cq_example_providers :: sorted_relation region r4
            {
                r4(a, b, c) => ((a, b, c,)) : - r0(a) => bind a <-
                ((__cq_store_r0).iter()), r1(a, b) => bind b <-
                ((__cq_index_i0).get(a).into_iter().flatten()), r3(b, c) =>
                bind c <- ((__cq_index_i1).get(b).into_iter().flatten());
            }
        } request { lower r5 }
    } access
    {
        r0(a) => bind a <- ((__cq_store_r0).iter()), r1(a, b) => bind b <-
        ((__cq_index_i0).get(a).into_iter().flatten()), r3(b, c) => bind c <-
        ((__cq_index_i1).get(b).into_iter().flatten())
    } witness { row((a, b, c,)) fields { 0 = (a); 1 = (b); 2 = (c); } }
} witness { row((a, c,)) fields { 0 = (a); 1 = (c); } }
```

Actual output (formatted where Rust syntax permits):

```rust
::mini_linq::resume_physical! {
    resume
    {
        plan
        {
            struct ProjectedChain; relation L(a : Value); relation
            R(a : Value, b : Value); relation S(b : Value, c : Value);
            answer(a, c) : - L(a), R(a, b), S(b, c); relational
            {
                r0 = rename L { a - > a }; r1 = rename R { a - > a, b - > b };
                r2 = natural_join r0 with r1; r3 = rename S
                { b - > b, c - > c }; r4 = natural_join r2 with r3; r5 =
                project r4 keep { a, c }; output r5 as answer(a, c);
            } indexes { i0 = @ r1 [0]; i1 = @ r3 [0]; } uses
            { r2.right = i0; r4.right = i1; } physical
            {
                relations
                {
                    L = :: cq_example_providers :: slice_rows; R = ::
                    cq_example_providers :: hash_relation; S = ::
                    cq_example_providers :: sorted_relation; r0 = ::
                    cq_example_providers :: slice_rows; r1 = ::
                    cq_example_providers :: hash_relation; r2 = ::
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
                    cq_example_providers :: rename; r2 = :: cq_example_providers
                    :: indexed_join; r3 = :: cq_example_providers :: rename; r4
                    = :: cq_example_providers :: indexed_join; r5 = ::
                    cq_example_providers :: project;
                }
            }
        } environment
        {
            stored L at(__cq_store_L) operations :: cq_example_providers ::
            slice_rows stored R at(__cq_store_R) operations ::
            cq_example_providers :: hash_relation stored S at(__cq_store_S)
            operations :: cq_example_providers :: sorted_relation stored r0
            at(__cq_store_r0) operations :: cq_example_providers :: slice_rows
            stored r1 at(__cq_store_r1) operations :: cq_example_providers ::
            hash_relation indexed i0 at(__cq_index_i0) operations ::
            cq_example_providers :: hash_relation region r2
            {
                r2(a, b) => ((a, b,)) : - r0(a) => bind a <-
                ((__cq_store_r0).iter()), r1(a, b) => bind b <-
                ((__cq_index_i0).get(a).into_iter().flatten());
            } stored r3 at(__cq_store_r3) operations :: cq_example_providers
            :: sorted_relation indexed i1 at(__cq_index_i1) operations ::
            cq_example_providers :: sorted_relation region r4
            {
                r4(a, b, c) => ((a, b, c,)) : - r0(a) => bind a <-
                ((__cq_store_r0).iter()), r1(a, b) => bind b <-
                ((__cq_index_i0).get(a).into_iter().flatten()), r3(b, c) =>
                bind c <- ((__cq_index_i1).get(b).into_iter().flatten());
            }
        } emitted
        {
            let __cq_store_L : & [Value] = L; let __cq_store_R : & :: std ::
            collections :: HashMap < Value, :: std :: vec :: Vec < Value > > =
            R; let __cq_store_S : & :: std :: collections :: BTreeMap < Value,
            :: std :: vec :: Vec < Value > > = S; let __cq_store_r0 : &
            [Value] = __cq_store_L; let __cq_store_r1 : & :: std ::
            collections :: HashMap < Value, :: std :: vec :: Vec < Value > > =
            __cq_store_R; let __cq_index_i0 : & :: std :: collections ::
            HashMap < Value, :: std :: vec :: Vec < Value > > = __cq_store_r1;
            let __cq_store_r3 : & :: std :: collections :: BTreeMap < Value,
            :: std :: vec :: Vec < Value > > = __cq_store_S; let __cq_index_i1
            : & :: std :: collections :: BTreeMap < Value, :: std :: vec ::
            Vec < Value > > = __cq_store_r3;
        } pending { lower r5 }
        continuations(:: mini_linq :: resume_physical, mini_linq :: access)
    } emitted {} result
    {
        region r5
        {
            r5(a, c) => ((a, c,)) : - r0(a) => bind a <-
            ((__cq_store_r0).iter()), r1(a, b) => bind b <-
            ((__cq_index_i0).get(a).into_iter().flatten()), r3(b, c) => bind c
            <- ((__cq_index_i1).get(b).into_iter().flatten());
        }
    }
}

```

---

## 2172002/041-resume_physical

[Raw input](raw/2172002/041-resume_physical.input.tokens) · [Raw output](raw/2172002/041-resume_physical.output.tokens)

Actual input:

```rust
resume
{
    plan
    {
        struct ProjectedChain; relation L(a : Value); relation
        R(a : Value, b : Value); relation S(b : Value, c : Value);
        answer(a, c) : - L(a), R(a, b), S(b, c); relational
        {
            r0 = rename L { a - > a }; r1 = rename R { a - > a, b - > b }; r2
            = natural_join r0 with r1; r3 = rename S { b - > b, c - > c }; r4
            = natural_join r2 with r3; r5 = project r4 keep { a, c }; output
            r5 as answer(a, c);
        } indexes { i0 = @ r1 [0]; i1 = @ r3 [0]; } uses
        { r2.right = i0; r4.right = i1; } physical
        {
            relations
            {
                L = :: cq_example_providers :: slice_rows; R = ::
                cq_example_providers :: hash_relation; S = ::
                cq_example_providers :: sorted_relation; r0 = ::
                cq_example_providers :: slice_rows; r1 = ::
                cq_example_providers :: hash_relation; r2 = ::
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
                cq_example_providers :: rename; r2 = :: cq_example_providers
                :: indexed_join; r3 = :: cq_example_providers :: rename; r4 =
                :: cq_example_providers :: indexed_join; r5 = ::
                cq_example_providers :: project;
            }
        }
    } environment
    {
        stored L at(__cq_store_L) operations :: cq_example_providers ::
        slice_rows stored R at(__cq_store_R) operations ::
        cq_example_providers :: hash_relation stored S at(__cq_store_S)
        operations :: cq_example_providers :: sorted_relation stored r0
        at(__cq_store_r0) operations :: cq_example_providers :: slice_rows
        stored r1 at(__cq_store_r1) operations :: cq_example_providers ::
        hash_relation indexed i0 at(__cq_index_i0) operations ::
        cq_example_providers :: hash_relation region r2
        {
            r2(a, b) => ((a, b,)) : - r0(a) => bind a <-
            ((__cq_store_r0).iter()), r1(a, b) => bind b <-
            ((__cq_index_i0).get(a).into_iter().flatten());
        } stored r3 at(__cq_store_r3) operations :: cq_example_providers ::
        sorted_relation indexed i1 at(__cq_index_i1) operations ::
        cq_example_providers :: sorted_relation region r4
        {
            r4(a, b, c) => ((a, b, c,)) : - r0(a) => bind a <-
            ((__cq_store_r0).iter()), r1(a, b) => bind b <-
            ((__cq_index_i0).get(a).into_iter().flatten()), r3(b, c) => bind c
            <- ((__cq_index_i1).get(b).into_iter().flatten());
        }
    } emitted
    {
        let __cq_store_L : & [Value] = L; let __cq_store_R : & :: std ::
        collections :: HashMap < Value, :: std :: vec :: Vec < Value > > = R;
        let __cq_store_S : & :: std :: collections :: BTreeMap < Value, :: std
        :: vec :: Vec < Value > > = S; let __cq_store_r0 : & [Value] =
        __cq_store_L; let __cq_store_r1 : & :: std :: collections :: HashMap <
        Value, :: std :: vec :: Vec < Value > > = __cq_store_R; let
        __cq_index_i0 : & :: std :: collections :: HashMap < Value, :: std ::
        vec :: Vec < Value > > = __cq_store_r1; let __cq_store_r3 : & :: std
        :: collections :: BTreeMap < Value, :: std :: vec :: Vec < Value > > =
        __cq_store_S; let __cq_index_i1 : & :: std :: collections :: BTreeMap
        < Value, :: std :: vec :: Vec < Value > > = __cq_store_r3;
    } pending { lower r5 }
    continuations(:: mini_linq :: resume_physical, mini_linq :: access)
} emitted {} result
{
    region r5
    {
        r5(a, c) => ((a, c,)) : - r0(a) => bind a <- ((__cq_store_r0).iter()),
        r1(a, b) => bind b <- ((__cq_index_i0).get(a).into_iter().flatten()),
        r3(b, c) => bind c <- ((__cq_index_i1).get(b).into_iter().flatten());
    }
}
```

Actual output (formatted where Rust syntax permits):

```rust
{
    let __cq_store_L : & [Value] = L; let __cq_store_R : & :: std ::
    collections :: HashMap < Value, :: std :: vec :: Vec < Value > > = R; let
    __cq_store_S : & :: std :: collections :: BTreeMap < Value, :: std :: vec
    :: Vec < Value > > = S; let __cq_store_r0 : & [Value] = __cq_store_L; let
    __cq_store_r1 : & :: std :: collections :: HashMap < Value, :: std :: vec
    :: Vec < Value > > = __cq_store_R; let __cq_index_i0 : & :: std ::
    collections :: HashMap < Value, :: std :: vec :: Vec < Value > > =
    __cq_store_r1; let __cq_store_r3 : & :: std :: collections :: BTreeMap <
    Value, :: std :: vec :: Vec < Value > > = __cq_store_S; let __cq_index_i1
    : & :: std :: collections :: BTreeMap < Value, :: std :: vec :: Vec <
    Value > > = __cq_store_r3; mini_linq :: access!
    {
        answer(a, c) => ((a, c,)) : - r0(a) => bind a <-
        ((__cq_store_r0).iter()), r1(a, b) => bind b <-
        ((__cq_index_i0).get(a).into_iter().flatten()), r3(b, c) => bind c <-
        ((__cq_index_i1).get(b).into_iter().flatten());
    }
}
```

---

## 2172002/042-access_to_set_loops

[Raw input](raw/2172002/042-access_to_set_loops.input.tokens) · [Raw output](raw/2172002/042-access_to_set_loops.output.tokens)

Actual input:

```rust
answer(a, c) => ((a, c,)) : - r0(a) => bind a <- ((__cq_store_r0).iter()),
r1(a, b) => bind b <- ((__cq_index_i0).get(a).into_iter().flatten()), r3(b, c)
=> bind c <- ((__cq_index_i1).get(b).into_iter().flatten());
```

Actual output (formatted where Rust syntax permits):

```rust
{
    let mut __cq_output = :: std :: collections :: HashSet :: new(); for a in
    ((__cq_store_r0).iter())
    {
        for b in ((__cq_index_i0).get(a).into_iter().flatten())
        {
            let a = :: core :: clone :: Clone :: clone(& a); for c in
            ((__cq_index_i1).get(b).into_iter().flatten())
            {
                let a = :: core :: clone :: Clone :: clone(& a);
                __cq_output.insert((a, c,));
            }
        }
    } __cq_output
}
```

---

## Declarative macros: separate compiler trace

```text
   Compiling proc-macro2 v1.0.107
   Compiling quote v1.0.47
   Compiling syn v3.0.3
   Compiling syn v2.0.119
   Compiling syn_derive v0.2.0 (https://github.com/StarGazerM/syn_derive?rev=97bbe5871ac5ca8e9ba64cf50ba78d75e8cb2562#97bbe587)
   Compiling contracts v0.6.10
   Compiling cq-ir v0.1.0 (/tmp/mini-linq-bind-query-20260919/workspace/crates/cq-ir)
   Compiling cq-index-policy v0.1.0 (/tmp/mini-linq-bind-query-20260919/workspace/crates/cq-index-policy)
   Compiling cq-query-lowering v0.1.0 (/tmp/mini-linq-bind-query-20260919/workspace/crates/cq-query-lowering)
   Compiling cq-access-lowering v0.1.0 (/tmp/mini-linq-bind-query-20260919/workspace/crates/cq-access-lowering)
   Compiling cq-physical-selection v0.1.0 (/tmp/mini-linq-bind-query-20260919/workspace/crates/cq-physical-selection)
   Compiling cq-physical-lowering v0.1.0 (/tmp/mini-linq-bind-query-20260919/workspace/crates/cq-physical-lowering)
   Compiling cq-example-providers v0.1.0 (/tmp/mini-linq-bind-query-20260919/workspace/crates/cq-example-providers)
   Compiling mini-linq v0.1.0 (/tmp/mini-linq-bind-query-20260919/workspace/crates/mini-linq)
note: trace_macro
  --> crates/mini-linq/tests/support/trace.rs:14:5
   |
14 | /     assert!(
15 | |         trace.join("verification.txt").is_file(),
16 | |         "trace must finish its source-identity verification"
17 | |     );
   | |_____^
   |
   = note: expanding `panic_2021! { "trace must finish its source-identity verification" }`
   = note: to `{
               $crate :: panicking ::
               panic_fmt($crate :: const_format_args!
               ("trace must finish its source-identity verification"));
           }`

note: trace_macro
  --> crates/mini-linq/tests/support/trace.rs:33:9
   |
33 |         assert_eq!(matches.len(), 1, "expected one captured {suffix}");
   |         ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^
   |
   = note: expanding `assert_eq! { matches.len(), 1, "expected one captured {suffix}" }`
   = note: to `match (& matches.len(), & 1)
           {
               (left_val, right_val) =>
               {
                   if ! (* left_val == * right_val)
                   {
                       let kind = $crate :: panicking :: AssertKind :: Eq; $crate ::
                       panicking ::
                       assert_failed(kind, & * left_val, & * right_val, $crate :: option
                       :: Option ::
                       Some($crate :: format_args! ("expected one captured {suffix}")));
                   }
               }
           }`

note: trace_macro
  --> crates/mini-linq/tests/support/trace.rs:41:9
   |
41 |         assert_eq!(actual, expected);
   |         ^^^^^^^^^^^^^^^^^^^^^^^^^^^^
   |
   = note: expanding `assert_eq! { actual, expected }`
   = note: to `match (& actual, & expected)
           {
               (left_val, right_val) =>
               {
                   if ! (* left_val == * right_val)
                   {
                       let kind = $crate :: panicking :: AssertKind :: Eq; $crate ::
                       panicking ::
                       assert_failed(kind, & * left_val, & * right_val, $crate :: option
                       :: Option :: None);
                   }
               }
           }`

note: trace_macro
  --> crates/mini-linq/tests/support/trace.rs:48:5
   |
48 |     assert_eq!(indexed, expected_ix);
   |     ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^
   |
   = note: expanding `assert_eq! { indexed, expected_ix }`
   = note: to `match (& indexed, & expected_ix)
           {
               (left_val, right_val) =>
               {
                   if ! (* left_val == * right_val)
                   {
                       let kind = $crate :: panicking :: AssertKind :: Eq; $crate ::
                       panicking ::
                       assert_failed(kind, & * left_val, & * right_val, $crate :: option
                       :: Option :: None);
                   }
               }
           }`

note: trace_macro
  --> crates/mini-linq/tests/support/trace.rs:52:61
   |
52 |       let expected: cq_ir::physical::Module = syn::parse_str(&format!(
   |  _____________________________________________________________^
53 | |         "{}\n{}",
54 | |         expected_ix_source, expected_selection_source
55 | |     ))
   | |_____^
   |
   = note: expanding `format! { "{}\n{}", expected_ix_source, expected_selection_source }`
   = note: to `$crate :: __export ::
           must_use({
               $crate :: fmt ::
               format($crate :: __export :: format_args!
               ("{}\n{}", expected_ix_source, expected_selection_source))
           })`

note: trace_macro
  --> crates/mini-linq/tests/support/trace.rs:57:5
   |
57 | /     assert_eq!(
58 | |         selected.to_token_stream().to_string(),
59 | |         expected.to_token_stream().to_string()
60 | |     );
   | |_____^
   |
   = note: expanding `assert_eq! { selected.to_token_stream().to_string(), expected.to_token_stream().to_string() }`
   = note: to `match
           (& selected.to_token_stream().to_string(), &
           expected.to_token_stream().to_string())
           {
               (left_val, right_val) =>
               {
                   if ! (* left_val == * right_val)
                   {
                       let kind = $crate :: panicking :: AssertKind :: Eq; $crate ::
                       panicking ::
                       assert_failed(kind, & * left_val, & * right_val, $crate :: option
                       :: Option :: None);
                   }
               }
           }`

note: trace_macro
  --> crates/mini-linq/tests/support/trace.rs:75:13
   |
75 | /             assert_eq!(
76 | |                 request.plan.to_token_stream().to_string(),
77 | |                 expected.to_token_stream().to_string()
78 | |             );
   | |_____________^
   |
   = note: expanding `assert_eq! { request.plan.to_token_stream().to_string(),
           expected.to_token_stream().to_string() }`
   = note: to `match
           (& request.plan.to_token_stream().to_string(), &
           expected.to_token_stream().to_string())
           {
               (left_val, right_val) =>
               {
                   if ! (* left_val == * right_val)
                   {
                       let kind = $crate :: panicking :: AssertKind :: Eq; $crate ::
                       panicking ::
                       assert_failed(kind, & * left_val, & * right_val, $crate :: option
                       :: Option :: None);
                   }
               }
           }`

note: trace_macro
  --> crates/mini-linq/tests/support/trace.rs:91:13
   |
91 |             matches!(expression, syn::Expr::Block(_)).then_some(expression)
   |             ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^
   |
   = note: expanding `matches! { expression, syn::Expr::Block(_) }`
   = note: to `#[allow(non_exhaustive_omitted_patterns)] match expression
           { syn::Expr::Block(_) => true, _ => false }`

note: trace_macro
  --> crates/mini-linq/tests/support/trace.rs:94:5
   |
94 |     assert_eq!(realized.len(), 1, "one completed Physical realization");
   |     ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^
   |
   = note: expanding `assert_eq! { realized.len(), 1, "one completed Physical realization" }`
   = note: to `match (& realized.len(), & 1)
           {
               (left_val, right_val) =>
               {
                   if ! (* left_val == * right_val)
                   {
                       let kind = $crate :: panicking :: AssertKind :: Eq; $crate ::
                       panicking ::
                       assert_failed(kind, & * left_val, & * right_val, $crate :: option
                       :: Option ::
                       Some($crate :: format_args!
                       ("one completed Physical realization")));
                   }
               }
           }`

note: trace_macro
   --> crates/mini-linq/tests/support/trace.rs:101:13
    |
101 |             panic!("expected host block")
    |             ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^
    |
    = note: expanding `panic_2021! { "expected host block" }`
    = note: to `{
                $crate :: panicking ::
                panic_fmt($crate :: const_format_args! ("expected host block"));
            }`

note: trace_macro
   --> crates/mini-linq/tests/support/trace.rs:106:18
    |
106 |             _ => panic!("expected final Access quotation"),
    |                  ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^
    |
    = note: expanding `panic_2021! { "expected final Access quotation" }`
    = note: to `{
                $crate :: panicking ::
                panic_fmt($crate :: const_format_args!
                ("expected final Access quotation"));
            }`

note: trace_macro
   --> crates/mini-linq/tests/support/trace.rs:112:5
    |
112 |     assert_eq!(normalize(realized[0].clone()), normalize(expected));
    |     ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^
    |
    = note: expanding `assert_eq! { normalize(realized[0].clone()), normalize(expected) }`
    = note: to `match (& normalize(realized[0].clone()), & normalize(expected))
            {
                (left_val, right_val) =>
                {
                    if ! (* left_val == * right_val)
                    {
                        let kind = $crate :: panicking :: AssertKind :: Eq; $crate ::
                        panicking ::
                        assert_failed(kind, & * left_val, & * right_val, $crate :: option
                        :: Option :: None);
                    }
                }
            }`

note: trace_macro
   --> crates/mini-linq/tests/support/trace.rs:133:5
    |
133 |     assert_eq!(found.len(), 1, "one Access-to-Pipeline expansion");
    |     ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^
    |
    = note: expanding `assert_eq! { found.len(), 1, "one Access-to-Pipeline expansion" }`
    = note: to `match (& found.len(), & 1)
            {
                (left_val, right_val) =>
                {
                    if ! (* left_val == * right_val)
                    {
                        let kind = $crate :: panicking :: AssertKind :: Eq; $crate ::
                        panicking ::
                        assert_failed(kind, & * left_val, & * right_val, $crate :: option
                        :: Option ::
                        Some($crate :: format_args!
                        ("one Access-to-Pipeline expansion")));
                    }
                }
            }`

note: trace_macro
   --> crates/mini-linq/tests/support/trace.rs:139:5
    |
139 |     assert_eq!(actual, expected);
    |     ^^^^^^^^^^^^^^^^^^^^^^^^^^^^
    |
    = note: expanding `assert_eq! { actual, expected }`
    = note: to `match (& actual, & expected)
            {
                (left_val, right_val) =>
                {
                    if ! (* left_val == * right_val)
                    {
                        let kind = $crate :: panicking :: AssertKind :: Eq; $crate ::
                        panicking ::
                        assert_failed(kind, & * left_val, & * right_val, $crate :: option
                        :: Option :: None);
                    }
                }
            }`

note: trace_macro
  --> crates/mini-linq/tests/physical_query.rs:59:5
   |
59 | /     ::mini_linq::cq_to_ra_written_order! {
60 | |         struct ProjectedChain;
61 | |         relation L(a: Value);
62 | |         relation R(a: Value, b: Value);
63 | |         relation S(b: Value, c: Value);
64 | |         answer(a, c) :- L(a), R(a, b), S(b, c);
65 | |     }
   | |_____^
   |
   = note: expanding `explicit_policy! { struct ProjectedChain; relation L(a : Value); relation
           R(a : Value, b : Value); relation S(b : Value, c : Value); answer(a, c) : -
           L(a), R(a, b), S(b, c); relational
           {
               r0 = rename L { a - > a }; r1 = rename R { a - > a, b - > b }; r2 =
               natural_join r0 with r1; r3 = rename S { b - > b, c - > c }; r4 =
               natural_join r2 with r3; r5 = project r4 keep { a, c }; output r5 as
               answer(a, c);
           } indexes { i0 = @ r1 [0]; i1 = @ r3 [0]; } uses
           { r2.right = i0; r4.right = i1; } }`
   = note: to `:: mini_linq :: given_physical_bindings!
           {
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
           }`
   = note: this note originates in the macro `::mini_linq::cq_to_ra_written_order` (in Nightly builds, run with -Z macro-backtrace for more info)

note: trace_macro
  --> crates/mini-linq/tests/physical_query.rs:94:20
   |
94 |         (Value(1), vec![Value(10), Value(10), Value(11)]),
   |                    ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^
   |
   = note: expanding `vec! { Value(10), Value(10), Value(11) }`
   = note: to `$crate :: boxed ::
           box_assume_init_into_vec_unsafe($crate :: intrinsics ::
           write_box_via_move($crate :: boxed :: Box :: new_uninit(),
           [Value(10), Value(10), Value(11)]))`

note: trace_macro
  --> crates/mini-linq/tests/physical_query.rs:95:20
   |
95 |         (Value(2), vec![Value(20)]),
   |                    ^^^^^^^^^^^^^^^
   |
   = note: expanding `vec! { Value(20) }`
   = note: to `$crate :: boxed ::
           box_assume_init_into_vec_unsafe($crate :: intrinsics ::
           write_box_via_move($crate :: boxed :: Box :: new_uninit(), [Value(20)]))`

note: trace_macro
  --> crates/mini-linq/tests/physical_query.rs:96:20
   |
96 |         (Value(3), vec![Value(30)]),
   |                    ^^^^^^^^^^^^^^^
   |
   = note: expanding `vec! { Value(30) }`
   = note: to `$crate :: boxed ::
           box_assume_init_into_vec_unsafe($crate :: intrinsics ::
           write_box_via_move($crate :: boxed :: Box :: new_uninit(), [Value(30)]))`

note: trace_macro
  --> crates/mini-linq/tests/physical_query.rs:99:21
   |
99 |         (Value(10), vec![Value(100), Value(101), Value(101)]),
   |                     ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^
   |
   = note: expanding `vec! { Value(100), Value(101), Value(101) }`
   = note: to `$crate :: boxed ::
           box_assume_init_into_vec_unsafe($crate :: intrinsics ::
           write_box_via_move($crate :: boxed :: Box :: new_uninit(),
           [Value(100), Value(101), Value(101)]))`

note: trace_macro
   --> crates/mini-linq/tests/physical_query.rs:101:21
    |
101 |         (Value(11), vec![Value(100)]),
    |                     ^^^^^^^^^^^^^^^^
    |
    = note: expanding `vec! { Value(100) }`
    = note: to `$crate :: boxed ::
            box_assume_init_into_vec_unsafe($crate :: intrinsics ::
            write_box_via_move($crate :: boxed :: Box :: new_uninit(), [Value(100)]))`

note: trace_macro
   --> crates/mini-linq/tests/physical_query.rs:102:21
    |
102 |         (Value(20), vec![Value(200)]),
    |                     ^^^^^^^^^^^^^^^^
    |
    = note: expanding `vec! { Value(200) }`
    = note: to `$crate :: boxed ::
            box_assume_init_into_vec_unsafe($crate :: intrinsics ::
            write_box_via_move($crate :: boxed :: Box :: new_uninit(), [Value(200)]))`

note: trace_macro
   --> crates/mini-linq/tests/physical_query.rs:103:21
    |
103 |         (Value(99), vec![Value(999)]),
    |                     ^^^^^^^^^^^^^^^^
    |
    = note: expanding `vec! { Value(999) }`
    = note: to `$crate :: boxed ::
            box_assume_init_into_vec_unsafe($crate :: intrinsics ::
            write_box_via_move($crate :: boxed :: Box :: new_uninit(), [Value(999)]))`

note: trace_macro
   --> crates/mini-linq/tests/physical_query.rs:106:5
    |
106 |     assert_eq!(result, oracle(&left, &middle, &right));
    |     ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^
    |
    = note: expanding `assert_eq! { result, oracle(&left, &middle, &right) }`
    = note: to `match (& result, & oracle(&left, &middle, &right))
            {
                (left_val, right_val) =>
                {
                    if ! (* left_val == * right_val)
                    {
                        let kind = $crate :: panicking :: AssertKind :: Eq; $crate ::
                        panicking ::
                        assert_failed(kind, & * left_val, & * right_val, $crate :: option
                        :: Option :: None);
                    }
                }
            }`

note: trace_macro
   --> crates/mini-linq/tests/physical_query.rs:107:5
    |
107 |     assert_eq!(result.len(), 3);
    |     ^^^^^^^^^^^^^^^^^^^^^^^^^^^
    |
    = note: expanding `assert_eq! { result.len(), 3 }`
    = note: to `match (& result.len(), & 3)
            {
                (left_val, right_val) =>
                {
                    if ! (* left_val == * right_val)
                    {
                        let kind = $crate :: panicking :: AssertKind :: Eq; $crate ::
                        panicking ::
                        assert_failed(kind, & * left_val, & * right_val, $crate :: option
                        :: Option :: None);
                    }
                }
            }`

note: trace_macro
   --> crates/mini-linq/tests/physical_query.rs:141:9
    |
141 |         assert_eq!(run(&left, &middle, &right), oracle(&left, &middle, &right));
    |         ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^
    |
    = note: expanding `assert_eq! { run(&left, &middle, &right), oracle(&left, &middle, &right) }`
    = note: to `match (& run(&left, &middle, &right), & oracle(&left, &middle, &right))
            {
                (left_val, right_val) =>
                {
                    if ! (* left_val == * right_val)
                    {
                        let kind = $crate :: panicking :: AssertKind :: Eq; $crate ::
                        panicking ::
                        assert_failed(kind, & * left_val, & * right_val, $crate :: option
                        :: Option :: None);
                    }
                }
            }`

note: trace_macro
   --> crates/mini-linq/tests/physical_query.rs:150:10
    |
150 |           &format!(
    |  __________^
151 | |             "{}\n{}",
152 | |             include_str!("fixtures/physical-query/expected.ra"),
153 | |             include_str!("fixtures/physical-query/expected.indexes"),
154 | |         ),
    | |_________^
    |
    = note: expanding `format! { "{}\n{}", include_str!("fixtures/physical-query/expected.ra"),
            include_str!("fixtures/physical-query/expected.indexes"), }`
    = note: to `$crate :: __export ::
            must_use({
                $crate :: fmt ::
                format($crate :: __export :: format_args!
                ("{}\n{}", include_str!("fixtures/physical-query/expected.ra"),
                include_str!("fixtures/physical-query/expected.indexes"),))
            })`

    Finished `dev` profile [unoptimized + debuginfo] target(s) in 5.71s

```
