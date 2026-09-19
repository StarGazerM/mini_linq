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


## 2172042/001-index_demands

[Raw input](raw/2172042/001-index_demands.input.tokens) · [Raw output](raw/2172042/001-index_demands.output.tokens)

Actual input:

```rust
struct Heterogeneous; relation L(a: Value); relation R(a: Value, b: Value);
relation S(b: Value, c: Value); answer(a, b, c) :- L(a), R(a, b), S(b, c);
relational
{
    l = rename L {a -> a}; r = rename R {a -> a, b -> b}; s = rename S
    {b -> b, c -> c}; j0 = natural_join l with r; j1 = natural_join j0 with s;
    output j1 as answer(a, b, c);
}
```

Actual output (formatted where Rust syntax permits):

```rust
mini_linq::choose_physical! {
    struct Heterogeneous; relation L(a : Value); relation
    R(a : Value, b : Value); relation S(b : Value, c : Value); answer(a, b, c)
    : - L(a), R(a, b), S(b, c); relational
    {
        l = rename L { a - > a }; r = rename R { a - > a, b - > b }; s =
        rename S { b - > b, c - > c }; j0 = natural_join l with r; j1 =
        natural_join j0 with s; output j1 as answer(a, b, c);
    } indexes { i0 = @ r [0]; i1 = @ s [0]; } uses
    { j0.right = i0; j1.right = i1; }
}

```

---

## 2172042/002-given_physical_bindings

[Raw input](raw/2172042/002-given_physical_bindings.input.tokens) · [Raw output](raw/2172042/002-given_physical_bindings.output.tokens)

Actual input:

```rust
struct Heterogeneous; relation L(a : Value); relation R(a : Value, b : Value);
relation S(b : Value, c : Value); answer(a, b, c) : - L(a), R(a, b), S(b, c);
relational
{
    l = rename L { a - > a }; r = rename R { a - > a, b - > b }; s = rename S
    { b - > b, c - > c }; j0 = natural_join l with r; j1 = natural_join j0
    with s; output j1 as answer(a, b, c);
} indexes { i0 = @ r [0]; i1 = @ s [0]; } uses
{ j0.right = i0; j1.right = i1; } physical
{
    relations
    {
        L = :: cq_example_providers :: slice_rows; R = :: cq_example_providers
        :: hash_relation; S = :: cq_example_providers :: sorted_relation; l =
        :: cq_example_providers :: slice_rows; r = :: cq_example_providers ::
        hash_relation; s = :: cq_example_providers :: sorted_relation; j0 = ::
        cq_example_providers :: borrowed_fields; j1 = :: cq_example_providers
        :: borrowed_fields;
    } indexes
    {
        i0 = :: cq_example_providers :: hash_relation; i1 = ::
        cq_example_providers :: sorted_relation;
    } lowerings
    {
        l = :: cq_example_providers :: rename; r = :: cq_example_providers ::
        rename; s = :: cq_example_providers :: rename; j0 = ::
        cq_example_providers :: indexed_join; j1 = :: cq_example_providers ::
        indexed_join;
    }
}
```

Actual output (formatted where Rust syntax permits):

```rust
mini_linq::realize_physical! {
    struct Heterogeneous; relation L(a : Value); relation
    R(a : Value, b : Value); relation S(b : Value, c : Value); answer(a, b, c)
    : - L(a), R(a, b), S(b, c); relational
    {
        l = rename L { a - > a }; r = rename R { a - > a, b - > b }; s =
        rename S { b - > b, c - > c }; j0 = natural_join l with r; j1 =
        natural_join j0 with s; output j1 as answer(a, b, c);
    } indexes { i0 = @ r [0]; i1 = @ s [0]; } uses
    { j0.right = i0; j1.right = i1; } physical
    {
        relations
        {
            L = :: cq_example_providers :: slice_rows; R = ::
            cq_example_providers :: hash_relation; S = :: cq_example_providers
            :: sorted_relation; l = :: cq_example_providers :: slice_rows; r =
            :: cq_example_providers :: hash_relation; s = ::
            cq_example_providers :: sorted_relation; j0 = ::
            cq_example_providers :: borrowed_fields; j1 = ::
            cq_example_providers :: borrowed_fields;
        } indexes
        {
            i0 = :: cq_example_providers :: hash_relation; i1 = ::
            cq_example_providers :: sorted_relation;
        } lowerings
        {
            l = :: cq_example_providers :: rename; r = :: cq_example_providers
            :: rename; s = :: cq_example_providers :: rename; j0 = ::
            cq_example_providers :: indexed_join; j1 = :: cq_example_providers
            :: indexed_join;
        }
    }
}

```

---

## 2172042/003-realize_physical

[Raw input](raw/2172042/003-realize_physical.input.tokens) · [Raw output](raw/2172042/003-realize_physical.output.tokens)

Actual input:

```rust
struct Heterogeneous; relation L(a : Value); relation R(a : Value, b : Value);
relation S(b : Value, c : Value); answer(a, b, c) : - L(a), R(a, b), S(b, c);
relational
{
    l = rename L { a - > a }; r = rename R { a - > a, b - > b }; s = rename S
    { b - > b, c - > c }; j0 = natural_join l with r; j1 = natural_join j0
    with s; output j1 as answer(a, b, c);
} indexes { i0 = @ r [0]; i1 = @ s [0]; } uses
{ j0.right = i0; j1.right = i1; } physical
{
    relations
    {
        L = :: cq_example_providers :: slice_rows; R = :: cq_example_providers
        :: hash_relation; S = :: cq_example_providers :: sorted_relation; l =
        :: cq_example_providers :: slice_rows; r = :: cq_example_providers ::
        hash_relation; s = :: cq_example_providers :: sorted_relation; j0 = ::
        cq_example_providers :: borrowed_fields; j1 = :: cq_example_providers
        :: borrowed_fields;
    } indexes
    {
        i0 = :: cq_example_providers :: hash_relation; i1 = ::
        cq_example_providers :: sorted_relation;
    } lowerings
    {
        l = :: cq_example_providers :: rename; r = :: cq_example_providers ::
        rename; s = :: cq_example_providers :: rename; j0 = ::
        cq_example_providers :: indexed_join; j1 = :: cq_example_providers ::
        indexed_join;
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
            struct Heterogeneous; relation L(a : Value); relation
            R(a : Value, b : Value); relation S(b : Value, c : Value);
            answer(a, b, c) : - L(a), R(a, b), S(b, c); relational
            {
                l = rename L { a - > a }; r = rename R { a - > a, b - > b }; s
                = rename S { b - > b, c - > c }; j0 = natural_join l with r;
                j1 = natural_join j0 with s; output j1 as answer(a, b, c);
            } indexes { i0 = @ r [0]; i1 = @ s [0]; } uses
            { j0.right = i0; j1.right = i1; } physical
            {
                relations
                {
                    L = :: cq_example_providers :: slice_rows; R = ::
                    cq_example_providers :: hash_relation; S = ::
                    cq_example_providers :: sorted_relation; l = ::
                    cq_example_providers :: slice_rows; r = ::
                    cq_example_providers :: hash_relation; s = ::
                    cq_example_providers :: sorted_relation; j0 = ::
                    cq_example_providers :: borrowed_fields; j1 = ::
                    cq_example_providers :: borrowed_fields;
                } indexes
                {
                    i0 = :: cq_example_providers :: hash_relation; i1 = ::
                    cq_example_providers :: sorted_relation;
                } lowerings
                {
                    l = :: cq_example_providers :: rename; r = ::
                    cq_example_providers :: rename; s = :: cq_example_providers
                    :: rename; j0 = :: cq_example_providers :: indexed_join; j1
                    = :: cq_example_providers :: indexed_join;
                }
            }
        } environment {} emitted {} pending { input L from(L) }
        continuations(:: mini_linq :: resume_physical, mini_linq :: access)
    } plan
    {
        struct Heterogeneous; relation L(a : Value); relation
        R(a : Value, b : Value); relation S(b : Value, c : Value);
        answer(a, b, c) : - L(a), R(a, b), S(b, c); relational
        {
            l = rename L { a - > a }; r = rename R { a - > a, b - > b }; s =
            rename S { b - > b, c - > c }; j0 = natural_join l with r; j1 =
            natural_join j0 with s; output j1 as answer(a, b, c);
        } indexes { i0 = @ r [0]; i1 = @ s [0]; } uses
        { j0.right = i0; j1.right = i1; } physical
        {
            relations
            {
                L = :: cq_example_providers :: slice_rows; R = ::
                cq_example_providers :: hash_relation; S = ::
                cq_example_providers :: sorted_relation; l = ::
                cq_example_providers :: slice_rows; r = ::
                cq_example_providers :: hash_relation; s = ::
                cq_example_providers :: sorted_relation; j0 = ::
                cq_example_providers :: borrowed_fields; j1 = ::
                cq_example_providers :: borrowed_fields;
            } indexes
            {
                i0 = :: cq_example_providers :: hash_relation; i1 = ::
                cq_example_providers :: sorted_relation;
            } lowerings
            {
                l = :: cq_example_providers :: rename; r = ::
                cq_example_providers :: rename; s = :: cq_example_providers ::
                rename; j0 = :: cq_example_providers :: indexed_join; j1 = ::
                cq_example_providers :: indexed_join;
            }
        }
    } environment {} request { input L from(L) }
}

```

---

## 2172042/004-cq-example-providers::slice_rows

[Raw input](raw/2172042/004-cq-example-providers::slice_rows.input.tokens) · [Raw output](raw/2172042/004-cq-example-providers::slice_rows.output.tokens)

Actual input:

```rust
reply_to(:: mini_linq :: resume_physical)
{
    plan
    {
        struct Heterogeneous; relation L(a : Value); relation
        R(a : Value, b : Value); relation S(b : Value, c : Value);
        answer(a, b, c) : - L(a), R(a, b), S(b, c); relational
        {
            l = rename L { a - > a }; r = rename R { a - > a, b - > b }; s =
            rename S { b - > b, c - > c }; j0 = natural_join l with r; j1 =
            natural_join j0 with s; output j1 as answer(a, b, c);
        } indexes { i0 = @ r [0]; i1 = @ s [0]; } uses
        { j0.right = i0; j1.right = i1; } physical
        {
            relations
            {
                L = :: cq_example_providers :: slice_rows; R = ::
                cq_example_providers :: hash_relation; S = ::
                cq_example_providers :: sorted_relation; l = ::
                cq_example_providers :: slice_rows; r = ::
                cq_example_providers :: hash_relation; s = ::
                cq_example_providers :: sorted_relation; j0 = ::
                cq_example_providers :: borrowed_fields; j1 = ::
                cq_example_providers :: borrowed_fields;
            } indexes
            {
                i0 = :: cq_example_providers :: hash_relation; i1 = ::
                cq_example_providers :: sorted_relation;
            } lowerings
            {
                l = :: cq_example_providers :: rename; r = ::
                cq_example_providers :: rename; s = :: cq_example_providers ::
                rename; j0 = :: cq_example_providers :: indexed_join; j1 = ::
                cq_example_providers :: indexed_join;
            }
        }
    } environment {} emitted {} pending { input L from(L) }
    continuations(:: mini_linq :: resume_physical, mini_linq :: access)
} plan
{
    struct Heterogeneous; relation L(a : Value); relation
    R(a : Value, b : Value); relation S(b : Value, c : Value); answer(a, b, c)
    : - L(a), R(a, b), S(b, c); relational
    {
        l = rename L { a - > a }; r = rename R { a - > a, b - > b }; s =
        rename S { b - > b, c - > c }; j0 = natural_join l with r; j1 =
        natural_join j0 with s; output j1 as answer(a, b, c);
    } indexes { i0 = @ r [0]; i1 = @ s [0]; } uses
    { j0.right = i0; j1.right = i1; } physical
    {
        relations
        {
            L = :: cq_example_providers :: slice_rows; R = ::
            cq_example_providers :: hash_relation; S = :: cq_example_providers
            :: sorted_relation; l = :: cq_example_providers :: slice_rows; r =
            :: cq_example_providers :: hash_relation; s = ::
            cq_example_providers :: sorted_relation; j0 = ::
            cq_example_providers :: borrowed_fields; j1 = ::
            cq_example_providers :: borrowed_fields;
        } indexes
        {
            i0 = :: cq_example_providers :: hash_relation; i1 = ::
            cq_example_providers :: sorted_relation;
        } lowerings
        {
            l = :: cq_example_providers :: rename; r = :: cq_example_providers
            :: rename; s = :: cq_example_providers :: rename; j0 = ::
            cq_example_providers :: indexed_join; j1 = :: cq_example_providers
            :: indexed_join;
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
            struct Heterogeneous; relation L(a : Value); relation
            R(a : Value, b : Value); relation S(b : Value, c : Value);
            answer(a, b, c) : - L(a), R(a, b), S(b, c); relational
            {
                l = rename L { a - > a }; r = rename R { a - > a, b - > b }; s
                = rename S { b - > b, c - > c }; j0 = natural_join l with r;
                j1 = natural_join j0 with s; output j1 as answer(a, b, c);
            } indexes { i0 = @ r [0]; i1 = @ s [0]; } uses
            { j0.right = i0; j1.right = i1; } physical
            {
                relations
                {
                    L = :: cq_example_providers :: slice_rows; R = ::
                    cq_example_providers :: hash_relation; S = ::
                    cq_example_providers :: sorted_relation; l = ::
                    cq_example_providers :: slice_rows; r = ::
                    cq_example_providers :: hash_relation; s = ::
                    cq_example_providers :: sorted_relation; j0 = ::
                    cq_example_providers :: borrowed_fields; j1 = ::
                    cq_example_providers :: borrowed_fields;
                } indexes
                {
                    i0 = :: cq_example_providers :: hash_relation; i1 = ::
                    cq_example_providers :: sorted_relation;
                } lowerings
                {
                    l = :: cq_example_providers :: rename; r = ::
                    cq_example_providers :: rename; s = :: cq_example_providers
                    :: rename; j0 = :: cq_example_providers :: indexed_join; j1
                    = :: cq_example_providers :: indexed_join;
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

## 2172042/005-resume_physical

[Raw input](raw/2172042/005-resume_physical.input.tokens) · [Raw output](raw/2172042/005-resume_physical.output.tokens)

Actual input:

```rust
resume
{
    plan
    {
        struct Heterogeneous; relation L(a : Value); relation
        R(a : Value, b : Value); relation S(b : Value, c : Value);
        answer(a, b, c) : - L(a), R(a, b), S(b, c); relational
        {
            l = rename L { a - > a }; r = rename R { a - > a, b - > b }; s =
            rename S { b - > b, c - > c }; j0 = natural_join l with r; j1 =
            natural_join j0 with s; output j1 as answer(a, b, c);
        } indexes { i0 = @ r [0]; i1 = @ s [0]; } uses
        { j0.right = i0; j1.right = i1; } physical
        {
            relations
            {
                L = :: cq_example_providers :: slice_rows; R = ::
                cq_example_providers :: hash_relation; S = ::
                cq_example_providers :: sorted_relation; l = ::
                cq_example_providers :: slice_rows; r = ::
                cq_example_providers :: hash_relation; s = ::
                cq_example_providers :: sorted_relation; j0 = ::
                cq_example_providers :: borrowed_fields; j1 = ::
                cq_example_providers :: borrowed_fields;
            } indexes
            {
                i0 = :: cq_example_providers :: hash_relation; i1 = ::
                cq_example_providers :: sorted_relation;
            } lowerings
            {
                l = :: cq_example_providers :: rename; r = ::
                cq_example_providers :: rename; s = :: cq_example_providers ::
                rename; j0 = :: cq_example_providers :: indexed_join; j1 = ::
                cq_example_providers :: indexed_join;
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
            struct Heterogeneous; relation L(a : Value); relation
            R(a : Value, b : Value); relation S(b : Value, c : Value);
            answer(a, b, c) : - L(a), R(a, b), S(b, c); relational
            {
                l = rename L { a - > a }; r = rename R { a - > a, b - > b }; s
                = rename S { b - > b, c - > c }; j0 = natural_join l with r;
                j1 = natural_join j0 with s; output j1 as answer(a, b, c);
            } indexes { i0 = @ r [0]; i1 = @ s [0]; } uses
            { j0.right = i0; j1.right = i1; } physical
            {
                relations
                {
                    L = :: cq_example_providers :: slice_rows; R = ::
                    cq_example_providers :: hash_relation; S = ::
                    cq_example_providers :: sorted_relation; l = ::
                    cq_example_providers :: slice_rows; r = ::
                    cq_example_providers :: hash_relation; s = ::
                    cq_example_providers :: sorted_relation; j0 = ::
                    cq_example_providers :: borrowed_fields; j1 = ::
                    cq_example_providers :: borrowed_fields;
                } indexes
                {
                    i0 = :: cq_example_providers :: hash_relation; i1 = ::
                    cq_example_providers :: sorted_relation;
                } lowerings
                {
                    l = :: cq_example_providers :: rename; r = ::
                    cq_example_providers :: rename; s = :: cq_example_providers
                    :: rename; j0 = :: cq_example_providers :: indexed_join; j1
                    = :: cq_example_providers :: indexed_join;
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
        struct Heterogeneous; relation L(a : Value); relation
        R(a : Value, b : Value); relation S(b : Value, c : Value);
        answer(a, b, c) : - L(a), R(a, b), S(b, c); relational
        {
            l = rename L { a - > a }; r = rename R { a - > a, b - > b }; s =
            rename S { b - > b, c - > c }; j0 = natural_join l with r; j1 =
            natural_join j0 with s; output j1 as answer(a, b, c);
        } indexes { i0 = @ r [0]; i1 = @ s [0]; } uses
        { j0.right = i0; j1.right = i1; } physical
        {
            relations
            {
                L = :: cq_example_providers :: slice_rows; R = ::
                cq_example_providers :: hash_relation; S = ::
                cq_example_providers :: sorted_relation; l = ::
                cq_example_providers :: slice_rows; r = ::
                cq_example_providers :: hash_relation; s = ::
                cq_example_providers :: sorted_relation; j0 = ::
                cq_example_providers :: borrowed_fields; j1 = ::
                cq_example_providers :: borrowed_fields;
            } indexes
            {
                i0 = :: cq_example_providers :: hash_relation; i1 = ::
                cq_example_providers :: sorted_relation;
            } lowerings
            {
                l = :: cq_example_providers :: rename; r = ::
                cq_example_providers :: rename; s = :: cq_example_providers ::
                rename; j0 = :: cq_example_providers :: indexed_join; j1 = ::
                cq_example_providers :: indexed_join;
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

## 2172042/006-cq-example-providers::hash_relation

[Raw input](raw/2172042/006-cq-example-providers::hash_relation.input.tokens) · [Raw output](raw/2172042/006-cq-example-providers::hash_relation.output.tokens)

Actual input:

```rust
reply_to(:: mini_linq :: resume_physical)
{
    plan
    {
        struct Heterogeneous; relation L(a : Value); relation
        R(a : Value, b : Value); relation S(b : Value, c : Value);
        answer(a, b, c) : - L(a), R(a, b), S(b, c); relational
        {
            l = rename L { a - > a }; r = rename R { a - > a, b - > b }; s =
            rename S { b - > b, c - > c }; j0 = natural_join l with r; j1 =
            natural_join j0 with s; output j1 as answer(a, b, c);
        } indexes { i0 = @ r [0]; i1 = @ s [0]; } uses
        { j0.right = i0; j1.right = i1; } physical
        {
            relations
            {
                L = :: cq_example_providers :: slice_rows; R = ::
                cq_example_providers :: hash_relation; S = ::
                cq_example_providers :: sorted_relation; l = ::
                cq_example_providers :: slice_rows; r = ::
                cq_example_providers :: hash_relation; s = ::
                cq_example_providers :: sorted_relation; j0 = ::
                cq_example_providers :: borrowed_fields; j1 = ::
                cq_example_providers :: borrowed_fields;
            } indexes
            {
                i0 = :: cq_example_providers :: hash_relation; i1 = ::
                cq_example_providers :: sorted_relation;
            } lowerings
            {
                l = :: cq_example_providers :: rename; r = ::
                cq_example_providers :: rename; s = :: cq_example_providers ::
                rename; j0 = :: cq_example_providers :: indexed_join; j1 = ::
                cq_example_providers :: indexed_join;
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
    struct Heterogeneous; relation L(a : Value); relation
    R(a : Value, b : Value); relation S(b : Value, c : Value); answer(a, b, c)
    : - L(a), R(a, b), S(b, c); relational
    {
        l = rename L { a - > a }; r = rename R { a - > a, b - > b }; s =
        rename S { b - > b, c - > c }; j0 = natural_join l with r; j1 =
        natural_join j0 with s; output j1 as answer(a, b, c);
    } indexes { i0 = @ r [0]; i1 = @ s [0]; } uses
    { j0.right = i0; j1.right = i1; } physical
    {
        relations
        {
            L = :: cq_example_providers :: slice_rows; R = ::
            cq_example_providers :: hash_relation; S = :: cq_example_providers
            :: sorted_relation; l = :: cq_example_providers :: slice_rows; r =
            :: cq_example_providers :: hash_relation; s = ::
            cq_example_providers :: sorted_relation; j0 = ::
            cq_example_providers :: borrowed_fields; j1 = ::
            cq_example_providers :: borrowed_fields;
        } indexes
        {
            i0 = :: cq_example_providers :: hash_relation; i1 = ::
            cq_example_providers :: sorted_relation;
        } lowerings
        {
            l = :: cq_example_providers :: rename; r = :: cq_example_providers
            :: rename; s = :: cq_example_providers :: rename; j0 = ::
            cq_example_providers :: indexed_join; j1 = :: cq_example_providers
            :: indexed_join;
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
            struct Heterogeneous; relation L(a : Value); relation
            R(a : Value, b : Value); relation S(b : Value, c : Value);
            answer(a, b, c) : - L(a), R(a, b), S(b, c); relational
            {
                l = rename L { a - > a }; r = rename R { a - > a, b - > b }; s
                = rename S { b - > b, c - > c }; j0 = natural_join l with r;
                j1 = natural_join j0 with s; output j1 as answer(a, b, c);
            } indexes { i0 = @ r [0]; i1 = @ s [0]; } uses
            { j0.right = i0; j1.right = i1; } physical
            {
                relations
                {
                    L = :: cq_example_providers :: slice_rows; R = ::
                    cq_example_providers :: hash_relation; S = ::
                    cq_example_providers :: sorted_relation; l = ::
                    cq_example_providers :: slice_rows; r = ::
                    cq_example_providers :: hash_relation; s = ::
                    cq_example_providers :: sorted_relation; j0 = ::
                    cq_example_providers :: borrowed_fields; j1 = ::
                    cq_example_providers :: borrowed_fields;
                } indexes
                {
                    i0 = :: cq_example_providers :: hash_relation; i1 = ::
                    cq_example_providers :: sorted_relation;
                } lowerings
                {
                    l = :: cq_example_providers :: rename; r = ::
                    cq_example_providers :: rename; s = :: cq_example_providers
                    :: rename; j0 = :: cq_example_providers :: indexed_join; j1
                    = :: cq_example_providers :: indexed_join;
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

## 2172042/007-resume_physical

[Raw input](raw/2172042/007-resume_physical.input.tokens) · [Raw output](raw/2172042/007-resume_physical.output.tokens)

Actual input:

```rust
resume
{
    plan
    {
        struct Heterogeneous; relation L(a : Value); relation
        R(a : Value, b : Value); relation S(b : Value, c : Value);
        answer(a, b, c) : - L(a), R(a, b), S(b, c); relational
        {
            l = rename L { a - > a }; r = rename R { a - > a, b - > b }; s =
            rename S { b - > b, c - > c }; j0 = natural_join l with r; j1 =
            natural_join j0 with s; output j1 as answer(a, b, c);
        } indexes { i0 = @ r [0]; i1 = @ s [0]; } uses
        { j0.right = i0; j1.right = i1; } physical
        {
            relations
            {
                L = :: cq_example_providers :: slice_rows; R = ::
                cq_example_providers :: hash_relation; S = ::
                cq_example_providers :: sorted_relation; l = ::
                cq_example_providers :: slice_rows; r = ::
                cq_example_providers :: hash_relation; s = ::
                cq_example_providers :: sorted_relation; j0 = ::
                cq_example_providers :: borrowed_fields; j1 = ::
                cq_example_providers :: borrowed_fields;
            } indexes
            {
                i0 = :: cq_example_providers :: hash_relation; i1 = ::
                cq_example_providers :: sorted_relation;
            } lowerings
            {
                l = :: cq_example_providers :: rename; r = ::
                cq_example_providers :: rename; s = :: cq_example_providers ::
                rename; j0 = :: cq_example_providers :: indexed_join; j1 = ::
                cq_example_providers :: indexed_join;
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
            struct Heterogeneous; relation L(a : Value); relation
            R(a : Value, b : Value); relation S(b : Value, c : Value);
            answer(a, b, c) : - L(a), R(a, b), S(b, c); relational
            {
                l = rename L { a - > a }; r = rename R { a - > a, b - > b }; s
                = rename S { b - > b, c - > c }; j0 = natural_join l with r;
                j1 = natural_join j0 with s; output j1 as answer(a, b, c);
            } indexes { i0 = @ r [0]; i1 = @ s [0]; } uses
            { j0.right = i0; j1.right = i1; } physical
            {
                relations
                {
                    L = :: cq_example_providers :: slice_rows; R = ::
                    cq_example_providers :: hash_relation; S = ::
                    cq_example_providers :: sorted_relation; l = ::
                    cq_example_providers :: slice_rows; r = ::
                    cq_example_providers :: hash_relation; s = ::
                    cq_example_providers :: sorted_relation; j0 = ::
                    cq_example_providers :: borrowed_fields; j1 = ::
                    cq_example_providers :: borrowed_fields;
                } indexes
                {
                    i0 = :: cq_example_providers :: hash_relation; i1 = ::
                    cq_example_providers :: sorted_relation;
                } lowerings
                {
                    l = :: cq_example_providers :: rename; r = ::
                    cq_example_providers :: rename; s = :: cq_example_providers
                    :: rename; j0 = :: cq_example_providers :: indexed_join; j1
                    = :: cq_example_providers :: indexed_join;
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
        struct Heterogeneous; relation L(a : Value); relation
        R(a : Value, b : Value); relation S(b : Value, c : Value);
        answer(a, b, c) : - L(a), R(a, b), S(b, c); relational
        {
            l = rename L { a - > a }; r = rename R { a - > a, b - > b }; s =
            rename S { b - > b, c - > c }; j0 = natural_join l with r; j1 =
            natural_join j0 with s; output j1 as answer(a, b, c);
        } indexes { i0 = @ r [0]; i1 = @ s [0]; } uses
        { j0.right = i0; j1.right = i1; } physical
        {
            relations
            {
                L = :: cq_example_providers :: slice_rows; R = ::
                cq_example_providers :: hash_relation; S = ::
                cq_example_providers :: sorted_relation; l = ::
                cq_example_providers :: slice_rows; r = ::
                cq_example_providers :: hash_relation; s = ::
                cq_example_providers :: sorted_relation; j0 = ::
                cq_example_providers :: borrowed_fields; j1 = ::
                cq_example_providers :: borrowed_fields;
            } indexes
            {
                i0 = :: cq_example_providers :: hash_relation; i1 = ::
                cq_example_providers :: sorted_relation;
            } lowerings
            {
                l = :: cq_example_providers :: rename; r = ::
                cq_example_providers :: rename; s = :: cq_example_providers ::
                rename; j0 = :: cq_example_providers :: indexed_join; j1 = ::
                cq_example_providers :: indexed_join;
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

## 2172042/008-cq-example-providers::sorted_relation

[Raw input](raw/2172042/008-cq-example-providers::sorted_relation.input.tokens) · [Raw output](raw/2172042/008-cq-example-providers::sorted_relation.output.tokens)

Actual input:

```rust
reply_to(:: mini_linq :: resume_physical)
{
    plan
    {
        struct Heterogeneous; relation L(a : Value); relation
        R(a : Value, b : Value); relation S(b : Value, c : Value);
        answer(a, b, c) : - L(a), R(a, b), S(b, c); relational
        {
            l = rename L { a - > a }; r = rename R { a - > a, b - > b }; s =
            rename S { b - > b, c - > c }; j0 = natural_join l with r; j1 =
            natural_join j0 with s; output j1 as answer(a, b, c);
        } indexes { i0 = @ r [0]; i1 = @ s [0]; } uses
        { j0.right = i0; j1.right = i1; } physical
        {
            relations
            {
                L = :: cq_example_providers :: slice_rows; R = ::
                cq_example_providers :: hash_relation; S = ::
                cq_example_providers :: sorted_relation; l = ::
                cq_example_providers :: slice_rows; r = ::
                cq_example_providers :: hash_relation; s = ::
                cq_example_providers :: sorted_relation; j0 = ::
                cq_example_providers :: borrowed_fields; j1 = ::
                cq_example_providers :: borrowed_fields;
            } indexes
            {
                i0 = :: cq_example_providers :: hash_relation; i1 = ::
                cq_example_providers :: sorted_relation;
            } lowerings
            {
                l = :: cq_example_providers :: rename; r = ::
                cq_example_providers :: rename; s = :: cq_example_providers ::
                rename; j0 = :: cq_example_providers :: indexed_join; j1 = ::
                cq_example_providers :: indexed_join;
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
    struct Heterogeneous; relation L(a : Value); relation
    R(a : Value, b : Value); relation S(b : Value, c : Value); answer(a, b, c)
    : - L(a), R(a, b), S(b, c); relational
    {
        l = rename L { a - > a }; r = rename R { a - > a, b - > b }; s =
        rename S { b - > b, c - > c }; j0 = natural_join l with r; j1 =
        natural_join j0 with s; output j1 as answer(a, b, c);
    } indexes { i0 = @ r [0]; i1 = @ s [0]; } uses
    { j0.right = i0; j1.right = i1; } physical
    {
        relations
        {
            L = :: cq_example_providers :: slice_rows; R = ::
            cq_example_providers :: hash_relation; S = :: cq_example_providers
            :: sorted_relation; l = :: cq_example_providers :: slice_rows; r =
            :: cq_example_providers :: hash_relation; s = ::
            cq_example_providers :: sorted_relation; j0 = ::
            cq_example_providers :: borrowed_fields; j1 = ::
            cq_example_providers :: borrowed_fields;
        } indexes
        {
            i0 = :: cq_example_providers :: hash_relation; i1 = ::
            cq_example_providers :: sorted_relation;
        } lowerings
        {
            l = :: cq_example_providers :: rename; r = :: cq_example_providers
            :: rename; s = :: cq_example_providers :: rename; j0 = ::
            cq_example_providers :: indexed_join; j1 = :: cq_example_providers
            :: indexed_join;
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
            struct Heterogeneous; relation L(a : Value); relation
            R(a : Value, b : Value); relation S(b : Value, c : Value);
            answer(a, b, c) : - L(a), R(a, b), S(b, c); relational
            {
                l = rename L { a - > a }; r = rename R { a - > a, b - > b }; s
                = rename S { b - > b, c - > c }; j0 = natural_join l with r;
                j1 = natural_join j0 with s; output j1 as answer(a, b, c);
            } indexes { i0 = @ r [0]; i1 = @ s [0]; } uses
            { j0.right = i0; j1.right = i1; } physical
            {
                relations
                {
                    L = :: cq_example_providers :: slice_rows; R = ::
                    cq_example_providers :: hash_relation; S = ::
                    cq_example_providers :: sorted_relation; l = ::
                    cq_example_providers :: slice_rows; r = ::
                    cq_example_providers :: hash_relation; s = ::
                    cq_example_providers :: sorted_relation; j0 = ::
                    cq_example_providers :: borrowed_fields; j1 = ::
                    cq_example_providers :: borrowed_fields;
                } indexes
                {
                    i0 = :: cq_example_providers :: hash_relation; i1 = ::
                    cq_example_providers :: sorted_relation;
                } lowerings
                {
                    l = :: cq_example_providers :: rename; r = ::
                    cq_example_providers :: rename; s = :: cq_example_providers
                    :: rename; j0 = :: cq_example_providers :: indexed_join; j1
                    = :: cq_example_providers :: indexed_join;
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

## 2172042/009-resume_physical

[Raw input](raw/2172042/009-resume_physical.input.tokens) · [Raw output](raw/2172042/009-resume_physical.output.tokens)

Actual input:

```rust
resume
{
    plan
    {
        struct Heterogeneous; relation L(a : Value); relation
        R(a : Value, b : Value); relation S(b : Value, c : Value);
        answer(a, b, c) : - L(a), R(a, b), S(b, c); relational
        {
            l = rename L { a - > a }; r = rename R { a - > a, b - > b }; s =
            rename S { b - > b, c - > c }; j0 = natural_join l with r; j1 =
            natural_join j0 with s; output j1 as answer(a, b, c);
        } indexes { i0 = @ r [0]; i1 = @ s [0]; } uses
        { j0.right = i0; j1.right = i1; } physical
        {
            relations
            {
                L = :: cq_example_providers :: slice_rows; R = ::
                cq_example_providers :: hash_relation; S = ::
                cq_example_providers :: sorted_relation; l = ::
                cq_example_providers :: slice_rows; r = ::
                cq_example_providers :: hash_relation; s = ::
                cq_example_providers :: sorted_relation; j0 = ::
                cq_example_providers :: borrowed_fields; j1 = ::
                cq_example_providers :: borrowed_fields;
            } indexes
            {
                i0 = :: cq_example_providers :: hash_relation; i1 = ::
                cq_example_providers :: sorted_relation;
            } lowerings
            {
                l = :: cq_example_providers :: rename; r = ::
                cq_example_providers :: rename; s = :: cq_example_providers ::
                rename; j0 = :: cq_example_providers :: indexed_join; j1 = ::
                cq_example_providers :: indexed_join;
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
            struct Heterogeneous; relation L(a : Value); relation
            R(a : Value, b : Value); relation S(b : Value, c : Value);
            answer(a, b, c) : - L(a), R(a, b), S(b, c); relational
            {
                l = rename L { a - > a }; r = rename R { a - > a, b - > b }; s
                = rename S { b - > b, c - > c }; j0 = natural_join l with r;
                j1 = natural_join j0 with s; output j1 as answer(a, b, c);
            } indexes { i0 = @ r [0]; i1 = @ s [0]; } uses
            { j0.right = i0; j1.right = i1; } physical
            {
                relations
                {
                    L = :: cq_example_providers :: slice_rows; R = ::
                    cq_example_providers :: hash_relation; S = ::
                    cq_example_providers :: sorted_relation; l = ::
                    cq_example_providers :: slice_rows; r = ::
                    cq_example_providers :: hash_relation; s = ::
                    cq_example_providers :: sorted_relation; j0 = ::
                    cq_example_providers :: borrowed_fields; j1 = ::
                    cq_example_providers :: borrowed_fields;
                } indexes
                {
                    i0 = :: cq_example_providers :: hash_relation; i1 = ::
                    cq_example_providers :: sorted_relation;
                } lowerings
                {
                    l = :: cq_example_providers :: rename; r = ::
                    cq_example_providers :: rename; s = :: cq_example_providers
                    :: rename; j0 = :: cq_example_providers :: indexed_join; j1
                    = :: cq_example_providers :: indexed_join;
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
        } pending { lower l }
        continuations(:: mini_linq :: resume_physical, mini_linq :: access)
    } plan
    {
        struct Heterogeneous; relation L(a : Value); relation
        R(a : Value, b : Value); relation S(b : Value, c : Value);
        answer(a, b, c) : - L(a), R(a, b), S(b, c); relational
        {
            l = rename L { a - > a }; r = rename R { a - > a, b - > b }; s =
            rename S { b - > b, c - > c }; j0 = natural_join l with r; j1 =
            natural_join j0 with s; output j1 as answer(a, b, c);
        } indexes { i0 = @ r [0]; i1 = @ s [0]; } uses
        { j0.right = i0; j1.right = i1; } physical
        {
            relations
            {
                L = :: cq_example_providers :: slice_rows; R = ::
                cq_example_providers :: hash_relation; S = ::
                cq_example_providers :: sorted_relation; l = ::
                cq_example_providers :: slice_rows; r = ::
                cq_example_providers :: hash_relation; s = ::
                cq_example_providers :: sorted_relation; j0 = ::
                cq_example_providers :: borrowed_fields; j1 = ::
                cq_example_providers :: borrowed_fields;
            } indexes
            {
                i0 = :: cq_example_providers :: hash_relation; i1 = ::
                cq_example_providers :: sorted_relation;
            } lowerings
            {
                l = :: cq_example_providers :: rename; r = ::
                cq_example_providers :: rename; s = :: cq_example_providers ::
                rename; j0 = :: cq_example_providers :: indexed_join; j1 = ::
                cq_example_providers :: indexed_join;
            }
        }
    } environment
    {
        stored L at(__cq_store_L) operations :: cq_example_providers ::
        slice_rows stored R at(__cq_store_R) operations ::
        cq_example_providers :: hash_relation stored S at(__cq_store_S)
        operations :: cq_example_providers :: sorted_relation
    } request { lower l }
}

```

---

## 2172042/010-cq-example-providers::rename

[Raw input](raw/2172042/010-cq-example-providers::rename.input.tokens) · [Raw output](raw/2172042/010-cq-example-providers::rename.output.tokens)

Actual input:

```rust
reply_to(:: mini_linq :: resume_physical)
{
    plan
    {
        struct Heterogeneous; relation L(a : Value); relation
        R(a : Value, b : Value); relation S(b : Value, c : Value);
        answer(a, b, c) : - L(a), R(a, b), S(b, c); relational
        {
            l = rename L { a - > a }; r = rename R { a - > a, b - > b }; s =
            rename S { b - > b, c - > c }; j0 = natural_join l with r; j1 =
            natural_join j0 with s; output j1 as answer(a, b, c);
        } indexes { i0 = @ r [0]; i1 = @ s [0]; } uses
        { j0.right = i0; j1.right = i1; } physical
        {
            relations
            {
                L = :: cq_example_providers :: slice_rows; R = ::
                cq_example_providers :: hash_relation; S = ::
                cq_example_providers :: sorted_relation; l = ::
                cq_example_providers :: slice_rows; r = ::
                cq_example_providers :: hash_relation; s = ::
                cq_example_providers :: sorted_relation; j0 = ::
                cq_example_providers :: borrowed_fields; j1 = ::
                cq_example_providers :: borrowed_fields;
            } indexes
            {
                i0 = :: cq_example_providers :: hash_relation; i1 = ::
                cq_example_providers :: sorted_relation;
            } lowerings
            {
                l = :: cq_example_providers :: rename; r = ::
                cq_example_providers :: rename; s = :: cq_example_providers ::
                rename; j0 = :: cq_example_providers :: indexed_join; j1 = ::
                cq_example_providers :: indexed_join;
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
    } pending { lower l }
    continuations(:: mini_linq :: resume_physical, mini_linq :: access)
} plan
{
    struct Heterogeneous; relation L(a : Value); relation
    R(a : Value, b : Value); relation S(b : Value, c : Value); answer(a, b, c)
    : - L(a), R(a, b), S(b, c); relational
    {
        l = rename L { a - > a }; r = rename R { a - > a, b - > b }; s =
        rename S { b - > b, c - > c }; j0 = natural_join l with r; j1 =
        natural_join j0 with s; output j1 as answer(a, b, c);
    } indexes { i0 = @ r [0]; i1 = @ s [0]; } uses
    { j0.right = i0; j1.right = i1; } physical
    {
        relations
        {
            L = :: cq_example_providers :: slice_rows; R = ::
            cq_example_providers :: hash_relation; S = :: cq_example_providers
            :: sorted_relation; l = :: cq_example_providers :: slice_rows; r =
            :: cq_example_providers :: hash_relation; s = ::
            cq_example_providers :: sorted_relation; j0 = ::
            cq_example_providers :: borrowed_fields; j1 = ::
            cq_example_providers :: borrowed_fields;
        } indexes
        {
            i0 = :: cq_example_providers :: hash_relation; i1 = ::
            cq_example_providers :: sorted_relation;
        } lowerings
        {
            l = :: cq_example_providers :: rename; r = :: cq_example_providers
            :: rename; s = :: cq_example_providers :: rename; j0 = ::
            cq_example_providers :: indexed_join; j1 = :: cq_example_providers
            :: indexed_join;
        }
    }
} environment
{
    stored L at(__cq_store_L) operations :: cq_example_providers :: slice_rows
    stored R at(__cq_store_R) operations :: cq_example_providers ::
    hash_relation stored S at(__cq_store_S) operations :: cq_example_providers
    :: sorted_relation
} request { lower l }
```

Actual output (formatted where Rust syntax permits):

```rust
::cq_example_providers::slice_rows! {
    reply_to(:: mini_linq :: resume_physical)
    {
        plan
        {
            struct Heterogeneous; relation L(a : Value); relation
            R(a : Value, b : Value); relation S(b : Value, c : Value);
            answer(a, b, c) : - L(a), R(a, b), S(b, c); relational
            {
                l = rename L { a - > a }; r = rename R { a - > a, b - > b }; s
                = rename S { b - > b, c - > c }; j0 = natural_join l with r;
                j1 = natural_join j0 with s; output j1 as answer(a, b, c);
            } indexes { i0 = @ r [0]; i1 = @ s [0]; } uses
            { j0.right = i0; j1.right = i1; } physical
            {
                relations
                {
                    L = :: cq_example_providers :: slice_rows; R = ::
                    cq_example_providers :: hash_relation; S = ::
                    cq_example_providers :: sorted_relation; l = ::
                    cq_example_providers :: slice_rows; r = ::
                    cq_example_providers :: hash_relation; s = ::
                    cq_example_providers :: sorted_relation; j0 = ::
                    cq_example_providers :: borrowed_fields; j1 = ::
                    cq_example_providers :: borrowed_fields;
                } indexes
                {
                    i0 = :: cq_example_providers :: hash_relation; i1 = ::
                    cq_example_providers :: sorted_relation;
                } lowerings
                {
                    l = :: cq_example_providers :: rename; r = ::
                    cq_example_providers :: rename; s = :: cq_example_providers
                    :: rename; j0 = :: cq_example_providers :: indexed_join; j1
                    = :: cq_example_providers :: indexed_join;
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
        } pending { lower l }
        continuations(:: mini_linq :: resume_physical, mini_linq :: access)
    } plan
    {
        struct Heterogeneous; relation L(a : Value); relation
        R(a : Value, b : Value); relation S(b : Value, c : Value);
        answer(a, b, c) : - L(a), R(a, b), S(b, c); relational
        {
            l = rename L { a - > a }; r = rename R { a - > a, b - > b }; s =
            rename S { b - > b, c - > c }; j0 = natural_join l with r; j1 =
            natural_join j0 with s; output j1 as answer(a, b, c);
        } indexes { i0 = @ r [0]; i1 = @ s [0]; } uses
        { j0.right = i0; j1.right = i1; } physical
        {
            relations
            {
                L = :: cq_example_providers :: slice_rows; R = ::
                cq_example_providers :: hash_relation; S = ::
                cq_example_providers :: sorted_relation; l = ::
                cq_example_providers :: slice_rows; r = ::
                cq_example_providers :: hash_relation; s = ::
                cq_example_providers :: sorted_relation; j0 = ::
                cq_example_providers :: borrowed_fields; j1 = ::
                cq_example_providers :: borrowed_fields;
            } indexes
            {
                i0 = :: cq_example_providers :: hash_relation; i1 = ::
                cq_example_providers :: sorted_relation;
            } lowerings
            {
                l = :: cq_example_providers :: rename; r = ::
                cq_example_providers :: rename; s = :: cq_example_providers ::
                rename; j0 = :: cq_example_providers :: indexed_join; j1 = ::
                cq_example_providers :: indexed_join;
            }
        }
    } environment
    {
        stored L at(__cq_store_L) operations :: cq_example_providers ::
        slice_rows stored R at(__cq_store_R) operations ::
        cq_example_providers :: hash_relation stored S at(__cq_store_S)
        operations :: cq_example_providers :: sorted_relation
    } request { input l from(__cq_store_L) }
}

```

---

## 2172042/011-cq-example-providers::slice_rows

[Raw input](raw/2172042/011-cq-example-providers::slice_rows.input.tokens) · [Raw output](raw/2172042/011-cq-example-providers::slice_rows.output.tokens)

Actual input:

```rust
reply_to(:: mini_linq :: resume_physical)
{
    plan
    {
        struct Heterogeneous; relation L(a : Value); relation
        R(a : Value, b : Value); relation S(b : Value, c : Value);
        answer(a, b, c) : - L(a), R(a, b), S(b, c); relational
        {
            l = rename L { a - > a }; r = rename R { a - > a, b - > b }; s =
            rename S { b - > b, c - > c }; j0 = natural_join l with r; j1 =
            natural_join j0 with s; output j1 as answer(a, b, c);
        } indexes { i0 = @ r [0]; i1 = @ s [0]; } uses
        { j0.right = i0; j1.right = i1; } physical
        {
            relations
            {
                L = :: cq_example_providers :: slice_rows; R = ::
                cq_example_providers :: hash_relation; S = ::
                cq_example_providers :: sorted_relation; l = ::
                cq_example_providers :: slice_rows; r = ::
                cq_example_providers :: hash_relation; s = ::
                cq_example_providers :: sorted_relation; j0 = ::
                cq_example_providers :: borrowed_fields; j1 = ::
                cq_example_providers :: borrowed_fields;
            } indexes
            {
                i0 = :: cq_example_providers :: hash_relation; i1 = ::
                cq_example_providers :: sorted_relation;
            } lowerings
            {
                l = :: cq_example_providers :: rename; r = ::
                cq_example_providers :: rename; s = :: cq_example_providers ::
                rename; j0 = :: cq_example_providers :: indexed_join; j1 = ::
                cq_example_providers :: indexed_join;
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
    } pending { lower l }
    continuations(:: mini_linq :: resume_physical, mini_linq :: access)
} plan
{
    struct Heterogeneous; relation L(a : Value); relation
    R(a : Value, b : Value); relation S(b : Value, c : Value); answer(a, b, c)
    : - L(a), R(a, b), S(b, c); relational
    {
        l = rename L { a - > a }; r = rename R { a - > a, b - > b }; s =
        rename S { b - > b, c - > c }; j0 = natural_join l with r; j1 =
        natural_join j0 with s; output j1 as answer(a, b, c);
    } indexes { i0 = @ r [0]; i1 = @ s [0]; } uses
    { j0.right = i0; j1.right = i1; } physical
    {
        relations
        {
            L = :: cq_example_providers :: slice_rows; R = ::
            cq_example_providers :: hash_relation; S = :: cq_example_providers
            :: sorted_relation; l = :: cq_example_providers :: slice_rows; r =
            :: cq_example_providers :: hash_relation; s = ::
            cq_example_providers :: sorted_relation; j0 = ::
            cq_example_providers :: borrowed_fields; j1 = ::
            cq_example_providers :: borrowed_fields;
        } indexes
        {
            i0 = :: cq_example_providers :: hash_relation; i1 = ::
            cq_example_providers :: sorted_relation;
        } lowerings
        {
            l = :: cq_example_providers :: rename; r = :: cq_example_providers
            :: rename; s = :: cq_example_providers :: rename; j0 = ::
            cq_example_providers :: indexed_join; j1 = :: cq_example_providers
            :: indexed_join;
        }
    }
} environment
{
    stored L at(__cq_store_L) operations :: cq_example_providers :: slice_rows
    stored R at(__cq_store_R) operations :: cq_example_providers ::
    hash_relation stored S at(__cq_store_S) operations :: cq_example_providers
    :: sorted_relation
} request { input l from(__cq_store_L) }
```

Actual output (formatted where Rust syntax permits):

```rust
::mini_linq::resume_physical! {
    resume
    {
        plan
        {
            struct Heterogeneous; relation L(a : Value); relation
            R(a : Value, b : Value); relation S(b : Value, c : Value);
            answer(a, b, c) : - L(a), R(a, b), S(b, c); relational
            {
                l = rename L { a - > a }; r = rename R { a - > a, b - > b }; s
                = rename S { b - > b, c - > c }; j0 = natural_join l with r;
                j1 = natural_join j0 with s; output j1 as answer(a, b, c);
            } indexes { i0 = @ r [0]; i1 = @ s [0]; } uses
            { j0.right = i0; j1.right = i1; } physical
            {
                relations
                {
                    L = :: cq_example_providers :: slice_rows; R = ::
                    cq_example_providers :: hash_relation; S = ::
                    cq_example_providers :: sorted_relation; l = ::
                    cq_example_providers :: slice_rows; r = ::
                    cq_example_providers :: hash_relation; s = ::
                    cq_example_providers :: sorted_relation; j0 = ::
                    cq_example_providers :: borrowed_fields; j1 = ::
                    cq_example_providers :: borrowed_fields;
                } indexes
                {
                    i0 = :: cq_example_providers :: hash_relation; i1 = ::
                    cq_example_providers :: sorted_relation;
                } lowerings
                {
                    l = :: cq_example_providers :: rename; r = ::
                    cq_example_providers :: rename; s = :: cq_example_providers
                    :: rename; j0 = :: cq_example_providers :: indexed_join; j1
                    = :: cq_example_providers :: indexed_join;
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
        } pending { lower l }
        continuations(:: mini_linq :: resume_physical, mini_linq :: access)
    } emitted { let __cq_store_l : & [Value] = __cq_store_L; } result
    {
        stored l at(__cq_store_l) operations :: cq_example_providers ::
        slice_rows
    }
}

```

---

## 2172042/012-resume_physical

[Raw input](raw/2172042/012-resume_physical.input.tokens) · [Raw output](raw/2172042/012-resume_physical.output.tokens)

Actual input:

```rust
resume
{
    plan
    {
        struct Heterogeneous; relation L(a : Value); relation
        R(a : Value, b : Value); relation S(b : Value, c : Value);
        answer(a, b, c) : - L(a), R(a, b), S(b, c); relational
        {
            l = rename L { a - > a }; r = rename R { a - > a, b - > b }; s =
            rename S { b - > b, c - > c }; j0 = natural_join l with r; j1 =
            natural_join j0 with s; output j1 as answer(a, b, c);
        } indexes { i0 = @ r [0]; i1 = @ s [0]; } uses
        { j0.right = i0; j1.right = i1; } physical
        {
            relations
            {
                L = :: cq_example_providers :: slice_rows; R = ::
                cq_example_providers :: hash_relation; S = ::
                cq_example_providers :: sorted_relation; l = ::
                cq_example_providers :: slice_rows; r = ::
                cq_example_providers :: hash_relation; s = ::
                cq_example_providers :: sorted_relation; j0 = ::
                cq_example_providers :: borrowed_fields; j1 = ::
                cq_example_providers :: borrowed_fields;
            } indexes
            {
                i0 = :: cq_example_providers :: hash_relation; i1 = ::
                cq_example_providers :: sorted_relation;
            } lowerings
            {
                l = :: cq_example_providers :: rename; r = ::
                cq_example_providers :: rename; s = :: cq_example_providers ::
                rename; j0 = :: cq_example_providers :: indexed_join; j1 = ::
                cq_example_providers :: indexed_join;
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
    } pending { lower l }
    continuations(:: mini_linq :: resume_physical, mini_linq :: access)
} emitted { let __cq_store_l : & [Value] = __cq_store_L; } result
{ stored l at(__cq_store_l) operations :: cq_example_providers :: slice_rows }
```

Actual output (formatted where Rust syntax permits):

```rust
::cq_example_providers::rename! {
    reply_to(:: mini_linq :: resume_physical)
    {
        plan
        {
            struct Heterogeneous; relation L(a : Value); relation
            R(a : Value, b : Value); relation S(b : Value, c : Value);
            answer(a, b, c) : - L(a), R(a, b), S(b, c); relational
            {
                l = rename L { a - > a }; r = rename R { a - > a, b - > b }; s
                = rename S { b - > b, c - > c }; j0 = natural_join l with r;
                j1 = natural_join j0 with s; output j1 as answer(a, b, c);
            } indexes { i0 = @ r [0]; i1 = @ s [0]; } uses
            { j0.right = i0; j1.right = i1; } physical
            {
                relations
                {
                    L = :: cq_example_providers :: slice_rows; R = ::
                    cq_example_providers :: hash_relation; S = ::
                    cq_example_providers :: sorted_relation; l = ::
                    cq_example_providers :: slice_rows; r = ::
                    cq_example_providers :: hash_relation; s = ::
                    cq_example_providers :: sorted_relation; j0 = ::
                    cq_example_providers :: borrowed_fields; j1 = ::
                    cq_example_providers :: borrowed_fields;
                } indexes
                {
                    i0 = :: cq_example_providers :: hash_relation; i1 = ::
                    cq_example_providers :: sorted_relation;
                } lowerings
                {
                    l = :: cq_example_providers :: rename; r = ::
                    cq_example_providers :: rename; s = :: cq_example_providers
                    :: rename; j0 = :: cq_example_providers :: indexed_join; j1
                    = :: cq_example_providers :: indexed_join;
                }
            }
        } environment
        {
            stored L at(__cq_store_L) operations :: cq_example_providers ::
            slice_rows stored R at(__cq_store_R) operations ::
            cq_example_providers :: hash_relation stored S at(__cq_store_S)
            operations :: cq_example_providers :: sorted_relation stored l
            at(__cq_store_l) operations :: cq_example_providers :: slice_rows
        } emitted
        {
            let __cq_store_L : & [Value] = L; let __cq_store_R : & :: std ::
            collections :: HashMap < Value, :: std :: vec :: Vec < Value > > =
            R; let __cq_store_S : & :: std :: collections :: BTreeMap < Value,
            :: std :: vec :: Vec < Value > > = S; let __cq_store_l : & [Value]
            = __cq_store_L;
        } pending { lower r }
        continuations(:: mini_linq :: resume_physical, mini_linq :: access)
    } plan
    {
        struct Heterogeneous; relation L(a : Value); relation
        R(a : Value, b : Value); relation S(b : Value, c : Value);
        answer(a, b, c) : - L(a), R(a, b), S(b, c); relational
        {
            l = rename L { a - > a }; r = rename R { a - > a, b - > b }; s =
            rename S { b - > b, c - > c }; j0 = natural_join l with r; j1 =
            natural_join j0 with s; output j1 as answer(a, b, c);
        } indexes { i0 = @ r [0]; i1 = @ s [0]; } uses
        { j0.right = i0; j1.right = i1; } physical
        {
            relations
            {
                L = :: cq_example_providers :: slice_rows; R = ::
                cq_example_providers :: hash_relation; S = ::
                cq_example_providers :: sorted_relation; l = ::
                cq_example_providers :: slice_rows; r = ::
                cq_example_providers :: hash_relation; s = ::
                cq_example_providers :: sorted_relation; j0 = ::
                cq_example_providers :: borrowed_fields; j1 = ::
                cq_example_providers :: borrowed_fields;
            } indexes
            {
                i0 = :: cq_example_providers :: hash_relation; i1 = ::
                cq_example_providers :: sorted_relation;
            } lowerings
            {
                l = :: cq_example_providers :: rename; r = ::
                cq_example_providers :: rename; s = :: cq_example_providers ::
                rename; j0 = :: cq_example_providers :: indexed_join; j1 = ::
                cq_example_providers :: indexed_join;
            }
        }
    } environment
    {
        stored L at(__cq_store_L) operations :: cq_example_providers ::
        slice_rows stored R at(__cq_store_R) operations ::
        cq_example_providers :: hash_relation stored S at(__cq_store_S)
        operations :: cq_example_providers :: sorted_relation stored l
        at(__cq_store_l) operations :: cq_example_providers :: slice_rows
    } request { lower r }
}

```

---

## 2172042/013-cq-example-providers::rename

[Raw input](raw/2172042/013-cq-example-providers::rename.input.tokens) · [Raw output](raw/2172042/013-cq-example-providers::rename.output.tokens)

Actual input:

```rust
reply_to(:: mini_linq :: resume_physical)
{
    plan
    {
        struct Heterogeneous; relation L(a : Value); relation
        R(a : Value, b : Value); relation S(b : Value, c : Value);
        answer(a, b, c) : - L(a), R(a, b), S(b, c); relational
        {
            l = rename L { a - > a }; r = rename R { a - > a, b - > b }; s =
            rename S { b - > b, c - > c }; j0 = natural_join l with r; j1 =
            natural_join j0 with s; output j1 as answer(a, b, c);
        } indexes { i0 = @ r [0]; i1 = @ s [0]; } uses
        { j0.right = i0; j1.right = i1; } physical
        {
            relations
            {
                L = :: cq_example_providers :: slice_rows; R = ::
                cq_example_providers :: hash_relation; S = ::
                cq_example_providers :: sorted_relation; l = ::
                cq_example_providers :: slice_rows; r = ::
                cq_example_providers :: hash_relation; s = ::
                cq_example_providers :: sorted_relation; j0 = ::
                cq_example_providers :: borrowed_fields; j1 = ::
                cq_example_providers :: borrowed_fields;
            } indexes
            {
                i0 = :: cq_example_providers :: hash_relation; i1 = ::
                cq_example_providers :: sorted_relation;
            } lowerings
            {
                l = :: cq_example_providers :: rename; r = ::
                cq_example_providers :: rename; s = :: cq_example_providers ::
                rename; j0 = :: cq_example_providers :: indexed_join; j1 = ::
                cq_example_providers :: indexed_join;
            }
        }
    } environment
    {
        stored L at(__cq_store_L) operations :: cq_example_providers ::
        slice_rows stored R at(__cq_store_R) operations ::
        cq_example_providers :: hash_relation stored S at(__cq_store_S)
        operations :: cq_example_providers :: sorted_relation stored l
        at(__cq_store_l) operations :: cq_example_providers :: slice_rows
    } emitted
    {
        let __cq_store_L : & [Value] = L; let __cq_store_R : & :: std ::
        collections :: HashMap < Value, :: std :: vec :: Vec < Value > > = R;
        let __cq_store_S : & :: std :: collections :: BTreeMap < Value, :: std
        :: vec :: Vec < Value > > = S; let __cq_store_l : & [Value] =
        __cq_store_L;
    } pending { lower r }
    continuations(:: mini_linq :: resume_physical, mini_linq :: access)
} plan
{
    struct Heterogeneous; relation L(a : Value); relation
    R(a : Value, b : Value); relation S(b : Value, c : Value); answer(a, b, c)
    : - L(a), R(a, b), S(b, c); relational
    {
        l = rename L { a - > a }; r = rename R { a - > a, b - > b }; s =
        rename S { b - > b, c - > c }; j0 = natural_join l with r; j1 =
        natural_join j0 with s; output j1 as answer(a, b, c);
    } indexes { i0 = @ r [0]; i1 = @ s [0]; } uses
    { j0.right = i0; j1.right = i1; } physical
    {
        relations
        {
            L = :: cq_example_providers :: slice_rows; R = ::
            cq_example_providers :: hash_relation; S = :: cq_example_providers
            :: sorted_relation; l = :: cq_example_providers :: slice_rows; r =
            :: cq_example_providers :: hash_relation; s = ::
            cq_example_providers :: sorted_relation; j0 = ::
            cq_example_providers :: borrowed_fields; j1 = ::
            cq_example_providers :: borrowed_fields;
        } indexes
        {
            i0 = :: cq_example_providers :: hash_relation; i1 = ::
            cq_example_providers :: sorted_relation;
        } lowerings
        {
            l = :: cq_example_providers :: rename; r = :: cq_example_providers
            :: rename; s = :: cq_example_providers :: rename; j0 = ::
            cq_example_providers :: indexed_join; j1 = :: cq_example_providers
            :: indexed_join;
        }
    }
} environment
{
    stored L at(__cq_store_L) operations :: cq_example_providers :: slice_rows
    stored R at(__cq_store_R) operations :: cq_example_providers ::
    hash_relation stored S at(__cq_store_S) operations :: cq_example_providers
    :: sorted_relation stored l at(__cq_store_l) operations ::
    cq_example_providers :: slice_rows
} request { lower r }
```

Actual output (formatted where Rust syntax permits):

```rust
::cq_example_providers::hash_relation! {
    reply_to(:: mini_linq :: resume_physical)
    {
        plan
        {
            struct Heterogeneous; relation L(a : Value); relation
            R(a : Value, b : Value); relation S(b : Value, c : Value);
            answer(a, b, c) : - L(a), R(a, b), S(b, c); relational
            {
                l = rename L { a - > a }; r = rename R { a - > a, b - > b }; s
                = rename S { b - > b, c - > c }; j0 = natural_join l with r;
                j1 = natural_join j0 with s; output j1 as answer(a, b, c);
            } indexes { i0 = @ r [0]; i1 = @ s [0]; } uses
            { j0.right = i0; j1.right = i1; } physical
            {
                relations
                {
                    L = :: cq_example_providers :: slice_rows; R = ::
                    cq_example_providers :: hash_relation; S = ::
                    cq_example_providers :: sorted_relation; l = ::
                    cq_example_providers :: slice_rows; r = ::
                    cq_example_providers :: hash_relation; s = ::
                    cq_example_providers :: sorted_relation; j0 = ::
                    cq_example_providers :: borrowed_fields; j1 = ::
                    cq_example_providers :: borrowed_fields;
                } indexes
                {
                    i0 = :: cq_example_providers :: hash_relation; i1 = ::
                    cq_example_providers :: sorted_relation;
                } lowerings
                {
                    l = :: cq_example_providers :: rename; r = ::
                    cq_example_providers :: rename; s = :: cq_example_providers
                    :: rename; j0 = :: cq_example_providers :: indexed_join; j1
                    = :: cq_example_providers :: indexed_join;
                }
            }
        } environment
        {
            stored L at(__cq_store_L) operations :: cq_example_providers ::
            slice_rows stored R at(__cq_store_R) operations ::
            cq_example_providers :: hash_relation stored S at(__cq_store_S)
            operations :: cq_example_providers :: sorted_relation stored l
            at(__cq_store_l) operations :: cq_example_providers :: slice_rows
        } emitted
        {
            let __cq_store_L : & [Value] = L; let __cq_store_R : & :: std ::
            collections :: HashMap < Value, :: std :: vec :: Vec < Value > > =
            R; let __cq_store_S : & :: std :: collections :: BTreeMap < Value,
            :: std :: vec :: Vec < Value > > = S; let __cq_store_l : & [Value]
            = __cq_store_L;
        } pending { lower r }
        continuations(:: mini_linq :: resume_physical, mini_linq :: access)
    } plan
    {
        struct Heterogeneous; relation L(a : Value); relation
        R(a : Value, b : Value); relation S(b : Value, c : Value);
        answer(a, b, c) : - L(a), R(a, b), S(b, c); relational
        {
            l = rename L { a - > a }; r = rename R { a - > a, b - > b }; s =
            rename S { b - > b, c - > c }; j0 = natural_join l with r; j1 =
            natural_join j0 with s; output j1 as answer(a, b, c);
        } indexes { i0 = @ r [0]; i1 = @ s [0]; } uses
        { j0.right = i0; j1.right = i1; } physical
        {
            relations
            {
                L = :: cq_example_providers :: slice_rows; R = ::
                cq_example_providers :: hash_relation; S = ::
                cq_example_providers :: sorted_relation; l = ::
                cq_example_providers :: slice_rows; r = ::
                cq_example_providers :: hash_relation; s = ::
                cq_example_providers :: sorted_relation; j0 = ::
                cq_example_providers :: borrowed_fields; j1 = ::
                cq_example_providers :: borrowed_fields;
            } indexes
            {
                i0 = :: cq_example_providers :: hash_relation; i1 = ::
                cq_example_providers :: sorted_relation;
            } lowerings
            {
                l = :: cq_example_providers :: rename; r = ::
                cq_example_providers :: rename; s = :: cq_example_providers ::
                rename; j0 = :: cq_example_providers :: indexed_join; j1 = ::
                cq_example_providers :: indexed_join;
            }
        }
    } environment
    {
        stored L at(__cq_store_L) operations :: cq_example_providers ::
        slice_rows stored R at(__cq_store_R) operations ::
        cq_example_providers :: hash_relation stored S at(__cq_store_S)
        operations :: cq_example_providers :: sorted_relation stored l
        at(__cq_store_l) operations :: cq_example_providers :: slice_rows
    } request { input r from(__cq_store_R) }
}

```

---

## 2172042/014-cq-example-providers::hash_relation

[Raw input](raw/2172042/014-cq-example-providers::hash_relation.input.tokens) · [Raw output](raw/2172042/014-cq-example-providers::hash_relation.output.tokens)

Actual input:

```rust
reply_to(:: mini_linq :: resume_physical)
{
    plan
    {
        struct Heterogeneous; relation L(a : Value); relation
        R(a : Value, b : Value); relation S(b : Value, c : Value);
        answer(a, b, c) : - L(a), R(a, b), S(b, c); relational
        {
            l = rename L { a - > a }; r = rename R { a - > a, b - > b }; s =
            rename S { b - > b, c - > c }; j0 = natural_join l with r; j1 =
            natural_join j0 with s; output j1 as answer(a, b, c);
        } indexes { i0 = @ r [0]; i1 = @ s [0]; } uses
        { j0.right = i0; j1.right = i1; } physical
        {
            relations
            {
                L = :: cq_example_providers :: slice_rows; R = ::
                cq_example_providers :: hash_relation; S = ::
                cq_example_providers :: sorted_relation; l = ::
                cq_example_providers :: slice_rows; r = ::
                cq_example_providers :: hash_relation; s = ::
                cq_example_providers :: sorted_relation; j0 = ::
                cq_example_providers :: borrowed_fields; j1 = ::
                cq_example_providers :: borrowed_fields;
            } indexes
            {
                i0 = :: cq_example_providers :: hash_relation; i1 = ::
                cq_example_providers :: sorted_relation;
            } lowerings
            {
                l = :: cq_example_providers :: rename; r = ::
                cq_example_providers :: rename; s = :: cq_example_providers ::
                rename; j0 = :: cq_example_providers :: indexed_join; j1 = ::
                cq_example_providers :: indexed_join;
            }
        }
    } environment
    {
        stored L at(__cq_store_L) operations :: cq_example_providers ::
        slice_rows stored R at(__cq_store_R) operations ::
        cq_example_providers :: hash_relation stored S at(__cq_store_S)
        operations :: cq_example_providers :: sorted_relation stored l
        at(__cq_store_l) operations :: cq_example_providers :: slice_rows
    } emitted
    {
        let __cq_store_L : & [Value] = L; let __cq_store_R : & :: std ::
        collections :: HashMap < Value, :: std :: vec :: Vec < Value > > = R;
        let __cq_store_S : & :: std :: collections :: BTreeMap < Value, :: std
        :: vec :: Vec < Value > > = S; let __cq_store_l : & [Value] =
        __cq_store_L;
    } pending { lower r }
    continuations(:: mini_linq :: resume_physical, mini_linq :: access)
} plan
{
    struct Heterogeneous; relation L(a : Value); relation
    R(a : Value, b : Value); relation S(b : Value, c : Value); answer(a, b, c)
    : - L(a), R(a, b), S(b, c); relational
    {
        l = rename L { a - > a }; r = rename R { a - > a, b - > b }; s =
        rename S { b - > b, c - > c }; j0 = natural_join l with r; j1 =
        natural_join j0 with s; output j1 as answer(a, b, c);
    } indexes { i0 = @ r [0]; i1 = @ s [0]; } uses
    { j0.right = i0; j1.right = i1; } physical
    {
        relations
        {
            L = :: cq_example_providers :: slice_rows; R = ::
            cq_example_providers :: hash_relation; S = :: cq_example_providers
            :: sorted_relation; l = :: cq_example_providers :: slice_rows; r =
            :: cq_example_providers :: hash_relation; s = ::
            cq_example_providers :: sorted_relation; j0 = ::
            cq_example_providers :: borrowed_fields; j1 = ::
            cq_example_providers :: borrowed_fields;
        } indexes
        {
            i0 = :: cq_example_providers :: hash_relation; i1 = ::
            cq_example_providers :: sorted_relation;
        } lowerings
        {
            l = :: cq_example_providers :: rename; r = :: cq_example_providers
            :: rename; s = :: cq_example_providers :: rename; j0 = ::
            cq_example_providers :: indexed_join; j1 = :: cq_example_providers
            :: indexed_join;
        }
    }
} environment
{
    stored L at(__cq_store_L) operations :: cq_example_providers :: slice_rows
    stored R at(__cq_store_R) operations :: cq_example_providers ::
    hash_relation stored S at(__cq_store_S) operations :: cq_example_providers
    :: sorted_relation stored l at(__cq_store_l) operations ::
    cq_example_providers :: slice_rows
} request { input r from(__cq_store_R) }
```

Actual output (formatted where Rust syntax permits):

```rust
::mini_linq::resume_physical! {
    resume
    {
        plan
        {
            struct Heterogeneous; relation L(a : Value); relation
            R(a : Value, b : Value); relation S(b : Value, c : Value);
            answer(a, b, c) : - L(a), R(a, b), S(b, c); relational
            {
                l = rename L { a - > a }; r = rename R { a - > a, b - > b }; s
                = rename S { b - > b, c - > c }; j0 = natural_join l with r;
                j1 = natural_join j0 with s; output j1 as answer(a, b, c);
            } indexes { i0 = @ r [0]; i1 = @ s [0]; } uses
            { j0.right = i0; j1.right = i1; } physical
            {
                relations
                {
                    L = :: cq_example_providers :: slice_rows; R = ::
                    cq_example_providers :: hash_relation; S = ::
                    cq_example_providers :: sorted_relation; l = ::
                    cq_example_providers :: slice_rows; r = ::
                    cq_example_providers :: hash_relation; s = ::
                    cq_example_providers :: sorted_relation; j0 = ::
                    cq_example_providers :: borrowed_fields; j1 = ::
                    cq_example_providers :: borrowed_fields;
                } indexes
                {
                    i0 = :: cq_example_providers :: hash_relation; i1 = ::
                    cq_example_providers :: sorted_relation;
                } lowerings
                {
                    l = :: cq_example_providers :: rename; r = ::
                    cq_example_providers :: rename; s = :: cq_example_providers
                    :: rename; j0 = :: cq_example_providers :: indexed_join; j1
                    = :: cq_example_providers :: indexed_join;
                }
            }
        } environment
        {
            stored L at(__cq_store_L) operations :: cq_example_providers ::
            slice_rows stored R at(__cq_store_R) operations ::
            cq_example_providers :: hash_relation stored S at(__cq_store_S)
            operations :: cq_example_providers :: sorted_relation stored l
            at(__cq_store_l) operations :: cq_example_providers :: slice_rows
        } emitted
        {
            let __cq_store_L : & [Value] = L; let __cq_store_R : & :: std ::
            collections :: HashMap < Value, :: std :: vec :: Vec < Value > > =
            R; let __cq_store_S : & :: std :: collections :: BTreeMap < Value,
            :: std :: vec :: Vec < Value > > = S; let __cq_store_l : & [Value]
            = __cq_store_L;
        } pending { lower r }
        continuations(:: mini_linq :: resume_physical, mini_linq :: access)
    } emitted
    {
        let __cq_store_r : & :: std :: collections :: HashMap < Value, :: std
        :: vec :: Vec < Value >> = __cq_store_R;
    } result
    {
        stored r at(__cq_store_r) operations :: cq_example_providers ::
        hash_relation
    }
}

```

---

## 2172042/015-resume_physical

[Raw input](raw/2172042/015-resume_physical.input.tokens) · [Raw output](raw/2172042/015-resume_physical.output.tokens)

Actual input:

```rust
resume
{
    plan
    {
        struct Heterogeneous; relation L(a : Value); relation
        R(a : Value, b : Value); relation S(b : Value, c : Value);
        answer(a, b, c) : - L(a), R(a, b), S(b, c); relational
        {
            l = rename L { a - > a }; r = rename R { a - > a, b - > b }; s =
            rename S { b - > b, c - > c }; j0 = natural_join l with r; j1 =
            natural_join j0 with s; output j1 as answer(a, b, c);
        } indexes { i0 = @ r [0]; i1 = @ s [0]; } uses
        { j0.right = i0; j1.right = i1; } physical
        {
            relations
            {
                L = :: cq_example_providers :: slice_rows; R = ::
                cq_example_providers :: hash_relation; S = ::
                cq_example_providers :: sorted_relation; l = ::
                cq_example_providers :: slice_rows; r = ::
                cq_example_providers :: hash_relation; s = ::
                cq_example_providers :: sorted_relation; j0 = ::
                cq_example_providers :: borrowed_fields; j1 = ::
                cq_example_providers :: borrowed_fields;
            } indexes
            {
                i0 = :: cq_example_providers :: hash_relation; i1 = ::
                cq_example_providers :: sorted_relation;
            } lowerings
            {
                l = :: cq_example_providers :: rename; r = ::
                cq_example_providers :: rename; s = :: cq_example_providers ::
                rename; j0 = :: cq_example_providers :: indexed_join; j1 = ::
                cq_example_providers :: indexed_join;
            }
        }
    } environment
    {
        stored L at(__cq_store_L) operations :: cq_example_providers ::
        slice_rows stored R at(__cq_store_R) operations ::
        cq_example_providers :: hash_relation stored S at(__cq_store_S)
        operations :: cq_example_providers :: sorted_relation stored l
        at(__cq_store_l) operations :: cq_example_providers :: slice_rows
    } emitted
    {
        let __cq_store_L : & [Value] = L; let __cq_store_R : & :: std ::
        collections :: HashMap < Value, :: std :: vec :: Vec < Value > > = R;
        let __cq_store_S : & :: std :: collections :: BTreeMap < Value, :: std
        :: vec :: Vec < Value > > = S; let __cq_store_l : & [Value] =
        __cq_store_L;
    } pending { lower r }
    continuations(:: mini_linq :: resume_physical, mini_linq :: access)
} emitted
{
    let __cq_store_r : & :: std :: collections :: HashMap < Value, :: std ::
    vec :: Vec < Value >> = __cq_store_R;
} result
{
    stored r at(__cq_store_r) operations :: cq_example_providers ::
    hash_relation
}
```

Actual output (formatted where Rust syntax permits):

```rust
::cq_example_providers::rename! {
    reply_to(:: mini_linq :: resume_physical)
    {
        plan
        {
            struct Heterogeneous; relation L(a : Value); relation
            R(a : Value, b : Value); relation S(b : Value, c : Value);
            answer(a, b, c) : - L(a), R(a, b), S(b, c); relational
            {
                l = rename L { a - > a }; r = rename R { a - > a, b - > b }; s
                = rename S { b - > b, c - > c }; j0 = natural_join l with r;
                j1 = natural_join j0 with s; output j1 as answer(a, b, c);
            } indexes { i0 = @ r [0]; i1 = @ s [0]; } uses
            { j0.right = i0; j1.right = i1; } physical
            {
                relations
                {
                    L = :: cq_example_providers :: slice_rows; R = ::
                    cq_example_providers :: hash_relation; S = ::
                    cq_example_providers :: sorted_relation; l = ::
                    cq_example_providers :: slice_rows; r = ::
                    cq_example_providers :: hash_relation; s = ::
                    cq_example_providers :: sorted_relation; j0 = ::
                    cq_example_providers :: borrowed_fields; j1 = ::
                    cq_example_providers :: borrowed_fields;
                } indexes
                {
                    i0 = :: cq_example_providers :: hash_relation; i1 = ::
                    cq_example_providers :: sorted_relation;
                } lowerings
                {
                    l = :: cq_example_providers :: rename; r = ::
                    cq_example_providers :: rename; s = :: cq_example_providers
                    :: rename; j0 = :: cq_example_providers :: indexed_join; j1
                    = :: cq_example_providers :: indexed_join;
                }
            }
        } environment
        {
            stored L at(__cq_store_L) operations :: cq_example_providers ::
            slice_rows stored R at(__cq_store_R) operations ::
            cq_example_providers :: hash_relation stored S at(__cq_store_S)
            operations :: cq_example_providers :: sorted_relation stored l
            at(__cq_store_l) operations :: cq_example_providers :: slice_rows
            stored r at(__cq_store_r) operations :: cq_example_providers ::
            hash_relation
        } emitted
        {
            let __cq_store_L : & [Value] = L; let __cq_store_R : & :: std ::
            collections :: HashMap < Value, :: std :: vec :: Vec < Value > > =
            R; let __cq_store_S : & :: std :: collections :: BTreeMap < Value,
            :: std :: vec :: Vec < Value > > = S; let __cq_store_l : & [Value]
            = __cq_store_L; let __cq_store_r : & :: std :: collections ::
            HashMap < Value, :: std :: vec :: Vec < Value > > = __cq_store_R;
        } pending { lower s }
        continuations(:: mini_linq :: resume_physical, mini_linq :: access)
    } plan
    {
        struct Heterogeneous; relation L(a : Value); relation
        R(a : Value, b : Value); relation S(b : Value, c : Value);
        answer(a, b, c) : - L(a), R(a, b), S(b, c); relational
        {
            l = rename L { a - > a }; r = rename R { a - > a, b - > b }; s =
            rename S { b - > b, c - > c }; j0 = natural_join l with r; j1 =
            natural_join j0 with s; output j1 as answer(a, b, c);
        } indexes { i0 = @ r [0]; i1 = @ s [0]; } uses
        { j0.right = i0; j1.right = i1; } physical
        {
            relations
            {
                L = :: cq_example_providers :: slice_rows; R = ::
                cq_example_providers :: hash_relation; S = ::
                cq_example_providers :: sorted_relation; l = ::
                cq_example_providers :: slice_rows; r = ::
                cq_example_providers :: hash_relation; s = ::
                cq_example_providers :: sorted_relation; j0 = ::
                cq_example_providers :: borrowed_fields; j1 = ::
                cq_example_providers :: borrowed_fields;
            } indexes
            {
                i0 = :: cq_example_providers :: hash_relation; i1 = ::
                cq_example_providers :: sorted_relation;
            } lowerings
            {
                l = :: cq_example_providers :: rename; r = ::
                cq_example_providers :: rename; s = :: cq_example_providers ::
                rename; j0 = :: cq_example_providers :: indexed_join; j1 = ::
                cq_example_providers :: indexed_join;
            }
        }
    } environment
    {
        stored L at(__cq_store_L) operations :: cq_example_providers ::
        slice_rows stored R at(__cq_store_R) operations ::
        cq_example_providers :: hash_relation stored S at(__cq_store_S)
        operations :: cq_example_providers :: sorted_relation stored l
        at(__cq_store_l) operations :: cq_example_providers :: slice_rows
        stored r at(__cq_store_r) operations :: cq_example_providers ::
        hash_relation
    } request { lower s }
}

```

---

## 2172042/016-cq-example-providers::rename

[Raw input](raw/2172042/016-cq-example-providers::rename.input.tokens) · [Raw output](raw/2172042/016-cq-example-providers::rename.output.tokens)

Actual input:

```rust
reply_to(:: mini_linq :: resume_physical)
{
    plan
    {
        struct Heterogeneous; relation L(a : Value); relation
        R(a : Value, b : Value); relation S(b : Value, c : Value);
        answer(a, b, c) : - L(a), R(a, b), S(b, c); relational
        {
            l = rename L { a - > a }; r = rename R { a - > a, b - > b }; s =
            rename S { b - > b, c - > c }; j0 = natural_join l with r; j1 =
            natural_join j0 with s; output j1 as answer(a, b, c);
        } indexes { i0 = @ r [0]; i1 = @ s [0]; } uses
        { j0.right = i0; j1.right = i1; } physical
        {
            relations
            {
                L = :: cq_example_providers :: slice_rows; R = ::
                cq_example_providers :: hash_relation; S = ::
                cq_example_providers :: sorted_relation; l = ::
                cq_example_providers :: slice_rows; r = ::
                cq_example_providers :: hash_relation; s = ::
                cq_example_providers :: sorted_relation; j0 = ::
                cq_example_providers :: borrowed_fields; j1 = ::
                cq_example_providers :: borrowed_fields;
            } indexes
            {
                i0 = :: cq_example_providers :: hash_relation; i1 = ::
                cq_example_providers :: sorted_relation;
            } lowerings
            {
                l = :: cq_example_providers :: rename; r = ::
                cq_example_providers :: rename; s = :: cq_example_providers ::
                rename; j0 = :: cq_example_providers :: indexed_join; j1 = ::
                cq_example_providers :: indexed_join;
            }
        }
    } environment
    {
        stored L at(__cq_store_L) operations :: cq_example_providers ::
        slice_rows stored R at(__cq_store_R) operations ::
        cq_example_providers :: hash_relation stored S at(__cq_store_S)
        operations :: cq_example_providers :: sorted_relation stored l
        at(__cq_store_l) operations :: cq_example_providers :: slice_rows
        stored r at(__cq_store_r) operations :: cq_example_providers ::
        hash_relation
    } emitted
    {
        let __cq_store_L : & [Value] = L; let __cq_store_R : & :: std ::
        collections :: HashMap < Value, :: std :: vec :: Vec < Value > > = R;
        let __cq_store_S : & :: std :: collections :: BTreeMap < Value, :: std
        :: vec :: Vec < Value > > = S; let __cq_store_l : & [Value] =
        __cq_store_L; let __cq_store_r : & :: std :: collections :: HashMap <
        Value, :: std :: vec :: Vec < Value > > = __cq_store_R;
    } pending { lower s }
    continuations(:: mini_linq :: resume_physical, mini_linq :: access)
} plan
{
    struct Heterogeneous; relation L(a : Value); relation
    R(a : Value, b : Value); relation S(b : Value, c : Value); answer(a, b, c)
    : - L(a), R(a, b), S(b, c); relational
    {
        l = rename L { a - > a }; r = rename R { a - > a, b - > b }; s =
        rename S { b - > b, c - > c }; j0 = natural_join l with r; j1 =
        natural_join j0 with s; output j1 as answer(a, b, c);
    } indexes { i0 = @ r [0]; i1 = @ s [0]; } uses
    { j0.right = i0; j1.right = i1; } physical
    {
        relations
        {
            L = :: cq_example_providers :: slice_rows; R = ::
            cq_example_providers :: hash_relation; S = :: cq_example_providers
            :: sorted_relation; l = :: cq_example_providers :: slice_rows; r =
            :: cq_example_providers :: hash_relation; s = ::
            cq_example_providers :: sorted_relation; j0 = ::
            cq_example_providers :: borrowed_fields; j1 = ::
            cq_example_providers :: borrowed_fields;
        } indexes
        {
            i0 = :: cq_example_providers :: hash_relation; i1 = ::
            cq_example_providers :: sorted_relation;
        } lowerings
        {
            l = :: cq_example_providers :: rename; r = :: cq_example_providers
            :: rename; s = :: cq_example_providers :: rename; j0 = ::
            cq_example_providers :: indexed_join; j1 = :: cq_example_providers
            :: indexed_join;
        }
    }
} environment
{
    stored L at(__cq_store_L) operations :: cq_example_providers :: slice_rows
    stored R at(__cq_store_R) operations :: cq_example_providers ::
    hash_relation stored S at(__cq_store_S) operations :: cq_example_providers
    :: sorted_relation stored l at(__cq_store_l) operations ::
    cq_example_providers :: slice_rows stored r at(__cq_store_r) operations ::
    cq_example_providers :: hash_relation
} request { lower s }
```

Actual output (formatted where Rust syntax permits):

```rust
::cq_example_providers::sorted_relation! {
    reply_to(:: mini_linq :: resume_physical)
    {
        plan
        {
            struct Heterogeneous; relation L(a : Value); relation
            R(a : Value, b : Value); relation S(b : Value, c : Value);
            answer(a, b, c) : - L(a), R(a, b), S(b, c); relational
            {
                l = rename L { a - > a }; r = rename R { a - > a, b - > b }; s
                = rename S { b - > b, c - > c }; j0 = natural_join l with r;
                j1 = natural_join j0 with s; output j1 as answer(a, b, c);
            } indexes { i0 = @ r [0]; i1 = @ s [0]; } uses
            { j0.right = i0; j1.right = i1; } physical
            {
                relations
                {
                    L = :: cq_example_providers :: slice_rows; R = ::
                    cq_example_providers :: hash_relation; S = ::
                    cq_example_providers :: sorted_relation; l = ::
                    cq_example_providers :: slice_rows; r = ::
                    cq_example_providers :: hash_relation; s = ::
                    cq_example_providers :: sorted_relation; j0 = ::
                    cq_example_providers :: borrowed_fields; j1 = ::
                    cq_example_providers :: borrowed_fields;
                } indexes
                {
                    i0 = :: cq_example_providers :: hash_relation; i1 = ::
                    cq_example_providers :: sorted_relation;
                } lowerings
                {
                    l = :: cq_example_providers :: rename; r = ::
                    cq_example_providers :: rename; s = :: cq_example_providers
                    :: rename; j0 = :: cq_example_providers :: indexed_join; j1
                    = :: cq_example_providers :: indexed_join;
                }
            }
        } environment
        {
            stored L at(__cq_store_L) operations :: cq_example_providers ::
            slice_rows stored R at(__cq_store_R) operations ::
            cq_example_providers :: hash_relation stored S at(__cq_store_S)
            operations :: cq_example_providers :: sorted_relation stored l
            at(__cq_store_l) operations :: cq_example_providers :: slice_rows
            stored r at(__cq_store_r) operations :: cq_example_providers ::
            hash_relation
        } emitted
        {
            let __cq_store_L : & [Value] = L; let __cq_store_R : & :: std ::
            collections :: HashMap < Value, :: std :: vec :: Vec < Value > > =
            R; let __cq_store_S : & :: std :: collections :: BTreeMap < Value,
            :: std :: vec :: Vec < Value > > = S; let __cq_store_l : & [Value]
            = __cq_store_L; let __cq_store_r : & :: std :: collections ::
            HashMap < Value, :: std :: vec :: Vec < Value > > = __cq_store_R;
        } pending { lower s }
        continuations(:: mini_linq :: resume_physical, mini_linq :: access)
    } plan
    {
        struct Heterogeneous; relation L(a : Value); relation
        R(a : Value, b : Value); relation S(b : Value, c : Value);
        answer(a, b, c) : - L(a), R(a, b), S(b, c); relational
        {
            l = rename L { a - > a }; r = rename R { a - > a, b - > b }; s =
            rename S { b - > b, c - > c }; j0 = natural_join l with r; j1 =
            natural_join j0 with s; output j1 as answer(a, b, c);
        } indexes { i0 = @ r [0]; i1 = @ s [0]; } uses
        { j0.right = i0; j1.right = i1; } physical
        {
            relations
            {
                L = :: cq_example_providers :: slice_rows; R = ::
                cq_example_providers :: hash_relation; S = ::
                cq_example_providers :: sorted_relation; l = ::
                cq_example_providers :: slice_rows; r = ::
                cq_example_providers :: hash_relation; s = ::
                cq_example_providers :: sorted_relation; j0 = ::
                cq_example_providers :: borrowed_fields; j1 = ::
                cq_example_providers :: borrowed_fields;
            } indexes
            {
                i0 = :: cq_example_providers :: hash_relation; i1 = ::
                cq_example_providers :: sorted_relation;
            } lowerings
            {
                l = :: cq_example_providers :: rename; r = ::
                cq_example_providers :: rename; s = :: cq_example_providers ::
                rename; j0 = :: cq_example_providers :: indexed_join; j1 = ::
                cq_example_providers :: indexed_join;
            }
        }
    } environment
    {
        stored L at(__cq_store_L) operations :: cq_example_providers ::
        slice_rows stored R at(__cq_store_R) operations ::
        cq_example_providers :: hash_relation stored S at(__cq_store_S)
        operations :: cq_example_providers :: sorted_relation stored l
        at(__cq_store_l) operations :: cq_example_providers :: slice_rows
        stored r at(__cq_store_r) operations :: cq_example_providers ::
        hash_relation
    } request { input s from(__cq_store_S) }
}

```

---

## 2172042/017-cq-example-providers::sorted_relation

[Raw input](raw/2172042/017-cq-example-providers::sorted_relation.input.tokens) · [Raw output](raw/2172042/017-cq-example-providers::sorted_relation.output.tokens)

Actual input:

```rust
reply_to(:: mini_linq :: resume_physical)
{
    plan
    {
        struct Heterogeneous; relation L(a : Value); relation
        R(a : Value, b : Value); relation S(b : Value, c : Value);
        answer(a, b, c) : - L(a), R(a, b), S(b, c); relational
        {
            l = rename L { a - > a }; r = rename R { a - > a, b - > b }; s =
            rename S { b - > b, c - > c }; j0 = natural_join l with r; j1 =
            natural_join j0 with s; output j1 as answer(a, b, c);
        } indexes { i0 = @ r [0]; i1 = @ s [0]; } uses
        { j0.right = i0; j1.right = i1; } physical
        {
            relations
            {
                L = :: cq_example_providers :: slice_rows; R = ::
                cq_example_providers :: hash_relation; S = ::
                cq_example_providers :: sorted_relation; l = ::
                cq_example_providers :: slice_rows; r = ::
                cq_example_providers :: hash_relation; s = ::
                cq_example_providers :: sorted_relation; j0 = ::
                cq_example_providers :: borrowed_fields; j1 = ::
                cq_example_providers :: borrowed_fields;
            } indexes
            {
                i0 = :: cq_example_providers :: hash_relation; i1 = ::
                cq_example_providers :: sorted_relation;
            } lowerings
            {
                l = :: cq_example_providers :: rename; r = ::
                cq_example_providers :: rename; s = :: cq_example_providers ::
                rename; j0 = :: cq_example_providers :: indexed_join; j1 = ::
                cq_example_providers :: indexed_join;
            }
        }
    } environment
    {
        stored L at(__cq_store_L) operations :: cq_example_providers ::
        slice_rows stored R at(__cq_store_R) operations ::
        cq_example_providers :: hash_relation stored S at(__cq_store_S)
        operations :: cq_example_providers :: sorted_relation stored l
        at(__cq_store_l) operations :: cq_example_providers :: slice_rows
        stored r at(__cq_store_r) operations :: cq_example_providers ::
        hash_relation
    } emitted
    {
        let __cq_store_L : & [Value] = L; let __cq_store_R : & :: std ::
        collections :: HashMap < Value, :: std :: vec :: Vec < Value > > = R;
        let __cq_store_S : & :: std :: collections :: BTreeMap < Value, :: std
        :: vec :: Vec < Value > > = S; let __cq_store_l : & [Value] =
        __cq_store_L; let __cq_store_r : & :: std :: collections :: HashMap <
        Value, :: std :: vec :: Vec < Value > > = __cq_store_R;
    } pending { lower s }
    continuations(:: mini_linq :: resume_physical, mini_linq :: access)
} plan
{
    struct Heterogeneous; relation L(a : Value); relation
    R(a : Value, b : Value); relation S(b : Value, c : Value); answer(a, b, c)
    : - L(a), R(a, b), S(b, c); relational
    {
        l = rename L { a - > a }; r = rename R { a - > a, b - > b }; s =
        rename S { b - > b, c - > c }; j0 = natural_join l with r; j1 =
        natural_join j0 with s; output j1 as answer(a, b, c);
    } indexes { i0 = @ r [0]; i1 = @ s [0]; } uses
    { j0.right = i0; j1.right = i1; } physical
    {
        relations
        {
            L = :: cq_example_providers :: slice_rows; R = ::
            cq_example_providers :: hash_relation; S = :: cq_example_providers
            :: sorted_relation; l = :: cq_example_providers :: slice_rows; r =
            :: cq_example_providers :: hash_relation; s = ::
            cq_example_providers :: sorted_relation; j0 = ::
            cq_example_providers :: borrowed_fields; j1 = ::
            cq_example_providers :: borrowed_fields;
        } indexes
        {
            i0 = :: cq_example_providers :: hash_relation; i1 = ::
            cq_example_providers :: sorted_relation;
        } lowerings
        {
            l = :: cq_example_providers :: rename; r = :: cq_example_providers
            :: rename; s = :: cq_example_providers :: rename; j0 = ::
            cq_example_providers :: indexed_join; j1 = :: cq_example_providers
            :: indexed_join;
        }
    }
} environment
{
    stored L at(__cq_store_L) operations :: cq_example_providers :: slice_rows
    stored R at(__cq_store_R) operations :: cq_example_providers ::
    hash_relation stored S at(__cq_store_S) operations :: cq_example_providers
    :: sorted_relation stored l at(__cq_store_l) operations ::
    cq_example_providers :: slice_rows stored r at(__cq_store_r) operations ::
    cq_example_providers :: hash_relation
} request { input s from(__cq_store_S) }
```

Actual output (formatted where Rust syntax permits):

```rust
::mini_linq::resume_physical! {
    resume
    {
        plan
        {
            struct Heterogeneous; relation L(a : Value); relation
            R(a : Value, b : Value); relation S(b : Value, c : Value);
            answer(a, b, c) : - L(a), R(a, b), S(b, c); relational
            {
                l = rename L { a - > a }; r = rename R { a - > a, b - > b }; s
                = rename S { b - > b, c - > c }; j0 = natural_join l with r;
                j1 = natural_join j0 with s; output j1 as answer(a, b, c);
            } indexes { i0 = @ r [0]; i1 = @ s [0]; } uses
            { j0.right = i0; j1.right = i1; } physical
            {
                relations
                {
                    L = :: cq_example_providers :: slice_rows; R = ::
                    cq_example_providers :: hash_relation; S = ::
                    cq_example_providers :: sorted_relation; l = ::
                    cq_example_providers :: slice_rows; r = ::
                    cq_example_providers :: hash_relation; s = ::
                    cq_example_providers :: sorted_relation; j0 = ::
                    cq_example_providers :: borrowed_fields; j1 = ::
                    cq_example_providers :: borrowed_fields;
                } indexes
                {
                    i0 = :: cq_example_providers :: hash_relation; i1 = ::
                    cq_example_providers :: sorted_relation;
                } lowerings
                {
                    l = :: cq_example_providers :: rename; r = ::
                    cq_example_providers :: rename; s = :: cq_example_providers
                    :: rename; j0 = :: cq_example_providers :: indexed_join; j1
                    = :: cq_example_providers :: indexed_join;
                }
            }
        } environment
        {
            stored L at(__cq_store_L) operations :: cq_example_providers ::
            slice_rows stored R at(__cq_store_R) operations ::
            cq_example_providers :: hash_relation stored S at(__cq_store_S)
            operations :: cq_example_providers :: sorted_relation stored l
            at(__cq_store_l) operations :: cq_example_providers :: slice_rows
            stored r at(__cq_store_r) operations :: cq_example_providers ::
            hash_relation
        } emitted
        {
            let __cq_store_L : & [Value] = L; let __cq_store_R : & :: std ::
            collections :: HashMap < Value, :: std :: vec :: Vec < Value > > =
            R; let __cq_store_S : & :: std :: collections :: BTreeMap < Value,
            :: std :: vec :: Vec < Value > > = S; let __cq_store_l : & [Value]
            = __cq_store_L; let __cq_store_r : & :: std :: collections ::
            HashMap < Value, :: std :: vec :: Vec < Value > > = __cq_store_R;
        } pending { lower s }
        continuations(:: mini_linq :: resume_physical, mini_linq :: access)
    } emitted
    {
        let __cq_store_s : & :: std :: collections :: BTreeMap < Value, :: std
        :: vec :: Vec < Value >> = __cq_store_S;
    } result
    {
        stored s at(__cq_store_s) operations :: cq_example_providers ::
        sorted_relation
    }
}

```

---

## 2172042/018-resume_physical

[Raw input](raw/2172042/018-resume_physical.input.tokens) · [Raw output](raw/2172042/018-resume_physical.output.tokens)

Actual input:

```rust
resume
{
    plan
    {
        struct Heterogeneous; relation L(a : Value); relation
        R(a : Value, b : Value); relation S(b : Value, c : Value);
        answer(a, b, c) : - L(a), R(a, b), S(b, c); relational
        {
            l = rename L { a - > a }; r = rename R { a - > a, b - > b }; s =
            rename S { b - > b, c - > c }; j0 = natural_join l with r; j1 =
            natural_join j0 with s; output j1 as answer(a, b, c);
        } indexes { i0 = @ r [0]; i1 = @ s [0]; } uses
        { j0.right = i0; j1.right = i1; } physical
        {
            relations
            {
                L = :: cq_example_providers :: slice_rows; R = ::
                cq_example_providers :: hash_relation; S = ::
                cq_example_providers :: sorted_relation; l = ::
                cq_example_providers :: slice_rows; r = ::
                cq_example_providers :: hash_relation; s = ::
                cq_example_providers :: sorted_relation; j0 = ::
                cq_example_providers :: borrowed_fields; j1 = ::
                cq_example_providers :: borrowed_fields;
            } indexes
            {
                i0 = :: cq_example_providers :: hash_relation; i1 = ::
                cq_example_providers :: sorted_relation;
            } lowerings
            {
                l = :: cq_example_providers :: rename; r = ::
                cq_example_providers :: rename; s = :: cq_example_providers ::
                rename; j0 = :: cq_example_providers :: indexed_join; j1 = ::
                cq_example_providers :: indexed_join;
            }
        }
    } environment
    {
        stored L at(__cq_store_L) operations :: cq_example_providers ::
        slice_rows stored R at(__cq_store_R) operations ::
        cq_example_providers :: hash_relation stored S at(__cq_store_S)
        operations :: cq_example_providers :: sorted_relation stored l
        at(__cq_store_l) operations :: cq_example_providers :: slice_rows
        stored r at(__cq_store_r) operations :: cq_example_providers ::
        hash_relation
    } emitted
    {
        let __cq_store_L : & [Value] = L; let __cq_store_R : & :: std ::
        collections :: HashMap < Value, :: std :: vec :: Vec < Value > > = R;
        let __cq_store_S : & :: std :: collections :: BTreeMap < Value, :: std
        :: vec :: Vec < Value > > = S; let __cq_store_l : & [Value] =
        __cq_store_L; let __cq_store_r : & :: std :: collections :: HashMap <
        Value, :: std :: vec :: Vec < Value > > = __cq_store_R;
    } pending { lower s }
    continuations(:: mini_linq :: resume_physical, mini_linq :: access)
} emitted
{
    let __cq_store_s : & :: std :: collections :: BTreeMap < Value, :: std ::
    vec :: Vec < Value >> = __cq_store_S;
} result
{
    stored s at(__cq_store_s) operations :: cq_example_providers ::
    sorted_relation
}
```

Actual output (formatted where Rust syntax permits):

```rust
::cq_example_providers::hash_relation! {
    reply_to(:: mini_linq :: resume_physical)
    {
        plan
        {
            struct Heterogeneous; relation L(a : Value); relation
            R(a : Value, b : Value); relation S(b : Value, c : Value);
            answer(a, b, c) : - L(a), R(a, b), S(b, c); relational
            {
                l = rename L { a - > a }; r = rename R { a - > a, b - > b }; s
                = rename S { b - > b, c - > c }; j0 = natural_join l with r;
                j1 = natural_join j0 with s; output j1 as answer(a, b, c);
            } indexes { i0 = @ r [0]; i1 = @ s [0]; } uses
            { j0.right = i0; j1.right = i1; } physical
            {
                relations
                {
                    L = :: cq_example_providers :: slice_rows; R = ::
                    cq_example_providers :: hash_relation; S = ::
                    cq_example_providers :: sorted_relation; l = ::
                    cq_example_providers :: slice_rows; r = ::
                    cq_example_providers :: hash_relation; s = ::
                    cq_example_providers :: sorted_relation; j0 = ::
                    cq_example_providers :: borrowed_fields; j1 = ::
                    cq_example_providers :: borrowed_fields;
                } indexes
                {
                    i0 = :: cq_example_providers :: hash_relation; i1 = ::
                    cq_example_providers :: sorted_relation;
                } lowerings
                {
                    l = :: cq_example_providers :: rename; r = ::
                    cq_example_providers :: rename; s = :: cq_example_providers
                    :: rename; j0 = :: cq_example_providers :: indexed_join; j1
                    = :: cq_example_providers :: indexed_join;
                }
            }
        } environment
        {
            stored L at(__cq_store_L) operations :: cq_example_providers ::
            slice_rows stored R at(__cq_store_R) operations ::
            cq_example_providers :: hash_relation stored S at(__cq_store_S)
            operations :: cq_example_providers :: sorted_relation stored l
            at(__cq_store_l) operations :: cq_example_providers :: slice_rows
            stored r at(__cq_store_r) operations :: cq_example_providers ::
            hash_relation stored s at(__cq_store_s) operations ::
            cq_example_providers :: sorted_relation
        } emitted
        {
            let __cq_store_L : & [Value] = L; let __cq_store_R : & :: std ::
            collections :: HashMap < Value, :: std :: vec :: Vec < Value > > =
            R; let __cq_store_S : & :: std :: collections :: BTreeMap < Value,
            :: std :: vec :: Vec < Value > > = S; let __cq_store_l : & [Value]
            = __cq_store_L; let __cq_store_r : & :: std :: collections ::
            HashMap < Value, :: std :: vec :: Vec < Value > > = __cq_store_R;
            let __cq_store_s : & :: std :: collections :: BTreeMap < Value, ::
            std :: vec :: Vec < Value > > = __cq_store_S;
        } pending { index i0 }
        continuations(:: mini_linq :: resume_physical, mini_linq :: access)
    } plan
    {
        struct Heterogeneous; relation L(a : Value); relation
        R(a : Value, b : Value); relation S(b : Value, c : Value);
        answer(a, b, c) : - L(a), R(a, b), S(b, c); relational
        {
            l = rename L { a - > a }; r = rename R { a - > a, b - > b }; s =
            rename S { b - > b, c - > c }; j0 = natural_join l with r; j1 =
            natural_join j0 with s; output j1 as answer(a, b, c);
        } indexes { i0 = @ r [0]; i1 = @ s [0]; } uses
        { j0.right = i0; j1.right = i1; } physical
        {
            relations
            {
                L = :: cq_example_providers :: slice_rows; R = ::
                cq_example_providers :: hash_relation; S = ::
                cq_example_providers :: sorted_relation; l = ::
                cq_example_providers :: slice_rows; r = ::
                cq_example_providers :: hash_relation; s = ::
                cq_example_providers :: sorted_relation; j0 = ::
                cq_example_providers :: borrowed_fields; j1 = ::
                cq_example_providers :: borrowed_fields;
            } indexes
            {
                i0 = :: cq_example_providers :: hash_relation; i1 = ::
                cq_example_providers :: sorted_relation;
            } lowerings
            {
                l = :: cq_example_providers :: rename; r = ::
                cq_example_providers :: rename; s = :: cq_example_providers ::
                rename; j0 = :: cq_example_providers :: indexed_join; j1 = ::
                cq_example_providers :: indexed_join;
            }
        }
    } environment
    {
        stored L at(__cq_store_L) operations :: cq_example_providers ::
        slice_rows stored R at(__cq_store_R) operations ::
        cq_example_providers :: hash_relation stored S at(__cq_store_S)
        operations :: cq_example_providers :: sorted_relation stored l
        at(__cq_store_l) operations :: cq_example_providers :: slice_rows
        stored r at(__cq_store_r) operations :: cq_example_providers ::
        hash_relation stored s at(__cq_store_s) operations ::
        cq_example_providers :: sorted_relation
    } request { index i0 }
}

```

---

## 2172042/019-cq-example-providers::hash_relation

[Raw input](raw/2172042/019-cq-example-providers::hash_relation.input.tokens) · [Raw output](raw/2172042/019-cq-example-providers::hash_relation.output.tokens)

Actual input:

```rust
reply_to(:: mini_linq :: resume_physical)
{
    plan
    {
        struct Heterogeneous; relation L(a : Value); relation
        R(a : Value, b : Value); relation S(b : Value, c : Value);
        answer(a, b, c) : - L(a), R(a, b), S(b, c); relational
        {
            l = rename L { a - > a }; r = rename R { a - > a, b - > b }; s =
            rename S { b - > b, c - > c }; j0 = natural_join l with r; j1 =
            natural_join j0 with s; output j1 as answer(a, b, c);
        } indexes { i0 = @ r [0]; i1 = @ s [0]; } uses
        { j0.right = i0; j1.right = i1; } physical
        {
            relations
            {
                L = :: cq_example_providers :: slice_rows; R = ::
                cq_example_providers :: hash_relation; S = ::
                cq_example_providers :: sorted_relation; l = ::
                cq_example_providers :: slice_rows; r = ::
                cq_example_providers :: hash_relation; s = ::
                cq_example_providers :: sorted_relation; j0 = ::
                cq_example_providers :: borrowed_fields; j1 = ::
                cq_example_providers :: borrowed_fields;
            } indexes
            {
                i0 = :: cq_example_providers :: hash_relation; i1 = ::
                cq_example_providers :: sorted_relation;
            } lowerings
            {
                l = :: cq_example_providers :: rename; r = ::
                cq_example_providers :: rename; s = :: cq_example_providers ::
                rename; j0 = :: cq_example_providers :: indexed_join; j1 = ::
                cq_example_providers :: indexed_join;
            }
        }
    } environment
    {
        stored L at(__cq_store_L) operations :: cq_example_providers ::
        slice_rows stored R at(__cq_store_R) operations ::
        cq_example_providers :: hash_relation stored S at(__cq_store_S)
        operations :: cq_example_providers :: sorted_relation stored l
        at(__cq_store_l) operations :: cq_example_providers :: slice_rows
        stored r at(__cq_store_r) operations :: cq_example_providers ::
        hash_relation stored s at(__cq_store_s) operations ::
        cq_example_providers :: sorted_relation
    } emitted
    {
        let __cq_store_L : & [Value] = L; let __cq_store_R : & :: std ::
        collections :: HashMap < Value, :: std :: vec :: Vec < Value > > = R;
        let __cq_store_S : & :: std :: collections :: BTreeMap < Value, :: std
        :: vec :: Vec < Value > > = S; let __cq_store_l : & [Value] =
        __cq_store_L; let __cq_store_r : & :: std :: collections :: HashMap <
        Value, :: std :: vec :: Vec < Value > > = __cq_store_R; let
        __cq_store_s : & :: std :: collections :: BTreeMap < Value, :: std ::
        vec :: Vec < Value > > = __cq_store_S;
    } pending { index i0 }
    continuations(:: mini_linq :: resume_physical, mini_linq :: access)
} plan
{
    struct Heterogeneous; relation L(a : Value); relation
    R(a : Value, b : Value); relation S(b : Value, c : Value); answer(a, b, c)
    : - L(a), R(a, b), S(b, c); relational
    {
        l = rename L { a - > a }; r = rename R { a - > a, b - > b }; s =
        rename S { b - > b, c - > c }; j0 = natural_join l with r; j1 =
        natural_join j0 with s; output j1 as answer(a, b, c);
    } indexes { i0 = @ r [0]; i1 = @ s [0]; } uses
    { j0.right = i0; j1.right = i1; } physical
    {
        relations
        {
            L = :: cq_example_providers :: slice_rows; R = ::
            cq_example_providers :: hash_relation; S = :: cq_example_providers
            :: sorted_relation; l = :: cq_example_providers :: slice_rows; r =
            :: cq_example_providers :: hash_relation; s = ::
            cq_example_providers :: sorted_relation; j0 = ::
            cq_example_providers :: borrowed_fields; j1 = ::
            cq_example_providers :: borrowed_fields;
        } indexes
        {
            i0 = :: cq_example_providers :: hash_relation; i1 = ::
            cq_example_providers :: sorted_relation;
        } lowerings
        {
            l = :: cq_example_providers :: rename; r = :: cq_example_providers
            :: rename; s = :: cq_example_providers :: rename; j0 = ::
            cq_example_providers :: indexed_join; j1 = :: cq_example_providers
            :: indexed_join;
        }
    }
} environment
{
    stored L at(__cq_store_L) operations :: cq_example_providers :: slice_rows
    stored R at(__cq_store_R) operations :: cq_example_providers ::
    hash_relation stored S at(__cq_store_S) operations :: cq_example_providers
    :: sorted_relation stored l at(__cq_store_l) operations ::
    cq_example_providers :: slice_rows stored r at(__cq_store_r) operations ::
    cq_example_providers :: hash_relation stored s at(__cq_store_s) operations
    :: cq_example_providers :: sorted_relation
} request { index i0 }
```

Actual output (formatted where Rust syntax permits):

```rust
::mini_linq::resume_physical! {
    resume
    {
        plan
        {
            struct Heterogeneous; relation L(a : Value); relation
            R(a : Value, b : Value); relation S(b : Value, c : Value);
            answer(a, b, c) : - L(a), R(a, b), S(b, c); relational
            {
                l = rename L { a - > a }; r = rename R { a - > a, b - > b }; s
                = rename S { b - > b, c - > c }; j0 = natural_join l with r;
                j1 = natural_join j0 with s; output j1 as answer(a, b, c);
            } indexes { i0 = @ r [0]; i1 = @ s [0]; } uses
            { j0.right = i0; j1.right = i1; } physical
            {
                relations
                {
                    L = :: cq_example_providers :: slice_rows; R = ::
                    cq_example_providers :: hash_relation; S = ::
                    cq_example_providers :: sorted_relation; l = ::
                    cq_example_providers :: slice_rows; r = ::
                    cq_example_providers :: hash_relation; s = ::
                    cq_example_providers :: sorted_relation; j0 = ::
                    cq_example_providers :: borrowed_fields; j1 = ::
                    cq_example_providers :: borrowed_fields;
                } indexes
                {
                    i0 = :: cq_example_providers :: hash_relation; i1 = ::
                    cq_example_providers :: sorted_relation;
                } lowerings
                {
                    l = :: cq_example_providers :: rename; r = ::
                    cq_example_providers :: rename; s = :: cq_example_providers
                    :: rename; j0 = :: cq_example_providers :: indexed_join; j1
                    = :: cq_example_providers :: indexed_join;
                }
            }
        } environment
        {
            stored L at(__cq_store_L) operations :: cq_example_providers ::
            slice_rows stored R at(__cq_store_R) operations ::
            cq_example_providers :: hash_relation stored S at(__cq_store_S)
            operations :: cq_example_providers :: sorted_relation stored l
            at(__cq_store_l) operations :: cq_example_providers :: slice_rows
            stored r at(__cq_store_r) operations :: cq_example_providers ::
            hash_relation stored s at(__cq_store_s) operations ::
            cq_example_providers :: sorted_relation
        } emitted
        {
            let __cq_store_L : & [Value] = L; let __cq_store_R : & :: std ::
            collections :: HashMap < Value, :: std :: vec :: Vec < Value > > =
            R; let __cq_store_S : & :: std :: collections :: BTreeMap < Value,
            :: std :: vec :: Vec < Value > > = S; let __cq_store_l : & [Value]
            = __cq_store_L; let __cq_store_r : & :: std :: collections ::
            HashMap < Value, :: std :: vec :: Vec < Value > > = __cq_store_R;
            let __cq_store_s : & :: std :: collections :: BTreeMap < Value, ::
            std :: vec :: Vec < Value > > = __cq_store_S;
        } pending { index i0 }
        continuations(:: mini_linq :: resume_physical, mini_linq :: access)
    } emitted
    {
        let __cq_index_i0 : & :: std :: collections :: HashMap < Value, :: std
        :: vec :: Vec < Value >> = __cq_store_r;
    } result
    {
        indexed i0 at(__cq_index_i0) operations :: cq_example_providers ::
        hash_relation
    }
}

```

---

## 2172042/020-resume_physical

[Raw input](raw/2172042/020-resume_physical.input.tokens) · [Raw output](raw/2172042/020-resume_physical.output.tokens)

Actual input:

```rust
resume
{
    plan
    {
        struct Heterogeneous; relation L(a : Value); relation
        R(a : Value, b : Value); relation S(b : Value, c : Value);
        answer(a, b, c) : - L(a), R(a, b), S(b, c); relational
        {
            l = rename L { a - > a }; r = rename R { a - > a, b - > b }; s =
            rename S { b - > b, c - > c }; j0 = natural_join l with r; j1 =
            natural_join j0 with s; output j1 as answer(a, b, c);
        } indexes { i0 = @ r [0]; i1 = @ s [0]; } uses
        { j0.right = i0; j1.right = i1; } physical
        {
            relations
            {
                L = :: cq_example_providers :: slice_rows; R = ::
                cq_example_providers :: hash_relation; S = ::
                cq_example_providers :: sorted_relation; l = ::
                cq_example_providers :: slice_rows; r = ::
                cq_example_providers :: hash_relation; s = ::
                cq_example_providers :: sorted_relation; j0 = ::
                cq_example_providers :: borrowed_fields; j1 = ::
                cq_example_providers :: borrowed_fields;
            } indexes
            {
                i0 = :: cq_example_providers :: hash_relation; i1 = ::
                cq_example_providers :: sorted_relation;
            } lowerings
            {
                l = :: cq_example_providers :: rename; r = ::
                cq_example_providers :: rename; s = :: cq_example_providers ::
                rename; j0 = :: cq_example_providers :: indexed_join; j1 = ::
                cq_example_providers :: indexed_join;
            }
        }
    } environment
    {
        stored L at(__cq_store_L) operations :: cq_example_providers ::
        slice_rows stored R at(__cq_store_R) operations ::
        cq_example_providers :: hash_relation stored S at(__cq_store_S)
        operations :: cq_example_providers :: sorted_relation stored l
        at(__cq_store_l) operations :: cq_example_providers :: slice_rows
        stored r at(__cq_store_r) operations :: cq_example_providers ::
        hash_relation stored s at(__cq_store_s) operations ::
        cq_example_providers :: sorted_relation
    } emitted
    {
        let __cq_store_L : & [Value] = L; let __cq_store_R : & :: std ::
        collections :: HashMap < Value, :: std :: vec :: Vec < Value > > = R;
        let __cq_store_S : & :: std :: collections :: BTreeMap < Value, :: std
        :: vec :: Vec < Value > > = S; let __cq_store_l : & [Value] =
        __cq_store_L; let __cq_store_r : & :: std :: collections :: HashMap <
        Value, :: std :: vec :: Vec < Value > > = __cq_store_R; let
        __cq_store_s : & :: std :: collections :: BTreeMap < Value, :: std ::
        vec :: Vec < Value > > = __cq_store_S;
    } pending { index i0 }
    continuations(:: mini_linq :: resume_physical, mini_linq :: access)
} emitted
{
    let __cq_index_i0 : & :: std :: collections :: HashMap < Value, :: std ::
    vec :: Vec < Value >> = __cq_store_r;
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
            struct Heterogeneous; relation L(a : Value); relation
            R(a : Value, b : Value); relation S(b : Value, c : Value);
            answer(a, b, c) : - L(a), R(a, b), S(b, c); relational
            {
                l = rename L { a - > a }; r = rename R { a - > a, b - > b }; s
                = rename S { b - > b, c - > c }; j0 = natural_join l with r;
                j1 = natural_join j0 with s; output j1 as answer(a, b, c);
            } indexes { i0 = @ r [0]; i1 = @ s [0]; } uses
            { j0.right = i0; j1.right = i1; } physical
            {
                relations
                {
                    L = :: cq_example_providers :: slice_rows; R = ::
                    cq_example_providers :: hash_relation; S = ::
                    cq_example_providers :: sorted_relation; l = ::
                    cq_example_providers :: slice_rows; r = ::
                    cq_example_providers :: hash_relation; s = ::
                    cq_example_providers :: sorted_relation; j0 = ::
                    cq_example_providers :: borrowed_fields; j1 = ::
                    cq_example_providers :: borrowed_fields;
                } indexes
                {
                    i0 = :: cq_example_providers :: hash_relation; i1 = ::
                    cq_example_providers :: sorted_relation;
                } lowerings
                {
                    l = :: cq_example_providers :: rename; r = ::
                    cq_example_providers :: rename; s = :: cq_example_providers
                    :: rename; j0 = :: cq_example_providers :: indexed_join; j1
                    = :: cq_example_providers :: indexed_join;
                }
            }
        } environment
        {
            stored L at(__cq_store_L) operations :: cq_example_providers ::
            slice_rows stored R at(__cq_store_R) operations ::
            cq_example_providers :: hash_relation stored S at(__cq_store_S)
            operations :: cq_example_providers :: sorted_relation stored l
            at(__cq_store_l) operations :: cq_example_providers :: slice_rows
            stored r at(__cq_store_r) operations :: cq_example_providers ::
            hash_relation stored s at(__cq_store_s) operations ::
            cq_example_providers :: sorted_relation indexed i0
            at(__cq_index_i0) operations :: cq_example_providers ::
            hash_relation
        } emitted
        {
            let __cq_store_L : & [Value] = L; let __cq_store_R : & :: std ::
            collections :: HashMap < Value, :: std :: vec :: Vec < Value > > =
            R; let __cq_store_S : & :: std :: collections :: BTreeMap < Value,
            :: std :: vec :: Vec < Value > > = S; let __cq_store_l : & [Value]
            = __cq_store_L; let __cq_store_r : & :: std :: collections ::
            HashMap < Value, :: std :: vec :: Vec < Value > > = __cq_store_R;
            let __cq_store_s : & :: std :: collections :: BTreeMap < Value, ::
            std :: vec :: Vec < Value > > = __cq_store_S; let __cq_index_i0 :
            & :: std :: collections :: HashMap < Value, :: std :: vec :: Vec <
            Value > > = __cq_store_r;
        } pending { lower j0 }
        continuations(:: mini_linq :: resume_physical, mini_linq :: access)
    } plan
    {
        struct Heterogeneous; relation L(a : Value); relation
        R(a : Value, b : Value); relation S(b : Value, c : Value);
        answer(a, b, c) : - L(a), R(a, b), S(b, c); relational
        {
            l = rename L { a - > a }; r = rename R { a - > a, b - > b }; s =
            rename S { b - > b, c - > c }; j0 = natural_join l with r; j1 =
            natural_join j0 with s; output j1 as answer(a, b, c);
        } indexes { i0 = @ r [0]; i1 = @ s [0]; } uses
        { j0.right = i0; j1.right = i1; } physical
        {
            relations
            {
                L = :: cq_example_providers :: slice_rows; R = ::
                cq_example_providers :: hash_relation; S = ::
                cq_example_providers :: sorted_relation; l = ::
                cq_example_providers :: slice_rows; r = ::
                cq_example_providers :: hash_relation; s = ::
                cq_example_providers :: sorted_relation; j0 = ::
                cq_example_providers :: borrowed_fields; j1 = ::
                cq_example_providers :: borrowed_fields;
            } indexes
            {
                i0 = :: cq_example_providers :: hash_relation; i1 = ::
                cq_example_providers :: sorted_relation;
            } lowerings
            {
                l = :: cq_example_providers :: rename; r = ::
                cq_example_providers :: rename; s = :: cq_example_providers ::
                rename; j0 = :: cq_example_providers :: indexed_join; j1 = ::
                cq_example_providers :: indexed_join;
            }
        }
    } environment
    {
        stored L at(__cq_store_L) operations :: cq_example_providers ::
        slice_rows stored R at(__cq_store_R) operations ::
        cq_example_providers :: hash_relation stored S at(__cq_store_S)
        operations :: cq_example_providers :: sorted_relation stored l
        at(__cq_store_l) operations :: cq_example_providers :: slice_rows
        stored r at(__cq_store_r) operations :: cq_example_providers ::
        hash_relation stored s at(__cq_store_s) operations ::
        cq_example_providers :: sorted_relation indexed i0 at(__cq_index_i0)
        operations :: cq_example_providers :: hash_relation
    } request { lower j0 }
}

```

---

## 2172042/021-cq-example-providers::indexed_join

[Raw input](raw/2172042/021-cq-example-providers::indexed_join.input.tokens) · [Raw output](raw/2172042/021-cq-example-providers::indexed_join.output.tokens)

Actual input:

```rust
reply_to(:: mini_linq :: resume_physical)
{
    plan
    {
        struct Heterogeneous; relation L(a : Value); relation
        R(a : Value, b : Value); relation S(b : Value, c : Value);
        answer(a, b, c) : - L(a), R(a, b), S(b, c); relational
        {
            l = rename L { a - > a }; r = rename R { a - > a, b - > b }; s =
            rename S { b - > b, c - > c }; j0 = natural_join l with r; j1 =
            natural_join j0 with s; output j1 as answer(a, b, c);
        } indexes { i0 = @ r [0]; i1 = @ s [0]; } uses
        { j0.right = i0; j1.right = i1; } physical
        {
            relations
            {
                L = :: cq_example_providers :: slice_rows; R = ::
                cq_example_providers :: hash_relation; S = ::
                cq_example_providers :: sorted_relation; l = ::
                cq_example_providers :: slice_rows; r = ::
                cq_example_providers :: hash_relation; s = ::
                cq_example_providers :: sorted_relation; j0 = ::
                cq_example_providers :: borrowed_fields; j1 = ::
                cq_example_providers :: borrowed_fields;
            } indexes
            {
                i0 = :: cq_example_providers :: hash_relation; i1 = ::
                cq_example_providers :: sorted_relation;
            } lowerings
            {
                l = :: cq_example_providers :: rename; r = ::
                cq_example_providers :: rename; s = :: cq_example_providers ::
                rename; j0 = :: cq_example_providers :: indexed_join; j1 = ::
                cq_example_providers :: indexed_join;
            }
        }
    } environment
    {
        stored L at(__cq_store_L) operations :: cq_example_providers ::
        slice_rows stored R at(__cq_store_R) operations ::
        cq_example_providers :: hash_relation stored S at(__cq_store_S)
        operations :: cq_example_providers :: sorted_relation stored l
        at(__cq_store_l) operations :: cq_example_providers :: slice_rows
        stored r at(__cq_store_r) operations :: cq_example_providers ::
        hash_relation stored s at(__cq_store_s) operations ::
        cq_example_providers :: sorted_relation indexed i0 at(__cq_index_i0)
        operations :: cq_example_providers :: hash_relation
    } emitted
    {
        let __cq_store_L : & [Value] = L; let __cq_store_R : & :: std ::
        collections :: HashMap < Value, :: std :: vec :: Vec < Value > > = R;
        let __cq_store_S : & :: std :: collections :: BTreeMap < Value, :: std
        :: vec :: Vec < Value > > = S; let __cq_store_l : & [Value] =
        __cq_store_L; let __cq_store_r : & :: std :: collections :: HashMap <
        Value, :: std :: vec :: Vec < Value > > = __cq_store_R; let
        __cq_store_s : & :: std :: collections :: BTreeMap < Value, :: std ::
        vec :: Vec < Value > > = __cq_store_S; let __cq_index_i0 : & :: std ::
        collections :: HashMap < Value, :: std :: vec :: Vec < Value > > =
        __cq_store_r;
    } pending { lower j0 }
    continuations(:: mini_linq :: resume_physical, mini_linq :: access)
} plan
{
    struct Heterogeneous; relation L(a : Value); relation
    R(a : Value, b : Value); relation S(b : Value, c : Value); answer(a, b, c)
    : - L(a), R(a, b), S(b, c); relational
    {
        l = rename L { a - > a }; r = rename R { a - > a, b - > b }; s =
        rename S { b - > b, c - > c }; j0 = natural_join l with r; j1 =
        natural_join j0 with s; output j1 as answer(a, b, c);
    } indexes { i0 = @ r [0]; i1 = @ s [0]; } uses
    { j0.right = i0; j1.right = i1; } physical
    {
        relations
        {
            L = :: cq_example_providers :: slice_rows; R = ::
            cq_example_providers :: hash_relation; S = :: cq_example_providers
            :: sorted_relation; l = :: cq_example_providers :: slice_rows; r =
            :: cq_example_providers :: hash_relation; s = ::
            cq_example_providers :: sorted_relation; j0 = ::
            cq_example_providers :: borrowed_fields; j1 = ::
            cq_example_providers :: borrowed_fields;
        } indexes
        {
            i0 = :: cq_example_providers :: hash_relation; i1 = ::
            cq_example_providers :: sorted_relation;
        } lowerings
        {
            l = :: cq_example_providers :: rename; r = :: cq_example_providers
            :: rename; s = :: cq_example_providers :: rename; j0 = ::
            cq_example_providers :: indexed_join; j1 = :: cq_example_providers
            :: indexed_join;
        }
    }
} environment
{
    stored L at(__cq_store_L) operations :: cq_example_providers :: slice_rows
    stored R at(__cq_store_R) operations :: cq_example_providers ::
    hash_relation stored S at(__cq_store_S) operations :: cq_example_providers
    :: sorted_relation stored l at(__cq_store_l) operations ::
    cq_example_providers :: slice_rows stored r at(__cq_store_r) operations ::
    cq_example_providers :: hash_relation stored s at(__cq_store_s) operations
    :: cq_example_providers :: sorted_relation indexed i0 at(__cq_index_i0)
    operations :: cq_example_providers :: hash_relation
} request { lower j0 }
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
                struct Heterogeneous; relation L(a : Value); relation
                R(a : Value, b : Value); relation S(b : Value, c : Value);
                answer(a, b, c) : - L(a), R(a, b), S(b, c); relational
                {
                    l = rename L { a - > a }; r = rename R { a - > a, b - > b };
                    s = rename S { b - > b, c - > c }; j0 = natural_join l with
                    r; j1 = natural_join j0 with s; output j1 as
                    answer(a, b, c);
                } indexes { i0 = @ r [0]; i1 = @ s [0]; } uses
                { j0.right = i0; j1.right = i1; } physical
                {
                    relations
                    {
                        L = :: cq_example_providers :: slice_rows; R = ::
                        cq_example_providers :: hash_relation; S = ::
                        cq_example_providers :: sorted_relation; l = ::
                        cq_example_providers :: slice_rows; r = ::
                        cq_example_providers :: hash_relation; s = ::
                        cq_example_providers :: sorted_relation; j0 = ::
                        cq_example_providers :: borrowed_fields; j1 = ::
                        cq_example_providers :: borrowed_fields;
                    } indexes
                    {
                        i0 = :: cq_example_providers :: hash_relation; i1 = ::
                        cq_example_providers :: sorted_relation;
                    } lowerings
                    {
                        l = :: cq_example_providers :: rename; r = ::
                        cq_example_providers :: rename; s = :: cq_example_providers
                        :: rename; j0 = :: cq_example_providers :: indexed_join; j1
                        = :: cq_example_providers :: indexed_join;
                    }
                }
            } environment
            {
                stored L at(__cq_store_L) operations :: cq_example_providers
                :: slice_rows stored R at(__cq_store_R) operations ::
                cq_example_providers :: hash_relation stored S
                at(__cq_store_S) operations :: cq_example_providers ::
                sorted_relation stored l at(__cq_store_l) operations ::
                cq_example_providers :: slice_rows stored r at(__cq_store_r)
                operations :: cq_example_providers :: hash_relation stored s
                at(__cq_store_s) operations :: cq_example_providers ::
                sorted_relation indexed i0 at(__cq_index_i0) operations ::
                cq_example_providers :: hash_relation
            } emitted
            {
                let __cq_store_L : & [Value] = L; let __cq_store_R : & :: std
                :: collections :: HashMap < Value, :: std :: vec :: Vec <
                Value > > = R; let __cq_store_S : & :: std :: collections ::
                BTreeMap < Value, :: std :: vec :: Vec < Value > > = S; let
                __cq_store_l : & [Value] = __cq_store_L; let __cq_store_r : &
                :: std :: collections :: HashMap < Value, :: std :: vec :: Vec
                < Value > > = __cq_store_R; let __cq_store_s : & :: std ::
                collections :: BTreeMap < Value, :: std :: vec :: Vec < Value
                > > = __cq_store_S; let __cq_index_i0 : & :: std ::
                collections :: HashMap < Value, :: std :: vec :: Vec < Value >
                > = __cq_store_r;
            } pending { lower j0 }
            continuations(:: mini_linq :: resume_physical, mini_linq ::
            access)
        } plan
        {
            struct Heterogeneous; relation L(a : Value); relation
            R(a : Value, b : Value); relation S(b : Value, c : Value);
            answer(a, b, c) : - L(a), R(a, b), S(b, c); relational
            {
                l = rename L { a - > a }; r = rename R { a - > a, b - > b }; s
                = rename S { b - > b, c - > c }; j0 = natural_join l with r;
                j1 = natural_join j0 with s; output j1 as answer(a, b, c);
            } indexes { i0 = @ r [0]; i1 = @ s [0]; } uses
            { j0.right = i0; j1.right = i1; } physical
            {
                relations
                {
                    L = :: cq_example_providers :: slice_rows; R = ::
                    cq_example_providers :: hash_relation; S = ::
                    cq_example_providers :: sorted_relation; l = ::
                    cq_example_providers :: slice_rows; r = ::
                    cq_example_providers :: hash_relation; s = ::
                    cq_example_providers :: sorted_relation; j0 = ::
                    cq_example_providers :: borrowed_fields; j1 = ::
                    cq_example_providers :: borrowed_fields;
                } indexes
                {
                    i0 = :: cq_example_providers :: hash_relation; i1 = ::
                    cq_example_providers :: sorted_relation;
                } lowerings
                {
                    l = :: cq_example_providers :: rename; r = ::
                    cq_example_providers :: rename; s = :: cq_example_providers
                    :: rename; j0 = :: cq_example_providers :: indexed_join; j1
                    = :: cq_example_providers :: indexed_join;
                }
            }
        } environment
        {
            stored L at(__cq_store_L) operations :: cq_example_providers ::
            slice_rows stored R at(__cq_store_R) operations ::
            cq_example_providers :: hash_relation stored S at(__cq_store_S)
            operations :: cq_example_providers :: sorted_relation stored l
            at(__cq_store_l) operations :: cq_example_providers :: slice_rows
            stored r at(__cq_store_r) operations :: cq_example_providers ::
            hash_relation stored s at(__cq_store_s) operations ::
            cq_example_providers :: sorted_relation indexed i0
            at(__cq_index_i0) operations :: cq_example_providers ::
            hash_relation
        } request { lower j0 }
    } plan
    {
        struct Heterogeneous; relation L(a : Value); relation
        R(a : Value, b : Value); relation S(b : Value, c : Value);
        answer(a, b, c) : - L(a), R(a, b), S(b, c); relational
        {
            l = rename L { a - > a }; r = rename R { a - > a, b - > b }; s =
            rename S { b - > b, c - > c }; j0 = natural_join l with r; j1 =
            natural_join j0 with s; output j1 as answer(a, b, c);
        } indexes { i0 = @ r [0]; i1 = @ s [0]; } uses
        { j0.right = i0; j1.right = i1; } physical
        {
            relations
            {
                L = :: cq_example_providers :: slice_rows; R = ::
                cq_example_providers :: hash_relation; S = ::
                cq_example_providers :: sorted_relation; l = ::
                cq_example_providers :: slice_rows; r = ::
                cq_example_providers :: hash_relation; s = ::
                cq_example_providers :: sorted_relation; j0 = ::
                cq_example_providers :: borrowed_fields; j1 = ::
                cq_example_providers :: borrowed_fields;
            } indexes
            {
                i0 = :: cq_example_providers :: hash_relation; i1 = ::
                cq_example_providers :: sorted_relation;
            } lowerings
            {
                l = :: cq_example_providers :: rename; r = ::
                cq_example_providers :: rename; s = :: cq_example_providers ::
                rename; j0 = :: cq_example_providers :: indexed_join; j1 = ::
                cq_example_providers :: indexed_join;
            }
        }
    } environment
    {
        stored L at(__cq_store_L) operations :: cq_example_providers ::
        slice_rows stored R at(__cq_store_R) operations ::
        cq_example_providers :: hash_relation stored S at(__cq_store_S)
        operations :: cq_example_providers :: sorted_relation stored l
        at(__cq_store_l) operations :: cq_example_providers :: slice_rows
        stored r at(__cq_store_r) operations :: cq_example_providers ::
        hash_relation stored s at(__cq_store_s) operations ::
        cq_example_providers :: sorted_relation indexed i0 at(__cq_index_i0)
        operations :: cq_example_providers :: hash_relation
    } request { rows l }
}

```

---

## 2172042/022-cq-example-providers::slice_rows

[Raw input](raw/2172042/022-cq-example-providers::slice_rows.input.tokens) · [Raw output](raw/2172042/022-cq-example-providers::slice_rows.output.tokens)

Actual input:

```rust
reply_to(:: cq_example_providers :: indexed_join_rows)
{
    reply_to(:: mini_linq :: resume_physical)
    {
        plan
        {
            struct Heterogeneous; relation L(a : Value); relation
            R(a : Value, b : Value); relation S(b : Value, c : Value);
            answer(a, b, c) : - L(a), R(a, b), S(b, c); relational
            {
                l = rename L { a - > a }; r = rename R { a - > a, b - > b }; s
                = rename S { b - > b, c - > c }; j0 = natural_join l with r;
                j1 = natural_join j0 with s; output j1 as answer(a, b, c);
            } indexes { i0 = @ r [0]; i1 = @ s [0]; } uses
            { j0.right = i0; j1.right = i1; } physical
            {
                relations
                {
                    L = :: cq_example_providers :: slice_rows; R = ::
                    cq_example_providers :: hash_relation; S = ::
                    cq_example_providers :: sorted_relation; l = ::
                    cq_example_providers :: slice_rows; r = ::
                    cq_example_providers :: hash_relation; s = ::
                    cq_example_providers :: sorted_relation; j0 = ::
                    cq_example_providers :: borrowed_fields; j1 = ::
                    cq_example_providers :: borrowed_fields;
                } indexes
                {
                    i0 = :: cq_example_providers :: hash_relation; i1 = ::
                    cq_example_providers :: sorted_relation;
                } lowerings
                {
                    l = :: cq_example_providers :: rename; r = ::
                    cq_example_providers :: rename; s = :: cq_example_providers
                    :: rename; j0 = :: cq_example_providers :: indexed_join; j1
                    = :: cq_example_providers :: indexed_join;
                }
            }
        } environment
        {
            stored L at(__cq_store_L) operations :: cq_example_providers ::
            slice_rows stored R at(__cq_store_R) operations ::
            cq_example_providers :: hash_relation stored S at(__cq_store_S)
            operations :: cq_example_providers :: sorted_relation stored l
            at(__cq_store_l) operations :: cq_example_providers :: slice_rows
            stored r at(__cq_store_r) operations :: cq_example_providers ::
            hash_relation stored s at(__cq_store_s) operations ::
            cq_example_providers :: sorted_relation indexed i0
            at(__cq_index_i0) operations :: cq_example_providers ::
            hash_relation
        } emitted
        {
            let __cq_store_L : & [Value] = L; let __cq_store_R : & :: std ::
            collections :: HashMap < Value, :: std :: vec :: Vec < Value > > =
            R; let __cq_store_S : & :: std :: collections :: BTreeMap < Value,
            :: std :: vec :: Vec < Value > > = S; let __cq_store_l : & [Value]
            = __cq_store_L; let __cq_store_r : & :: std :: collections ::
            HashMap < Value, :: std :: vec :: Vec < Value > > = __cq_store_R;
            let __cq_store_s : & :: std :: collections :: BTreeMap < Value, ::
            std :: vec :: Vec < Value > > = __cq_store_S; let __cq_index_i0 :
            & :: std :: collections :: HashMap < Value, :: std :: vec :: Vec <
            Value > > = __cq_store_r;
        } pending { lower j0 }
        continuations(:: mini_linq :: resume_physical, mini_linq :: access)
    } plan
    {
        struct Heterogeneous; relation L(a : Value); relation
        R(a : Value, b : Value); relation S(b : Value, c : Value);
        answer(a, b, c) : - L(a), R(a, b), S(b, c); relational
        {
            l = rename L { a - > a }; r = rename R { a - > a, b - > b }; s =
            rename S { b - > b, c - > c }; j0 = natural_join l with r; j1 =
            natural_join j0 with s; output j1 as answer(a, b, c);
        } indexes { i0 = @ r [0]; i1 = @ s [0]; } uses
        { j0.right = i0; j1.right = i1; } physical
        {
            relations
            {
                L = :: cq_example_providers :: slice_rows; R = ::
                cq_example_providers :: hash_relation; S = ::
                cq_example_providers :: sorted_relation; l = ::
                cq_example_providers :: slice_rows; r = ::
                cq_example_providers :: hash_relation; s = ::
                cq_example_providers :: sorted_relation; j0 = ::
                cq_example_providers :: borrowed_fields; j1 = ::
                cq_example_providers :: borrowed_fields;
            } indexes
            {
                i0 = :: cq_example_providers :: hash_relation; i1 = ::
                cq_example_providers :: sorted_relation;
            } lowerings
            {
                l = :: cq_example_providers :: rename; r = ::
                cq_example_providers :: rename; s = :: cq_example_providers ::
                rename; j0 = :: cq_example_providers :: indexed_join; j1 = ::
                cq_example_providers :: indexed_join;
            }
        }
    } environment
    {
        stored L at(__cq_store_L) operations :: cq_example_providers ::
        slice_rows stored R at(__cq_store_R) operations ::
        cq_example_providers :: hash_relation stored S at(__cq_store_S)
        operations :: cq_example_providers :: sorted_relation stored l
        at(__cq_store_l) operations :: cq_example_providers :: slice_rows
        stored r at(__cq_store_r) operations :: cq_example_providers ::
        hash_relation stored s at(__cq_store_s) operations ::
        cq_example_providers :: sorted_relation indexed i0 at(__cq_index_i0)
        operations :: cq_example_providers :: hash_relation
    } request { lower j0 }
} plan
{
    struct Heterogeneous; relation L(a : Value); relation
    R(a : Value, b : Value); relation S(b : Value, c : Value); answer(a, b, c)
    : - L(a), R(a, b), S(b, c); relational
    {
        l = rename L { a - > a }; r = rename R { a - > a, b - > b }; s =
        rename S { b - > b, c - > c }; j0 = natural_join l with r; j1 =
        natural_join j0 with s; output j1 as answer(a, b, c);
    } indexes { i0 = @ r [0]; i1 = @ s [0]; } uses
    { j0.right = i0; j1.right = i1; } physical
    {
        relations
        {
            L = :: cq_example_providers :: slice_rows; R = ::
            cq_example_providers :: hash_relation; S = :: cq_example_providers
            :: sorted_relation; l = :: cq_example_providers :: slice_rows; r =
            :: cq_example_providers :: hash_relation; s = ::
            cq_example_providers :: sorted_relation; j0 = ::
            cq_example_providers :: borrowed_fields; j1 = ::
            cq_example_providers :: borrowed_fields;
        } indexes
        {
            i0 = :: cq_example_providers :: hash_relation; i1 = ::
            cq_example_providers :: sorted_relation;
        } lowerings
        {
            l = :: cq_example_providers :: rename; r = :: cq_example_providers
            :: rename; s = :: cq_example_providers :: rename; j0 = ::
            cq_example_providers :: indexed_join; j1 = :: cq_example_providers
            :: indexed_join;
        }
    }
} environment
{
    stored L at(__cq_store_L) operations :: cq_example_providers :: slice_rows
    stored R at(__cq_store_R) operations :: cq_example_providers ::
    hash_relation stored S at(__cq_store_S) operations :: cq_example_providers
    :: sorted_relation stored l at(__cq_store_l) operations ::
    cq_example_providers :: slice_rows stored r at(__cq_store_r) operations ::
    cq_example_providers :: hash_relation stored s at(__cq_store_s) operations
    :: cq_example_providers :: sorted_relation indexed i0 at(__cq_index_i0)
    operations :: cq_example_providers :: hash_relation
} request { rows l }
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
                struct Heterogeneous; relation L(a : Value); relation
                R(a : Value, b : Value); relation S(b : Value, c : Value);
                answer(a, b, c) : - L(a), R(a, b), S(b, c); relational
                {
                    l = rename L { a - > a }; r = rename R { a - > a, b - > b };
                    s = rename S { b - > b, c - > c }; j0 = natural_join l with
                    r; j1 = natural_join j0 with s; output j1 as
                    answer(a, b, c);
                } indexes { i0 = @ r [0]; i1 = @ s [0]; } uses
                { j0.right = i0; j1.right = i1; } physical
                {
                    relations
                    {
                        L = :: cq_example_providers :: slice_rows; R = ::
                        cq_example_providers :: hash_relation; S = ::
                        cq_example_providers :: sorted_relation; l = ::
                        cq_example_providers :: slice_rows; r = ::
                        cq_example_providers :: hash_relation; s = ::
                        cq_example_providers :: sorted_relation; j0 = ::
                        cq_example_providers :: borrowed_fields; j1 = ::
                        cq_example_providers :: borrowed_fields;
                    } indexes
                    {
                        i0 = :: cq_example_providers :: hash_relation; i1 = ::
                        cq_example_providers :: sorted_relation;
                    } lowerings
                    {
                        l = :: cq_example_providers :: rename; r = ::
                        cq_example_providers :: rename; s = :: cq_example_providers
                        :: rename; j0 = :: cq_example_providers :: indexed_join; j1
                        = :: cq_example_providers :: indexed_join;
                    }
                }
            } environment
            {
                stored L at(__cq_store_L) operations :: cq_example_providers
                :: slice_rows stored R at(__cq_store_R) operations ::
                cq_example_providers :: hash_relation stored S
                at(__cq_store_S) operations :: cq_example_providers ::
                sorted_relation stored l at(__cq_store_l) operations ::
                cq_example_providers :: slice_rows stored r at(__cq_store_r)
                operations :: cq_example_providers :: hash_relation stored s
                at(__cq_store_s) operations :: cq_example_providers ::
                sorted_relation indexed i0 at(__cq_index_i0) operations ::
                cq_example_providers :: hash_relation
            } emitted
            {
                let __cq_store_L : & [Value] = L; let __cq_store_R : & :: std
                :: collections :: HashMap < Value, :: std :: vec :: Vec <
                Value > > = R; let __cq_store_S : & :: std :: collections ::
                BTreeMap < Value, :: std :: vec :: Vec < Value > > = S; let
                __cq_store_l : & [Value] = __cq_store_L; let __cq_store_r : &
                :: std :: collections :: HashMap < Value, :: std :: vec :: Vec
                < Value > > = __cq_store_R; let __cq_store_s : & :: std ::
                collections :: BTreeMap < Value, :: std :: vec :: Vec < Value
                > > = __cq_store_S; let __cq_index_i0 : & :: std ::
                collections :: HashMap < Value, :: std :: vec :: Vec < Value >
                > = __cq_store_r;
            } pending { lower j0 }
            continuations(:: mini_linq :: resume_physical, mini_linq ::
            access)
        } plan
        {
            struct Heterogeneous; relation L(a : Value); relation
            R(a : Value, b : Value); relation S(b : Value, c : Value);
            answer(a, b, c) : - L(a), R(a, b), S(b, c); relational
            {
                l = rename L { a - > a }; r = rename R { a - > a, b - > b }; s
                = rename S { b - > b, c - > c }; j0 = natural_join l with r;
                j1 = natural_join j0 with s; output j1 as answer(a, b, c);
            } indexes { i0 = @ r [0]; i1 = @ s [0]; } uses
            { j0.right = i0; j1.right = i1; } physical
            {
                relations
                {
                    L = :: cq_example_providers :: slice_rows; R = ::
                    cq_example_providers :: hash_relation; S = ::
                    cq_example_providers :: sorted_relation; l = ::
                    cq_example_providers :: slice_rows; r = ::
                    cq_example_providers :: hash_relation; s = ::
                    cq_example_providers :: sorted_relation; j0 = ::
                    cq_example_providers :: borrowed_fields; j1 = ::
                    cq_example_providers :: borrowed_fields;
                } indexes
                {
                    i0 = :: cq_example_providers :: hash_relation; i1 = ::
                    cq_example_providers :: sorted_relation;
                } lowerings
                {
                    l = :: cq_example_providers :: rename; r = ::
                    cq_example_providers :: rename; s = :: cq_example_providers
                    :: rename; j0 = :: cq_example_providers :: indexed_join; j1
                    = :: cq_example_providers :: indexed_join;
                }
            }
        } environment
        {
            stored L at(__cq_store_L) operations :: cq_example_providers ::
            slice_rows stored R at(__cq_store_R) operations ::
            cq_example_providers :: hash_relation stored S at(__cq_store_S)
            operations :: cq_example_providers :: sorted_relation stored l
            at(__cq_store_l) operations :: cq_example_providers :: slice_rows
            stored r at(__cq_store_r) operations :: cq_example_providers ::
            hash_relation stored s at(__cq_store_s) operations ::
            cq_example_providers :: sorted_relation indexed i0
            at(__cq_index_i0) operations :: cq_example_providers ::
            hash_relation
        } request { lower j0 }
    } source((__cq_store_l).iter()) item(a) witness
    { row(a) fields { 0 = (a); } }
}

```

---

## 2172042/023-cq-example-providers::indexed_join_rows

[Raw input](raw/2172042/023-cq-example-providers::indexed_join_rows.input.tokens) · [Raw output](raw/2172042/023-cq-example-providers::indexed_join_rows.output.tokens)

Actual input:

```rust
resume
{
    reply_to(:: mini_linq :: resume_physical)
    {
        plan
        {
            struct Heterogeneous; relation L(a : Value); relation
            R(a : Value, b : Value); relation S(b : Value, c : Value);
            answer(a, b, c) : - L(a), R(a, b), S(b, c); relational
            {
                l = rename L { a - > a }; r = rename R { a - > a, b - > b }; s
                = rename S { b - > b, c - > c }; j0 = natural_join l with r;
                j1 = natural_join j0 with s; output j1 as answer(a, b, c);
            } indexes { i0 = @ r [0]; i1 = @ s [0]; } uses
            { j0.right = i0; j1.right = i1; } physical
            {
                relations
                {
                    L = :: cq_example_providers :: slice_rows; R = ::
                    cq_example_providers :: hash_relation; S = ::
                    cq_example_providers :: sorted_relation; l = ::
                    cq_example_providers :: slice_rows; r = ::
                    cq_example_providers :: hash_relation; s = ::
                    cq_example_providers :: sorted_relation; j0 = ::
                    cq_example_providers :: borrowed_fields; j1 = ::
                    cq_example_providers :: borrowed_fields;
                } indexes
                {
                    i0 = :: cq_example_providers :: hash_relation; i1 = ::
                    cq_example_providers :: sorted_relation;
                } lowerings
                {
                    l = :: cq_example_providers :: rename; r = ::
                    cq_example_providers :: rename; s = :: cq_example_providers
                    :: rename; j0 = :: cq_example_providers :: indexed_join; j1
                    = :: cq_example_providers :: indexed_join;
                }
            }
        } environment
        {
            stored L at(__cq_store_L) operations :: cq_example_providers ::
            slice_rows stored R at(__cq_store_R) operations ::
            cq_example_providers :: hash_relation stored S at(__cq_store_S)
            operations :: cq_example_providers :: sorted_relation stored l
            at(__cq_store_l) operations :: cq_example_providers :: slice_rows
            stored r at(__cq_store_r) operations :: cq_example_providers ::
            hash_relation stored s at(__cq_store_s) operations ::
            cq_example_providers :: sorted_relation indexed i0
            at(__cq_index_i0) operations :: cq_example_providers ::
            hash_relation
        } emitted
        {
            let __cq_store_L : & [Value] = L; let __cq_store_R : & :: std ::
            collections :: HashMap < Value, :: std :: vec :: Vec < Value > > =
            R; let __cq_store_S : & :: std :: collections :: BTreeMap < Value,
            :: std :: vec :: Vec < Value > > = S; let __cq_store_l : & [Value]
            = __cq_store_L; let __cq_store_r : & :: std :: collections ::
            HashMap < Value, :: std :: vec :: Vec < Value > > = __cq_store_R;
            let __cq_store_s : & :: std :: collections :: BTreeMap < Value, ::
            std :: vec :: Vec < Value > > = __cq_store_S; let __cq_index_i0 :
            & :: std :: collections :: HashMap < Value, :: std :: vec :: Vec <
            Value > > = __cq_store_r;
        } pending { lower j0 }
        continuations(:: mini_linq :: resume_physical, mini_linq :: access)
    } plan
    {
        struct Heterogeneous; relation L(a : Value); relation
        R(a : Value, b : Value); relation S(b : Value, c : Value);
        answer(a, b, c) : - L(a), R(a, b), S(b, c); relational
        {
            l = rename L { a - > a }; r = rename R { a - > a, b - > b }; s =
            rename S { b - > b, c - > c }; j0 = natural_join l with r; j1 =
            natural_join j0 with s; output j1 as answer(a, b, c);
        } indexes { i0 = @ r [0]; i1 = @ s [0]; } uses
        { j0.right = i0; j1.right = i1; } physical
        {
            relations
            {
                L = :: cq_example_providers :: slice_rows; R = ::
                cq_example_providers :: hash_relation; S = ::
                cq_example_providers :: sorted_relation; l = ::
                cq_example_providers :: slice_rows; r = ::
                cq_example_providers :: hash_relation; s = ::
                cq_example_providers :: sorted_relation; j0 = ::
                cq_example_providers :: borrowed_fields; j1 = ::
                cq_example_providers :: borrowed_fields;
            } indexes
            {
                i0 = :: cq_example_providers :: hash_relation; i1 = ::
                cq_example_providers :: sorted_relation;
            } lowerings
            {
                l = :: cq_example_providers :: rename; r = ::
                cq_example_providers :: rename; s = :: cq_example_providers ::
                rename; j0 = :: cq_example_providers :: indexed_join; j1 = ::
                cq_example_providers :: indexed_join;
            }
        }
    } environment
    {
        stored L at(__cq_store_L) operations :: cq_example_providers ::
        slice_rows stored R at(__cq_store_R) operations ::
        cq_example_providers :: hash_relation stored S at(__cq_store_S)
        operations :: cq_example_providers :: sorted_relation stored l
        at(__cq_store_l) operations :: cq_example_providers :: slice_rows
        stored r at(__cq_store_r) operations :: cq_example_providers ::
        hash_relation stored s at(__cq_store_s) operations ::
        cq_example_providers :: sorted_relation indexed i0 at(__cq_index_i0)
        operations :: cq_example_providers :: hash_relation
    } request { lower j0 }
} source((__cq_store_l).iter()) item(a) witness { row(a) fields { 0 = (a); } }
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
                    struct Heterogeneous; relation L(a : Value); relation
                    R(a : Value, b : Value); relation S(b : Value, c : Value);
                    answer(a, b, c) : - L(a), R(a, b), S(b, c); relational
                    {
                        l = rename L { a - > a }; r = rename R { a - > a, b - > b };
                        s = rename S { b - > b, c - > c }; j0 = natural_join l with
                        r; j1 = natural_join j0 with s; output j1 as
                        answer(a, b, c);
                    } indexes { i0 = @ r [0]; i1 = @ s [0]; } uses
                    { j0.right = i0; j1.right = i1; } physical
                    {
                        relations
                        {
                            L = :: cq_example_providers :: slice_rows; R = ::
                            cq_example_providers :: hash_relation; S = ::
                            cq_example_providers :: sorted_relation; l = ::
                            cq_example_providers :: slice_rows; r = ::
                            cq_example_providers :: hash_relation; s = ::
                            cq_example_providers :: sorted_relation; j0 = ::
                            cq_example_providers :: borrowed_fields; j1 = ::
                            cq_example_providers :: borrowed_fields;
                        } indexes
                        {
                            i0 = :: cq_example_providers :: hash_relation; i1 = ::
                            cq_example_providers :: sorted_relation;
                        } lowerings
                        {
                            l = :: cq_example_providers :: rename; r = ::
                            cq_example_providers :: rename; s = :: cq_example_providers
                            :: rename; j0 = :: cq_example_providers :: indexed_join; j1
                            = :: cq_example_providers :: indexed_join;
                        }
                    }
                } environment
                {
                    stored L at(__cq_store_L) operations :: cq_example_providers
                    :: slice_rows stored R at(__cq_store_R) operations ::
                    cq_example_providers :: hash_relation stored S
                    at(__cq_store_S) operations :: cq_example_providers ::
                    sorted_relation stored l at(__cq_store_l) operations ::
                    cq_example_providers :: slice_rows stored r at(__cq_store_r)
                    operations :: cq_example_providers :: hash_relation stored s
                    at(__cq_store_s) operations :: cq_example_providers ::
                    sorted_relation indexed i0 at(__cq_index_i0) operations ::
                    cq_example_providers :: hash_relation
                } emitted
                {
                    let __cq_store_L : & [Value] = L; let __cq_store_R : & ::
                    std :: collections :: HashMap < Value, :: std :: vec :: Vec
                    < Value > > = R; let __cq_store_S : & :: std :: collections
                    :: BTreeMap < Value, :: std :: vec :: Vec < Value > > = S;
                    let __cq_store_l : & [Value] = __cq_store_L; let
                    __cq_store_r : & :: std :: collections :: HashMap < Value,
                    :: std :: vec :: Vec < Value > > = __cq_store_R; let
                    __cq_store_s : & :: std :: collections :: BTreeMap < Value,
                    :: std :: vec :: Vec < Value > > = __cq_store_S; let
                    __cq_index_i0 : & :: std :: collections :: HashMap < Value,
                    :: std :: vec :: Vec < Value > > = __cq_store_r;
                } pending { lower j0 }
                continuations(:: mini_linq :: resume_physical, mini_linq ::
                access)
            } plan
            {
                struct Heterogeneous; relation L(a : Value); relation
                R(a : Value, b : Value); relation S(b : Value, c : Value);
                answer(a, b, c) : - L(a), R(a, b), S(b, c); relational
                {
                    l = rename L { a - > a }; r = rename R { a - > a, b - > b };
                    s = rename S { b - > b, c - > c }; j0 = natural_join l with
                    r; j1 = natural_join j0 with s; output j1 as
                    answer(a, b, c);
                } indexes { i0 = @ r [0]; i1 = @ s [0]; } uses
                { j0.right = i0; j1.right = i1; } physical
                {
                    relations
                    {
                        L = :: cq_example_providers :: slice_rows; R = ::
                        cq_example_providers :: hash_relation; S = ::
                        cq_example_providers :: sorted_relation; l = ::
                        cq_example_providers :: slice_rows; r = ::
                        cq_example_providers :: hash_relation; s = ::
                        cq_example_providers :: sorted_relation; j0 = ::
                        cq_example_providers :: borrowed_fields; j1 = ::
                        cq_example_providers :: borrowed_fields;
                    } indexes
                    {
                        i0 = :: cq_example_providers :: hash_relation; i1 = ::
                        cq_example_providers :: sorted_relation;
                    } lowerings
                    {
                        l = :: cq_example_providers :: rename; r = ::
                        cq_example_providers :: rename; s = :: cq_example_providers
                        :: rename; j0 = :: cq_example_providers :: indexed_join; j1
                        = :: cq_example_providers :: indexed_join;
                    }
                }
            } environment
            {
                stored L at(__cq_store_L) operations :: cq_example_providers
                :: slice_rows stored R at(__cq_store_R) operations ::
                cq_example_providers :: hash_relation stored S
                at(__cq_store_S) operations :: cq_example_providers ::
                sorted_relation stored l at(__cq_store_l) operations ::
                cq_example_providers :: slice_rows stored r at(__cq_store_r)
                operations :: cq_example_providers :: hash_relation stored s
                at(__cq_store_s) operations :: cq_example_providers ::
                sorted_relation indexed i0 at(__cq_index_i0) operations ::
                cq_example_providers :: hash_relation
            } request { lower j0 }
        } access { l(a) => bind a <- ((__cq_store_l).iter()) } witness
        { row(a) fields { 0 = (a); } }
    } plan
    {
        struct Heterogeneous; relation L(a : Value); relation
        R(a : Value, b : Value); relation S(b : Value, c : Value);
        answer(a, b, c) : - L(a), R(a, b), S(b, c); relational
        {
            l = rename L { a - > a }; r = rename R { a - > a, b - > b }; s =
            rename S { b - > b, c - > c }; j0 = natural_join l with r; j1 =
            natural_join j0 with s; output j1 as answer(a, b, c);
        } indexes { i0 = @ r [0]; i1 = @ s [0]; } uses
        { j0.right = i0; j1.right = i1; } physical
        {
            relations
            {
                L = :: cq_example_providers :: slice_rows; R = ::
                cq_example_providers :: hash_relation; S = ::
                cq_example_providers :: sorted_relation; l = ::
                cq_example_providers :: slice_rows; r = ::
                cq_example_providers :: hash_relation; s = ::
                cq_example_providers :: sorted_relation; j0 = ::
                cq_example_providers :: borrowed_fields; j1 = ::
                cq_example_providers :: borrowed_fields;
            } indexes
            {
                i0 = :: cq_example_providers :: hash_relation; i1 = ::
                cq_example_providers :: sorted_relation;
            } lowerings
            {
                l = :: cq_example_providers :: rename; r = ::
                cq_example_providers :: rename; s = :: cq_example_providers ::
                rename; j0 = :: cq_example_providers :: indexed_join; j1 = ::
                cq_example_providers :: indexed_join;
            }
        }
    } environment
    {
        stored L at(__cq_store_L) operations :: cq_example_providers ::
        slice_rows stored R at(__cq_store_R) operations ::
        cq_example_providers :: hash_relation stored S at(__cq_store_S)
        operations :: cq_example_providers :: sorted_relation stored l
        at(__cq_store_l) operations :: cq_example_providers :: slice_rows
        stored r at(__cq_store_r) operations :: cq_example_providers ::
        hash_relation stored s at(__cq_store_s) operations ::
        cq_example_providers :: sorted_relation indexed i0 at(__cq_index_i0)
        operations :: cq_example_providers :: hash_relation
    } request { matches i0 values [(a)] }
}

```

---

## 2172042/024-cq-example-providers::hash_relation

[Raw input](raw/2172042/024-cq-example-providers::hash_relation.input.tokens) · [Raw output](raw/2172042/024-cq-example-providers::hash_relation.output.tokens)

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
                struct Heterogeneous; relation L(a : Value); relation
                R(a : Value, b : Value); relation S(b : Value, c : Value);
                answer(a, b, c) : - L(a), R(a, b), S(b, c); relational
                {
                    l = rename L { a - > a }; r = rename R { a - > a, b - > b };
                    s = rename S { b - > b, c - > c }; j0 = natural_join l with
                    r; j1 = natural_join j0 with s; output j1 as
                    answer(a, b, c);
                } indexes { i0 = @ r [0]; i1 = @ s [0]; } uses
                { j0.right = i0; j1.right = i1; } physical
                {
                    relations
                    {
                        L = :: cq_example_providers :: slice_rows; R = ::
                        cq_example_providers :: hash_relation; S = ::
                        cq_example_providers :: sorted_relation; l = ::
                        cq_example_providers :: slice_rows; r = ::
                        cq_example_providers :: hash_relation; s = ::
                        cq_example_providers :: sorted_relation; j0 = ::
                        cq_example_providers :: borrowed_fields; j1 = ::
                        cq_example_providers :: borrowed_fields;
                    } indexes
                    {
                        i0 = :: cq_example_providers :: hash_relation; i1 = ::
                        cq_example_providers :: sorted_relation;
                    } lowerings
                    {
                        l = :: cq_example_providers :: rename; r = ::
                        cq_example_providers :: rename; s = :: cq_example_providers
                        :: rename; j0 = :: cq_example_providers :: indexed_join; j1
                        = :: cq_example_providers :: indexed_join;
                    }
                }
            } environment
            {
                stored L at(__cq_store_L) operations :: cq_example_providers
                :: slice_rows stored R at(__cq_store_R) operations ::
                cq_example_providers :: hash_relation stored S
                at(__cq_store_S) operations :: cq_example_providers ::
                sorted_relation stored l at(__cq_store_l) operations ::
                cq_example_providers :: slice_rows stored r at(__cq_store_r)
                operations :: cq_example_providers :: hash_relation stored s
                at(__cq_store_s) operations :: cq_example_providers ::
                sorted_relation indexed i0 at(__cq_index_i0) operations ::
                cq_example_providers :: hash_relation
            } emitted
            {
                let __cq_store_L : & [Value] = L; let __cq_store_R : & :: std
                :: collections :: HashMap < Value, :: std :: vec :: Vec <
                Value > > = R; let __cq_store_S : & :: std :: collections ::
                BTreeMap < Value, :: std :: vec :: Vec < Value > > = S; let
                __cq_store_l : & [Value] = __cq_store_L; let __cq_store_r : &
                :: std :: collections :: HashMap < Value, :: std :: vec :: Vec
                < Value > > = __cq_store_R; let __cq_store_s : & :: std ::
                collections :: BTreeMap < Value, :: std :: vec :: Vec < Value
                > > = __cq_store_S; let __cq_index_i0 : & :: std ::
                collections :: HashMap < Value, :: std :: vec :: Vec < Value >
                > = __cq_store_r;
            } pending { lower j0 }
            continuations(:: mini_linq :: resume_physical, mini_linq ::
            access)
        } plan
        {
            struct Heterogeneous; relation L(a : Value); relation
            R(a : Value, b : Value); relation S(b : Value, c : Value);
            answer(a, b, c) : - L(a), R(a, b), S(b, c); relational
            {
                l = rename L { a - > a }; r = rename R { a - > a, b - > b }; s
                = rename S { b - > b, c - > c }; j0 = natural_join l with r;
                j1 = natural_join j0 with s; output j1 as answer(a, b, c);
            } indexes { i0 = @ r [0]; i1 = @ s [0]; } uses
            { j0.right = i0; j1.right = i1; } physical
            {
                relations
                {
                    L = :: cq_example_providers :: slice_rows; R = ::
                    cq_example_providers :: hash_relation; S = ::
                    cq_example_providers :: sorted_relation; l = ::
                    cq_example_providers :: slice_rows; r = ::
                    cq_example_providers :: hash_relation; s = ::
                    cq_example_providers :: sorted_relation; j0 = ::
                    cq_example_providers :: borrowed_fields; j1 = ::
                    cq_example_providers :: borrowed_fields;
                } indexes
                {
                    i0 = :: cq_example_providers :: hash_relation; i1 = ::
                    cq_example_providers :: sorted_relation;
                } lowerings
                {
                    l = :: cq_example_providers :: rename; r = ::
                    cq_example_providers :: rename; s = :: cq_example_providers
                    :: rename; j0 = :: cq_example_providers :: indexed_join; j1
                    = :: cq_example_providers :: indexed_join;
                }
            }
        } environment
        {
            stored L at(__cq_store_L) operations :: cq_example_providers ::
            slice_rows stored R at(__cq_store_R) operations ::
            cq_example_providers :: hash_relation stored S at(__cq_store_S)
            operations :: cq_example_providers :: sorted_relation stored l
            at(__cq_store_l) operations :: cq_example_providers :: slice_rows
            stored r at(__cq_store_r) operations :: cq_example_providers ::
            hash_relation stored s at(__cq_store_s) operations ::
            cq_example_providers :: sorted_relation indexed i0
            at(__cq_index_i0) operations :: cq_example_providers ::
            hash_relation
        } request { lower j0 }
    } access { l(a) => bind a <- ((__cq_store_l).iter()) } witness
    { row(a) fields { 0 = (a); } }
} plan
{
    struct Heterogeneous; relation L(a : Value); relation
    R(a : Value, b : Value); relation S(b : Value, c : Value); answer(a, b, c)
    : - L(a), R(a, b), S(b, c); relational
    {
        l = rename L { a - > a }; r = rename R { a - > a, b - > b }; s =
        rename S { b - > b, c - > c }; j0 = natural_join l with r; j1 =
        natural_join j0 with s; output j1 as answer(a, b, c);
    } indexes { i0 = @ r [0]; i1 = @ s [0]; } uses
    { j0.right = i0; j1.right = i1; } physical
    {
        relations
        {
            L = :: cq_example_providers :: slice_rows; R = ::
            cq_example_providers :: hash_relation; S = :: cq_example_providers
            :: sorted_relation; l = :: cq_example_providers :: slice_rows; r =
            :: cq_example_providers :: hash_relation; s = ::
            cq_example_providers :: sorted_relation; j0 = ::
            cq_example_providers :: borrowed_fields; j1 = ::
            cq_example_providers :: borrowed_fields;
        } indexes
        {
            i0 = :: cq_example_providers :: hash_relation; i1 = ::
            cq_example_providers :: sorted_relation;
        } lowerings
        {
            l = :: cq_example_providers :: rename; r = :: cq_example_providers
            :: rename; s = :: cq_example_providers :: rename; j0 = ::
            cq_example_providers :: indexed_join; j1 = :: cq_example_providers
            :: indexed_join;
        }
    }
} environment
{
    stored L at(__cq_store_L) operations :: cq_example_providers :: slice_rows
    stored R at(__cq_store_R) operations :: cq_example_providers ::
    hash_relation stored S at(__cq_store_S) operations :: cq_example_providers
    :: sorted_relation stored l at(__cq_store_l) operations ::
    cq_example_providers :: slice_rows stored r at(__cq_store_r) operations ::
    cq_example_providers :: hash_relation stored s at(__cq_store_s) operations
    :: cq_example_providers :: sorted_relation indexed i0 at(__cq_index_i0)
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
                    struct Heterogeneous; relation L(a : Value); relation
                    R(a : Value, b : Value); relation S(b : Value, c : Value);
                    answer(a, b, c) : - L(a), R(a, b), S(b, c); relational
                    {
                        l = rename L { a - > a }; r = rename R { a - > a, b - > b };
                        s = rename S { b - > b, c - > c }; j0 = natural_join l with
                        r; j1 = natural_join j0 with s; output j1 as
                        answer(a, b, c);
                    } indexes { i0 = @ r [0]; i1 = @ s [0]; } uses
                    { j0.right = i0; j1.right = i1; } physical
                    {
                        relations
                        {
                            L = :: cq_example_providers :: slice_rows; R = ::
                            cq_example_providers :: hash_relation; S = ::
                            cq_example_providers :: sorted_relation; l = ::
                            cq_example_providers :: slice_rows; r = ::
                            cq_example_providers :: hash_relation; s = ::
                            cq_example_providers :: sorted_relation; j0 = ::
                            cq_example_providers :: borrowed_fields; j1 = ::
                            cq_example_providers :: borrowed_fields;
                        } indexes
                        {
                            i0 = :: cq_example_providers :: hash_relation; i1 = ::
                            cq_example_providers :: sorted_relation;
                        } lowerings
                        {
                            l = :: cq_example_providers :: rename; r = ::
                            cq_example_providers :: rename; s = :: cq_example_providers
                            :: rename; j0 = :: cq_example_providers :: indexed_join; j1
                            = :: cq_example_providers :: indexed_join;
                        }
                    }
                } environment
                {
                    stored L at(__cq_store_L) operations :: cq_example_providers
                    :: slice_rows stored R at(__cq_store_R) operations ::
                    cq_example_providers :: hash_relation stored S
                    at(__cq_store_S) operations :: cq_example_providers ::
                    sorted_relation stored l at(__cq_store_l) operations ::
                    cq_example_providers :: slice_rows stored r at(__cq_store_r)
                    operations :: cq_example_providers :: hash_relation stored s
                    at(__cq_store_s) operations :: cq_example_providers ::
                    sorted_relation indexed i0 at(__cq_index_i0) operations ::
                    cq_example_providers :: hash_relation
                } emitted
                {
                    let __cq_store_L : & [Value] = L; let __cq_store_R : & ::
                    std :: collections :: HashMap < Value, :: std :: vec :: Vec
                    < Value > > = R; let __cq_store_S : & :: std :: collections
                    :: BTreeMap < Value, :: std :: vec :: Vec < Value > > = S;
                    let __cq_store_l : & [Value] = __cq_store_L; let
                    __cq_store_r : & :: std :: collections :: HashMap < Value,
                    :: std :: vec :: Vec < Value > > = __cq_store_R; let
                    __cq_store_s : & :: std :: collections :: BTreeMap < Value,
                    :: std :: vec :: Vec < Value > > = __cq_store_S; let
                    __cq_index_i0 : & :: std :: collections :: HashMap < Value,
                    :: std :: vec :: Vec < Value > > = __cq_store_r;
                } pending { lower j0 }
                continuations(:: mini_linq :: resume_physical, mini_linq ::
                access)
            } plan
            {
                struct Heterogeneous; relation L(a : Value); relation
                R(a : Value, b : Value); relation S(b : Value, c : Value);
                answer(a, b, c) : - L(a), R(a, b), S(b, c); relational
                {
                    l = rename L { a - > a }; r = rename R { a - > a, b - > b };
                    s = rename S { b - > b, c - > c }; j0 = natural_join l with
                    r; j1 = natural_join j0 with s; output j1 as
                    answer(a, b, c);
                } indexes { i0 = @ r [0]; i1 = @ s [0]; } uses
                { j0.right = i0; j1.right = i1; } physical
                {
                    relations
                    {
                        L = :: cq_example_providers :: slice_rows; R = ::
                        cq_example_providers :: hash_relation; S = ::
                        cq_example_providers :: sorted_relation; l = ::
                        cq_example_providers :: slice_rows; r = ::
                        cq_example_providers :: hash_relation; s = ::
                        cq_example_providers :: sorted_relation; j0 = ::
                        cq_example_providers :: borrowed_fields; j1 = ::
                        cq_example_providers :: borrowed_fields;
                    } indexes
                    {
                        i0 = :: cq_example_providers :: hash_relation; i1 = ::
                        cq_example_providers :: sorted_relation;
                    } lowerings
                    {
                        l = :: cq_example_providers :: rename; r = ::
                        cq_example_providers :: rename; s = :: cq_example_providers
                        :: rename; j0 = :: cq_example_providers :: indexed_join; j1
                        = :: cq_example_providers :: indexed_join;
                    }
                }
            } environment
            {
                stored L at(__cq_store_L) operations :: cq_example_providers
                :: slice_rows stored R at(__cq_store_R) operations ::
                cq_example_providers :: hash_relation stored S
                at(__cq_store_S) operations :: cq_example_providers ::
                sorted_relation stored l at(__cq_store_l) operations ::
                cq_example_providers :: slice_rows stored r at(__cq_store_r)
                operations :: cq_example_providers :: hash_relation stored s
                at(__cq_store_s) operations :: cq_example_providers ::
                sorted_relation indexed i0 at(__cq_index_i0) operations ::
                cq_example_providers :: hash_relation
            } request { lower j0 }
        } access { l(a) => bind a <- ((__cq_store_l).iter()) } witness
        { row(a) fields { 0 = (a); } }
    } source((__cq_index_i0).get(a).into_iter().flatten()) item(b) witness
    { row((a, b,)) fields { 0 = (a); 1 = (b); } }
}

```

---

## 2172042/025-cq-example-providers::indexed_join_matches

[Raw input](raw/2172042/025-cq-example-providers::indexed_join_matches.input.tokens) · [Raw output](raw/2172042/025-cq-example-providers::indexed_join_matches.output.tokens)

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
                struct Heterogeneous; relation L(a : Value); relation
                R(a : Value, b : Value); relation S(b : Value, c : Value);
                answer(a, b, c) : - L(a), R(a, b), S(b, c); relational
                {
                    l = rename L { a - > a }; r = rename R { a - > a, b - > b };
                    s = rename S { b - > b, c - > c }; j0 = natural_join l with
                    r; j1 = natural_join j0 with s; output j1 as
                    answer(a, b, c);
                } indexes { i0 = @ r [0]; i1 = @ s [0]; } uses
                { j0.right = i0; j1.right = i1; } physical
                {
                    relations
                    {
                        L = :: cq_example_providers :: slice_rows; R = ::
                        cq_example_providers :: hash_relation; S = ::
                        cq_example_providers :: sorted_relation; l = ::
                        cq_example_providers :: slice_rows; r = ::
                        cq_example_providers :: hash_relation; s = ::
                        cq_example_providers :: sorted_relation; j0 = ::
                        cq_example_providers :: borrowed_fields; j1 = ::
                        cq_example_providers :: borrowed_fields;
                    } indexes
                    {
                        i0 = :: cq_example_providers :: hash_relation; i1 = ::
                        cq_example_providers :: sorted_relation;
                    } lowerings
                    {
                        l = :: cq_example_providers :: rename; r = ::
                        cq_example_providers :: rename; s = :: cq_example_providers
                        :: rename; j0 = :: cq_example_providers :: indexed_join; j1
                        = :: cq_example_providers :: indexed_join;
                    }
                }
            } environment
            {
                stored L at(__cq_store_L) operations :: cq_example_providers
                :: slice_rows stored R at(__cq_store_R) operations ::
                cq_example_providers :: hash_relation stored S
                at(__cq_store_S) operations :: cq_example_providers ::
                sorted_relation stored l at(__cq_store_l) operations ::
                cq_example_providers :: slice_rows stored r at(__cq_store_r)
                operations :: cq_example_providers :: hash_relation stored s
                at(__cq_store_s) operations :: cq_example_providers ::
                sorted_relation indexed i0 at(__cq_index_i0) operations ::
                cq_example_providers :: hash_relation
            } emitted
            {
                let __cq_store_L : & [Value] = L; let __cq_store_R : & :: std
                :: collections :: HashMap < Value, :: std :: vec :: Vec <
                Value > > = R; let __cq_store_S : & :: std :: collections ::
                BTreeMap < Value, :: std :: vec :: Vec < Value > > = S; let
                __cq_store_l : & [Value] = __cq_store_L; let __cq_store_r : &
                :: std :: collections :: HashMap < Value, :: std :: vec :: Vec
                < Value > > = __cq_store_R; let __cq_store_s : & :: std ::
                collections :: BTreeMap < Value, :: std :: vec :: Vec < Value
                > > = __cq_store_S; let __cq_index_i0 : & :: std ::
                collections :: HashMap < Value, :: std :: vec :: Vec < Value >
                > = __cq_store_r;
            } pending { lower j0 }
            continuations(:: mini_linq :: resume_physical, mini_linq ::
            access)
        } plan
        {
            struct Heterogeneous; relation L(a : Value); relation
            R(a : Value, b : Value); relation S(b : Value, c : Value);
            answer(a, b, c) : - L(a), R(a, b), S(b, c); relational
            {
                l = rename L { a - > a }; r = rename R { a - > a, b - > b }; s
                = rename S { b - > b, c - > c }; j0 = natural_join l with r;
                j1 = natural_join j0 with s; output j1 as answer(a, b, c);
            } indexes { i0 = @ r [0]; i1 = @ s [0]; } uses
            { j0.right = i0; j1.right = i1; } physical
            {
                relations
                {
                    L = :: cq_example_providers :: slice_rows; R = ::
                    cq_example_providers :: hash_relation; S = ::
                    cq_example_providers :: sorted_relation; l = ::
                    cq_example_providers :: slice_rows; r = ::
                    cq_example_providers :: hash_relation; s = ::
                    cq_example_providers :: sorted_relation; j0 = ::
                    cq_example_providers :: borrowed_fields; j1 = ::
                    cq_example_providers :: borrowed_fields;
                } indexes
                {
                    i0 = :: cq_example_providers :: hash_relation; i1 = ::
                    cq_example_providers :: sorted_relation;
                } lowerings
                {
                    l = :: cq_example_providers :: rename; r = ::
                    cq_example_providers :: rename; s = :: cq_example_providers
                    :: rename; j0 = :: cq_example_providers :: indexed_join; j1
                    = :: cq_example_providers :: indexed_join;
                }
            }
        } environment
        {
            stored L at(__cq_store_L) operations :: cq_example_providers ::
            slice_rows stored R at(__cq_store_R) operations ::
            cq_example_providers :: hash_relation stored S at(__cq_store_S)
            operations :: cq_example_providers :: sorted_relation stored l
            at(__cq_store_l) operations :: cq_example_providers :: slice_rows
            stored r at(__cq_store_r) operations :: cq_example_providers ::
            hash_relation stored s at(__cq_store_s) operations ::
            cq_example_providers :: sorted_relation indexed i0
            at(__cq_index_i0) operations :: cq_example_providers ::
            hash_relation
        } request { lower j0 }
    } access { l(a) => bind a <- ((__cq_store_l).iter()) } witness
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
                    struct Heterogeneous; relation L(a : Value); relation
                    R(a : Value, b : Value); relation S(b : Value, c : Value);
                    answer(a, b, c) : - L(a), R(a, b), S(b, c); relational
                    {
                        l = rename L { a - > a }; r = rename R { a - > a, b - > b };
                        s = rename S { b - > b, c - > c }; j0 = natural_join l with
                        r; j1 = natural_join j0 with s; output j1 as
                        answer(a, b, c);
                    } indexes { i0 = @ r [0]; i1 = @ s [0]; } uses
                    { j0.right = i0; j1.right = i1; } physical
                    {
                        relations
                        {
                            L = :: cq_example_providers :: slice_rows; R = ::
                            cq_example_providers :: hash_relation; S = ::
                            cq_example_providers :: sorted_relation; l = ::
                            cq_example_providers :: slice_rows; r = ::
                            cq_example_providers :: hash_relation; s = ::
                            cq_example_providers :: sorted_relation; j0 = ::
                            cq_example_providers :: borrowed_fields; j1 = ::
                            cq_example_providers :: borrowed_fields;
                        } indexes
                        {
                            i0 = :: cq_example_providers :: hash_relation; i1 = ::
                            cq_example_providers :: sorted_relation;
                        } lowerings
                        {
                            l = :: cq_example_providers :: rename; r = ::
                            cq_example_providers :: rename; s = :: cq_example_providers
                            :: rename; j0 = :: cq_example_providers :: indexed_join; j1
                            = :: cq_example_providers :: indexed_join;
                        }
                    }
                } environment
                {
                    stored L at(__cq_store_L) operations :: cq_example_providers
                    :: slice_rows stored R at(__cq_store_R) operations ::
                    cq_example_providers :: hash_relation stored S
                    at(__cq_store_S) operations :: cq_example_providers ::
                    sorted_relation stored l at(__cq_store_l) operations ::
                    cq_example_providers :: slice_rows stored r at(__cq_store_r)
                    operations :: cq_example_providers :: hash_relation stored s
                    at(__cq_store_s) operations :: cq_example_providers ::
                    sorted_relation indexed i0 at(__cq_index_i0) operations ::
                    cq_example_providers :: hash_relation
                } emitted
                {
                    let __cq_store_L : & [Value] = L; let __cq_store_R : & ::
                    std :: collections :: HashMap < Value, :: std :: vec :: Vec
                    < Value > > = R; let __cq_store_S : & :: std :: collections
                    :: BTreeMap < Value, :: std :: vec :: Vec < Value > > = S;
                    let __cq_store_l : & [Value] = __cq_store_L; let
                    __cq_store_r : & :: std :: collections :: HashMap < Value,
                    :: std :: vec :: Vec < Value > > = __cq_store_R; let
                    __cq_store_s : & :: std :: collections :: BTreeMap < Value,
                    :: std :: vec :: Vec < Value > > = __cq_store_S; let
                    __cq_index_i0 : & :: std :: collections :: HashMap < Value,
                    :: std :: vec :: Vec < Value > > = __cq_store_r;
                } pending { lower j0 }
                continuations(:: mini_linq :: resume_physical, mini_linq ::
                access)
            } plan
            {
                struct Heterogeneous; relation L(a : Value); relation
                R(a : Value, b : Value); relation S(b : Value, c : Value);
                answer(a, b, c) : - L(a), R(a, b), S(b, c); relational
                {
                    l = rename L { a - > a }; r = rename R { a - > a, b - > b };
                    s = rename S { b - > b, c - > c }; j0 = natural_join l with
                    r; j1 = natural_join j0 with s; output j1 as
                    answer(a, b, c);
                } indexes { i0 = @ r [0]; i1 = @ s [0]; } uses
                { j0.right = i0; j1.right = i1; } physical
                {
                    relations
                    {
                        L = :: cq_example_providers :: slice_rows; R = ::
                        cq_example_providers :: hash_relation; S = ::
                        cq_example_providers :: sorted_relation; l = ::
                        cq_example_providers :: slice_rows; r = ::
                        cq_example_providers :: hash_relation; s = ::
                        cq_example_providers :: sorted_relation; j0 = ::
                        cq_example_providers :: borrowed_fields; j1 = ::
                        cq_example_providers :: borrowed_fields;
                    } indexes
                    {
                        i0 = :: cq_example_providers :: hash_relation; i1 = ::
                        cq_example_providers :: sorted_relation;
                    } lowerings
                    {
                        l = :: cq_example_providers :: rename; r = ::
                        cq_example_providers :: rename; s = :: cq_example_providers
                        :: rename; j0 = :: cq_example_providers :: indexed_join; j1
                        = :: cq_example_providers :: indexed_join;
                    }
                }
            } environment
            {
                stored L at(__cq_store_L) operations :: cq_example_providers
                :: slice_rows stored R at(__cq_store_R) operations ::
                cq_example_providers :: hash_relation stored S
                at(__cq_store_S) operations :: cq_example_providers ::
                sorted_relation stored l at(__cq_store_l) operations ::
                cq_example_providers :: slice_rows stored r at(__cq_store_r)
                operations :: cq_example_providers :: hash_relation stored s
                at(__cq_store_s) operations :: cq_example_providers ::
                sorted_relation indexed i0 at(__cq_index_i0) operations ::
                cq_example_providers :: hash_relation
            } request { lower j0 }
        } access
        {
            l(a) => bind a <- ((__cq_store_l).iter()), r(a, b) => bind b <-
            ((__cq_index_i0).get(a).into_iter().flatten())
        } witness { row(a) fields { 0 = (a); } }
    } plan
    {
        struct Heterogeneous; relation L(a : Value); relation
        R(a : Value, b : Value); relation S(b : Value, c : Value);
        answer(a, b, c) : - L(a), R(a, b), S(b, c); relational
        {
            l = rename L { a - > a }; r = rename R { a - > a, b - > b }; s =
            rename S { b - > b, c - > c }; j0 = natural_join l with r; j1 =
            natural_join j0 with s; output j1 as answer(a, b, c);
        } indexes { i0 = @ r [0]; i1 = @ s [0]; } uses
        { j0.right = i0; j1.right = i1; } physical
        {
            relations
            {
                L = :: cq_example_providers :: slice_rows; R = ::
                cq_example_providers :: hash_relation; S = ::
                cq_example_providers :: sorted_relation; l = ::
                cq_example_providers :: slice_rows; r = ::
                cq_example_providers :: hash_relation; s = ::
                cq_example_providers :: sorted_relation; j0 = ::
                cq_example_providers :: borrowed_fields; j1 = ::
                cq_example_providers :: borrowed_fields;
            } indexes
            {
                i0 = :: cq_example_providers :: hash_relation; i1 = ::
                cq_example_providers :: sorted_relation;
            } lowerings
            {
                l = :: cq_example_providers :: rename; r = ::
                cq_example_providers :: rename; s = :: cq_example_providers ::
                rename; j0 = :: cq_example_providers :: indexed_join; j1 = ::
                cq_example_providers :: indexed_join;
            }
        }
    } environment
    {
        stored L at(__cq_store_L) operations :: cq_example_providers ::
        slice_rows stored R at(__cq_store_R) operations ::
        cq_example_providers :: hash_relation stored S at(__cq_store_S)
        operations :: cq_example_providers :: sorted_relation stored l
        at(__cq_store_l) operations :: cq_example_providers :: slice_rows
        stored r at(__cq_store_r) operations :: cq_example_providers ::
        hash_relation stored s at(__cq_store_s) operations ::
        cq_example_providers :: sorted_relation indexed i0 at(__cq_index_i0)
        operations :: cq_example_providers :: hash_relation
    } request
    {
        output j0 fields { 0 = (a); 1 = (b); } whole
        { l = (a); r = ((a, b,)); }
    }
}

```

---

## 2172042/026-cq-example-providers::borrowed_fields

[Raw input](raw/2172042/026-cq-example-providers::borrowed_fields.input.tokens) · [Raw output](raw/2172042/026-cq-example-providers::borrowed_fields.output.tokens)

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
                struct Heterogeneous; relation L(a : Value); relation
                R(a : Value, b : Value); relation S(b : Value, c : Value);
                answer(a, b, c) : - L(a), R(a, b), S(b, c); relational
                {
                    l = rename L { a - > a }; r = rename R { a - > a, b - > b };
                    s = rename S { b - > b, c - > c }; j0 = natural_join l with
                    r; j1 = natural_join j0 with s; output j1 as
                    answer(a, b, c);
                } indexes { i0 = @ r [0]; i1 = @ s [0]; } uses
                { j0.right = i0; j1.right = i1; } physical
                {
                    relations
                    {
                        L = :: cq_example_providers :: slice_rows; R = ::
                        cq_example_providers :: hash_relation; S = ::
                        cq_example_providers :: sorted_relation; l = ::
                        cq_example_providers :: slice_rows; r = ::
                        cq_example_providers :: hash_relation; s = ::
                        cq_example_providers :: sorted_relation; j0 = ::
                        cq_example_providers :: borrowed_fields; j1 = ::
                        cq_example_providers :: borrowed_fields;
                    } indexes
                    {
                        i0 = :: cq_example_providers :: hash_relation; i1 = ::
                        cq_example_providers :: sorted_relation;
                    } lowerings
                    {
                        l = :: cq_example_providers :: rename; r = ::
                        cq_example_providers :: rename; s = :: cq_example_providers
                        :: rename; j0 = :: cq_example_providers :: indexed_join; j1
                        = :: cq_example_providers :: indexed_join;
                    }
                }
            } environment
            {
                stored L at(__cq_store_L) operations :: cq_example_providers
                :: slice_rows stored R at(__cq_store_R) operations ::
                cq_example_providers :: hash_relation stored S
                at(__cq_store_S) operations :: cq_example_providers ::
                sorted_relation stored l at(__cq_store_l) operations ::
                cq_example_providers :: slice_rows stored r at(__cq_store_r)
                operations :: cq_example_providers :: hash_relation stored s
                at(__cq_store_s) operations :: cq_example_providers ::
                sorted_relation indexed i0 at(__cq_index_i0) operations ::
                cq_example_providers :: hash_relation
            } emitted
            {
                let __cq_store_L : & [Value] = L; let __cq_store_R : & :: std
                :: collections :: HashMap < Value, :: std :: vec :: Vec <
                Value > > = R; let __cq_store_S : & :: std :: collections ::
                BTreeMap < Value, :: std :: vec :: Vec < Value > > = S; let
                __cq_store_l : & [Value] = __cq_store_L; let __cq_store_r : &
                :: std :: collections :: HashMap < Value, :: std :: vec :: Vec
                < Value > > = __cq_store_R; let __cq_store_s : & :: std ::
                collections :: BTreeMap < Value, :: std :: vec :: Vec < Value
                > > = __cq_store_S; let __cq_index_i0 : & :: std ::
                collections :: HashMap < Value, :: std :: vec :: Vec < Value >
                > = __cq_store_r;
            } pending { lower j0 }
            continuations(:: mini_linq :: resume_physical, mini_linq ::
            access)
        } plan
        {
            struct Heterogeneous; relation L(a : Value); relation
            R(a : Value, b : Value); relation S(b : Value, c : Value);
            answer(a, b, c) : - L(a), R(a, b), S(b, c); relational
            {
                l = rename L { a - > a }; r = rename R { a - > a, b - > b }; s
                = rename S { b - > b, c - > c }; j0 = natural_join l with r;
                j1 = natural_join j0 with s; output j1 as answer(a, b, c);
            } indexes { i0 = @ r [0]; i1 = @ s [0]; } uses
            { j0.right = i0; j1.right = i1; } physical
            {
                relations
                {
                    L = :: cq_example_providers :: slice_rows; R = ::
                    cq_example_providers :: hash_relation; S = ::
                    cq_example_providers :: sorted_relation; l = ::
                    cq_example_providers :: slice_rows; r = ::
                    cq_example_providers :: hash_relation; s = ::
                    cq_example_providers :: sorted_relation; j0 = ::
                    cq_example_providers :: borrowed_fields; j1 = ::
                    cq_example_providers :: borrowed_fields;
                } indexes
                {
                    i0 = :: cq_example_providers :: hash_relation; i1 = ::
                    cq_example_providers :: sorted_relation;
                } lowerings
                {
                    l = :: cq_example_providers :: rename; r = ::
                    cq_example_providers :: rename; s = :: cq_example_providers
                    :: rename; j0 = :: cq_example_providers :: indexed_join; j1
                    = :: cq_example_providers :: indexed_join;
                }
            }
        } environment
        {
            stored L at(__cq_store_L) operations :: cq_example_providers ::
            slice_rows stored R at(__cq_store_R) operations ::
            cq_example_providers :: hash_relation stored S at(__cq_store_S)
            operations :: cq_example_providers :: sorted_relation stored l
            at(__cq_store_l) operations :: cq_example_providers :: slice_rows
            stored r at(__cq_store_r) operations :: cq_example_providers ::
            hash_relation stored s at(__cq_store_s) operations ::
            cq_example_providers :: sorted_relation indexed i0
            at(__cq_index_i0) operations :: cq_example_providers ::
            hash_relation
        } request { lower j0 }
    } access
    {
        l(a) => bind a <- ((__cq_store_l).iter()), r(a, b) => bind b <-
        ((__cq_index_i0).get(a).into_iter().flatten())
    } witness { row(a) fields { 0 = (a); } }
} plan
{
    struct Heterogeneous; relation L(a : Value); relation
    R(a : Value, b : Value); relation S(b : Value, c : Value); answer(a, b, c)
    : - L(a), R(a, b), S(b, c); relational
    {
        l = rename L { a - > a }; r = rename R { a - > a, b - > b }; s =
        rename S { b - > b, c - > c }; j0 = natural_join l with r; j1 =
        natural_join j0 with s; output j1 as answer(a, b, c);
    } indexes { i0 = @ r [0]; i1 = @ s [0]; } uses
    { j0.right = i0; j1.right = i1; } physical
    {
        relations
        {
            L = :: cq_example_providers :: slice_rows; R = ::
            cq_example_providers :: hash_relation; S = :: cq_example_providers
            :: sorted_relation; l = :: cq_example_providers :: slice_rows; r =
            :: cq_example_providers :: hash_relation; s = ::
            cq_example_providers :: sorted_relation; j0 = ::
            cq_example_providers :: borrowed_fields; j1 = ::
            cq_example_providers :: borrowed_fields;
        } indexes
        {
            i0 = :: cq_example_providers :: hash_relation; i1 = ::
            cq_example_providers :: sorted_relation;
        } lowerings
        {
            l = :: cq_example_providers :: rename; r = :: cq_example_providers
            :: rename; s = :: cq_example_providers :: rename; j0 = ::
            cq_example_providers :: indexed_join; j1 = :: cq_example_providers
            :: indexed_join;
        }
    }
} environment
{
    stored L at(__cq_store_L) operations :: cq_example_providers :: slice_rows
    stored R at(__cq_store_R) operations :: cq_example_providers ::
    hash_relation stored S at(__cq_store_S) operations :: cq_example_providers
    :: sorted_relation stored l at(__cq_store_l) operations ::
    cq_example_providers :: slice_rows stored r at(__cq_store_r) operations ::
    cq_example_providers :: hash_relation stored s at(__cq_store_s) operations
    :: cq_example_providers :: sorted_relation indexed i0 at(__cq_index_i0)
    operations :: cq_example_providers :: hash_relation
} request
{ output j0 fields { 0 = (a); 1 = (b); } whole { l = (a); r = ((a, b,)); } }
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
                    struct Heterogeneous; relation L(a : Value); relation
                    R(a : Value, b : Value); relation S(b : Value, c : Value);
                    answer(a, b, c) : - L(a), R(a, b), S(b, c); relational
                    {
                        l = rename L { a - > a }; r = rename R { a - > a, b - > b };
                        s = rename S { b - > b, c - > c }; j0 = natural_join l with
                        r; j1 = natural_join j0 with s; output j1 as
                        answer(a, b, c);
                    } indexes { i0 = @ r [0]; i1 = @ s [0]; } uses
                    { j0.right = i0; j1.right = i1; } physical
                    {
                        relations
                        {
                            L = :: cq_example_providers :: slice_rows; R = ::
                            cq_example_providers :: hash_relation; S = ::
                            cq_example_providers :: sorted_relation; l = ::
                            cq_example_providers :: slice_rows; r = ::
                            cq_example_providers :: hash_relation; s = ::
                            cq_example_providers :: sorted_relation; j0 = ::
                            cq_example_providers :: borrowed_fields; j1 = ::
                            cq_example_providers :: borrowed_fields;
                        } indexes
                        {
                            i0 = :: cq_example_providers :: hash_relation; i1 = ::
                            cq_example_providers :: sorted_relation;
                        } lowerings
                        {
                            l = :: cq_example_providers :: rename; r = ::
                            cq_example_providers :: rename; s = :: cq_example_providers
                            :: rename; j0 = :: cq_example_providers :: indexed_join; j1
                            = :: cq_example_providers :: indexed_join;
                        }
                    }
                } environment
                {
                    stored L at(__cq_store_L) operations :: cq_example_providers
                    :: slice_rows stored R at(__cq_store_R) operations ::
                    cq_example_providers :: hash_relation stored S
                    at(__cq_store_S) operations :: cq_example_providers ::
                    sorted_relation stored l at(__cq_store_l) operations ::
                    cq_example_providers :: slice_rows stored r at(__cq_store_r)
                    operations :: cq_example_providers :: hash_relation stored s
                    at(__cq_store_s) operations :: cq_example_providers ::
                    sorted_relation indexed i0 at(__cq_index_i0) operations ::
                    cq_example_providers :: hash_relation
                } emitted
                {
                    let __cq_store_L : & [Value] = L; let __cq_store_R : & ::
                    std :: collections :: HashMap < Value, :: std :: vec :: Vec
                    < Value > > = R; let __cq_store_S : & :: std :: collections
                    :: BTreeMap < Value, :: std :: vec :: Vec < Value > > = S;
                    let __cq_store_l : & [Value] = __cq_store_L; let
                    __cq_store_r : & :: std :: collections :: HashMap < Value,
                    :: std :: vec :: Vec < Value > > = __cq_store_R; let
                    __cq_store_s : & :: std :: collections :: BTreeMap < Value,
                    :: std :: vec :: Vec < Value > > = __cq_store_S; let
                    __cq_index_i0 : & :: std :: collections :: HashMap < Value,
                    :: std :: vec :: Vec < Value > > = __cq_store_r;
                } pending { lower j0 }
                continuations(:: mini_linq :: resume_physical, mini_linq ::
                access)
            } plan
            {
                struct Heterogeneous; relation L(a : Value); relation
                R(a : Value, b : Value); relation S(b : Value, c : Value);
                answer(a, b, c) : - L(a), R(a, b), S(b, c); relational
                {
                    l = rename L { a - > a }; r = rename R { a - > a, b - > b };
                    s = rename S { b - > b, c - > c }; j0 = natural_join l with
                    r; j1 = natural_join j0 with s; output j1 as
                    answer(a, b, c);
                } indexes { i0 = @ r [0]; i1 = @ s [0]; } uses
                { j0.right = i0; j1.right = i1; } physical
                {
                    relations
                    {
                        L = :: cq_example_providers :: slice_rows; R = ::
                        cq_example_providers :: hash_relation; S = ::
                        cq_example_providers :: sorted_relation; l = ::
                        cq_example_providers :: slice_rows; r = ::
                        cq_example_providers :: hash_relation; s = ::
                        cq_example_providers :: sorted_relation; j0 = ::
                        cq_example_providers :: borrowed_fields; j1 = ::
                        cq_example_providers :: borrowed_fields;
                    } indexes
                    {
                        i0 = :: cq_example_providers :: hash_relation; i1 = ::
                        cq_example_providers :: sorted_relation;
                    } lowerings
                    {
                        l = :: cq_example_providers :: rename; r = ::
                        cq_example_providers :: rename; s = :: cq_example_providers
                        :: rename; j0 = :: cq_example_providers :: indexed_join; j1
                        = :: cq_example_providers :: indexed_join;
                    }
                }
            } environment
            {
                stored L at(__cq_store_L) operations :: cq_example_providers
                :: slice_rows stored R at(__cq_store_R) operations ::
                cq_example_providers :: hash_relation stored S
                at(__cq_store_S) operations :: cq_example_providers ::
                sorted_relation stored l at(__cq_store_l) operations ::
                cq_example_providers :: slice_rows stored r at(__cq_store_r)
                operations :: cq_example_providers :: hash_relation stored s
                at(__cq_store_s) operations :: cq_example_providers ::
                sorted_relation indexed i0 at(__cq_index_i0) operations ::
                cq_example_providers :: hash_relation
            } request { lower j0 }
        } access
        {
            l(a) => bind a <- ((__cq_store_l).iter()), r(a, b) => bind b <-
            ((__cq_index_i0).get(a).into_iter().flatten())
        } witness { row(a) fields { 0 = (a); } }
    } witness { row((a, b,)) fields { 0 = (a); 1 = (b); } }
}

```

---

## 2172042/027-cq-example-providers::indexed_join_output

[Raw input](raw/2172042/027-cq-example-providers::indexed_join_output.input.tokens) · [Raw output](raw/2172042/027-cq-example-providers::indexed_join_output.output.tokens)

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
                struct Heterogeneous; relation L(a : Value); relation
                R(a : Value, b : Value); relation S(b : Value, c : Value);
                answer(a, b, c) : - L(a), R(a, b), S(b, c); relational
                {
                    l = rename L { a - > a }; r = rename R { a - > a, b - > b };
                    s = rename S { b - > b, c - > c }; j0 = natural_join l with
                    r; j1 = natural_join j0 with s; output j1 as
                    answer(a, b, c);
                } indexes { i0 = @ r [0]; i1 = @ s [0]; } uses
                { j0.right = i0; j1.right = i1; } physical
                {
                    relations
                    {
                        L = :: cq_example_providers :: slice_rows; R = ::
                        cq_example_providers :: hash_relation; S = ::
                        cq_example_providers :: sorted_relation; l = ::
                        cq_example_providers :: slice_rows; r = ::
                        cq_example_providers :: hash_relation; s = ::
                        cq_example_providers :: sorted_relation; j0 = ::
                        cq_example_providers :: borrowed_fields; j1 = ::
                        cq_example_providers :: borrowed_fields;
                    } indexes
                    {
                        i0 = :: cq_example_providers :: hash_relation; i1 = ::
                        cq_example_providers :: sorted_relation;
                    } lowerings
                    {
                        l = :: cq_example_providers :: rename; r = ::
                        cq_example_providers :: rename; s = :: cq_example_providers
                        :: rename; j0 = :: cq_example_providers :: indexed_join; j1
                        = :: cq_example_providers :: indexed_join;
                    }
                }
            } environment
            {
                stored L at(__cq_store_L) operations :: cq_example_providers
                :: slice_rows stored R at(__cq_store_R) operations ::
                cq_example_providers :: hash_relation stored S
                at(__cq_store_S) operations :: cq_example_providers ::
                sorted_relation stored l at(__cq_store_l) operations ::
                cq_example_providers :: slice_rows stored r at(__cq_store_r)
                operations :: cq_example_providers :: hash_relation stored s
                at(__cq_store_s) operations :: cq_example_providers ::
                sorted_relation indexed i0 at(__cq_index_i0) operations ::
                cq_example_providers :: hash_relation
            } emitted
            {
                let __cq_store_L : & [Value] = L; let __cq_store_R : & :: std
                :: collections :: HashMap < Value, :: std :: vec :: Vec <
                Value > > = R; let __cq_store_S : & :: std :: collections ::
                BTreeMap < Value, :: std :: vec :: Vec < Value > > = S; let
                __cq_store_l : & [Value] = __cq_store_L; let __cq_store_r : &
                :: std :: collections :: HashMap < Value, :: std :: vec :: Vec
                < Value > > = __cq_store_R; let __cq_store_s : & :: std ::
                collections :: BTreeMap < Value, :: std :: vec :: Vec < Value
                > > = __cq_store_S; let __cq_index_i0 : & :: std ::
                collections :: HashMap < Value, :: std :: vec :: Vec < Value >
                > = __cq_store_r;
            } pending { lower j0 }
            continuations(:: mini_linq :: resume_physical, mini_linq ::
            access)
        } plan
        {
            struct Heterogeneous; relation L(a : Value); relation
            R(a : Value, b : Value); relation S(b : Value, c : Value);
            answer(a, b, c) : - L(a), R(a, b), S(b, c); relational
            {
                l = rename L { a - > a }; r = rename R { a - > a, b - > b }; s
                = rename S { b - > b, c - > c }; j0 = natural_join l with r;
                j1 = natural_join j0 with s; output j1 as answer(a, b, c);
            } indexes { i0 = @ r [0]; i1 = @ s [0]; } uses
            { j0.right = i0; j1.right = i1; } physical
            {
                relations
                {
                    L = :: cq_example_providers :: slice_rows; R = ::
                    cq_example_providers :: hash_relation; S = ::
                    cq_example_providers :: sorted_relation; l = ::
                    cq_example_providers :: slice_rows; r = ::
                    cq_example_providers :: hash_relation; s = ::
                    cq_example_providers :: sorted_relation; j0 = ::
                    cq_example_providers :: borrowed_fields; j1 = ::
                    cq_example_providers :: borrowed_fields;
                } indexes
                {
                    i0 = :: cq_example_providers :: hash_relation; i1 = ::
                    cq_example_providers :: sorted_relation;
                } lowerings
                {
                    l = :: cq_example_providers :: rename; r = ::
                    cq_example_providers :: rename; s = :: cq_example_providers
                    :: rename; j0 = :: cq_example_providers :: indexed_join; j1
                    = :: cq_example_providers :: indexed_join;
                }
            }
        } environment
        {
            stored L at(__cq_store_L) operations :: cq_example_providers ::
            slice_rows stored R at(__cq_store_R) operations ::
            cq_example_providers :: hash_relation stored S at(__cq_store_S)
            operations :: cq_example_providers :: sorted_relation stored l
            at(__cq_store_l) operations :: cq_example_providers :: slice_rows
            stored r at(__cq_store_r) operations :: cq_example_providers ::
            hash_relation stored s at(__cq_store_s) operations ::
            cq_example_providers :: sorted_relation indexed i0
            at(__cq_index_i0) operations :: cq_example_providers ::
            hash_relation
        } request { lower j0 }
    } access
    {
        l(a) => bind a <- ((__cq_store_l).iter()), r(a, b) => bind b <-
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
            struct Heterogeneous; relation L(a : Value); relation
            R(a : Value, b : Value); relation S(b : Value, c : Value);
            answer(a, b, c) : - L(a), R(a, b), S(b, c); relational
            {
                l = rename L { a - > a }; r = rename R { a - > a, b - > b }; s
                = rename S { b - > b, c - > c }; j0 = natural_join l with r;
                j1 = natural_join j0 with s; output j1 as answer(a, b, c);
            } indexes { i0 = @ r [0]; i1 = @ s [0]; } uses
            { j0.right = i0; j1.right = i1; } physical
            {
                relations
                {
                    L = :: cq_example_providers :: slice_rows; R = ::
                    cq_example_providers :: hash_relation; S = ::
                    cq_example_providers :: sorted_relation; l = ::
                    cq_example_providers :: slice_rows; r = ::
                    cq_example_providers :: hash_relation; s = ::
                    cq_example_providers :: sorted_relation; j0 = ::
                    cq_example_providers :: borrowed_fields; j1 = ::
                    cq_example_providers :: borrowed_fields;
                } indexes
                {
                    i0 = :: cq_example_providers :: hash_relation; i1 = ::
                    cq_example_providers :: sorted_relation;
                } lowerings
                {
                    l = :: cq_example_providers :: rename; r = ::
                    cq_example_providers :: rename; s = :: cq_example_providers
                    :: rename; j0 = :: cq_example_providers :: indexed_join; j1
                    = :: cq_example_providers :: indexed_join;
                }
            }
        } environment
        {
            stored L at(__cq_store_L) operations :: cq_example_providers ::
            slice_rows stored R at(__cq_store_R) operations ::
            cq_example_providers :: hash_relation stored S at(__cq_store_S)
            operations :: cq_example_providers :: sorted_relation stored l
            at(__cq_store_l) operations :: cq_example_providers :: slice_rows
            stored r at(__cq_store_r) operations :: cq_example_providers ::
            hash_relation stored s at(__cq_store_s) operations ::
            cq_example_providers :: sorted_relation indexed i0
            at(__cq_index_i0) operations :: cq_example_providers ::
            hash_relation
        } emitted
        {
            let __cq_store_L : & [Value] = L; let __cq_store_R : & :: std ::
            collections :: HashMap < Value, :: std :: vec :: Vec < Value > > =
            R; let __cq_store_S : & :: std :: collections :: BTreeMap < Value,
            :: std :: vec :: Vec < Value > > = S; let __cq_store_l : & [Value]
            = __cq_store_L; let __cq_store_r : & :: std :: collections ::
            HashMap < Value, :: std :: vec :: Vec < Value > > = __cq_store_R;
            let __cq_store_s : & :: std :: collections :: BTreeMap < Value, ::
            std :: vec :: Vec < Value > > = __cq_store_S; let __cq_index_i0 :
            & :: std :: collections :: HashMap < Value, :: std :: vec :: Vec <
            Value > > = __cq_store_r;
        } pending { lower j0 }
        continuations(:: mini_linq :: resume_physical, mini_linq :: access)
    } emitted {} result
    {
        region j0
        {
            j0(a, b) => ((a, b,)) : - l(a) => bind a <-
            ((__cq_store_l).iter()), r(a, b) => bind b <-
            ((__cq_index_i0).get(a).into_iter().flatten());
        }
    }
}

```

---

## 2172042/028-resume_physical

[Raw input](raw/2172042/028-resume_physical.input.tokens) · [Raw output](raw/2172042/028-resume_physical.output.tokens)

Actual input:

```rust
resume
{
    plan
    {
        struct Heterogeneous; relation L(a : Value); relation
        R(a : Value, b : Value); relation S(b : Value, c : Value);
        answer(a, b, c) : - L(a), R(a, b), S(b, c); relational
        {
            l = rename L { a - > a }; r = rename R { a - > a, b - > b }; s =
            rename S { b - > b, c - > c }; j0 = natural_join l with r; j1 =
            natural_join j0 with s; output j1 as answer(a, b, c);
        } indexes { i0 = @ r [0]; i1 = @ s [0]; } uses
        { j0.right = i0; j1.right = i1; } physical
        {
            relations
            {
                L = :: cq_example_providers :: slice_rows; R = ::
                cq_example_providers :: hash_relation; S = ::
                cq_example_providers :: sorted_relation; l = ::
                cq_example_providers :: slice_rows; r = ::
                cq_example_providers :: hash_relation; s = ::
                cq_example_providers :: sorted_relation; j0 = ::
                cq_example_providers :: borrowed_fields; j1 = ::
                cq_example_providers :: borrowed_fields;
            } indexes
            {
                i0 = :: cq_example_providers :: hash_relation; i1 = ::
                cq_example_providers :: sorted_relation;
            } lowerings
            {
                l = :: cq_example_providers :: rename; r = ::
                cq_example_providers :: rename; s = :: cq_example_providers ::
                rename; j0 = :: cq_example_providers :: indexed_join; j1 = ::
                cq_example_providers :: indexed_join;
            }
        }
    } environment
    {
        stored L at(__cq_store_L) operations :: cq_example_providers ::
        slice_rows stored R at(__cq_store_R) operations ::
        cq_example_providers :: hash_relation stored S at(__cq_store_S)
        operations :: cq_example_providers :: sorted_relation stored l
        at(__cq_store_l) operations :: cq_example_providers :: slice_rows
        stored r at(__cq_store_r) operations :: cq_example_providers ::
        hash_relation stored s at(__cq_store_s) operations ::
        cq_example_providers :: sorted_relation indexed i0 at(__cq_index_i0)
        operations :: cq_example_providers :: hash_relation
    } emitted
    {
        let __cq_store_L : & [Value] = L; let __cq_store_R : & :: std ::
        collections :: HashMap < Value, :: std :: vec :: Vec < Value > > = R;
        let __cq_store_S : & :: std :: collections :: BTreeMap < Value, :: std
        :: vec :: Vec < Value > > = S; let __cq_store_l : & [Value] =
        __cq_store_L; let __cq_store_r : & :: std :: collections :: HashMap <
        Value, :: std :: vec :: Vec < Value > > = __cq_store_R; let
        __cq_store_s : & :: std :: collections :: BTreeMap < Value, :: std ::
        vec :: Vec < Value > > = __cq_store_S; let __cq_index_i0 : & :: std ::
        collections :: HashMap < Value, :: std :: vec :: Vec < Value > > =
        __cq_store_r;
    } pending { lower j0 }
    continuations(:: mini_linq :: resume_physical, mini_linq :: access)
} emitted {} result
{
    region j0
    {
        j0(a, b) => ((a, b,)) : - l(a) => bind a <- ((__cq_store_l).iter()),
        r(a, b) => bind b <- ((__cq_index_i0).get(a).into_iter().flatten());
    }
}
```

Actual output (formatted where Rust syntax permits):

```rust
::cq_example_providers::sorted_relation! {
    reply_to(:: mini_linq :: resume_physical)
    {
        plan
        {
            struct Heterogeneous; relation L(a : Value); relation
            R(a : Value, b : Value); relation S(b : Value, c : Value);
            answer(a, b, c) : - L(a), R(a, b), S(b, c); relational
            {
                l = rename L { a - > a }; r = rename R { a - > a, b - > b }; s
                = rename S { b - > b, c - > c }; j0 = natural_join l with r;
                j1 = natural_join j0 with s; output j1 as answer(a, b, c);
            } indexes { i0 = @ r [0]; i1 = @ s [0]; } uses
            { j0.right = i0; j1.right = i1; } physical
            {
                relations
                {
                    L = :: cq_example_providers :: slice_rows; R = ::
                    cq_example_providers :: hash_relation; S = ::
                    cq_example_providers :: sorted_relation; l = ::
                    cq_example_providers :: slice_rows; r = ::
                    cq_example_providers :: hash_relation; s = ::
                    cq_example_providers :: sorted_relation; j0 = ::
                    cq_example_providers :: borrowed_fields; j1 = ::
                    cq_example_providers :: borrowed_fields;
                } indexes
                {
                    i0 = :: cq_example_providers :: hash_relation; i1 = ::
                    cq_example_providers :: sorted_relation;
                } lowerings
                {
                    l = :: cq_example_providers :: rename; r = ::
                    cq_example_providers :: rename; s = :: cq_example_providers
                    :: rename; j0 = :: cq_example_providers :: indexed_join; j1
                    = :: cq_example_providers :: indexed_join;
                }
            }
        } environment
        {
            stored L at(__cq_store_L) operations :: cq_example_providers ::
            slice_rows stored R at(__cq_store_R) operations ::
            cq_example_providers :: hash_relation stored S at(__cq_store_S)
            operations :: cq_example_providers :: sorted_relation stored l
            at(__cq_store_l) operations :: cq_example_providers :: slice_rows
            stored r at(__cq_store_r) operations :: cq_example_providers ::
            hash_relation stored s at(__cq_store_s) operations ::
            cq_example_providers :: sorted_relation indexed i0
            at(__cq_index_i0) operations :: cq_example_providers ::
            hash_relation region j0
            {
                j0(a, b) => ((a, b,)) : - l(a) => bind a <-
                ((__cq_store_l).iter()), r(a, b) => bind b <-
                ((__cq_index_i0).get(a).into_iter().flatten());
            }
        } emitted
        {
            let __cq_store_L : & [Value] = L; let __cq_store_R : & :: std ::
            collections :: HashMap < Value, :: std :: vec :: Vec < Value > > =
            R; let __cq_store_S : & :: std :: collections :: BTreeMap < Value,
            :: std :: vec :: Vec < Value > > = S; let __cq_store_l : & [Value]
            = __cq_store_L; let __cq_store_r : & :: std :: collections ::
            HashMap < Value, :: std :: vec :: Vec < Value > > = __cq_store_R;
            let __cq_store_s : & :: std :: collections :: BTreeMap < Value, ::
            std :: vec :: Vec < Value > > = __cq_store_S; let __cq_index_i0 :
            & :: std :: collections :: HashMap < Value, :: std :: vec :: Vec <
            Value > > = __cq_store_r;
        } pending { index i1 }
        continuations(:: mini_linq :: resume_physical, mini_linq :: access)
    } plan
    {
        struct Heterogeneous; relation L(a : Value); relation
        R(a : Value, b : Value); relation S(b : Value, c : Value);
        answer(a, b, c) : - L(a), R(a, b), S(b, c); relational
        {
            l = rename L { a - > a }; r = rename R { a - > a, b - > b }; s =
            rename S { b - > b, c - > c }; j0 = natural_join l with r; j1 =
            natural_join j0 with s; output j1 as answer(a, b, c);
        } indexes { i0 = @ r [0]; i1 = @ s [0]; } uses
        { j0.right = i0; j1.right = i1; } physical
        {
            relations
            {
                L = :: cq_example_providers :: slice_rows; R = ::
                cq_example_providers :: hash_relation; S = ::
                cq_example_providers :: sorted_relation; l = ::
                cq_example_providers :: slice_rows; r = ::
                cq_example_providers :: hash_relation; s = ::
                cq_example_providers :: sorted_relation; j0 = ::
                cq_example_providers :: borrowed_fields; j1 = ::
                cq_example_providers :: borrowed_fields;
            } indexes
            {
                i0 = :: cq_example_providers :: hash_relation; i1 = ::
                cq_example_providers :: sorted_relation;
            } lowerings
            {
                l = :: cq_example_providers :: rename; r = ::
                cq_example_providers :: rename; s = :: cq_example_providers ::
                rename; j0 = :: cq_example_providers :: indexed_join; j1 = ::
                cq_example_providers :: indexed_join;
            }
        }
    } environment
    {
        stored L at(__cq_store_L) operations :: cq_example_providers ::
        slice_rows stored R at(__cq_store_R) operations ::
        cq_example_providers :: hash_relation stored S at(__cq_store_S)
        operations :: cq_example_providers :: sorted_relation stored l
        at(__cq_store_l) operations :: cq_example_providers :: slice_rows
        stored r at(__cq_store_r) operations :: cq_example_providers ::
        hash_relation stored s at(__cq_store_s) operations ::
        cq_example_providers :: sorted_relation indexed i0 at(__cq_index_i0)
        operations :: cq_example_providers :: hash_relation region j0
        {
            j0(a, b) => ((a, b,)) : - l(a) => bind a <-
            ((__cq_store_l).iter()), r(a, b) => bind b <-
            ((__cq_index_i0).get(a).into_iter().flatten());
        }
    } request { index i1 }
}

```

---

## 2172042/029-cq-example-providers::sorted_relation

[Raw input](raw/2172042/029-cq-example-providers::sorted_relation.input.tokens) · [Raw output](raw/2172042/029-cq-example-providers::sorted_relation.output.tokens)

Actual input:

```rust
reply_to(:: mini_linq :: resume_physical)
{
    plan
    {
        struct Heterogeneous; relation L(a : Value); relation
        R(a : Value, b : Value); relation S(b : Value, c : Value);
        answer(a, b, c) : - L(a), R(a, b), S(b, c); relational
        {
            l = rename L { a - > a }; r = rename R { a - > a, b - > b }; s =
            rename S { b - > b, c - > c }; j0 = natural_join l with r; j1 =
            natural_join j0 with s; output j1 as answer(a, b, c);
        } indexes { i0 = @ r [0]; i1 = @ s [0]; } uses
        { j0.right = i0; j1.right = i1; } physical
        {
            relations
            {
                L = :: cq_example_providers :: slice_rows; R = ::
                cq_example_providers :: hash_relation; S = ::
                cq_example_providers :: sorted_relation; l = ::
                cq_example_providers :: slice_rows; r = ::
                cq_example_providers :: hash_relation; s = ::
                cq_example_providers :: sorted_relation; j0 = ::
                cq_example_providers :: borrowed_fields; j1 = ::
                cq_example_providers :: borrowed_fields;
            } indexes
            {
                i0 = :: cq_example_providers :: hash_relation; i1 = ::
                cq_example_providers :: sorted_relation;
            } lowerings
            {
                l = :: cq_example_providers :: rename; r = ::
                cq_example_providers :: rename; s = :: cq_example_providers ::
                rename; j0 = :: cq_example_providers :: indexed_join; j1 = ::
                cq_example_providers :: indexed_join;
            }
        }
    } environment
    {
        stored L at(__cq_store_L) operations :: cq_example_providers ::
        slice_rows stored R at(__cq_store_R) operations ::
        cq_example_providers :: hash_relation stored S at(__cq_store_S)
        operations :: cq_example_providers :: sorted_relation stored l
        at(__cq_store_l) operations :: cq_example_providers :: slice_rows
        stored r at(__cq_store_r) operations :: cq_example_providers ::
        hash_relation stored s at(__cq_store_s) operations ::
        cq_example_providers :: sorted_relation indexed i0 at(__cq_index_i0)
        operations :: cq_example_providers :: hash_relation region j0
        {
            j0(a, b) => ((a, b,)) : - l(a) => bind a <-
            ((__cq_store_l).iter()), r(a, b) => bind b <-
            ((__cq_index_i0).get(a).into_iter().flatten());
        }
    } emitted
    {
        let __cq_store_L : & [Value] = L; let __cq_store_R : & :: std ::
        collections :: HashMap < Value, :: std :: vec :: Vec < Value > > = R;
        let __cq_store_S : & :: std :: collections :: BTreeMap < Value, :: std
        :: vec :: Vec < Value > > = S; let __cq_store_l : & [Value] =
        __cq_store_L; let __cq_store_r : & :: std :: collections :: HashMap <
        Value, :: std :: vec :: Vec < Value > > = __cq_store_R; let
        __cq_store_s : & :: std :: collections :: BTreeMap < Value, :: std ::
        vec :: Vec < Value > > = __cq_store_S; let __cq_index_i0 : & :: std ::
        collections :: HashMap < Value, :: std :: vec :: Vec < Value > > =
        __cq_store_r;
    } pending { index i1 }
    continuations(:: mini_linq :: resume_physical, mini_linq :: access)
} plan
{
    struct Heterogeneous; relation L(a : Value); relation
    R(a : Value, b : Value); relation S(b : Value, c : Value); answer(a, b, c)
    : - L(a), R(a, b), S(b, c); relational
    {
        l = rename L { a - > a }; r = rename R { a - > a, b - > b }; s =
        rename S { b - > b, c - > c }; j0 = natural_join l with r; j1 =
        natural_join j0 with s; output j1 as answer(a, b, c);
    } indexes { i0 = @ r [0]; i1 = @ s [0]; } uses
    { j0.right = i0; j1.right = i1; } physical
    {
        relations
        {
            L = :: cq_example_providers :: slice_rows; R = ::
            cq_example_providers :: hash_relation; S = :: cq_example_providers
            :: sorted_relation; l = :: cq_example_providers :: slice_rows; r =
            :: cq_example_providers :: hash_relation; s = ::
            cq_example_providers :: sorted_relation; j0 = ::
            cq_example_providers :: borrowed_fields; j1 = ::
            cq_example_providers :: borrowed_fields;
        } indexes
        {
            i0 = :: cq_example_providers :: hash_relation; i1 = ::
            cq_example_providers :: sorted_relation;
        } lowerings
        {
            l = :: cq_example_providers :: rename; r = :: cq_example_providers
            :: rename; s = :: cq_example_providers :: rename; j0 = ::
            cq_example_providers :: indexed_join; j1 = :: cq_example_providers
            :: indexed_join;
        }
    }
} environment
{
    stored L at(__cq_store_L) operations :: cq_example_providers :: slice_rows
    stored R at(__cq_store_R) operations :: cq_example_providers ::
    hash_relation stored S at(__cq_store_S) operations :: cq_example_providers
    :: sorted_relation stored l at(__cq_store_l) operations ::
    cq_example_providers :: slice_rows stored r at(__cq_store_r) operations ::
    cq_example_providers :: hash_relation stored s at(__cq_store_s) operations
    :: cq_example_providers :: sorted_relation indexed i0 at(__cq_index_i0)
    operations :: cq_example_providers :: hash_relation region j0
    {
        j0(a, b) => ((a, b,)) : - l(a) => bind a <- ((__cq_store_l).iter()),
        r(a, b) => bind b <- ((__cq_index_i0).get(a).into_iter().flatten());
    }
} request { index i1 }
```

Actual output (formatted where Rust syntax permits):

```rust
::mini_linq::resume_physical! {
    resume
    {
        plan
        {
            struct Heterogeneous; relation L(a : Value); relation
            R(a : Value, b : Value); relation S(b : Value, c : Value);
            answer(a, b, c) : - L(a), R(a, b), S(b, c); relational
            {
                l = rename L { a - > a }; r = rename R { a - > a, b - > b }; s
                = rename S { b - > b, c - > c }; j0 = natural_join l with r;
                j1 = natural_join j0 with s; output j1 as answer(a, b, c);
            } indexes { i0 = @ r [0]; i1 = @ s [0]; } uses
            { j0.right = i0; j1.right = i1; } physical
            {
                relations
                {
                    L = :: cq_example_providers :: slice_rows; R = ::
                    cq_example_providers :: hash_relation; S = ::
                    cq_example_providers :: sorted_relation; l = ::
                    cq_example_providers :: slice_rows; r = ::
                    cq_example_providers :: hash_relation; s = ::
                    cq_example_providers :: sorted_relation; j0 = ::
                    cq_example_providers :: borrowed_fields; j1 = ::
                    cq_example_providers :: borrowed_fields;
                } indexes
                {
                    i0 = :: cq_example_providers :: hash_relation; i1 = ::
                    cq_example_providers :: sorted_relation;
                } lowerings
                {
                    l = :: cq_example_providers :: rename; r = ::
                    cq_example_providers :: rename; s = :: cq_example_providers
                    :: rename; j0 = :: cq_example_providers :: indexed_join; j1
                    = :: cq_example_providers :: indexed_join;
                }
            }
        } environment
        {
            stored L at(__cq_store_L) operations :: cq_example_providers ::
            slice_rows stored R at(__cq_store_R) operations ::
            cq_example_providers :: hash_relation stored S at(__cq_store_S)
            operations :: cq_example_providers :: sorted_relation stored l
            at(__cq_store_l) operations :: cq_example_providers :: slice_rows
            stored r at(__cq_store_r) operations :: cq_example_providers ::
            hash_relation stored s at(__cq_store_s) operations ::
            cq_example_providers :: sorted_relation indexed i0
            at(__cq_index_i0) operations :: cq_example_providers ::
            hash_relation region j0
            {
                j0(a, b) => ((a, b,)) : - l(a) => bind a <-
                ((__cq_store_l).iter()), r(a, b) => bind b <-
                ((__cq_index_i0).get(a).into_iter().flatten());
            }
        } emitted
        {
            let __cq_store_L : & [Value] = L; let __cq_store_R : & :: std ::
            collections :: HashMap < Value, :: std :: vec :: Vec < Value > > =
            R; let __cq_store_S : & :: std :: collections :: BTreeMap < Value,
            :: std :: vec :: Vec < Value > > = S; let __cq_store_l : & [Value]
            = __cq_store_L; let __cq_store_r : & :: std :: collections ::
            HashMap < Value, :: std :: vec :: Vec < Value > > = __cq_store_R;
            let __cq_store_s : & :: std :: collections :: BTreeMap < Value, ::
            std :: vec :: Vec < Value > > = __cq_store_S; let __cq_index_i0 :
            & :: std :: collections :: HashMap < Value, :: std :: vec :: Vec <
            Value > > = __cq_store_r;
        } pending { index i1 }
        continuations(:: mini_linq :: resume_physical, mini_linq :: access)
    } emitted
    {
        let __cq_index_i1 : & :: std :: collections :: BTreeMap < Value, ::
        std :: vec :: Vec < Value >> = __cq_store_s;
    } result
    {
        indexed i1 at(__cq_index_i1) operations :: cq_example_providers ::
        sorted_relation
    }
}

```

---

## 2172042/030-resume_physical

[Raw input](raw/2172042/030-resume_physical.input.tokens) · [Raw output](raw/2172042/030-resume_physical.output.tokens)

Actual input:

```rust
resume
{
    plan
    {
        struct Heterogeneous; relation L(a : Value); relation
        R(a : Value, b : Value); relation S(b : Value, c : Value);
        answer(a, b, c) : - L(a), R(a, b), S(b, c); relational
        {
            l = rename L { a - > a }; r = rename R { a - > a, b - > b }; s =
            rename S { b - > b, c - > c }; j0 = natural_join l with r; j1 =
            natural_join j0 with s; output j1 as answer(a, b, c);
        } indexes { i0 = @ r [0]; i1 = @ s [0]; } uses
        { j0.right = i0; j1.right = i1; } physical
        {
            relations
            {
                L = :: cq_example_providers :: slice_rows; R = ::
                cq_example_providers :: hash_relation; S = ::
                cq_example_providers :: sorted_relation; l = ::
                cq_example_providers :: slice_rows; r = ::
                cq_example_providers :: hash_relation; s = ::
                cq_example_providers :: sorted_relation; j0 = ::
                cq_example_providers :: borrowed_fields; j1 = ::
                cq_example_providers :: borrowed_fields;
            } indexes
            {
                i0 = :: cq_example_providers :: hash_relation; i1 = ::
                cq_example_providers :: sorted_relation;
            } lowerings
            {
                l = :: cq_example_providers :: rename; r = ::
                cq_example_providers :: rename; s = :: cq_example_providers ::
                rename; j0 = :: cq_example_providers :: indexed_join; j1 = ::
                cq_example_providers :: indexed_join;
            }
        }
    } environment
    {
        stored L at(__cq_store_L) operations :: cq_example_providers ::
        slice_rows stored R at(__cq_store_R) operations ::
        cq_example_providers :: hash_relation stored S at(__cq_store_S)
        operations :: cq_example_providers :: sorted_relation stored l
        at(__cq_store_l) operations :: cq_example_providers :: slice_rows
        stored r at(__cq_store_r) operations :: cq_example_providers ::
        hash_relation stored s at(__cq_store_s) operations ::
        cq_example_providers :: sorted_relation indexed i0 at(__cq_index_i0)
        operations :: cq_example_providers :: hash_relation region j0
        {
            j0(a, b) => ((a, b,)) : - l(a) => bind a <-
            ((__cq_store_l).iter()), r(a, b) => bind b <-
            ((__cq_index_i0).get(a).into_iter().flatten());
        }
    } emitted
    {
        let __cq_store_L : & [Value] = L; let __cq_store_R : & :: std ::
        collections :: HashMap < Value, :: std :: vec :: Vec < Value > > = R;
        let __cq_store_S : & :: std :: collections :: BTreeMap < Value, :: std
        :: vec :: Vec < Value > > = S; let __cq_store_l : & [Value] =
        __cq_store_L; let __cq_store_r : & :: std :: collections :: HashMap <
        Value, :: std :: vec :: Vec < Value > > = __cq_store_R; let
        __cq_store_s : & :: std :: collections :: BTreeMap < Value, :: std ::
        vec :: Vec < Value > > = __cq_store_S; let __cq_index_i0 : & :: std ::
        collections :: HashMap < Value, :: std :: vec :: Vec < Value > > =
        __cq_store_r;
    } pending { index i1 }
    continuations(:: mini_linq :: resume_physical, mini_linq :: access)
} emitted
{
    let __cq_index_i1 : & :: std :: collections :: BTreeMap < Value, :: std ::
    vec :: Vec < Value >> = __cq_store_s;
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
            struct Heterogeneous; relation L(a : Value); relation
            R(a : Value, b : Value); relation S(b : Value, c : Value);
            answer(a, b, c) : - L(a), R(a, b), S(b, c); relational
            {
                l = rename L { a - > a }; r = rename R { a - > a, b - > b }; s
                = rename S { b - > b, c - > c }; j0 = natural_join l with r;
                j1 = natural_join j0 with s; output j1 as answer(a, b, c);
            } indexes { i0 = @ r [0]; i1 = @ s [0]; } uses
            { j0.right = i0; j1.right = i1; } physical
            {
                relations
                {
                    L = :: cq_example_providers :: slice_rows; R = ::
                    cq_example_providers :: hash_relation; S = ::
                    cq_example_providers :: sorted_relation; l = ::
                    cq_example_providers :: slice_rows; r = ::
                    cq_example_providers :: hash_relation; s = ::
                    cq_example_providers :: sorted_relation; j0 = ::
                    cq_example_providers :: borrowed_fields; j1 = ::
                    cq_example_providers :: borrowed_fields;
                } indexes
                {
                    i0 = :: cq_example_providers :: hash_relation; i1 = ::
                    cq_example_providers :: sorted_relation;
                } lowerings
                {
                    l = :: cq_example_providers :: rename; r = ::
                    cq_example_providers :: rename; s = :: cq_example_providers
                    :: rename; j0 = :: cq_example_providers :: indexed_join; j1
                    = :: cq_example_providers :: indexed_join;
                }
            }
        } environment
        {
            stored L at(__cq_store_L) operations :: cq_example_providers ::
            slice_rows stored R at(__cq_store_R) operations ::
            cq_example_providers :: hash_relation stored S at(__cq_store_S)
            operations :: cq_example_providers :: sorted_relation stored l
            at(__cq_store_l) operations :: cq_example_providers :: slice_rows
            stored r at(__cq_store_r) operations :: cq_example_providers ::
            hash_relation stored s at(__cq_store_s) operations ::
            cq_example_providers :: sorted_relation indexed i0
            at(__cq_index_i0) operations :: cq_example_providers ::
            hash_relation region j0
            {
                j0(a, b) => ((a, b,)) : - l(a) => bind a <-
                ((__cq_store_l).iter()), r(a, b) => bind b <-
                ((__cq_index_i0).get(a).into_iter().flatten());
            } indexed i1 at(__cq_index_i1) operations :: cq_example_providers
            :: sorted_relation
        } emitted
        {
            let __cq_store_L : & [Value] = L; let __cq_store_R : & :: std ::
            collections :: HashMap < Value, :: std :: vec :: Vec < Value > > =
            R; let __cq_store_S : & :: std :: collections :: BTreeMap < Value,
            :: std :: vec :: Vec < Value > > = S; let __cq_store_l : & [Value]
            = __cq_store_L; let __cq_store_r : & :: std :: collections ::
            HashMap < Value, :: std :: vec :: Vec < Value > > = __cq_store_R;
            let __cq_store_s : & :: std :: collections :: BTreeMap < Value, ::
            std :: vec :: Vec < Value > > = __cq_store_S; let __cq_index_i0 :
            & :: std :: collections :: HashMap < Value, :: std :: vec :: Vec <
            Value > > = __cq_store_r; let __cq_index_i1 : & :: std ::
            collections :: BTreeMap < Value, :: std :: vec :: Vec < Value > >
            = __cq_store_s;
        } pending { lower j1 }
        continuations(:: mini_linq :: resume_physical, mini_linq :: access)
    } plan
    {
        struct Heterogeneous; relation L(a : Value); relation
        R(a : Value, b : Value); relation S(b : Value, c : Value);
        answer(a, b, c) : - L(a), R(a, b), S(b, c); relational
        {
            l = rename L { a - > a }; r = rename R { a - > a, b - > b }; s =
            rename S { b - > b, c - > c }; j0 = natural_join l with r; j1 =
            natural_join j0 with s; output j1 as answer(a, b, c);
        } indexes { i0 = @ r [0]; i1 = @ s [0]; } uses
        { j0.right = i0; j1.right = i1; } physical
        {
            relations
            {
                L = :: cq_example_providers :: slice_rows; R = ::
                cq_example_providers :: hash_relation; S = ::
                cq_example_providers :: sorted_relation; l = ::
                cq_example_providers :: slice_rows; r = ::
                cq_example_providers :: hash_relation; s = ::
                cq_example_providers :: sorted_relation; j0 = ::
                cq_example_providers :: borrowed_fields; j1 = ::
                cq_example_providers :: borrowed_fields;
            } indexes
            {
                i0 = :: cq_example_providers :: hash_relation; i1 = ::
                cq_example_providers :: sorted_relation;
            } lowerings
            {
                l = :: cq_example_providers :: rename; r = ::
                cq_example_providers :: rename; s = :: cq_example_providers ::
                rename; j0 = :: cq_example_providers :: indexed_join; j1 = ::
                cq_example_providers :: indexed_join;
            }
        }
    } environment
    {
        stored L at(__cq_store_L) operations :: cq_example_providers ::
        slice_rows stored R at(__cq_store_R) operations ::
        cq_example_providers :: hash_relation stored S at(__cq_store_S)
        operations :: cq_example_providers :: sorted_relation stored l
        at(__cq_store_l) operations :: cq_example_providers :: slice_rows
        stored r at(__cq_store_r) operations :: cq_example_providers ::
        hash_relation stored s at(__cq_store_s) operations ::
        cq_example_providers :: sorted_relation indexed i0 at(__cq_index_i0)
        operations :: cq_example_providers :: hash_relation region j0
        {
            j0(a, b) => ((a, b,)) : - l(a) => bind a <-
            ((__cq_store_l).iter()), r(a, b) => bind b <-
            ((__cq_index_i0).get(a).into_iter().flatten());
        } indexed i1 at(__cq_index_i1) operations :: cq_example_providers ::
        sorted_relation
    } request { lower j1 }
}

```

---

## 2172042/031-cq-example-providers::indexed_join

[Raw input](raw/2172042/031-cq-example-providers::indexed_join.input.tokens) · [Raw output](raw/2172042/031-cq-example-providers::indexed_join.output.tokens)

Actual input:

```rust
reply_to(:: mini_linq :: resume_physical)
{
    plan
    {
        struct Heterogeneous; relation L(a : Value); relation
        R(a : Value, b : Value); relation S(b : Value, c : Value);
        answer(a, b, c) : - L(a), R(a, b), S(b, c); relational
        {
            l = rename L { a - > a }; r = rename R { a - > a, b - > b }; s =
            rename S { b - > b, c - > c }; j0 = natural_join l with r; j1 =
            natural_join j0 with s; output j1 as answer(a, b, c);
        } indexes { i0 = @ r [0]; i1 = @ s [0]; } uses
        { j0.right = i0; j1.right = i1; } physical
        {
            relations
            {
                L = :: cq_example_providers :: slice_rows; R = ::
                cq_example_providers :: hash_relation; S = ::
                cq_example_providers :: sorted_relation; l = ::
                cq_example_providers :: slice_rows; r = ::
                cq_example_providers :: hash_relation; s = ::
                cq_example_providers :: sorted_relation; j0 = ::
                cq_example_providers :: borrowed_fields; j1 = ::
                cq_example_providers :: borrowed_fields;
            } indexes
            {
                i0 = :: cq_example_providers :: hash_relation; i1 = ::
                cq_example_providers :: sorted_relation;
            } lowerings
            {
                l = :: cq_example_providers :: rename; r = ::
                cq_example_providers :: rename; s = :: cq_example_providers ::
                rename; j0 = :: cq_example_providers :: indexed_join; j1 = ::
                cq_example_providers :: indexed_join;
            }
        }
    } environment
    {
        stored L at(__cq_store_L) operations :: cq_example_providers ::
        slice_rows stored R at(__cq_store_R) operations ::
        cq_example_providers :: hash_relation stored S at(__cq_store_S)
        operations :: cq_example_providers :: sorted_relation stored l
        at(__cq_store_l) operations :: cq_example_providers :: slice_rows
        stored r at(__cq_store_r) operations :: cq_example_providers ::
        hash_relation stored s at(__cq_store_s) operations ::
        cq_example_providers :: sorted_relation indexed i0 at(__cq_index_i0)
        operations :: cq_example_providers :: hash_relation region j0
        {
            j0(a, b) => ((a, b,)) : - l(a) => bind a <-
            ((__cq_store_l).iter()), r(a, b) => bind b <-
            ((__cq_index_i0).get(a).into_iter().flatten());
        } indexed i1 at(__cq_index_i1) operations :: cq_example_providers ::
        sorted_relation
    } emitted
    {
        let __cq_store_L : & [Value] = L; let __cq_store_R : & :: std ::
        collections :: HashMap < Value, :: std :: vec :: Vec < Value > > = R;
        let __cq_store_S : & :: std :: collections :: BTreeMap < Value, :: std
        :: vec :: Vec < Value > > = S; let __cq_store_l : & [Value] =
        __cq_store_L; let __cq_store_r : & :: std :: collections :: HashMap <
        Value, :: std :: vec :: Vec < Value > > = __cq_store_R; let
        __cq_store_s : & :: std :: collections :: BTreeMap < Value, :: std ::
        vec :: Vec < Value > > = __cq_store_S; let __cq_index_i0 : & :: std ::
        collections :: HashMap < Value, :: std :: vec :: Vec < Value > > =
        __cq_store_r; let __cq_index_i1 : & :: std :: collections :: BTreeMap
        < Value, :: std :: vec :: Vec < Value > > = __cq_store_s;
    } pending { lower j1 }
    continuations(:: mini_linq :: resume_physical, mini_linq :: access)
} plan
{
    struct Heterogeneous; relation L(a : Value); relation
    R(a : Value, b : Value); relation S(b : Value, c : Value); answer(a, b, c)
    : - L(a), R(a, b), S(b, c); relational
    {
        l = rename L { a - > a }; r = rename R { a - > a, b - > b }; s =
        rename S { b - > b, c - > c }; j0 = natural_join l with r; j1 =
        natural_join j0 with s; output j1 as answer(a, b, c);
    } indexes { i0 = @ r [0]; i1 = @ s [0]; } uses
    { j0.right = i0; j1.right = i1; } physical
    {
        relations
        {
            L = :: cq_example_providers :: slice_rows; R = ::
            cq_example_providers :: hash_relation; S = :: cq_example_providers
            :: sorted_relation; l = :: cq_example_providers :: slice_rows; r =
            :: cq_example_providers :: hash_relation; s = ::
            cq_example_providers :: sorted_relation; j0 = ::
            cq_example_providers :: borrowed_fields; j1 = ::
            cq_example_providers :: borrowed_fields;
        } indexes
        {
            i0 = :: cq_example_providers :: hash_relation; i1 = ::
            cq_example_providers :: sorted_relation;
        } lowerings
        {
            l = :: cq_example_providers :: rename; r = :: cq_example_providers
            :: rename; s = :: cq_example_providers :: rename; j0 = ::
            cq_example_providers :: indexed_join; j1 = :: cq_example_providers
            :: indexed_join;
        }
    }
} environment
{
    stored L at(__cq_store_L) operations :: cq_example_providers :: slice_rows
    stored R at(__cq_store_R) operations :: cq_example_providers ::
    hash_relation stored S at(__cq_store_S) operations :: cq_example_providers
    :: sorted_relation stored l at(__cq_store_l) operations ::
    cq_example_providers :: slice_rows stored r at(__cq_store_r) operations ::
    cq_example_providers :: hash_relation stored s at(__cq_store_s) operations
    :: cq_example_providers :: sorted_relation indexed i0 at(__cq_index_i0)
    operations :: cq_example_providers :: hash_relation region j0
    {
        j0(a, b) => ((a, b,)) : - l(a) => bind a <- ((__cq_store_l).iter()),
        r(a, b) => bind b <- ((__cq_index_i0).get(a).into_iter().flatten());
    } indexed i1 at(__cq_index_i1) operations :: cq_example_providers ::
    sorted_relation
} request { lower j1 }
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
                    struct Heterogeneous; relation L(a : Value); relation
                    R(a : Value, b : Value); relation S(b : Value, c : Value);
                    answer(a, b, c) : - L(a), R(a, b), S(b, c); relational
                    {
                        l = rename L { a - > a }; r = rename R { a - > a, b - > b };
                        s = rename S { b - > b, c - > c }; j0 = natural_join l with
                        r; j1 = natural_join j0 with s; output j1 as
                        answer(a, b, c);
                    } indexes { i0 = @ r [0]; i1 = @ s [0]; } uses
                    { j0.right = i0; j1.right = i1; } physical
                    {
                        relations
                        {
                            L = :: cq_example_providers :: slice_rows; R = ::
                            cq_example_providers :: hash_relation; S = ::
                            cq_example_providers :: sorted_relation; l = ::
                            cq_example_providers :: slice_rows; r = ::
                            cq_example_providers :: hash_relation; s = ::
                            cq_example_providers :: sorted_relation; j0 = ::
                            cq_example_providers :: borrowed_fields; j1 = ::
                            cq_example_providers :: borrowed_fields;
                        } indexes
                        {
                            i0 = :: cq_example_providers :: hash_relation; i1 = ::
                            cq_example_providers :: sorted_relation;
                        } lowerings
                        {
                            l = :: cq_example_providers :: rename; r = ::
                            cq_example_providers :: rename; s = :: cq_example_providers
                            :: rename; j0 = :: cq_example_providers :: indexed_join; j1
                            = :: cq_example_providers :: indexed_join;
                        }
                    }
                } environment
                {
                    stored L at(__cq_store_L) operations :: cq_example_providers
                    :: slice_rows stored R at(__cq_store_R) operations ::
                    cq_example_providers :: hash_relation stored S
                    at(__cq_store_S) operations :: cq_example_providers ::
                    sorted_relation stored l at(__cq_store_l) operations ::
                    cq_example_providers :: slice_rows stored r at(__cq_store_r)
                    operations :: cq_example_providers :: hash_relation stored s
                    at(__cq_store_s) operations :: cq_example_providers ::
                    sorted_relation indexed i0 at(__cq_index_i0) operations ::
                    cq_example_providers :: hash_relation region j0
                    {
                        j0(a, b) => ((a, b,)) : - l(a) => bind a <-
                        ((__cq_store_l).iter()), r(a, b) => bind b <-
                        ((__cq_index_i0).get(a).into_iter().flatten());
                    } indexed i1 at(__cq_index_i1) operations ::
                    cq_example_providers :: sorted_relation
                } emitted
                {
                    let __cq_store_L : & [Value] = L; let __cq_store_R : & ::
                    std :: collections :: HashMap < Value, :: std :: vec :: Vec
                    < Value > > = R; let __cq_store_S : & :: std :: collections
                    :: BTreeMap < Value, :: std :: vec :: Vec < Value > > = S;
                    let __cq_store_l : & [Value] = __cq_store_L; let
                    __cq_store_r : & :: std :: collections :: HashMap < Value,
                    :: std :: vec :: Vec < Value > > = __cq_store_R; let
                    __cq_store_s : & :: std :: collections :: BTreeMap < Value,
                    :: std :: vec :: Vec < Value > > = __cq_store_S; let
                    __cq_index_i0 : & :: std :: collections :: HashMap < Value,
                    :: std :: vec :: Vec < Value > > = __cq_store_r; let
                    __cq_index_i1 : & :: std :: collections :: BTreeMap < Value,
                    :: std :: vec :: Vec < Value > > = __cq_store_s;
                } pending { lower j1 }
                continuations(:: mini_linq :: resume_physical, mini_linq ::
                access)
            } plan
            {
                struct Heterogeneous; relation L(a : Value); relation
                R(a : Value, b : Value); relation S(b : Value, c : Value);
                answer(a, b, c) : - L(a), R(a, b), S(b, c); relational
                {
                    l = rename L { a - > a }; r = rename R { a - > a, b - > b };
                    s = rename S { b - > b, c - > c }; j0 = natural_join l with
                    r; j1 = natural_join j0 with s; output j1 as
                    answer(a, b, c);
                } indexes { i0 = @ r [0]; i1 = @ s [0]; } uses
                { j0.right = i0; j1.right = i1; } physical
                {
                    relations
                    {
                        L = :: cq_example_providers :: slice_rows; R = ::
                        cq_example_providers :: hash_relation; S = ::
                        cq_example_providers :: sorted_relation; l = ::
                        cq_example_providers :: slice_rows; r = ::
                        cq_example_providers :: hash_relation; s = ::
                        cq_example_providers :: sorted_relation; j0 = ::
                        cq_example_providers :: borrowed_fields; j1 = ::
                        cq_example_providers :: borrowed_fields;
                    } indexes
                    {
                        i0 = :: cq_example_providers :: hash_relation; i1 = ::
                        cq_example_providers :: sorted_relation;
                    } lowerings
                    {
                        l = :: cq_example_providers :: rename; r = ::
                        cq_example_providers :: rename; s = :: cq_example_providers
                        :: rename; j0 = :: cq_example_providers :: indexed_join; j1
                        = :: cq_example_providers :: indexed_join;
                    }
                }
            } environment
            {
                stored L at(__cq_store_L) operations :: cq_example_providers
                :: slice_rows stored R at(__cq_store_R) operations ::
                cq_example_providers :: hash_relation stored S
                at(__cq_store_S) operations :: cq_example_providers ::
                sorted_relation stored l at(__cq_store_l) operations ::
                cq_example_providers :: slice_rows stored r at(__cq_store_r)
                operations :: cq_example_providers :: hash_relation stored s
                at(__cq_store_s) operations :: cq_example_providers ::
                sorted_relation indexed i0 at(__cq_index_i0) operations ::
                cq_example_providers :: hash_relation region j0
                {
                    j0(a, b) => ((a, b,)) : - l(a) => bind a <-
                    ((__cq_store_l).iter()), r(a, b) => bind b <-
                    ((__cq_index_i0).get(a).into_iter().flatten());
                } indexed i1 at(__cq_index_i1) operations ::
                cq_example_providers :: sorted_relation
            } request { lower j1 }
        } access
        {
            l(a) => bind a <- ((__cq_store_l).iter()), r(a, b) => bind b <-
            ((__cq_index_i0).get(a).into_iter().flatten())
        } witness { row((a, b,)) fields { 0 = (a); 1 = (b); } }
    } plan
    {
        struct Heterogeneous; relation L(a : Value); relation
        R(a : Value, b : Value); relation S(b : Value, c : Value);
        answer(a, b, c) : - L(a), R(a, b), S(b, c); relational
        {
            l = rename L { a - > a }; r = rename R { a - > a, b - > b }; s =
            rename S { b - > b, c - > c }; j0 = natural_join l with r; j1 =
            natural_join j0 with s; output j1 as answer(a, b, c);
        } indexes { i0 = @ r [0]; i1 = @ s [0]; } uses
        { j0.right = i0; j1.right = i1; } physical
        {
            relations
            {
                L = :: cq_example_providers :: slice_rows; R = ::
                cq_example_providers :: hash_relation; S = ::
                cq_example_providers :: sorted_relation; l = ::
                cq_example_providers :: slice_rows; r = ::
                cq_example_providers :: hash_relation; s = ::
                cq_example_providers :: sorted_relation; j0 = ::
                cq_example_providers :: borrowed_fields; j1 = ::
                cq_example_providers :: borrowed_fields;
            } indexes
            {
                i0 = :: cq_example_providers :: hash_relation; i1 = ::
                cq_example_providers :: sorted_relation;
            } lowerings
            {
                l = :: cq_example_providers :: rename; r = ::
                cq_example_providers :: rename; s = :: cq_example_providers ::
                rename; j0 = :: cq_example_providers :: indexed_join; j1 = ::
                cq_example_providers :: indexed_join;
            }
        }
    } environment
    {
        stored L at(__cq_store_L) operations :: cq_example_providers ::
        slice_rows stored R at(__cq_store_R) operations ::
        cq_example_providers :: hash_relation stored S at(__cq_store_S)
        operations :: cq_example_providers :: sorted_relation stored l
        at(__cq_store_l) operations :: cq_example_providers :: slice_rows
        stored r at(__cq_store_r) operations :: cq_example_providers ::
        hash_relation stored s at(__cq_store_s) operations ::
        cq_example_providers :: sorted_relation indexed i0 at(__cq_index_i0)
        operations :: cq_example_providers :: hash_relation region j0
        {
            j0(a, b) => ((a, b,)) : - l(a) => bind a <-
            ((__cq_store_l).iter()), r(a, b) => bind b <-
            ((__cq_index_i0).get(a).into_iter().flatten());
        } indexed i1 at(__cq_index_i1) operations :: cq_example_providers ::
        sorted_relation
    } request { matches i1 values [(b)] }
}

```

---

## 2172042/032-cq-example-providers::sorted_relation

[Raw input](raw/2172042/032-cq-example-providers::sorted_relation.input.tokens) · [Raw output](raw/2172042/032-cq-example-providers::sorted_relation.output.tokens)

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
                struct Heterogeneous; relation L(a : Value); relation
                R(a : Value, b : Value); relation S(b : Value, c : Value);
                answer(a, b, c) : - L(a), R(a, b), S(b, c); relational
                {
                    l = rename L { a - > a }; r = rename R { a - > a, b - > b };
                    s = rename S { b - > b, c - > c }; j0 = natural_join l with
                    r; j1 = natural_join j0 with s; output j1 as
                    answer(a, b, c);
                } indexes { i0 = @ r [0]; i1 = @ s [0]; } uses
                { j0.right = i0; j1.right = i1; } physical
                {
                    relations
                    {
                        L = :: cq_example_providers :: slice_rows; R = ::
                        cq_example_providers :: hash_relation; S = ::
                        cq_example_providers :: sorted_relation; l = ::
                        cq_example_providers :: slice_rows; r = ::
                        cq_example_providers :: hash_relation; s = ::
                        cq_example_providers :: sorted_relation; j0 = ::
                        cq_example_providers :: borrowed_fields; j1 = ::
                        cq_example_providers :: borrowed_fields;
                    } indexes
                    {
                        i0 = :: cq_example_providers :: hash_relation; i1 = ::
                        cq_example_providers :: sorted_relation;
                    } lowerings
                    {
                        l = :: cq_example_providers :: rename; r = ::
                        cq_example_providers :: rename; s = :: cq_example_providers
                        :: rename; j0 = :: cq_example_providers :: indexed_join; j1
                        = :: cq_example_providers :: indexed_join;
                    }
                }
            } environment
            {
                stored L at(__cq_store_L) operations :: cq_example_providers
                :: slice_rows stored R at(__cq_store_R) operations ::
                cq_example_providers :: hash_relation stored S
                at(__cq_store_S) operations :: cq_example_providers ::
                sorted_relation stored l at(__cq_store_l) operations ::
                cq_example_providers :: slice_rows stored r at(__cq_store_r)
                operations :: cq_example_providers :: hash_relation stored s
                at(__cq_store_s) operations :: cq_example_providers ::
                sorted_relation indexed i0 at(__cq_index_i0) operations ::
                cq_example_providers :: hash_relation region j0
                {
                    j0(a, b) => ((a, b,)) : - l(a) => bind a <-
                    ((__cq_store_l).iter()), r(a, b) => bind b <-
                    ((__cq_index_i0).get(a).into_iter().flatten());
                } indexed i1 at(__cq_index_i1) operations ::
                cq_example_providers :: sorted_relation
            } emitted
            {
                let __cq_store_L : & [Value] = L; let __cq_store_R : & :: std
                :: collections :: HashMap < Value, :: std :: vec :: Vec <
                Value > > = R; let __cq_store_S : & :: std :: collections ::
                BTreeMap < Value, :: std :: vec :: Vec < Value > > = S; let
                __cq_store_l : & [Value] = __cq_store_L; let __cq_store_r : &
                :: std :: collections :: HashMap < Value, :: std :: vec :: Vec
                < Value > > = __cq_store_R; let __cq_store_s : & :: std ::
                collections :: BTreeMap < Value, :: std :: vec :: Vec < Value
                > > = __cq_store_S; let __cq_index_i0 : & :: std ::
                collections :: HashMap < Value, :: std :: vec :: Vec < Value >
                > = __cq_store_r; let __cq_index_i1 : & :: std :: collections
                :: BTreeMap < Value, :: std :: vec :: Vec < Value > > =
                __cq_store_s;
            } pending { lower j1 }
            continuations(:: mini_linq :: resume_physical, mini_linq ::
            access)
        } plan
        {
            struct Heterogeneous; relation L(a : Value); relation
            R(a : Value, b : Value); relation S(b : Value, c : Value);
            answer(a, b, c) : - L(a), R(a, b), S(b, c); relational
            {
                l = rename L { a - > a }; r = rename R { a - > a, b - > b }; s
                = rename S { b - > b, c - > c }; j0 = natural_join l with r;
                j1 = natural_join j0 with s; output j1 as answer(a, b, c);
            } indexes { i0 = @ r [0]; i1 = @ s [0]; } uses
            { j0.right = i0; j1.right = i1; } physical
            {
                relations
                {
                    L = :: cq_example_providers :: slice_rows; R = ::
                    cq_example_providers :: hash_relation; S = ::
                    cq_example_providers :: sorted_relation; l = ::
                    cq_example_providers :: slice_rows; r = ::
                    cq_example_providers :: hash_relation; s = ::
                    cq_example_providers :: sorted_relation; j0 = ::
                    cq_example_providers :: borrowed_fields; j1 = ::
                    cq_example_providers :: borrowed_fields;
                } indexes
                {
                    i0 = :: cq_example_providers :: hash_relation; i1 = ::
                    cq_example_providers :: sorted_relation;
                } lowerings
                {
                    l = :: cq_example_providers :: rename; r = ::
                    cq_example_providers :: rename; s = :: cq_example_providers
                    :: rename; j0 = :: cq_example_providers :: indexed_join; j1
                    = :: cq_example_providers :: indexed_join;
                }
            }
        } environment
        {
            stored L at(__cq_store_L) operations :: cq_example_providers ::
            slice_rows stored R at(__cq_store_R) operations ::
            cq_example_providers :: hash_relation stored S at(__cq_store_S)
            operations :: cq_example_providers :: sorted_relation stored l
            at(__cq_store_l) operations :: cq_example_providers :: slice_rows
            stored r at(__cq_store_r) operations :: cq_example_providers ::
            hash_relation stored s at(__cq_store_s) operations ::
            cq_example_providers :: sorted_relation indexed i0
            at(__cq_index_i0) operations :: cq_example_providers ::
            hash_relation region j0
            {
                j0(a, b) => ((a, b,)) : - l(a) => bind a <-
                ((__cq_store_l).iter()), r(a, b) => bind b <-
                ((__cq_index_i0).get(a).into_iter().flatten());
            } indexed i1 at(__cq_index_i1) operations :: cq_example_providers
            :: sorted_relation
        } request { lower j1 }
    } access
    {
        l(a) => bind a <- ((__cq_store_l).iter()), r(a, b) => bind b <-
        ((__cq_index_i0).get(a).into_iter().flatten())
    } witness { row((a, b,)) fields { 0 = (a); 1 = (b); } }
} plan
{
    struct Heterogeneous; relation L(a : Value); relation
    R(a : Value, b : Value); relation S(b : Value, c : Value); answer(a, b, c)
    : - L(a), R(a, b), S(b, c); relational
    {
        l = rename L { a - > a }; r = rename R { a - > a, b - > b }; s =
        rename S { b - > b, c - > c }; j0 = natural_join l with r; j1 =
        natural_join j0 with s; output j1 as answer(a, b, c);
    } indexes { i0 = @ r [0]; i1 = @ s [0]; } uses
    { j0.right = i0; j1.right = i1; } physical
    {
        relations
        {
            L = :: cq_example_providers :: slice_rows; R = ::
            cq_example_providers :: hash_relation; S = :: cq_example_providers
            :: sorted_relation; l = :: cq_example_providers :: slice_rows; r =
            :: cq_example_providers :: hash_relation; s = ::
            cq_example_providers :: sorted_relation; j0 = ::
            cq_example_providers :: borrowed_fields; j1 = ::
            cq_example_providers :: borrowed_fields;
        } indexes
        {
            i0 = :: cq_example_providers :: hash_relation; i1 = ::
            cq_example_providers :: sorted_relation;
        } lowerings
        {
            l = :: cq_example_providers :: rename; r = :: cq_example_providers
            :: rename; s = :: cq_example_providers :: rename; j0 = ::
            cq_example_providers :: indexed_join; j1 = :: cq_example_providers
            :: indexed_join;
        }
    }
} environment
{
    stored L at(__cq_store_L) operations :: cq_example_providers :: slice_rows
    stored R at(__cq_store_R) operations :: cq_example_providers ::
    hash_relation stored S at(__cq_store_S) operations :: cq_example_providers
    :: sorted_relation stored l at(__cq_store_l) operations ::
    cq_example_providers :: slice_rows stored r at(__cq_store_r) operations ::
    cq_example_providers :: hash_relation stored s at(__cq_store_s) operations
    :: cq_example_providers :: sorted_relation indexed i0 at(__cq_index_i0)
    operations :: cq_example_providers :: hash_relation region j0
    {
        j0(a, b) => ((a, b,)) : - l(a) => bind a <- ((__cq_store_l).iter()),
        r(a, b) => bind b <- ((__cq_index_i0).get(a).into_iter().flatten());
    } indexed i1 at(__cq_index_i1) operations :: cq_example_providers ::
    sorted_relation
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
                    struct Heterogeneous; relation L(a : Value); relation
                    R(a : Value, b : Value); relation S(b : Value, c : Value);
                    answer(a, b, c) : - L(a), R(a, b), S(b, c); relational
                    {
                        l = rename L { a - > a }; r = rename R { a - > a, b - > b };
                        s = rename S { b - > b, c - > c }; j0 = natural_join l with
                        r; j1 = natural_join j0 with s; output j1 as
                        answer(a, b, c);
                    } indexes { i0 = @ r [0]; i1 = @ s [0]; } uses
                    { j0.right = i0; j1.right = i1; } physical
                    {
                        relations
                        {
                            L = :: cq_example_providers :: slice_rows; R = ::
                            cq_example_providers :: hash_relation; S = ::
                            cq_example_providers :: sorted_relation; l = ::
                            cq_example_providers :: slice_rows; r = ::
                            cq_example_providers :: hash_relation; s = ::
                            cq_example_providers :: sorted_relation; j0 = ::
                            cq_example_providers :: borrowed_fields; j1 = ::
                            cq_example_providers :: borrowed_fields;
                        } indexes
                        {
                            i0 = :: cq_example_providers :: hash_relation; i1 = ::
                            cq_example_providers :: sorted_relation;
                        } lowerings
                        {
                            l = :: cq_example_providers :: rename; r = ::
                            cq_example_providers :: rename; s = :: cq_example_providers
                            :: rename; j0 = :: cq_example_providers :: indexed_join; j1
                            = :: cq_example_providers :: indexed_join;
                        }
                    }
                } environment
                {
                    stored L at(__cq_store_L) operations :: cq_example_providers
                    :: slice_rows stored R at(__cq_store_R) operations ::
                    cq_example_providers :: hash_relation stored S
                    at(__cq_store_S) operations :: cq_example_providers ::
                    sorted_relation stored l at(__cq_store_l) operations ::
                    cq_example_providers :: slice_rows stored r at(__cq_store_r)
                    operations :: cq_example_providers :: hash_relation stored s
                    at(__cq_store_s) operations :: cq_example_providers ::
                    sorted_relation indexed i0 at(__cq_index_i0) operations ::
                    cq_example_providers :: hash_relation region j0
                    {
                        j0(a, b) => ((a, b,)) : - l(a) => bind a <-
                        ((__cq_store_l).iter()), r(a, b) => bind b <-
                        ((__cq_index_i0).get(a).into_iter().flatten());
                    } indexed i1 at(__cq_index_i1) operations ::
                    cq_example_providers :: sorted_relation
                } emitted
                {
                    let __cq_store_L : & [Value] = L; let __cq_store_R : & ::
                    std :: collections :: HashMap < Value, :: std :: vec :: Vec
                    < Value > > = R; let __cq_store_S : & :: std :: collections
                    :: BTreeMap < Value, :: std :: vec :: Vec < Value > > = S;
                    let __cq_store_l : & [Value] = __cq_store_L; let
                    __cq_store_r : & :: std :: collections :: HashMap < Value,
                    :: std :: vec :: Vec < Value > > = __cq_store_R; let
                    __cq_store_s : & :: std :: collections :: BTreeMap < Value,
                    :: std :: vec :: Vec < Value > > = __cq_store_S; let
                    __cq_index_i0 : & :: std :: collections :: HashMap < Value,
                    :: std :: vec :: Vec < Value > > = __cq_store_r; let
                    __cq_index_i1 : & :: std :: collections :: BTreeMap < Value,
                    :: std :: vec :: Vec < Value > > = __cq_store_s;
                } pending { lower j1 }
                continuations(:: mini_linq :: resume_physical, mini_linq ::
                access)
            } plan
            {
                struct Heterogeneous; relation L(a : Value); relation
                R(a : Value, b : Value); relation S(b : Value, c : Value);
                answer(a, b, c) : - L(a), R(a, b), S(b, c); relational
                {
                    l = rename L { a - > a }; r = rename R { a - > a, b - > b };
                    s = rename S { b - > b, c - > c }; j0 = natural_join l with
                    r; j1 = natural_join j0 with s; output j1 as
                    answer(a, b, c);
                } indexes { i0 = @ r [0]; i1 = @ s [0]; } uses
                { j0.right = i0; j1.right = i1; } physical
                {
                    relations
                    {
                        L = :: cq_example_providers :: slice_rows; R = ::
                        cq_example_providers :: hash_relation; S = ::
                        cq_example_providers :: sorted_relation; l = ::
                        cq_example_providers :: slice_rows; r = ::
                        cq_example_providers :: hash_relation; s = ::
                        cq_example_providers :: sorted_relation; j0 = ::
                        cq_example_providers :: borrowed_fields; j1 = ::
                        cq_example_providers :: borrowed_fields;
                    } indexes
                    {
                        i0 = :: cq_example_providers :: hash_relation; i1 = ::
                        cq_example_providers :: sorted_relation;
                    } lowerings
                    {
                        l = :: cq_example_providers :: rename; r = ::
                        cq_example_providers :: rename; s = :: cq_example_providers
                        :: rename; j0 = :: cq_example_providers :: indexed_join; j1
                        = :: cq_example_providers :: indexed_join;
                    }
                }
            } environment
            {
                stored L at(__cq_store_L) operations :: cq_example_providers
                :: slice_rows stored R at(__cq_store_R) operations ::
                cq_example_providers :: hash_relation stored S
                at(__cq_store_S) operations :: cq_example_providers ::
                sorted_relation stored l at(__cq_store_l) operations ::
                cq_example_providers :: slice_rows stored r at(__cq_store_r)
                operations :: cq_example_providers :: hash_relation stored s
                at(__cq_store_s) operations :: cq_example_providers ::
                sorted_relation indexed i0 at(__cq_index_i0) operations ::
                cq_example_providers :: hash_relation region j0
                {
                    j0(a, b) => ((a, b,)) : - l(a) => bind a <-
                    ((__cq_store_l).iter()), r(a, b) => bind b <-
                    ((__cq_index_i0).get(a).into_iter().flatten());
                } indexed i1 at(__cq_index_i1) operations ::
                cq_example_providers :: sorted_relation
            } request { lower j1 }
        } access
        {
            l(a) => bind a <- ((__cq_store_l).iter()), r(a, b) => bind b <-
            ((__cq_index_i0).get(a).into_iter().flatten())
        } witness { row((a, b,)) fields { 0 = (a); 1 = (b); } }
    } source((__cq_index_i1).get(b).into_iter().flatten()) item(c) witness
    { row((b, c,)) fields { 0 = (b); 1 = (c); } }
}

```

---

## 2172042/033-cq-example-providers::indexed_join_matches

[Raw input](raw/2172042/033-cq-example-providers::indexed_join_matches.input.tokens) · [Raw output](raw/2172042/033-cq-example-providers::indexed_join_matches.output.tokens)

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
                struct Heterogeneous; relation L(a : Value); relation
                R(a : Value, b : Value); relation S(b : Value, c : Value);
                answer(a, b, c) : - L(a), R(a, b), S(b, c); relational
                {
                    l = rename L { a - > a }; r = rename R { a - > a, b - > b };
                    s = rename S { b - > b, c - > c }; j0 = natural_join l with
                    r; j1 = natural_join j0 with s; output j1 as
                    answer(a, b, c);
                } indexes { i0 = @ r [0]; i1 = @ s [0]; } uses
                { j0.right = i0; j1.right = i1; } physical
                {
                    relations
                    {
                        L = :: cq_example_providers :: slice_rows; R = ::
                        cq_example_providers :: hash_relation; S = ::
                        cq_example_providers :: sorted_relation; l = ::
                        cq_example_providers :: slice_rows; r = ::
                        cq_example_providers :: hash_relation; s = ::
                        cq_example_providers :: sorted_relation; j0 = ::
                        cq_example_providers :: borrowed_fields; j1 = ::
                        cq_example_providers :: borrowed_fields;
                    } indexes
                    {
                        i0 = :: cq_example_providers :: hash_relation; i1 = ::
                        cq_example_providers :: sorted_relation;
                    } lowerings
                    {
                        l = :: cq_example_providers :: rename; r = ::
                        cq_example_providers :: rename; s = :: cq_example_providers
                        :: rename; j0 = :: cq_example_providers :: indexed_join; j1
                        = :: cq_example_providers :: indexed_join;
                    }
                }
            } environment
            {
                stored L at(__cq_store_L) operations :: cq_example_providers
                :: slice_rows stored R at(__cq_store_R) operations ::
                cq_example_providers :: hash_relation stored S
                at(__cq_store_S) operations :: cq_example_providers ::
                sorted_relation stored l at(__cq_store_l) operations ::
                cq_example_providers :: slice_rows stored r at(__cq_store_r)
                operations :: cq_example_providers :: hash_relation stored s
                at(__cq_store_s) operations :: cq_example_providers ::
                sorted_relation indexed i0 at(__cq_index_i0) operations ::
                cq_example_providers :: hash_relation region j0
                {
                    j0(a, b) => ((a, b,)) : - l(a) => bind a <-
                    ((__cq_store_l).iter()), r(a, b) => bind b <-
                    ((__cq_index_i0).get(a).into_iter().flatten());
                } indexed i1 at(__cq_index_i1) operations ::
                cq_example_providers :: sorted_relation
            } emitted
            {
                let __cq_store_L : & [Value] = L; let __cq_store_R : & :: std
                :: collections :: HashMap < Value, :: std :: vec :: Vec <
                Value > > = R; let __cq_store_S : & :: std :: collections ::
                BTreeMap < Value, :: std :: vec :: Vec < Value > > = S; let
                __cq_store_l : & [Value] = __cq_store_L; let __cq_store_r : &
                :: std :: collections :: HashMap < Value, :: std :: vec :: Vec
                < Value > > = __cq_store_R; let __cq_store_s : & :: std ::
                collections :: BTreeMap < Value, :: std :: vec :: Vec < Value
                > > = __cq_store_S; let __cq_index_i0 : & :: std ::
                collections :: HashMap < Value, :: std :: vec :: Vec < Value >
                > = __cq_store_r; let __cq_index_i1 : & :: std :: collections
                :: BTreeMap < Value, :: std :: vec :: Vec < Value > > =
                __cq_store_s;
            } pending { lower j1 }
            continuations(:: mini_linq :: resume_physical, mini_linq ::
            access)
        } plan
        {
            struct Heterogeneous; relation L(a : Value); relation
            R(a : Value, b : Value); relation S(b : Value, c : Value);
            answer(a, b, c) : - L(a), R(a, b), S(b, c); relational
            {
                l = rename L { a - > a }; r = rename R { a - > a, b - > b }; s
                = rename S { b - > b, c - > c }; j0 = natural_join l with r;
                j1 = natural_join j0 with s; output j1 as answer(a, b, c);
            } indexes { i0 = @ r [0]; i1 = @ s [0]; } uses
            { j0.right = i0; j1.right = i1; } physical
            {
                relations
                {
                    L = :: cq_example_providers :: slice_rows; R = ::
                    cq_example_providers :: hash_relation; S = ::
                    cq_example_providers :: sorted_relation; l = ::
                    cq_example_providers :: slice_rows; r = ::
                    cq_example_providers :: hash_relation; s = ::
                    cq_example_providers :: sorted_relation; j0 = ::
                    cq_example_providers :: borrowed_fields; j1 = ::
                    cq_example_providers :: borrowed_fields;
                } indexes
                {
                    i0 = :: cq_example_providers :: hash_relation; i1 = ::
                    cq_example_providers :: sorted_relation;
                } lowerings
                {
                    l = :: cq_example_providers :: rename; r = ::
                    cq_example_providers :: rename; s = :: cq_example_providers
                    :: rename; j0 = :: cq_example_providers :: indexed_join; j1
                    = :: cq_example_providers :: indexed_join;
                }
            }
        } environment
        {
            stored L at(__cq_store_L) operations :: cq_example_providers ::
            slice_rows stored R at(__cq_store_R) operations ::
            cq_example_providers :: hash_relation stored S at(__cq_store_S)
            operations :: cq_example_providers :: sorted_relation stored l
            at(__cq_store_l) operations :: cq_example_providers :: slice_rows
            stored r at(__cq_store_r) operations :: cq_example_providers ::
            hash_relation stored s at(__cq_store_s) operations ::
            cq_example_providers :: sorted_relation indexed i0
            at(__cq_index_i0) operations :: cq_example_providers ::
            hash_relation region j0
            {
                j0(a, b) => ((a, b,)) : - l(a) => bind a <-
                ((__cq_store_l).iter()), r(a, b) => bind b <-
                ((__cq_index_i0).get(a).into_iter().flatten());
            } indexed i1 at(__cq_index_i1) operations :: cq_example_providers
            :: sorted_relation
        } request { lower j1 }
    } access
    {
        l(a) => bind a <- ((__cq_store_l).iter()), r(a, b) => bind b <-
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
                    struct Heterogeneous; relation L(a : Value); relation
                    R(a : Value, b : Value); relation S(b : Value, c : Value);
                    answer(a, b, c) : - L(a), R(a, b), S(b, c); relational
                    {
                        l = rename L { a - > a }; r = rename R { a - > a, b - > b };
                        s = rename S { b - > b, c - > c }; j0 = natural_join l with
                        r; j1 = natural_join j0 with s; output j1 as
                        answer(a, b, c);
                    } indexes { i0 = @ r [0]; i1 = @ s [0]; } uses
                    { j0.right = i0; j1.right = i1; } physical
                    {
                        relations
                        {
                            L = :: cq_example_providers :: slice_rows; R = ::
                            cq_example_providers :: hash_relation; S = ::
                            cq_example_providers :: sorted_relation; l = ::
                            cq_example_providers :: slice_rows; r = ::
                            cq_example_providers :: hash_relation; s = ::
                            cq_example_providers :: sorted_relation; j0 = ::
                            cq_example_providers :: borrowed_fields; j1 = ::
                            cq_example_providers :: borrowed_fields;
                        } indexes
                        {
                            i0 = :: cq_example_providers :: hash_relation; i1 = ::
                            cq_example_providers :: sorted_relation;
                        } lowerings
                        {
                            l = :: cq_example_providers :: rename; r = ::
                            cq_example_providers :: rename; s = :: cq_example_providers
                            :: rename; j0 = :: cq_example_providers :: indexed_join; j1
                            = :: cq_example_providers :: indexed_join;
                        }
                    }
                } environment
                {
                    stored L at(__cq_store_L) operations :: cq_example_providers
                    :: slice_rows stored R at(__cq_store_R) operations ::
                    cq_example_providers :: hash_relation stored S
                    at(__cq_store_S) operations :: cq_example_providers ::
                    sorted_relation stored l at(__cq_store_l) operations ::
                    cq_example_providers :: slice_rows stored r at(__cq_store_r)
                    operations :: cq_example_providers :: hash_relation stored s
                    at(__cq_store_s) operations :: cq_example_providers ::
                    sorted_relation indexed i0 at(__cq_index_i0) operations ::
                    cq_example_providers :: hash_relation region j0
                    {
                        j0(a, b) => ((a, b,)) : - l(a) => bind a <-
                        ((__cq_store_l).iter()), r(a, b) => bind b <-
                        ((__cq_index_i0).get(a).into_iter().flatten());
                    } indexed i1 at(__cq_index_i1) operations ::
                    cq_example_providers :: sorted_relation
                } emitted
                {
                    let __cq_store_L : & [Value] = L; let __cq_store_R : & ::
                    std :: collections :: HashMap < Value, :: std :: vec :: Vec
                    < Value > > = R; let __cq_store_S : & :: std :: collections
                    :: BTreeMap < Value, :: std :: vec :: Vec < Value > > = S;
                    let __cq_store_l : & [Value] = __cq_store_L; let
                    __cq_store_r : & :: std :: collections :: HashMap < Value,
                    :: std :: vec :: Vec < Value > > = __cq_store_R; let
                    __cq_store_s : & :: std :: collections :: BTreeMap < Value,
                    :: std :: vec :: Vec < Value > > = __cq_store_S; let
                    __cq_index_i0 : & :: std :: collections :: HashMap < Value,
                    :: std :: vec :: Vec < Value > > = __cq_store_r; let
                    __cq_index_i1 : & :: std :: collections :: BTreeMap < Value,
                    :: std :: vec :: Vec < Value > > = __cq_store_s;
                } pending { lower j1 }
                continuations(:: mini_linq :: resume_physical, mini_linq ::
                access)
            } plan
            {
                struct Heterogeneous; relation L(a : Value); relation
                R(a : Value, b : Value); relation S(b : Value, c : Value);
                answer(a, b, c) : - L(a), R(a, b), S(b, c); relational
                {
                    l = rename L { a - > a }; r = rename R { a - > a, b - > b };
                    s = rename S { b - > b, c - > c }; j0 = natural_join l with
                    r; j1 = natural_join j0 with s; output j1 as
                    answer(a, b, c);
                } indexes { i0 = @ r [0]; i1 = @ s [0]; } uses
                { j0.right = i0; j1.right = i1; } physical
                {
                    relations
                    {
                        L = :: cq_example_providers :: slice_rows; R = ::
                        cq_example_providers :: hash_relation; S = ::
                        cq_example_providers :: sorted_relation; l = ::
                        cq_example_providers :: slice_rows; r = ::
                        cq_example_providers :: hash_relation; s = ::
                        cq_example_providers :: sorted_relation; j0 = ::
                        cq_example_providers :: borrowed_fields; j1 = ::
                        cq_example_providers :: borrowed_fields;
                    } indexes
                    {
                        i0 = :: cq_example_providers :: hash_relation; i1 = ::
                        cq_example_providers :: sorted_relation;
                    } lowerings
                    {
                        l = :: cq_example_providers :: rename; r = ::
                        cq_example_providers :: rename; s = :: cq_example_providers
                        :: rename; j0 = :: cq_example_providers :: indexed_join; j1
                        = :: cq_example_providers :: indexed_join;
                    }
                }
            } environment
            {
                stored L at(__cq_store_L) operations :: cq_example_providers
                :: slice_rows stored R at(__cq_store_R) operations ::
                cq_example_providers :: hash_relation stored S
                at(__cq_store_S) operations :: cq_example_providers ::
                sorted_relation stored l at(__cq_store_l) operations ::
                cq_example_providers :: slice_rows stored r at(__cq_store_r)
                operations :: cq_example_providers :: hash_relation stored s
                at(__cq_store_s) operations :: cq_example_providers ::
                sorted_relation indexed i0 at(__cq_index_i0) operations ::
                cq_example_providers :: hash_relation region j0
                {
                    j0(a, b) => ((a, b,)) : - l(a) => bind a <-
                    ((__cq_store_l).iter()), r(a, b) => bind b <-
                    ((__cq_index_i0).get(a).into_iter().flatten());
                } indexed i1 at(__cq_index_i1) operations ::
                cq_example_providers :: sorted_relation
            } request { lower j1 }
        } access
        {
            l(a) => bind a <- ((__cq_store_l).iter()), r(a, b) => bind b <-
            ((__cq_index_i0).get(a).into_iter().flatten()), s(b, c) => bind c
            <- ((__cq_index_i1).get(b).into_iter().flatten())
        } witness { row((a, b,)) fields { 0 = (a); 1 = (b); } }
    } plan
    {
        struct Heterogeneous; relation L(a : Value); relation
        R(a : Value, b : Value); relation S(b : Value, c : Value);
        answer(a, b, c) : - L(a), R(a, b), S(b, c); relational
        {
            l = rename L { a - > a }; r = rename R { a - > a, b - > b }; s =
            rename S { b - > b, c - > c }; j0 = natural_join l with r; j1 =
            natural_join j0 with s; output j1 as answer(a, b, c);
        } indexes { i0 = @ r [0]; i1 = @ s [0]; } uses
        { j0.right = i0; j1.right = i1; } physical
        {
            relations
            {
                L = :: cq_example_providers :: slice_rows; R = ::
                cq_example_providers :: hash_relation; S = ::
                cq_example_providers :: sorted_relation; l = ::
                cq_example_providers :: slice_rows; r = ::
                cq_example_providers :: hash_relation; s = ::
                cq_example_providers :: sorted_relation; j0 = ::
                cq_example_providers :: borrowed_fields; j1 = ::
                cq_example_providers :: borrowed_fields;
            } indexes
            {
                i0 = :: cq_example_providers :: hash_relation; i1 = ::
                cq_example_providers :: sorted_relation;
            } lowerings
            {
                l = :: cq_example_providers :: rename; r = ::
                cq_example_providers :: rename; s = :: cq_example_providers ::
                rename; j0 = :: cq_example_providers :: indexed_join; j1 = ::
                cq_example_providers :: indexed_join;
            }
        }
    } environment
    {
        stored L at(__cq_store_L) operations :: cq_example_providers ::
        slice_rows stored R at(__cq_store_R) operations ::
        cq_example_providers :: hash_relation stored S at(__cq_store_S)
        operations :: cq_example_providers :: sorted_relation stored l
        at(__cq_store_l) operations :: cq_example_providers :: slice_rows
        stored r at(__cq_store_r) operations :: cq_example_providers ::
        hash_relation stored s at(__cq_store_s) operations ::
        cq_example_providers :: sorted_relation indexed i0 at(__cq_index_i0)
        operations :: cq_example_providers :: hash_relation region j0
        {
            j0(a, b) => ((a, b,)) : - l(a) => bind a <-
            ((__cq_store_l).iter()), r(a, b) => bind b <-
            ((__cq_index_i0).get(a).into_iter().flatten());
        } indexed i1 at(__cq_index_i1) operations :: cq_example_providers ::
        sorted_relation
    } request
    {
        output j1 fields { 0 = (a); 1 = (b); 2 = (c); } whole
        { j0 = ((a, b,)); s = ((b, c,)); }
    }
}

```

---

## 2172042/034-cq-example-providers::borrowed_fields

[Raw input](raw/2172042/034-cq-example-providers::borrowed_fields.input.tokens) · [Raw output](raw/2172042/034-cq-example-providers::borrowed_fields.output.tokens)

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
                struct Heterogeneous; relation L(a : Value); relation
                R(a : Value, b : Value); relation S(b : Value, c : Value);
                answer(a, b, c) : - L(a), R(a, b), S(b, c); relational
                {
                    l = rename L { a - > a }; r = rename R { a - > a, b - > b };
                    s = rename S { b - > b, c - > c }; j0 = natural_join l with
                    r; j1 = natural_join j0 with s; output j1 as
                    answer(a, b, c);
                } indexes { i0 = @ r [0]; i1 = @ s [0]; } uses
                { j0.right = i0; j1.right = i1; } physical
                {
                    relations
                    {
                        L = :: cq_example_providers :: slice_rows; R = ::
                        cq_example_providers :: hash_relation; S = ::
                        cq_example_providers :: sorted_relation; l = ::
                        cq_example_providers :: slice_rows; r = ::
                        cq_example_providers :: hash_relation; s = ::
                        cq_example_providers :: sorted_relation; j0 = ::
                        cq_example_providers :: borrowed_fields; j1 = ::
                        cq_example_providers :: borrowed_fields;
                    } indexes
                    {
                        i0 = :: cq_example_providers :: hash_relation; i1 = ::
                        cq_example_providers :: sorted_relation;
                    } lowerings
                    {
                        l = :: cq_example_providers :: rename; r = ::
                        cq_example_providers :: rename; s = :: cq_example_providers
                        :: rename; j0 = :: cq_example_providers :: indexed_join; j1
                        = :: cq_example_providers :: indexed_join;
                    }
                }
            } environment
            {
                stored L at(__cq_store_L) operations :: cq_example_providers
                :: slice_rows stored R at(__cq_store_R) operations ::
                cq_example_providers :: hash_relation stored S
                at(__cq_store_S) operations :: cq_example_providers ::
                sorted_relation stored l at(__cq_store_l) operations ::
                cq_example_providers :: slice_rows stored r at(__cq_store_r)
                operations :: cq_example_providers :: hash_relation stored s
                at(__cq_store_s) operations :: cq_example_providers ::
                sorted_relation indexed i0 at(__cq_index_i0) operations ::
                cq_example_providers :: hash_relation region j0
                {
                    j0(a, b) => ((a, b,)) : - l(a) => bind a <-
                    ((__cq_store_l).iter()), r(a, b) => bind b <-
                    ((__cq_index_i0).get(a).into_iter().flatten());
                } indexed i1 at(__cq_index_i1) operations ::
                cq_example_providers :: sorted_relation
            } emitted
            {
                let __cq_store_L : & [Value] = L; let __cq_store_R : & :: std
                :: collections :: HashMap < Value, :: std :: vec :: Vec <
                Value > > = R; let __cq_store_S : & :: std :: collections ::
                BTreeMap < Value, :: std :: vec :: Vec < Value > > = S; let
                __cq_store_l : & [Value] = __cq_store_L; let __cq_store_r : &
                :: std :: collections :: HashMap < Value, :: std :: vec :: Vec
                < Value > > = __cq_store_R; let __cq_store_s : & :: std ::
                collections :: BTreeMap < Value, :: std :: vec :: Vec < Value
                > > = __cq_store_S; let __cq_index_i0 : & :: std ::
                collections :: HashMap < Value, :: std :: vec :: Vec < Value >
                > = __cq_store_r; let __cq_index_i1 : & :: std :: collections
                :: BTreeMap < Value, :: std :: vec :: Vec < Value > > =
                __cq_store_s;
            } pending { lower j1 }
            continuations(:: mini_linq :: resume_physical, mini_linq ::
            access)
        } plan
        {
            struct Heterogeneous; relation L(a : Value); relation
            R(a : Value, b : Value); relation S(b : Value, c : Value);
            answer(a, b, c) : - L(a), R(a, b), S(b, c); relational
            {
                l = rename L { a - > a }; r = rename R { a - > a, b - > b }; s
                = rename S { b - > b, c - > c }; j0 = natural_join l with r;
                j1 = natural_join j0 with s; output j1 as answer(a, b, c);
            } indexes { i0 = @ r [0]; i1 = @ s [0]; } uses
            { j0.right = i0; j1.right = i1; } physical
            {
                relations
                {
                    L = :: cq_example_providers :: slice_rows; R = ::
                    cq_example_providers :: hash_relation; S = ::
                    cq_example_providers :: sorted_relation; l = ::
                    cq_example_providers :: slice_rows; r = ::
                    cq_example_providers :: hash_relation; s = ::
                    cq_example_providers :: sorted_relation; j0 = ::
                    cq_example_providers :: borrowed_fields; j1 = ::
                    cq_example_providers :: borrowed_fields;
                } indexes
                {
                    i0 = :: cq_example_providers :: hash_relation; i1 = ::
                    cq_example_providers :: sorted_relation;
                } lowerings
                {
                    l = :: cq_example_providers :: rename; r = ::
                    cq_example_providers :: rename; s = :: cq_example_providers
                    :: rename; j0 = :: cq_example_providers :: indexed_join; j1
                    = :: cq_example_providers :: indexed_join;
                }
            }
        } environment
        {
            stored L at(__cq_store_L) operations :: cq_example_providers ::
            slice_rows stored R at(__cq_store_R) operations ::
            cq_example_providers :: hash_relation stored S at(__cq_store_S)
            operations :: cq_example_providers :: sorted_relation stored l
            at(__cq_store_l) operations :: cq_example_providers :: slice_rows
            stored r at(__cq_store_r) operations :: cq_example_providers ::
            hash_relation stored s at(__cq_store_s) operations ::
            cq_example_providers :: sorted_relation indexed i0
            at(__cq_index_i0) operations :: cq_example_providers ::
            hash_relation region j0
            {
                j0(a, b) => ((a, b,)) : - l(a) => bind a <-
                ((__cq_store_l).iter()), r(a, b) => bind b <-
                ((__cq_index_i0).get(a).into_iter().flatten());
            } indexed i1 at(__cq_index_i1) operations :: cq_example_providers
            :: sorted_relation
        } request { lower j1 }
    } access
    {
        l(a) => bind a <- ((__cq_store_l).iter()), r(a, b) => bind b <-
        ((__cq_index_i0).get(a).into_iter().flatten()), s(b, c) => bind c <-
        ((__cq_index_i1).get(b).into_iter().flatten())
    } witness { row((a, b,)) fields { 0 = (a); 1 = (b); } }
} plan
{
    struct Heterogeneous; relation L(a : Value); relation
    R(a : Value, b : Value); relation S(b : Value, c : Value); answer(a, b, c)
    : - L(a), R(a, b), S(b, c); relational
    {
        l = rename L { a - > a }; r = rename R { a - > a, b - > b }; s =
        rename S { b - > b, c - > c }; j0 = natural_join l with r; j1 =
        natural_join j0 with s; output j1 as answer(a, b, c);
    } indexes { i0 = @ r [0]; i1 = @ s [0]; } uses
    { j0.right = i0; j1.right = i1; } physical
    {
        relations
        {
            L = :: cq_example_providers :: slice_rows; R = ::
            cq_example_providers :: hash_relation; S = :: cq_example_providers
            :: sorted_relation; l = :: cq_example_providers :: slice_rows; r =
            :: cq_example_providers :: hash_relation; s = ::
            cq_example_providers :: sorted_relation; j0 = ::
            cq_example_providers :: borrowed_fields; j1 = ::
            cq_example_providers :: borrowed_fields;
        } indexes
        {
            i0 = :: cq_example_providers :: hash_relation; i1 = ::
            cq_example_providers :: sorted_relation;
        } lowerings
        {
            l = :: cq_example_providers :: rename; r = :: cq_example_providers
            :: rename; s = :: cq_example_providers :: rename; j0 = ::
            cq_example_providers :: indexed_join; j1 = :: cq_example_providers
            :: indexed_join;
        }
    }
} environment
{
    stored L at(__cq_store_L) operations :: cq_example_providers :: slice_rows
    stored R at(__cq_store_R) operations :: cq_example_providers ::
    hash_relation stored S at(__cq_store_S) operations :: cq_example_providers
    :: sorted_relation stored l at(__cq_store_l) operations ::
    cq_example_providers :: slice_rows stored r at(__cq_store_r) operations ::
    cq_example_providers :: hash_relation stored s at(__cq_store_s) operations
    :: cq_example_providers :: sorted_relation indexed i0 at(__cq_index_i0)
    operations :: cq_example_providers :: hash_relation region j0
    {
        j0(a, b) => ((a, b,)) : - l(a) => bind a <- ((__cq_store_l).iter()),
        r(a, b) => bind b <- ((__cq_index_i0).get(a).into_iter().flatten());
    } indexed i1 at(__cq_index_i1) operations :: cq_example_providers ::
    sorted_relation
} request
{
    output j1 fields { 0 = (a); 1 = (b); 2 = (c); } whole
    { j0 = ((a, b,)); s = ((b, c,)); }
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
                    struct Heterogeneous; relation L(a : Value); relation
                    R(a : Value, b : Value); relation S(b : Value, c : Value);
                    answer(a, b, c) : - L(a), R(a, b), S(b, c); relational
                    {
                        l = rename L { a - > a }; r = rename R { a - > a, b - > b };
                        s = rename S { b - > b, c - > c }; j0 = natural_join l with
                        r; j1 = natural_join j0 with s; output j1 as
                        answer(a, b, c);
                    } indexes { i0 = @ r [0]; i1 = @ s [0]; } uses
                    { j0.right = i0; j1.right = i1; } physical
                    {
                        relations
                        {
                            L = :: cq_example_providers :: slice_rows; R = ::
                            cq_example_providers :: hash_relation; S = ::
                            cq_example_providers :: sorted_relation; l = ::
                            cq_example_providers :: slice_rows; r = ::
                            cq_example_providers :: hash_relation; s = ::
                            cq_example_providers :: sorted_relation; j0 = ::
                            cq_example_providers :: borrowed_fields; j1 = ::
                            cq_example_providers :: borrowed_fields;
                        } indexes
                        {
                            i0 = :: cq_example_providers :: hash_relation; i1 = ::
                            cq_example_providers :: sorted_relation;
                        } lowerings
                        {
                            l = :: cq_example_providers :: rename; r = ::
                            cq_example_providers :: rename; s = :: cq_example_providers
                            :: rename; j0 = :: cq_example_providers :: indexed_join; j1
                            = :: cq_example_providers :: indexed_join;
                        }
                    }
                } environment
                {
                    stored L at(__cq_store_L) operations :: cq_example_providers
                    :: slice_rows stored R at(__cq_store_R) operations ::
                    cq_example_providers :: hash_relation stored S
                    at(__cq_store_S) operations :: cq_example_providers ::
                    sorted_relation stored l at(__cq_store_l) operations ::
                    cq_example_providers :: slice_rows stored r at(__cq_store_r)
                    operations :: cq_example_providers :: hash_relation stored s
                    at(__cq_store_s) operations :: cq_example_providers ::
                    sorted_relation indexed i0 at(__cq_index_i0) operations ::
                    cq_example_providers :: hash_relation region j0
                    {
                        j0(a, b) => ((a, b,)) : - l(a) => bind a <-
                        ((__cq_store_l).iter()), r(a, b) => bind b <-
                        ((__cq_index_i0).get(a).into_iter().flatten());
                    } indexed i1 at(__cq_index_i1) operations ::
                    cq_example_providers :: sorted_relation
                } emitted
                {
                    let __cq_store_L : & [Value] = L; let __cq_store_R : & ::
                    std :: collections :: HashMap < Value, :: std :: vec :: Vec
                    < Value > > = R; let __cq_store_S : & :: std :: collections
                    :: BTreeMap < Value, :: std :: vec :: Vec < Value > > = S;
                    let __cq_store_l : & [Value] = __cq_store_L; let
                    __cq_store_r : & :: std :: collections :: HashMap < Value,
                    :: std :: vec :: Vec < Value > > = __cq_store_R; let
                    __cq_store_s : & :: std :: collections :: BTreeMap < Value,
                    :: std :: vec :: Vec < Value > > = __cq_store_S; let
                    __cq_index_i0 : & :: std :: collections :: HashMap < Value,
                    :: std :: vec :: Vec < Value > > = __cq_store_r; let
                    __cq_index_i1 : & :: std :: collections :: BTreeMap < Value,
                    :: std :: vec :: Vec < Value > > = __cq_store_s;
                } pending { lower j1 }
                continuations(:: mini_linq :: resume_physical, mini_linq ::
                access)
            } plan
            {
                struct Heterogeneous; relation L(a : Value); relation
                R(a : Value, b : Value); relation S(b : Value, c : Value);
                answer(a, b, c) : - L(a), R(a, b), S(b, c); relational
                {
                    l = rename L { a - > a }; r = rename R { a - > a, b - > b };
                    s = rename S { b - > b, c - > c }; j0 = natural_join l with
                    r; j1 = natural_join j0 with s; output j1 as
                    answer(a, b, c);
                } indexes { i0 = @ r [0]; i1 = @ s [0]; } uses
                { j0.right = i0; j1.right = i1; } physical
                {
                    relations
                    {
                        L = :: cq_example_providers :: slice_rows; R = ::
                        cq_example_providers :: hash_relation; S = ::
                        cq_example_providers :: sorted_relation; l = ::
                        cq_example_providers :: slice_rows; r = ::
                        cq_example_providers :: hash_relation; s = ::
                        cq_example_providers :: sorted_relation; j0 = ::
                        cq_example_providers :: borrowed_fields; j1 = ::
                        cq_example_providers :: borrowed_fields;
                    } indexes
                    {
                        i0 = :: cq_example_providers :: hash_relation; i1 = ::
                        cq_example_providers :: sorted_relation;
                    } lowerings
                    {
                        l = :: cq_example_providers :: rename; r = ::
                        cq_example_providers :: rename; s = :: cq_example_providers
                        :: rename; j0 = :: cq_example_providers :: indexed_join; j1
                        = :: cq_example_providers :: indexed_join;
                    }
                }
            } environment
            {
                stored L at(__cq_store_L) operations :: cq_example_providers
                :: slice_rows stored R at(__cq_store_R) operations ::
                cq_example_providers :: hash_relation stored S
                at(__cq_store_S) operations :: cq_example_providers ::
                sorted_relation stored l at(__cq_store_l) operations ::
                cq_example_providers :: slice_rows stored r at(__cq_store_r)
                operations :: cq_example_providers :: hash_relation stored s
                at(__cq_store_s) operations :: cq_example_providers ::
                sorted_relation indexed i0 at(__cq_index_i0) operations ::
                cq_example_providers :: hash_relation region j0
                {
                    j0(a, b) => ((a, b,)) : - l(a) => bind a <-
                    ((__cq_store_l).iter()), r(a, b) => bind b <-
                    ((__cq_index_i0).get(a).into_iter().flatten());
                } indexed i1 at(__cq_index_i1) operations ::
                cq_example_providers :: sorted_relation
            } request { lower j1 }
        } access
        {
            l(a) => bind a <- ((__cq_store_l).iter()), r(a, b) => bind b <-
            ((__cq_index_i0).get(a).into_iter().flatten()), s(b, c) => bind c
            <- ((__cq_index_i1).get(b).into_iter().flatten())
        } witness { row((a, b,)) fields { 0 = (a); 1 = (b); } }
    } witness { row((a, b, c,)) fields { 0 = (a); 1 = (b); 2 = (c); } }
}

```

---

## 2172042/035-cq-example-providers::indexed_join_output

[Raw input](raw/2172042/035-cq-example-providers::indexed_join_output.input.tokens) · [Raw output](raw/2172042/035-cq-example-providers::indexed_join_output.output.tokens)

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
                struct Heterogeneous; relation L(a : Value); relation
                R(a : Value, b : Value); relation S(b : Value, c : Value);
                answer(a, b, c) : - L(a), R(a, b), S(b, c); relational
                {
                    l = rename L { a - > a }; r = rename R { a - > a, b - > b };
                    s = rename S { b - > b, c - > c }; j0 = natural_join l with
                    r; j1 = natural_join j0 with s; output j1 as
                    answer(a, b, c);
                } indexes { i0 = @ r [0]; i1 = @ s [0]; } uses
                { j0.right = i0; j1.right = i1; } physical
                {
                    relations
                    {
                        L = :: cq_example_providers :: slice_rows; R = ::
                        cq_example_providers :: hash_relation; S = ::
                        cq_example_providers :: sorted_relation; l = ::
                        cq_example_providers :: slice_rows; r = ::
                        cq_example_providers :: hash_relation; s = ::
                        cq_example_providers :: sorted_relation; j0 = ::
                        cq_example_providers :: borrowed_fields; j1 = ::
                        cq_example_providers :: borrowed_fields;
                    } indexes
                    {
                        i0 = :: cq_example_providers :: hash_relation; i1 = ::
                        cq_example_providers :: sorted_relation;
                    } lowerings
                    {
                        l = :: cq_example_providers :: rename; r = ::
                        cq_example_providers :: rename; s = :: cq_example_providers
                        :: rename; j0 = :: cq_example_providers :: indexed_join; j1
                        = :: cq_example_providers :: indexed_join;
                    }
                }
            } environment
            {
                stored L at(__cq_store_L) operations :: cq_example_providers
                :: slice_rows stored R at(__cq_store_R) operations ::
                cq_example_providers :: hash_relation stored S
                at(__cq_store_S) operations :: cq_example_providers ::
                sorted_relation stored l at(__cq_store_l) operations ::
                cq_example_providers :: slice_rows stored r at(__cq_store_r)
                operations :: cq_example_providers :: hash_relation stored s
                at(__cq_store_s) operations :: cq_example_providers ::
                sorted_relation indexed i0 at(__cq_index_i0) operations ::
                cq_example_providers :: hash_relation region j0
                {
                    j0(a, b) => ((a, b,)) : - l(a) => bind a <-
                    ((__cq_store_l).iter()), r(a, b) => bind b <-
                    ((__cq_index_i0).get(a).into_iter().flatten());
                } indexed i1 at(__cq_index_i1) operations ::
                cq_example_providers :: sorted_relation
            } emitted
            {
                let __cq_store_L : & [Value] = L; let __cq_store_R : & :: std
                :: collections :: HashMap < Value, :: std :: vec :: Vec <
                Value > > = R; let __cq_store_S : & :: std :: collections ::
                BTreeMap < Value, :: std :: vec :: Vec < Value > > = S; let
                __cq_store_l : & [Value] = __cq_store_L; let __cq_store_r : &
                :: std :: collections :: HashMap < Value, :: std :: vec :: Vec
                < Value > > = __cq_store_R; let __cq_store_s : & :: std ::
                collections :: BTreeMap < Value, :: std :: vec :: Vec < Value
                > > = __cq_store_S; let __cq_index_i0 : & :: std ::
                collections :: HashMap < Value, :: std :: vec :: Vec < Value >
                > = __cq_store_r; let __cq_index_i1 : & :: std :: collections
                :: BTreeMap < Value, :: std :: vec :: Vec < Value > > =
                __cq_store_s;
            } pending { lower j1 }
            continuations(:: mini_linq :: resume_physical, mini_linq ::
            access)
        } plan
        {
            struct Heterogeneous; relation L(a : Value); relation
            R(a : Value, b : Value); relation S(b : Value, c : Value);
            answer(a, b, c) : - L(a), R(a, b), S(b, c); relational
            {
                l = rename L { a - > a }; r = rename R { a - > a, b - > b }; s
                = rename S { b - > b, c - > c }; j0 = natural_join l with r;
                j1 = natural_join j0 with s; output j1 as answer(a, b, c);
            } indexes { i0 = @ r [0]; i1 = @ s [0]; } uses
            { j0.right = i0; j1.right = i1; } physical
            {
                relations
                {
                    L = :: cq_example_providers :: slice_rows; R = ::
                    cq_example_providers :: hash_relation; S = ::
                    cq_example_providers :: sorted_relation; l = ::
                    cq_example_providers :: slice_rows; r = ::
                    cq_example_providers :: hash_relation; s = ::
                    cq_example_providers :: sorted_relation; j0 = ::
                    cq_example_providers :: borrowed_fields; j1 = ::
                    cq_example_providers :: borrowed_fields;
                } indexes
                {
                    i0 = :: cq_example_providers :: hash_relation; i1 = ::
                    cq_example_providers :: sorted_relation;
                } lowerings
                {
                    l = :: cq_example_providers :: rename; r = ::
                    cq_example_providers :: rename; s = :: cq_example_providers
                    :: rename; j0 = :: cq_example_providers :: indexed_join; j1
                    = :: cq_example_providers :: indexed_join;
                }
            }
        } environment
        {
            stored L at(__cq_store_L) operations :: cq_example_providers ::
            slice_rows stored R at(__cq_store_R) operations ::
            cq_example_providers :: hash_relation stored S at(__cq_store_S)
            operations :: cq_example_providers :: sorted_relation stored l
            at(__cq_store_l) operations :: cq_example_providers :: slice_rows
            stored r at(__cq_store_r) operations :: cq_example_providers ::
            hash_relation stored s at(__cq_store_s) operations ::
            cq_example_providers :: sorted_relation indexed i0
            at(__cq_index_i0) operations :: cq_example_providers ::
            hash_relation region j0
            {
                j0(a, b) => ((a, b,)) : - l(a) => bind a <-
                ((__cq_store_l).iter()), r(a, b) => bind b <-
                ((__cq_index_i0).get(a).into_iter().flatten());
            } indexed i1 at(__cq_index_i1) operations :: cq_example_providers
            :: sorted_relation
        } request { lower j1 }
    } access
    {
        l(a) => bind a <- ((__cq_store_l).iter()), r(a, b) => bind b <-
        ((__cq_index_i0).get(a).into_iter().flatten()), s(b, c) => bind c <-
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
            struct Heterogeneous; relation L(a : Value); relation
            R(a : Value, b : Value); relation S(b : Value, c : Value);
            answer(a, b, c) : - L(a), R(a, b), S(b, c); relational
            {
                l = rename L { a - > a }; r = rename R { a - > a, b - > b }; s
                = rename S { b - > b, c - > c }; j0 = natural_join l with r;
                j1 = natural_join j0 with s; output j1 as answer(a, b, c);
            } indexes { i0 = @ r [0]; i1 = @ s [0]; } uses
            { j0.right = i0; j1.right = i1; } physical
            {
                relations
                {
                    L = :: cq_example_providers :: slice_rows; R = ::
                    cq_example_providers :: hash_relation; S = ::
                    cq_example_providers :: sorted_relation; l = ::
                    cq_example_providers :: slice_rows; r = ::
                    cq_example_providers :: hash_relation; s = ::
                    cq_example_providers :: sorted_relation; j0 = ::
                    cq_example_providers :: borrowed_fields; j1 = ::
                    cq_example_providers :: borrowed_fields;
                } indexes
                {
                    i0 = :: cq_example_providers :: hash_relation; i1 = ::
                    cq_example_providers :: sorted_relation;
                } lowerings
                {
                    l = :: cq_example_providers :: rename; r = ::
                    cq_example_providers :: rename; s = :: cq_example_providers
                    :: rename; j0 = :: cq_example_providers :: indexed_join; j1
                    = :: cq_example_providers :: indexed_join;
                }
            }
        } environment
        {
            stored L at(__cq_store_L) operations :: cq_example_providers ::
            slice_rows stored R at(__cq_store_R) operations ::
            cq_example_providers :: hash_relation stored S at(__cq_store_S)
            operations :: cq_example_providers :: sorted_relation stored l
            at(__cq_store_l) operations :: cq_example_providers :: slice_rows
            stored r at(__cq_store_r) operations :: cq_example_providers ::
            hash_relation stored s at(__cq_store_s) operations ::
            cq_example_providers :: sorted_relation indexed i0
            at(__cq_index_i0) operations :: cq_example_providers ::
            hash_relation region j0
            {
                j0(a, b) => ((a, b,)) : - l(a) => bind a <-
                ((__cq_store_l).iter()), r(a, b) => bind b <-
                ((__cq_index_i0).get(a).into_iter().flatten());
            } indexed i1 at(__cq_index_i1) operations :: cq_example_providers
            :: sorted_relation
        } emitted
        {
            let __cq_store_L : & [Value] = L; let __cq_store_R : & :: std ::
            collections :: HashMap < Value, :: std :: vec :: Vec < Value > > =
            R; let __cq_store_S : & :: std :: collections :: BTreeMap < Value,
            :: std :: vec :: Vec < Value > > = S; let __cq_store_l : & [Value]
            = __cq_store_L; let __cq_store_r : & :: std :: collections ::
            HashMap < Value, :: std :: vec :: Vec < Value > > = __cq_store_R;
            let __cq_store_s : & :: std :: collections :: BTreeMap < Value, ::
            std :: vec :: Vec < Value > > = __cq_store_S; let __cq_index_i0 :
            & :: std :: collections :: HashMap < Value, :: std :: vec :: Vec <
            Value > > = __cq_store_r; let __cq_index_i1 : & :: std ::
            collections :: BTreeMap < Value, :: std :: vec :: Vec < Value > >
            = __cq_store_s;
        } pending { lower j1 }
        continuations(:: mini_linq :: resume_physical, mini_linq :: access)
    } emitted {} result
    {
        region j1
        {
            j1(a, b, c) => ((a, b, c,)) : - l(a) => bind a <-
            ((__cq_store_l).iter()), r(a, b) => bind b <-
            ((__cq_index_i0).get(a).into_iter().flatten()), s(b, c) => bind c
            <- ((__cq_index_i1).get(b).into_iter().flatten());
        }
    }
}

```

---

## 2172042/036-resume_physical

[Raw input](raw/2172042/036-resume_physical.input.tokens) · [Raw output](raw/2172042/036-resume_physical.output.tokens)

Actual input:

```rust
resume
{
    plan
    {
        struct Heterogeneous; relation L(a : Value); relation
        R(a : Value, b : Value); relation S(b : Value, c : Value);
        answer(a, b, c) : - L(a), R(a, b), S(b, c); relational
        {
            l = rename L { a - > a }; r = rename R { a - > a, b - > b }; s =
            rename S { b - > b, c - > c }; j0 = natural_join l with r; j1 =
            natural_join j0 with s; output j1 as answer(a, b, c);
        } indexes { i0 = @ r [0]; i1 = @ s [0]; } uses
        { j0.right = i0; j1.right = i1; } physical
        {
            relations
            {
                L = :: cq_example_providers :: slice_rows; R = ::
                cq_example_providers :: hash_relation; S = ::
                cq_example_providers :: sorted_relation; l = ::
                cq_example_providers :: slice_rows; r = ::
                cq_example_providers :: hash_relation; s = ::
                cq_example_providers :: sorted_relation; j0 = ::
                cq_example_providers :: borrowed_fields; j1 = ::
                cq_example_providers :: borrowed_fields;
            } indexes
            {
                i0 = :: cq_example_providers :: hash_relation; i1 = ::
                cq_example_providers :: sorted_relation;
            } lowerings
            {
                l = :: cq_example_providers :: rename; r = ::
                cq_example_providers :: rename; s = :: cq_example_providers ::
                rename; j0 = :: cq_example_providers :: indexed_join; j1 = ::
                cq_example_providers :: indexed_join;
            }
        }
    } environment
    {
        stored L at(__cq_store_L) operations :: cq_example_providers ::
        slice_rows stored R at(__cq_store_R) operations ::
        cq_example_providers :: hash_relation stored S at(__cq_store_S)
        operations :: cq_example_providers :: sorted_relation stored l
        at(__cq_store_l) operations :: cq_example_providers :: slice_rows
        stored r at(__cq_store_r) operations :: cq_example_providers ::
        hash_relation stored s at(__cq_store_s) operations ::
        cq_example_providers :: sorted_relation indexed i0 at(__cq_index_i0)
        operations :: cq_example_providers :: hash_relation region j0
        {
            j0(a, b) => ((a, b,)) : - l(a) => bind a <-
            ((__cq_store_l).iter()), r(a, b) => bind b <-
            ((__cq_index_i0).get(a).into_iter().flatten());
        } indexed i1 at(__cq_index_i1) operations :: cq_example_providers ::
        sorted_relation
    } emitted
    {
        let __cq_store_L : & [Value] = L; let __cq_store_R : & :: std ::
        collections :: HashMap < Value, :: std :: vec :: Vec < Value > > = R;
        let __cq_store_S : & :: std :: collections :: BTreeMap < Value, :: std
        :: vec :: Vec < Value > > = S; let __cq_store_l : & [Value] =
        __cq_store_L; let __cq_store_r : & :: std :: collections :: HashMap <
        Value, :: std :: vec :: Vec < Value > > = __cq_store_R; let
        __cq_store_s : & :: std :: collections :: BTreeMap < Value, :: std ::
        vec :: Vec < Value > > = __cq_store_S; let __cq_index_i0 : & :: std ::
        collections :: HashMap < Value, :: std :: vec :: Vec < Value > > =
        __cq_store_r; let __cq_index_i1 : & :: std :: collections :: BTreeMap
        < Value, :: std :: vec :: Vec < Value > > = __cq_store_s;
    } pending { lower j1 }
    continuations(:: mini_linq :: resume_physical, mini_linq :: access)
} emitted {} result
{
    region j1
    {
        j1(a, b, c) => ((a, b, c,)) : - l(a) => bind a <-
        ((__cq_store_l).iter()), r(a, b) => bind b <-
        ((__cq_index_i0).get(a).into_iter().flatten()), s(b, c) => bind c <-
        ((__cq_index_i1).get(b).into_iter().flatten());
    }
}
```

Actual output (formatted where Rust syntax permits):

```rust
{
    let __cq_store_L : & [Value] = L; let __cq_store_R : & :: std ::
    collections :: HashMap < Value, :: std :: vec :: Vec < Value > > = R; let
    __cq_store_S : & :: std :: collections :: BTreeMap < Value, :: std :: vec
    :: Vec < Value > > = S; let __cq_store_l : & [Value] = __cq_store_L; let
    __cq_store_r : & :: std :: collections :: HashMap < Value, :: std :: vec
    :: Vec < Value > > = __cq_store_R; let __cq_store_s : & :: std ::
    collections :: BTreeMap < Value, :: std :: vec :: Vec < Value > > =
    __cq_store_S; let __cq_index_i0 : & :: std :: collections :: HashMap <
    Value, :: std :: vec :: Vec < Value > > = __cq_store_r; let __cq_index_i1
    : & :: std :: collections :: BTreeMap < Value, :: std :: vec :: Vec <
    Value > > = __cq_store_s; mini_linq :: access!
    {
        answer(a, b, c) => ((a, b, c,)) : - l(a) => bind a <-
        ((__cq_store_l).iter()), r(a, b) => bind b <-
        ((__cq_index_i0).get(a).into_iter().flatten()), s(b, c) => bind c <-
        ((__cq_index_i1).get(b).into_iter().flatten());
    }
}
```

---

## 2172042/037-access_to_set_loops

[Raw input](raw/2172042/037-access_to_set_loops.input.tokens) · [Raw output](raw/2172042/037-access_to_set_loops.output.tokens)

Actual input:

```rust
answer(a, b, c) => ((a, b, c,)) : - l(a) => bind a <- ((__cq_store_l).iter()),
r(a, b) => bind b <- ((__cq_index_i0).get(a).into_iter().flatten()), s(b, c)
=> bind c <- ((__cq_index_i1).get(b).into_iter().flatten());
```

Actual output (formatted where Rust syntax permits):

```rust
{
    let mut __cq_output = :: std :: collections :: HashSet :: new(); for a in
    ((__cq_store_l).iter())
    {
        for b in ((__cq_index_i0).get(a).into_iter().flatten())
        {
            let a = :: core :: clone :: Clone :: clone(& a); for c in
            ((__cq_index_i1).get(b).into_iter().flatten())
            {
                let a = :: core :: clone :: Clone :: clone(& a); let b = ::
                core :: clone :: Clone :: clone(& b);
                __cq_output.insert((a, b, c,));
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
   Compiling cq-ir v0.1.0 (/tmp/mini-linq-bind-realization-20260919/workspace/crates/cq-ir)
   Compiling cq-access-lowering v0.1.0 (/tmp/mini-linq-bind-realization-20260919/workspace/crates/cq-access-lowering)
   Compiling cq-physical-lowering v0.1.0 (/tmp/mini-linq-bind-realization-20260919/workspace/crates/cq-physical-lowering)
   Compiling cq-index-policy v0.1.0 (/tmp/mini-linq-bind-realization-20260919/workspace/crates/cq-index-policy)
   Compiling cq-physical-selection v0.1.0 (/tmp/mini-linq-bind-realization-20260919/workspace/crates/cq-physical-selection)
   Compiling cq-example-providers v0.1.0 (/tmp/mini-linq-bind-realization-20260919/workspace/crates/cq-example-providers)
   Compiling mini-linq v0.1.0 (/tmp/mini-linq-bind-realization-20260919/workspace/crates/mini-linq)
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
  --> crates/mini-linq/tests/physical_realization.rs:57:5
   |
57 | /     ::mini_linq::index_demands! {
58 | |         struct Heterogeneous;
59 | |         relation L(a: Value);
60 | |         relation R(a: Value, b: Value);
...  |
71 | |     }
   | |_____^
   |
   = note: expanding `explicit_policy! { struct Heterogeneous; relation L(a : Value); relation R(a : Value, b : Value);
           relation S(b : Value, c : Value); answer(a, b, c) : - L(a), R(a, b), S(b, c);
           relational
           {
               l = rename L { a - > a }; r = rename R { a - > a, b - > b }; s = rename S
               { b - > b, c - > c }; j0 = natural_join l with r; j1 = natural_join j0
               with s; output j1 as answer(a, b, c);
           } indexes { i0 = @ r [0]; i1 = @ s [0]; } uses
           { j0.right = i0; j1.right = i1; } }`
   = note: to `:: mini_linq :: given_physical_bindings!
           {
               struct Heterogeneous; relation L(a : Value); relation
               R(a : Value, b : Value); relation S(b : Value, c : Value); answer(a, b, c)
               : - L(a), R(a, b), S(b, c); relational
               {
                   l = rename L { a - > a }; r = rename R { a - > a, b - > b }; s =
                   rename S { b - > b, c - > c }; j0 = natural_join l with r; j1 =
                   natural_join j0 with s; output j1 as answer(a, b, c);
               } indexes { i0 = @ r [0]; i1 = @ s [0]; } uses
               { j0.right = i0; j1.right = i1; } physical
               {
                   relations
                   {
                       L = :: cq_example_providers :: slice_rows; R = ::
                       cq_example_providers :: hash_relation; S = :: cq_example_providers
                       :: sorted_relation; l = :: cq_example_providers :: slice_rows; r =
                       :: cq_example_providers :: hash_relation; s = ::
                       cq_example_providers :: sorted_relation; j0 = ::
                       cq_example_providers :: borrowed_fields; j1 = ::
                       cq_example_providers :: borrowed_fields;
                   } indexes
                   {
                       i0 = :: cq_example_providers :: hash_relation; i1 = ::
                       cq_example_providers :: sorted_relation;
                   } lowerings
                   {
                       l = :: cq_example_providers :: rename; r = :: cq_example_providers
                       :: rename; s = :: cq_example_providers :: rename; j0 = ::
                       cq_example_providers :: indexed_join; j1 = :: cq_example_providers
                       :: indexed_join;
                   }
               }
           }`

note: trace_macro
   --> crates/mini-linq/tests/physical_realization.rs:101:20
    |
101 |         (Value(1), vec![Value(10), Value(10), Value(11)]),
    |                    ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^
    |
    = note: expanding `vec! { Value(10), Value(10), Value(11) }`
    = note: to `$crate :: boxed ::
            box_assume_init_into_vec_unsafe($crate :: intrinsics ::
            write_box_via_move($crate :: boxed :: Box :: new_uninit(),
            [Value(10), Value(10), Value(11)]))`

note: trace_macro
   --> crates/mini-linq/tests/physical_realization.rs:102:20
    |
102 |         (Value(2), vec![Value(20)]),
    |                    ^^^^^^^^^^^^^^^
    |
    = note: expanding `vec! { Value(20) }`
    = note: to `$crate :: boxed ::
            box_assume_init_into_vec_unsafe($crate :: intrinsics ::
            write_box_via_move($crate :: boxed :: Box :: new_uninit(), [Value(20)]))`

note: trace_macro
   --> crates/mini-linq/tests/physical_realization.rs:103:20
    |
103 |         (Value(3), vec![Value(30)]),
    |                    ^^^^^^^^^^^^^^^
    |
    = note: expanding `vec! { Value(30) }`
    = note: to `$crate :: boxed ::
            box_assume_init_into_vec_unsafe($crate :: intrinsics ::
            write_box_via_move($crate :: boxed :: Box :: new_uninit(), [Value(30)]))`

note: trace_macro
   --> crates/mini-linq/tests/physical_realization.rs:106:21
    |
106 |         (Value(10), vec![Value(100), Value(101), Value(101)]),
    |                     ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^
    |
    = note: expanding `vec! { Value(100), Value(101), Value(101) }`
    = note: to `$crate :: boxed ::
            box_assume_init_into_vec_unsafe($crate :: intrinsics ::
            write_box_via_move($crate :: boxed :: Box :: new_uninit(),
            [Value(100), Value(101), Value(101)]))`

note: trace_macro
   --> crates/mini-linq/tests/physical_realization.rs:107:21
    |
107 |         (Value(11), vec![]),
    |                     ^^^^^^
    |
    = note: expanding `vec! {  }`
    = note: to `$crate :: vec :: Vec :: new()`

note: trace_macro
   --> crates/mini-linq/tests/physical_realization.rs:108:21
    |
108 |         (Value(20), vec![Value(200)]),
    |                     ^^^^^^^^^^^^^^^^
    |
    = note: expanding `vec! { Value(200) }`
    = note: to `$crate :: boxed ::
            box_assume_init_into_vec_unsafe($crate :: intrinsics ::
            write_box_via_move($crate :: boxed :: Box :: new_uninit(), [Value(200)]))`

note: trace_macro
   --> crates/mini-linq/tests/physical_realization.rs:109:21
    |
109 |         (Value(99), vec![Value(999)]),
    |                     ^^^^^^^^^^^^^^^^
    |
    = note: expanding `vec! { Value(999) }`
    = note: to `$crate :: boxed ::
            box_assume_init_into_vec_unsafe($crate :: intrinsics ::
            write_box_via_move($crate :: boxed :: Box :: new_uninit(), [Value(999)]))`

note: trace_macro
   --> crates/mini-linq/tests/physical_realization.rs:112:5
    |
112 |     assert_eq!(actual, oracle(&left, &middle, &right));
    |     ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^
    |
    = note: expanding `assert_eq! { actual, oracle(&left, &middle, &right) }`
    = note: to `match (& actual, & oracle(&left, &middle, &right))
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
   --> crates/mini-linq/tests/physical_realization.rs:113:5
    |
113 |     assert_eq!(actual.len(), 3);
    |     ^^^^^^^^^^^^^^^^^^^^^^^^^^^
    |
    = note: expanding `assert_eq! { actual.len(), 3 }`
    = note: to `match (& actual.len(), & 3)
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
   --> crates/mini-linq/tests/physical_realization.rs:124:44
    |
124 |     let middle = HashMap::from([(Value(1), vec![Value(10)])]);
    |                                            ^^^^^^^^^^^^^^^
    |
    = note: expanding `vec! { Value(10) }`
    = note: to `$crate :: boxed ::
            box_assume_init_into_vec_unsafe($crate :: intrinsics ::
            write_box_via_move($crate :: boxed :: Box :: new_uninit(), [Value(10)]))`

note: trace_macro
   --> crates/mini-linq/tests/physical_realization.rs:125:45
    |
125 |     let right = BTreeMap::from([(Value(10), vec![Value(100)])]);
    |                                             ^^^^^^^^^^^^^^^^
    |
    = note: expanding `vec! { Value(100) }`
    = note: to `$crate :: boxed ::
            box_assume_init_into_vec_unsafe($crate :: intrinsics ::
            write_box_via_move($crate :: boxed :: Box :: new_uninit(), [Value(100)]))`

note: trace_macro
   --> crates/mini-linq/tests/physical_realization.rs:134:42
    |
134 |             &BTreeMap::from([(Value(11), vec![Value(100)])])
    |                                          ^^^^^^^^^^^^^^^^
    |
    = note: expanding `vec! { Value(100) }`
    = note: to `$crate :: boxed ::
            box_assume_init_into_vec_unsafe($crate :: intrinsics ::
            write_box_via_move($crate :: boxed :: Box :: new_uninit(), [Value(100)]))`

note: trace_macro
   --> crates/mini-linq/tests/physical_realization.rs:146:47
    |
146 |       let directory = std::env::temp_dir().join(format!(
    |  _______________________________________________^
147 | |         "mini-linq-physical-rejection-{}",
148 | |         std::process::id()
149 | |     ));
    | |_____^
    |
    = note: expanding `format! { "mini-linq-physical-rejection-{}", std::process::id() }`
    = note: to `$crate :: __export ::
            must_use({
                $crate :: fmt ::
                format($crate :: __export :: format_args!
                ("mini-linq-physical-rejection-{}", std::process::id()))
            })`

note: trace_macro
   --> crates/mini-linq/tests/physical_realization.rs:151:45
    |
151 |       fs::write(directory.join("Cargo.toml"), format!(r#"
    |  _____________________________________________^
152 | | [package]
153 | | name = "physical-rejection-fixture"
154 | | version = "0.0.0"
...   |
159 | | cq-example-providers = {{ path = {:?} }}
160 | | "#, manifest, workspace.join("crates/cq-example-providers"))).unwrap();
    | |____________________________________________________________^
    |
    = note: expanding `format! { r#"
            [package]
            name = "physical-rejection-fixture"
            version = "0.0.0"
            edition = "2024"
            [workspace]
            [dependencies]
            mini-linq = {{ path = {:?}, default-features = false, features = ["index-demands", "physical-selection", "physical-realization", "access-to-set-loops"] }}
            cq-example-providers = {{ path = {:?} }}
            "#,
            manifest, workspace.join("crates/cq-example-providers") }`
    = note: to `$crate :: __export ::
            must_use({
                $crate :: fmt ::
                format($crate :: __export :: format_args!
                (r#"
            [package]
            name = "physical-rejection-fixture"
            version = "0.0.0"
            edition = "2024"
            [workspace]
            [dependencies]
            mini-linq = {{ path = {:?}, default-features = false, features = ["index-demands", "physical-selection", "physical-realization", "access-to-set-loops"] }}
            cq-example-providers = {{ path = {:?} }}
            "#,
                manifest, workspace.join("crates/cq-example-providers")))
            })`

note: trace_macro
   --> crates/mini-linq/tests/physical_realization.rs:202:21
    |
202 |             None => assert!(output.status.success(), "valid control failed: {stderr}"),
    |                     ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^
    |
    = note: expanding `panic_2021! { "valid control failed: {stderr}" }`
    = note: to `{
                $crate :: panicking ::
                panic_fmt($crate :: const_format_args!
                ("valid control failed: {stderr}"));
            }`

note: trace_macro
   --> crates/mini-linq/tests/physical_realization.rs:204:17
    |
204 |                 assert!(!output.status.success(), "{name} unexpectedly compiled");
    |                 ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^
    |
    = note: expanding `panic_2021! { "{name} unexpectedly compiled" }`
    = note: to `{
                $crate :: panicking ::
                panic_fmt($crate :: const_format_args! ("{name} unexpectedly compiled"));
            }`

note: trace_macro
   --> crates/mini-linq/tests/physical_realization.rs:205:17
    |
205 | /                 assert!(
206 | |                     stderr.contains(message),
207 | |                     "{name} rejected for the wrong reason: {stderr}"
208 | |                 );
    | |_________________^
    |
    = note: expanding `panic_2021! { "{name} rejected for the wrong reason: {stderr}" }`
    = note: to `{
                $crate :: panicking ::
                panic_fmt($crate :: const_format_args!
                ("{name} rejected for the wrong reason: {stderr}"));
            }`

    Finished `dev` profile [unoptimized + debuginfo] target(s) in 5.70s

```
