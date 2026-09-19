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


## 2172007/001-cq_to_ra_written_order

[Raw input](raw/2172007/001-cq_to_ra_written_order.input.tokens) · [Raw output](raw/2172007/001-cq_to_ra_written_order.output.tokens)

Actual input:

```rust
struct PullChain; relation L(a: Value); relation R(a: Value, b: Value);
relation S(b: Value, c: Value); answer(a, c) :- L(a), R(a, b), S(b, c);
```

Actual output (formatted where Rust syntax permits):

```rust
mini_linq::choose_index_policy! {
    struct PullChain; relation L(a : Value); relation R(a : Value, b : Value);
    relation S(b : Value, c : Value); answer(a, c) : - L(a), R(a, b), S(b, c);
    relational
    {
        r0 = rename L { a - > a }; r1 = rename R { a - > a, b - > b }; r2 =
        natural_join r0 with r1; r3 = rename S { b - > b, c - > c }; r4 =
        natural_join r2 with r3; r5 = project r4 keep { a, c }; output r5 as
        answer(a, c);
    }
}

```

---

## 2172007/002-index_demands

[Raw input](raw/2172007/002-index_demands.input.tokens) · [Raw output](raw/2172007/002-index_demands.output.tokens)

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

---

## 2172007/003-given_physical_bindings

[Raw input](raw/2172007/003-given_physical_bindings.input.tokens) · [Raw output](raw/2172007/003-given_physical_bindings.output.tokens)

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
} indexes { i0 = @ r1 [0]; i1 = @ r3 [0]; } uses
{ r2.right = i0; r4.right = i1; } physical
{
    relations
    {
        L = observed_slice; R = observed_hash; S = observed_sorted; r0 =
        observed_slice; r1 = observed_hash; r2 = :: cq_example_providers ::
        borrowed_fields; r3 = observed_sorted; r4 = :: cq_example_providers ::
        borrowed_fields; r5 = :: cq_example_providers :: borrowed_fields;
    } indexes { i0 = observed_hash; i1 = observed_sorted; } lowerings
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
    struct PullChain; relation L(a : Value); relation R(a : Value, b : Value);
    relation S(b : Value, c : Value); answer(a, c) : - L(a), R(a, b), S(b, c);
    relational
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
            L = observed_slice; R = observed_hash; S = observed_sorted; r0 =
            observed_slice; r1 = observed_hash; r2 = :: cq_example_providers
            :: borrowed_fields; r3 = observed_sorted; r4 = ::
            cq_example_providers :: borrowed_fields; r5 = ::
            cq_example_providers :: borrowed_fields;
        } indexes { i0 = observed_hash; i1 = observed_sorted; } lowerings
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

## 2172007/004-realize_physical

[Raw input](raw/2172007/004-realize_physical.input.tokens) · [Raw output](raw/2172007/004-realize_physical.output.tokens)

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
} indexes { i0 = @ r1 [0]; i1 = @ r3 [0]; } uses
{ r2.right = i0; r4.right = i1; } physical
{
    relations
    {
        L = observed_slice; R = observed_hash; S = observed_sorted; r0 =
        observed_slice; r1 = observed_hash; r2 = :: cq_example_providers ::
        borrowed_fields; r3 = observed_sorted; r4 = :: cq_example_providers ::
        borrowed_fields; r5 = :: cq_example_providers :: borrowed_fields;
    } indexes { i0 = observed_hash; i1 = observed_sorted; } lowerings
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
observed_slice! {
    reply_to(:: mini_linq :: resume_physical)
    {
        plan
        {
            struct PullChain; relation L(a : Value); relation
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
                    L = observed_slice; R = observed_hash; S = observed_sorted;
                    r0 = observed_slice; r1 = observed_hash; r2 = ::
                    cq_example_providers :: borrowed_fields; r3 =
                    observed_sorted; r4 = :: cq_example_providers ::
                    borrowed_fields; r5 = :: cq_example_providers ::
                    borrowed_fields;
                } indexes { i0 = observed_hash; i1 = observed_sorted; }
                lowerings
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
        struct PullChain; relation L(a : Value); relation
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
                L = observed_slice; R = observed_hash; S = observed_sorted; r0
                = observed_slice; r1 = observed_hash; r2 = ::
                cq_example_providers :: borrowed_fields; r3 = observed_sorted;
                r4 = :: cq_example_providers :: borrowed_fields; r5 = ::
                cq_example_providers :: borrowed_fields;
            } indexes { i0 = observed_hash; i1 = observed_sorted; } lowerings
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

## 2172007/005-cq-example-providers::slice_rows

[Raw input](raw/2172007/005-cq-example-providers::slice_rows.input.tokens) · [Raw output](raw/2172007/005-cq-example-providers::slice_rows.output.tokens)

Actual input:

```rust
reply_to(observe_reply)
{
    callback(:: mini_linq :: resume_physical) kind(0) state
    {plan
        {
            struct PullChain; relation L(a : Value); relation
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
                    L = observed_slice; R = observed_hash; S = observed_sorted;
                    r0 = observed_slice; r1 = observed_hash; r2 = ::
                    cq_example_providers :: borrowed_fields; r3 =
                    observed_sorted; r4 = :: cq_example_providers ::
                    borrowed_fields; r5 = :: cq_example_providers ::
                    borrowed_fields;
                } indexes { i0 = observed_hash; i1 = observed_sorted; }
                lowerings
                {
                    r0 = :: cq_example_providers :: rename; r1 = ::
                    cq_example_providers :: rename; r2 = :: cq_example_providers
                    :: indexed_join; r3 = :: cq_example_providers :: rename; r4
                    = :: cq_example_providers :: indexed_join; r5 = ::
                    cq_example_providers :: project;
                }
            }
        } environment {} emitted {} pending { input L from(L) }
        continuations(:: mini_linq :: resume_physical, mini_linq :: access)}
} plan
{struct PullChain; relation L(a : Value); relation R(a : Value, b : Value);
    relation S(b : Value, c : Value); answer(a, c) : - L(a), R(a, b), S(b, c);
    relational
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
            L = observed_slice; R = observed_hash; S = observed_sorted; r0 =
            observed_slice; r1 = observed_hash; r2 = :: cq_example_providers
            :: borrowed_fields; r3 = observed_sorted; r4 = ::
            cq_example_providers :: borrowed_fields; r5 = ::
            cq_example_providers :: borrowed_fields;
        } indexes { i0 = observed_hash; i1 = observed_sorted; } lowerings
        {
            r0 = :: cq_example_providers :: rename; r1 = ::
            cq_example_providers :: rename; r2 = :: cq_example_providers ::
            indexed_join; r3 = :: cq_example_providers :: rename; r4 = ::
            cq_example_providers :: indexed_join; r5 = :: cq_example_providers
            :: project;
        }
    }} environment {} request {input L from(L)}
```

Actual output (formatted where Rust syntax permits):

```rust
observe_reply! {
    resume
    {
        callback(:: mini_linq :: resume_physical) kind(0) state
        {
            plan
            {
                struct PullChain; relation L(a : Value); relation
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
                        L = observed_slice; R = observed_hash; S = observed_sorted;
                        r0 = observed_slice; r1 = observed_hash; r2 = ::
                        cq_example_providers :: borrowed_fields; r3 =
                        observed_sorted; r4 = :: cq_example_providers ::
                        borrowed_fields; r5 = :: cq_example_providers ::
                        borrowed_fields;
                    } indexes { i0 = observed_hash; i1 = observed_sorted; }
                    lowerings
                    {
                        r0 = :: cq_example_providers :: rename; r1 = ::
                        cq_example_providers :: rename; r2 = :: cq_example_providers
                        :: indexed_join; r3 = :: cq_example_providers :: rename; r4
                        = :: cq_example_providers :: indexed_join; r5 = ::
                        cq_example_providers :: project;
                    }
                }
            } environment {} emitted {} pending { input L from(L) }
            continuations(:: mini_linq :: resume_physical, mini_linq ::
            access)
        }
    } emitted { let __cq_store_L : & [Value] = L; } result
    { stored L at(__cq_store_L) operations observed_slice }
}

```

---

## 2172007/006-resume_physical

[Raw input](raw/2172007/006-resume_physical.input.tokens) · [Raw output](raw/2172007/006-resume_physical.output.tokens)

Actual input:

```rust
resume
{plan
    {
        struct PullChain; relation L(a : Value); relation
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
                L = observed_slice; R = observed_hash; S = observed_sorted; r0
                = observed_slice; r1 = observed_hash; r2 = ::
                cq_example_providers :: borrowed_fields; r3 = observed_sorted;
                r4 = :: cq_example_providers :: borrowed_fields; r5 = ::
                cq_example_providers :: borrowed_fields;
            } indexes { i0 = observed_hash; i1 = observed_sorted; } lowerings
            {
                r0 = :: cq_example_providers :: rename; r1 = ::
                cq_example_providers :: rename; r2 = :: cq_example_providers
                :: indexed_join; r3 = :: cq_example_providers :: rename; r4 =
                :: cq_example_providers :: indexed_join; r5 = ::
                cq_example_providers :: project;
            }
        }
    } environment {} emitted {} pending { input L from(L) }
    continuations(:: mini_linq :: resume_physical, mini_linq :: access)}
emitted {let __cq_store_L : & [Value] = L;} result
{stored L at(__cq_store_L) operations observed_slice}
```

Actual output (formatted where Rust syntax permits):

```rust
observed_hash! {
    reply_to(:: mini_linq :: resume_physical)
    {
        plan
        {
            struct PullChain; relation L(a : Value); relation
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
                    L = observed_slice; R = observed_hash; S = observed_sorted;
                    r0 = observed_slice; r1 = observed_hash; r2 = ::
                    cq_example_providers :: borrowed_fields; r3 =
                    observed_sorted; r4 = :: cq_example_providers ::
                    borrowed_fields; r5 = :: cq_example_providers ::
                    borrowed_fields;
                } indexes { i0 = observed_hash; i1 = observed_sorted; }
                lowerings
                {
                    r0 = :: cq_example_providers :: rename; r1 = ::
                    cq_example_providers :: rename; r2 = :: cq_example_providers
                    :: indexed_join; r3 = :: cq_example_providers :: rename; r4
                    = :: cq_example_providers :: indexed_join; r5 = ::
                    cq_example_providers :: project;
                }
            }
        } environment { stored L at(__cq_store_L) operations observed_slice }
        emitted { let __cq_store_L : & [Value] = L; } pending
        { input R from(R) }
        continuations(:: mini_linq :: resume_physical, mini_linq :: access)
    } plan
    {
        struct PullChain; relation L(a : Value); relation
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
                L = observed_slice; R = observed_hash; S = observed_sorted; r0
                = observed_slice; r1 = observed_hash; r2 = ::
                cq_example_providers :: borrowed_fields; r3 = observed_sorted;
                r4 = :: cq_example_providers :: borrowed_fields; r5 = ::
                cq_example_providers :: borrowed_fields;
            } indexes { i0 = observed_hash; i1 = observed_sorted; } lowerings
            {
                r0 = :: cq_example_providers :: rename; r1 = ::
                cq_example_providers :: rename; r2 = :: cq_example_providers
                :: indexed_join; r3 = :: cq_example_providers :: rename; r4 =
                :: cq_example_providers :: indexed_join; r5 = ::
                cq_example_providers :: project;
            }
        }
    } environment { stored L at(__cq_store_L) operations observed_slice }
    request { input R from(R) }
}

```

---

## 2172007/007-cq-example-providers::hash_relation

[Raw input](raw/2172007/007-cq-example-providers::hash_relation.input.tokens) · [Raw output](raw/2172007/007-cq-example-providers::hash_relation.output.tokens)

Actual input:

```rust
reply_to(observe_reply)
{
    callback(:: mini_linq :: resume_physical) kind(1) state
    {plan
        {
            struct PullChain; relation L(a : Value); relation
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
                    L = observed_slice; R = observed_hash; S = observed_sorted;
                    r0 = observed_slice; r1 = observed_hash; r2 = ::
                    cq_example_providers :: borrowed_fields; r3 =
                    observed_sorted; r4 = :: cq_example_providers ::
                    borrowed_fields; r5 = :: cq_example_providers ::
                    borrowed_fields;
                } indexes { i0 = observed_hash; i1 = observed_sorted; }
                lowerings
                {
                    r0 = :: cq_example_providers :: rename; r1 = ::
                    cq_example_providers :: rename; r2 = :: cq_example_providers
                    :: indexed_join; r3 = :: cq_example_providers :: rename; r4
                    = :: cq_example_providers :: indexed_join; r5 = ::
                    cq_example_providers :: project;
                }
            }
        } environment { stored L at(__cq_store_L) operations observed_slice }
        emitted { let __cq_store_L : & [Value] = L; } pending
        { input R from(R) }
        continuations(:: mini_linq :: resume_physical, mini_linq :: access)}
} plan
{struct PullChain; relation L(a : Value); relation R(a : Value, b : Value);
    relation S(b : Value, c : Value); answer(a, c) : - L(a), R(a, b), S(b, c);
    relational
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
            L = observed_slice; R = observed_hash; S = observed_sorted; r0 =
            observed_slice; r1 = observed_hash; r2 = :: cq_example_providers
            :: borrowed_fields; r3 = observed_sorted; r4 = ::
            cq_example_providers :: borrowed_fields; r5 = ::
            cq_example_providers :: borrowed_fields;
        } indexes { i0 = observed_hash; i1 = observed_sorted; } lowerings
        {
            r0 = :: cq_example_providers :: rename; r1 = ::
            cq_example_providers :: rename; r2 = :: cq_example_providers ::
            indexed_join; r3 = :: cq_example_providers :: rename; r4 = ::
            cq_example_providers :: indexed_join; r5 = :: cq_example_providers
            :: project;
        }
    }} environment {stored L at(__cq_store_L) operations observed_slice}
request {input R from(R)}
```

Actual output (formatted where Rust syntax permits):

```rust
observe_reply! {
    resume
    {
        callback(:: mini_linq :: resume_physical) kind(1) state
        {
            plan
            {
                struct PullChain; relation L(a : Value); relation
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
                        L = observed_slice; R = observed_hash; S = observed_sorted;
                        r0 = observed_slice; r1 = observed_hash; r2 = ::
                        cq_example_providers :: borrowed_fields; r3 =
                        observed_sorted; r4 = :: cq_example_providers ::
                        borrowed_fields; r5 = :: cq_example_providers ::
                        borrowed_fields;
                    } indexes { i0 = observed_hash; i1 = observed_sorted; }
                    lowerings
                    {
                        r0 = :: cq_example_providers :: rename; r1 = ::
                        cq_example_providers :: rename; r2 = :: cq_example_providers
                        :: indexed_join; r3 = :: cq_example_providers :: rename; r4
                        = :: cq_example_providers :: indexed_join; r5 = ::
                        cq_example_providers :: project;
                    }
                }
            } environment
            { stored L at(__cq_store_L) operations observed_slice } emitted
            { let __cq_store_L : & [Value] = L; } pending { input R from(R) }
            continuations(:: mini_linq :: resume_physical, mini_linq ::
            access)
        }
    } emitted
    {
        let __cq_store_R : & :: std :: collections :: HashMap < Value, :: std
        :: vec :: Vec < Value >> = R;
    } result { stored R at(__cq_store_R) operations observed_hash }
}

```

---

## 2172007/008-resume_physical

[Raw input](raw/2172007/008-resume_physical.input.tokens) · [Raw output](raw/2172007/008-resume_physical.output.tokens)

Actual input:

```rust
resume
{plan
    {
        struct PullChain; relation L(a : Value); relation
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
                L = observed_slice; R = observed_hash; S = observed_sorted; r0
                = observed_slice; r1 = observed_hash; r2 = ::
                cq_example_providers :: borrowed_fields; r3 = observed_sorted;
                r4 = :: cq_example_providers :: borrowed_fields; r5 = ::
                cq_example_providers :: borrowed_fields;
            } indexes { i0 = observed_hash; i1 = observed_sorted; } lowerings
            {
                r0 = :: cq_example_providers :: rename; r1 = ::
                cq_example_providers :: rename; r2 = :: cq_example_providers
                :: indexed_join; r3 = :: cq_example_providers :: rename; r4 =
                :: cq_example_providers :: indexed_join; r5 = ::
                cq_example_providers :: project;
            }
        }
    } environment { stored L at(__cq_store_L) operations observed_slice }
    emitted { let __cq_store_L : & [Value] = L; } pending { input R from(R) }
    continuations(:: mini_linq :: resume_physical, mini_linq :: access)}
emitted
{let __cq_store_R : & :: std :: collections :: HashMap < Value, :: std :: vec
    :: Vec < Value >> = R;} result
{stored R at(__cq_store_R) operations observed_hash}
```

Actual output (formatted where Rust syntax permits):

```rust
observed_sorted! {
    reply_to(:: mini_linq :: resume_physical)
    {
        plan
        {
            struct PullChain; relation L(a : Value); relation
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
                    L = observed_slice; R = observed_hash; S = observed_sorted;
                    r0 = observed_slice; r1 = observed_hash; r2 = ::
                    cq_example_providers :: borrowed_fields; r3 =
                    observed_sorted; r4 = :: cq_example_providers ::
                    borrowed_fields; r5 = :: cq_example_providers ::
                    borrowed_fields;
                } indexes { i0 = observed_hash; i1 = observed_sorted; }
                lowerings
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
            stored L at(__cq_store_L) operations observed_slice stored R
            at(__cq_store_R) operations observed_hash
        } emitted
        {
            let __cq_store_L : & [Value] = L; let __cq_store_R : & :: std ::
            collections :: HashMap < Value, :: std :: vec :: Vec < Value > > =
            R;
        } pending { input S from(S) }
        continuations(:: mini_linq :: resume_physical, mini_linq :: access)
    } plan
    {
        struct PullChain; relation L(a : Value); relation
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
                L = observed_slice; R = observed_hash; S = observed_sorted; r0
                = observed_slice; r1 = observed_hash; r2 = ::
                cq_example_providers :: borrowed_fields; r3 = observed_sorted;
                r4 = :: cq_example_providers :: borrowed_fields; r5 = ::
                cq_example_providers :: borrowed_fields;
            } indexes { i0 = observed_hash; i1 = observed_sorted; } lowerings
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
        stored L at(__cq_store_L) operations observed_slice stored R
        at(__cq_store_R) operations observed_hash
    } request { input S from(S) }
}

```

---

## 2172007/009-cq-example-providers::sorted_relation

[Raw input](raw/2172007/009-cq-example-providers::sorted_relation.input.tokens) · [Raw output](raw/2172007/009-cq-example-providers::sorted_relation.output.tokens)

Actual input:

```rust
reply_to(observe_reply)
{
    callback(:: mini_linq :: resume_physical) kind(2) state
    {plan
        {
            struct PullChain; relation L(a : Value); relation
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
                    L = observed_slice; R = observed_hash; S = observed_sorted;
                    r0 = observed_slice; r1 = observed_hash; r2 = ::
                    cq_example_providers :: borrowed_fields; r3 =
                    observed_sorted; r4 = :: cq_example_providers ::
                    borrowed_fields; r5 = :: cq_example_providers ::
                    borrowed_fields;
                } indexes { i0 = observed_hash; i1 = observed_sorted; }
                lowerings
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
            stored L at(__cq_store_L) operations observed_slice stored R
            at(__cq_store_R) operations observed_hash
        } emitted
        {
            let __cq_store_L : & [Value] = L; let __cq_store_R : & :: std ::
            collections :: HashMap < Value, :: std :: vec :: Vec < Value > > =
            R;
        } pending { input S from(S) }
        continuations(:: mini_linq :: resume_physical, mini_linq :: access)}
} plan
{struct PullChain; relation L(a : Value); relation R(a : Value, b : Value);
    relation S(b : Value, c : Value); answer(a, c) : - L(a), R(a, b), S(b, c);
    relational
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
            L = observed_slice; R = observed_hash; S = observed_sorted; r0 =
            observed_slice; r1 = observed_hash; r2 = :: cq_example_providers
            :: borrowed_fields; r3 = observed_sorted; r4 = ::
            cq_example_providers :: borrowed_fields; r5 = ::
            cq_example_providers :: borrowed_fields;
        } indexes { i0 = observed_hash; i1 = observed_sorted; } lowerings
        {
            r0 = :: cq_example_providers :: rename; r1 = ::
            cq_example_providers :: rename; r2 = :: cq_example_providers ::
            indexed_join; r3 = :: cq_example_providers :: rename; r4 = ::
            cq_example_providers :: indexed_join; r5 = :: cq_example_providers
            :: project;
        }
    }} environment
{stored L at(__cq_store_L) operations observed_slice stored R at(__cq_store_R)
    operations observed_hash} request {input S from(S)}
```

Actual output (formatted where Rust syntax permits):

```rust
observe_reply! {
    resume
    {
        callback(:: mini_linq :: resume_physical) kind(2) state
        {
            plan
            {
                struct PullChain; relation L(a : Value); relation
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
                        L = observed_slice; R = observed_hash; S = observed_sorted;
                        r0 = observed_slice; r1 = observed_hash; r2 = ::
                        cq_example_providers :: borrowed_fields; r3 =
                        observed_sorted; r4 = :: cq_example_providers ::
                        borrowed_fields; r5 = :: cq_example_providers ::
                        borrowed_fields;
                    } indexes { i0 = observed_hash; i1 = observed_sorted; }
                    lowerings
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
                stored L at(__cq_store_L) operations observed_slice stored R
                at(__cq_store_R) operations observed_hash
            } emitted
            {
                let __cq_store_L : & [Value] = L; let __cq_store_R : & :: std
                :: collections :: HashMap < Value, :: std :: vec :: Vec <
                Value > > = R;
            } pending { input S from(S) }
            continuations(:: mini_linq :: resume_physical, mini_linq ::
            access)
        }
    } emitted
    {
        let __cq_store_S : & :: std :: collections :: BTreeMap < Value, :: std
        :: vec :: Vec < Value >> = S;
    } result { stored S at(__cq_store_S) operations observed_sorted }
}

```

---

## 2172007/010-resume_physical

[Raw input](raw/2172007/010-resume_physical.input.tokens) · [Raw output](raw/2172007/010-resume_physical.output.tokens)

Actual input:

```rust
resume
{plan
    {
        struct PullChain; relation L(a : Value); relation
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
                L = observed_slice; R = observed_hash; S = observed_sorted; r0
                = observed_slice; r1 = observed_hash; r2 = ::
                cq_example_providers :: borrowed_fields; r3 = observed_sorted;
                r4 = :: cq_example_providers :: borrowed_fields; r5 = ::
                cq_example_providers :: borrowed_fields;
            } indexes { i0 = observed_hash; i1 = observed_sorted; } lowerings
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
        stored L at(__cq_store_L) operations observed_slice stored R
        at(__cq_store_R) operations observed_hash
    } emitted
    {
        let __cq_store_L : & [Value] = L; let __cq_store_R : & :: std ::
        collections :: HashMap < Value, :: std :: vec :: Vec < Value > > = R;
    } pending { input S from(S) }
    continuations(:: mini_linq :: resume_physical, mini_linq :: access)}
emitted
{let __cq_store_S : & :: std :: collections :: BTreeMap < Value, :: std :: vec
    :: Vec < Value >> = S;} result
{stored S at(__cq_store_S) operations observed_sorted}
```

Actual output (formatted where Rust syntax permits):

```rust
::cq_example_providers::rename! {
    reply_to(:: mini_linq :: resume_physical)
    {
        plan
        {
            struct PullChain; relation L(a : Value); relation
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
                    L = observed_slice; R = observed_hash; S = observed_sorted;
                    r0 = observed_slice; r1 = observed_hash; r2 = ::
                    cq_example_providers :: borrowed_fields; r3 =
                    observed_sorted; r4 = :: cq_example_providers ::
                    borrowed_fields; r5 = :: cq_example_providers ::
                    borrowed_fields;
                } indexes { i0 = observed_hash; i1 = observed_sorted; }
                lowerings
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
            stored L at(__cq_store_L) operations observed_slice stored R
            at(__cq_store_R) operations observed_hash stored S
            at(__cq_store_S) operations observed_sorted
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
        struct PullChain; relation L(a : Value); relation
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
                L = observed_slice; R = observed_hash; S = observed_sorted; r0
                = observed_slice; r1 = observed_hash; r2 = ::
                cq_example_providers :: borrowed_fields; r3 = observed_sorted;
                r4 = :: cq_example_providers :: borrowed_fields; r5 = ::
                cq_example_providers :: borrowed_fields;
            } indexes { i0 = observed_hash; i1 = observed_sorted; } lowerings
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
        stored L at(__cq_store_L) operations observed_slice stored R
        at(__cq_store_R) operations observed_hash stored S at(__cq_store_S)
        operations observed_sorted
    } request { lower r0 }
}

```

---

## 2172007/011-cq-example-providers::rename

[Raw input](raw/2172007/011-cq-example-providers::rename.input.tokens) · [Raw output](raw/2172007/011-cq-example-providers::rename.output.tokens)

Actual input:

```rust
reply_to(:: mini_linq :: resume_physical)
{
    plan
    {
        struct PullChain; relation L(a : Value); relation
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
                L = observed_slice; R = observed_hash; S = observed_sorted; r0
                = observed_slice; r1 = observed_hash; r2 = ::
                cq_example_providers :: borrowed_fields; r3 = observed_sorted;
                r4 = :: cq_example_providers :: borrowed_fields; r5 = ::
                cq_example_providers :: borrowed_fields;
            } indexes { i0 = observed_hash; i1 = observed_sorted; } lowerings
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
        stored L at(__cq_store_L) operations observed_slice stored R
        at(__cq_store_R) operations observed_hash stored S at(__cq_store_S)
        operations observed_sorted
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
    struct PullChain; relation L(a : Value); relation R(a : Value, b : Value);
    relation S(b : Value, c : Value); answer(a, c) : - L(a), R(a, b), S(b, c);
    relational
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
            L = observed_slice; R = observed_hash; S = observed_sorted; r0 =
            observed_slice; r1 = observed_hash; r2 = :: cq_example_providers
            :: borrowed_fields; r3 = observed_sorted; r4 = ::
            cq_example_providers :: borrowed_fields; r5 = ::
            cq_example_providers :: borrowed_fields;
        } indexes { i0 = observed_hash; i1 = observed_sorted; } lowerings
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
    stored L at(__cq_store_L) operations observed_slice stored R
    at(__cq_store_R) operations observed_hash stored S at(__cq_store_S)
    operations observed_sorted
} request { lower r0 }
```

Actual output (formatted where Rust syntax permits):

```rust
observed_slice! {
    reply_to(:: mini_linq :: resume_physical)
    {
        plan
        {
            struct PullChain; relation L(a : Value); relation
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
                    L = observed_slice; R = observed_hash; S = observed_sorted;
                    r0 = observed_slice; r1 = observed_hash; r2 = ::
                    cq_example_providers :: borrowed_fields; r3 =
                    observed_sorted; r4 = :: cq_example_providers ::
                    borrowed_fields; r5 = :: cq_example_providers ::
                    borrowed_fields;
                } indexes { i0 = observed_hash; i1 = observed_sorted; }
                lowerings
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
            stored L at(__cq_store_L) operations observed_slice stored R
            at(__cq_store_R) operations observed_hash stored S
            at(__cq_store_S) operations observed_sorted
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
        struct PullChain; relation L(a : Value); relation
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
                L = observed_slice; R = observed_hash; S = observed_sorted; r0
                = observed_slice; r1 = observed_hash; r2 = ::
                cq_example_providers :: borrowed_fields; r3 = observed_sorted;
                r4 = :: cq_example_providers :: borrowed_fields; r5 = ::
                cq_example_providers :: borrowed_fields;
            } indexes { i0 = observed_hash; i1 = observed_sorted; } lowerings
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
        stored L at(__cq_store_L) operations observed_slice stored R
        at(__cq_store_R) operations observed_hash stored S at(__cq_store_S)
        operations observed_sorted
    } request { input r0 from(__cq_store_L) }
}

```

---

## 2172007/012-cq-example-providers::slice_rows

[Raw input](raw/2172007/012-cq-example-providers::slice_rows.input.tokens) · [Raw output](raw/2172007/012-cq-example-providers::slice_rows.output.tokens)

Actual input:

```rust
reply_to(observe_reply)
{
    callback(:: mini_linq :: resume_physical) kind(0) state
    {plan
        {
            struct PullChain; relation L(a : Value); relation
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
                    L = observed_slice; R = observed_hash; S = observed_sorted;
                    r0 = observed_slice; r1 = observed_hash; r2 = ::
                    cq_example_providers :: borrowed_fields; r3 =
                    observed_sorted; r4 = :: cq_example_providers ::
                    borrowed_fields; r5 = :: cq_example_providers ::
                    borrowed_fields;
                } indexes { i0 = observed_hash; i1 = observed_sorted; }
                lowerings
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
            stored L at(__cq_store_L) operations observed_slice stored R
            at(__cq_store_R) operations observed_hash stored S
            at(__cq_store_S) operations observed_sorted
        } emitted
        {
            let __cq_store_L : & [Value] = L; let __cq_store_R : & :: std ::
            collections :: HashMap < Value, :: std :: vec :: Vec < Value > > =
            R; let __cq_store_S : & :: std :: collections :: BTreeMap < Value,
            :: std :: vec :: Vec < Value > > = S;
        } pending { lower r0 }
        continuations(:: mini_linq :: resume_physical, mini_linq :: access)}
} plan
{struct PullChain; relation L(a : Value); relation R(a : Value, b : Value);
    relation S(b : Value, c : Value); answer(a, c) : - L(a), R(a, b), S(b, c);
    relational
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
            L = observed_slice; R = observed_hash; S = observed_sorted; r0 =
            observed_slice; r1 = observed_hash; r2 = :: cq_example_providers
            :: borrowed_fields; r3 = observed_sorted; r4 = ::
            cq_example_providers :: borrowed_fields; r5 = ::
            cq_example_providers :: borrowed_fields;
        } indexes { i0 = observed_hash; i1 = observed_sorted; } lowerings
        {
            r0 = :: cq_example_providers :: rename; r1 = ::
            cq_example_providers :: rename; r2 = :: cq_example_providers ::
            indexed_join; r3 = :: cq_example_providers :: rename; r4 = ::
            cq_example_providers :: indexed_join; r5 = :: cq_example_providers
            :: project;
        }
    }} environment
{stored L at(__cq_store_L) operations observed_slice stored R at(__cq_store_R)
    operations observed_hash stored S at(__cq_store_S) operations
    observed_sorted} request {input r0 from(__cq_store_L)}
```

Actual output (formatted where Rust syntax permits):

```rust
observe_reply! {
    resume
    {
        callback(:: mini_linq :: resume_physical) kind(0) state
        {
            plan
            {
                struct PullChain; relation L(a : Value); relation
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
                        L = observed_slice; R = observed_hash; S = observed_sorted;
                        r0 = observed_slice; r1 = observed_hash; r2 = ::
                        cq_example_providers :: borrowed_fields; r3 =
                        observed_sorted; r4 = :: cq_example_providers ::
                        borrowed_fields; r5 = :: cq_example_providers ::
                        borrowed_fields;
                    } indexes { i0 = observed_hash; i1 = observed_sorted; }
                    lowerings
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
                stored L at(__cq_store_L) operations observed_slice stored R
                at(__cq_store_R) operations observed_hash stored S
                at(__cq_store_S) operations observed_sorted
            } emitted
            {
                let __cq_store_L : & [Value] = L; let __cq_store_R : & :: std
                :: collections :: HashMap < Value, :: std :: vec :: Vec <
                Value > > = R; let __cq_store_S : & :: std :: collections ::
                BTreeMap < Value, :: std :: vec :: Vec < Value > > = S;
            } pending { lower r0 }
            continuations(:: mini_linq :: resume_physical, mini_linq ::
            access)
        }
    } emitted { let __cq_store_r0 : & [Value] = __cq_store_L; } result
    { stored r0 at(__cq_store_r0) operations observed_slice }
}

```

---

## 2172007/013-resume_physical

[Raw input](raw/2172007/013-resume_physical.input.tokens) · [Raw output](raw/2172007/013-resume_physical.output.tokens)

Actual input:

```rust
resume
{plan
    {
        struct PullChain; relation L(a : Value); relation
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
                L = observed_slice; R = observed_hash; S = observed_sorted; r0
                = observed_slice; r1 = observed_hash; r2 = ::
                cq_example_providers :: borrowed_fields; r3 = observed_sorted;
                r4 = :: cq_example_providers :: borrowed_fields; r5 = ::
                cq_example_providers :: borrowed_fields;
            } indexes { i0 = observed_hash; i1 = observed_sorted; } lowerings
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
        stored L at(__cq_store_L) operations observed_slice stored R
        at(__cq_store_R) operations observed_hash stored S at(__cq_store_S)
        operations observed_sorted
    } emitted
    {
        let __cq_store_L : & [Value] = L; let __cq_store_R : & :: std ::
        collections :: HashMap < Value, :: std :: vec :: Vec < Value > > = R;
        let __cq_store_S : & :: std :: collections :: BTreeMap < Value, :: std
        :: vec :: Vec < Value > > = S;
    } pending { lower r0 }
    continuations(:: mini_linq :: resume_physical, mini_linq :: access)}
emitted {let __cq_store_r0 : & [Value] = __cq_store_L;} result
{stored r0 at(__cq_store_r0) operations observed_slice}
```

Actual output (formatted where Rust syntax permits):

```rust
::cq_example_providers::rename! {
    reply_to(:: mini_linq :: resume_physical)
    {
        plan
        {
            struct PullChain; relation L(a : Value); relation
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
                    L = observed_slice; R = observed_hash; S = observed_sorted;
                    r0 = observed_slice; r1 = observed_hash; r2 = ::
                    cq_example_providers :: borrowed_fields; r3 =
                    observed_sorted; r4 = :: cq_example_providers ::
                    borrowed_fields; r5 = :: cq_example_providers ::
                    borrowed_fields;
                } indexes { i0 = observed_hash; i1 = observed_sorted; }
                lowerings
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
            stored L at(__cq_store_L) operations observed_slice stored R
            at(__cq_store_R) operations observed_hash stored S
            at(__cq_store_S) operations observed_sorted stored r0
            at(__cq_store_r0) operations observed_slice
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
        struct PullChain; relation L(a : Value); relation
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
                L = observed_slice; R = observed_hash; S = observed_sorted; r0
                = observed_slice; r1 = observed_hash; r2 = ::
                cq_example_providers :: borrowed_fields; r3 = observed_sorted;
                r4 = :: cq_example_providers :: borrowed_fields; r5 = ::
                cq_example_providers :: borrowed_fields;
            } indexes { i0 = observed_hash; i1 = observed_sorted; } lowerings
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
        stored L at(__cq_store_L) operations observed_slice stored R
        at(__cq_store_R) operations observed_hash stored S at(__cq_store_S)
        operations observed_sorted stored r0 at(__cq_store_r0) operations
        observed_slice
    } request { lower r1 }
}

```

---

## 2172007/014-cq-example-providers::rename

[Raw input](raw/2172007/014-cq-example-providers::rename.input.tokens) · [Raw output](raw/2172007/014-cq-example-providers::rename.output.tokens)

Actual input:

```rust
reply_to(:: mini_linq :: resume_physical)
{
    plan
    {
        struct PullChain; relation L(a : Value); relation
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
                L = observed_slice; R = observed_hash; S = observed_sorted; r0
                = observed_slice; r1 = observed_hash; r2 = ::
                cq_example_providers :: borrowed_fields; r3 = observed_sorted;
                r4 = :: cq_example_providers :: borrowed_fields; r5 = ::
                cq_example_providers :: borrowed_fields;
            } indexes { i0 = observed_hash; i1 = observed_sorted; } lowerings
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
        stored L at(__cq_store_L) operations observed_slice stored R
        at(__cq_store_R) operations observed_hash stored S at(__cq_store_S)
        operations observed_sorted stored r0 at(__cq_store_r0) operations
        observed_slice
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
    struct PullChain; relation L(a : Value); relation R(a : Value, b : Value);
    relation S(b : Value, c : Value); answer(a, c) : - L(a), R(a, b), S(b, c);
    relational
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
            L = observed_slice; R = observed_hash; S = observed_sorted; r0 =
            observed_slice; r1 = observed_hash; r2 = :: cq_example_providers
            :: borrowed_fields; r3 = observed_sorted; r4 = ::
            cq_example_providers :: borrowed_fields; r5 = ::
            cq_example_providers :: borrowed_fields;
        } indexes { i0 = observed_hash; i1 = observed_sorted; } lowerings
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
    stored L at(__cq_store_L) operations observed_slice stored R
    at(__cq_store_R) operations observed_hash stored S at(__cq_store_S)
    operations observed_sorted stored r0 at(__cq_store_r0) operations
    observed_slice
} request { lower r1 }
```

Actual output (formatted where Rust syntax permits):

```rust
observed_hash! {
    reply_to(:: mini_linq :: resume_physical)
    {
        plan
        {
            struct PullChain; relation L(a : Value); relation
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
                    L = observed_slice; R = observed_hash; S = observed_sorted;
                    r0 = observed_slice; r1 = observed_hash; r2 = ::
                    cq_example_providers :: borrowed_fields; r3 =
                    observed_sorted; r4 = :: cq_example_providers ::
                    borrowed_fields; r5 = :: cq_example_providers ::
                    borrowed_fields;
                } indexes { i0 = observed_hash; i1 = observed_sorted; }
                lowerings
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
            stored L at(__cq_store_L) operations observed_slice stored R
            at(__cq_store_R) operations observed_hash stored S
            at(__cq_store_S) operations observed_sorted stored r0
            at(__cq_store_r0) operations observed_slice
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
        struct PullChain; relation L(a : Value); relation
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
                L = observed_slice; R = observed_hash; S = observed_sorted; r0
                = observed_slice; r1 = observed_hash; r2 = ::
                cq_example_providers :: borrowed_fields; r3 = observed_sorted;
                r4 = :: cq_example_providers :: borrowed_fields; r5 = ::
                cq_example_providers :: borrowed_fields;
            } indexes { i0 = observed_hash; i1 = observed_sorted; } lowerings
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
        stored L at(__cq_store_L) operations observed_slice stored R
        at(__cq_store_R) operations observed_hash stored S at(__cq_store_S)
        operations observed_sorted stored r0 at(__cq_store_r0) operations
        observed_slice
    } request { input r1 from(__cq_store_R) }
}

```

---

## 2172007/015-cq-example-providers::hash_relation

[Raw input](raw/2172007/015-cq-example-providers::hash_relation.input.tokens) · [Raw output](raw/2172007/015-cq-example-providers::hash_relation.output.tokens)

Actual input:

```rust
reply_to(observe_reply)
{
    callback(:: mini_linq :: resume_physical) kind(1) state
    {plan
        {
            struct PullChain; relation L(a : Value); relation
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
                    L = observed_slice; R = observed_hash; S = observed_sorted;
                    r0 = observed_slice; r1 = observed_hash; r2 = ::
                    cq_example_providers :: borrowed_fields; r3 =
                    observed_sorted; r4 = :: cq_example_providers ::
                    borrowed_fields; r5 = :: cq_example_providers ::
                    borrowed_fields;
                } indexes { i0 = observed_hash; i1 = observed_sorted; }
                lowerings
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
            stored L at(__cq_store_L) operations observed_slice stored R
            at(__cq_store_R) operations observed_hash stored S
            at(__cq_store_S) operations observed_sorted stored r0
            at(__cq_store_r0) operations observed_slice
        } emitted
        {
            let __cq_store_L : & [Value] = L; let __cq_store_R : & :: std ::
            collections :: HashMap < Value, :: std :: vec :: Vec < Value > > =
            R; let __cq_store_S : & :: std :: collections :: BTreeMap < Value,
            :: std :: vec :: Vec < Value > > = S; let __cq_store_r0 : &
            [Value] = __cq_store_L;
        } pending { lower r1 }
        continuations(:: mini_linq :: resume_physical, mini_linq :: access)}
} plan
{struct PullChain; relation L(a : Value); relation R(a : Value, b : Value);
    relation S(b : Value, c : Value); answer(a, c) : - L(a), R(a, b), S(b, c);
    relational
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
            L = observed_slice; R = observed_hash; S = observed_sorted; r0 =
            observed_slice; r1 = observed_hash; r2 = :: cq_example_providers
            :: borrowed_fields; r3 = observed_sorted; r4 = ::
            cq_example_providers :: borrowed_fields; r5 = ::
            cq_example_providers :: borrowed_fields;
        } indexes { i0 = observed_hash; i1 = observed_sorted; } lowerings
        {
            r0 = :: cq_example_providers :: rename; r1 = ::
            cq_example_providers :: rename; r2 = :: cq_example_providers ::
            indexed_join; r3 = :: cq_example_providers :: rename; r4 = ::
            cq_example_providers :: indexed_join; r5 = :: cq_example_providers
            :: project;
        }
    }} environment
{stored L at(__cq_store_L) operations observed_slice stored R at(__cq_store_R)
    operations observed_hash stored S at(__cq_store_S) operations
    observed_sorted stored r0 at(__cq_store_r0) operations observed_slice}
request {input r1 from(__cq_store_R)}
```

Actual output (formatted where Rust syntax permits):

```rust
observe_reply! {
    resume
    {
        callback(:: mini_linq :: resume_physical) kind(1) state
        {
            plan
            {
                struct PullChain; relation L(a : Value); relation
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
                        L = observed_slice; R = observed_hash; S = observed_sorted;
                        r0 = observed_slice; r1 = observed_hash; r2 = ::
                        cq_example_providers :: borrowed_fields; r3 =
                        observed_sorted; r4 = :: cq_example_providers ::
                        borrowed_fields; r5 = :: cq_example_providers ::
                        borrowed_fields;
                    } indexes { i0 = observed_hash; i1 = observed_sorted; }
                    lowerings
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
                stored L at(__cq_store_L) operations observed_slice stored R
                at(__cq_store_R) operations observed_hash stored S
                at(__cq_store_S) operations observed_sorted stored r0
                at(__cq_store_r0) operations observed_slice
            } emitted
            {
                let __cq_store_L : & [Value] = L; let __cq_store_R : & :: std
                :: collections :: HashMap < Value, :: std :: vec :: Vec <
                Value > > = R; let __cq_store_S : & :: std :: collections ::
                BTreeMap < Value, :: std :: vec :: Vec < Value > > = S; let
                __cq_store_r0 : & [Value] = __cq_store_L;
            } pending { lower r1 }
            continuations(:: mini_linq :: resume_physical, mini_linq ::
            access)
        }
    } emitted
    {
        let __cq_store_r1 : & :: std :: collections :: HashMap < Value, :: std
        :: vec :: Vec < Value >> = __cq_store_R;
    } result { stored r1 at(__cq_store_r1) operations observed_hash }
}

```

---

## 2172007/016-resume_physical

[Raw input](raw/2172007/016-resume_physical.input.tokens) · [Raw output](raw/2172007/016-resume_physical.output.tokens)

Actual input:

```rust
resume
{plan
    {
        struct PullChain; relation L(a : Value); relation
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
                L = observed_slice; R = observed_hash; S = observed_sorted; r0
                = observed_slice; r1 = observed_hash; r2 = ::
                cq_example_providers :: borrowed_fields; r3 = observed_sorted;
                r4 = :: cq_example_providers :: borrowed_fields; r5 = ::
                cq_example_providers :: borrowed_fields;
            } indexes { i0 = observed_hash; i1 = observed_sorted; } lowerings
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
        stored L at(__cq_store_L) operations observed_slice stored R
        at(__cq_store_R) operations observed_hash stored S at(__cq_store_S)
        operations observed_sorted stored r0 at(__cq_store_r0) operations
        observed_slice
    } emitted
    {
        let __cq_store_L : & [Value] = L; let __cq_store_R : & :: std ::
        collections :: HashMap < Value, :: std :: vec :: Vec < Value > > = R;
        let __cq_store_S : & :: std :: collections :: BTreeMap < Value, :: std
        :: vec :: Vec < Value > > = S; let __cq_store_r0 : & [Value] =
        __cq_store_L;
    } pending { lower r1 }
    continuations(:: mini_linq :: resume_physical, mini_linq :: access)}
emitted
{let __cq_store_r1 : & :: std :: collections :: HashMap < Value, :: std :: vec
    :: Vec < Value >> = __cq_store_R;} result
{stored r1 at(__cq_store_r1) operations observed_hash}
```

Actual output (formatted where Rust syntax permits):

```rust
observed_hash! {
    reply_to(:: mini_linq :: resume_physical)
    {
        plan
        {
            struct PullChain; relation L(a : Value); relation
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
                    L = observed_slice; R = observed_hash; S = observed_sorted;
                    r0 = observed_slice; r1 = observed_hash; r2 = ::
                    cq_example_providers :: borrowed_fields; r3 =
                    observed_sorted; r4 = :: cq_example_providers ::
                    borrowed_fields; r5 = :: cq_example_providers ::
                    borrowed_fields;
                } indexes { i0 = observed_hash; i1 = observed_sorted; }
                lowerings
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
            stored L at(__cq_store_L) operations observed_slice stored R
            at(__cq_store_R) operations observed_hash stored S
            at(__cq_store_S) operations observed_sorted stored r0
            at(__cq_store_r0) operations observed_slice stored r1
            at(__cq_store_r1) operations observed_hash
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
        struct PullChain; relation L(a : Value); relation
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
                L = observed_slice; R = observed_hash; S = observed_sorted; r0
                = observed_slice; r1 = observed_hash; r2 = ::
                cq_example_providers :: borrowed_fields; r3 = observed_sorted;
                r4 = :: cq_example_providers :: borrowed_fields; r5 = ::
                cq_example_providers :: borrowed_fields;
            } indexes { i0 = observed_hash; i1 = observed_sorted; } lowerings
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
        stored L at(__cq_store_L) operations observed_slice stored R
        at(__cq_store_R) operations observed_hash stored S at(__cq_store_S)
        operations observed_sorted stored r0 at(__cq_store_r0) operations
        observed_slice stored r1 at(__cq_store_r1) operations observed_hash
    } request { index i0 }
}

```

---

## 2172007/017-cq-example-providers::hash_relation

[Raw input](raw/2172007/017-cq-example-providers::hash_relation.input.tokens) · [Raw output](raw/2172007/017-cq-example-providers::hash_relation.output.tokens)

Actual input:

```rust
reply_to(observe_reply)
{
    callback(:: mini_linq :: resume_physical) kind(1) state
    {plan
        {
            struct PullChain; relation L(a : Value); relation
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
                    L = observed_slice; R = observed_hash; S = observed_sorted;
                    r0 = observed_slice; r1 = observed_hash; r2 = ::
                    cq_example_providers :: borrowed_fields; r3 =
                    observed_sorted; r4 = :: cq_example_providers ::
                    borrowed_fields; r5 = :: cq_example_providers ::
                    borrowed_fields;
                } indexes { i0 = observed_hash; i1 = observed_sorted; }
                lowerings
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
            stored L at(__cq_store_L) operations observed_slice stored R
            at(__cq_store_R) operations observed_hash stored S
            at(__cq_store_S) operations observed_sorted stored r0
            at(__cq_store_r0) operations observed_slice stored r1
            at(__cq_store_r1) operations observed_hash
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
        continuations(:: mini_linq :: resume_physical, mini_linq :: access)}
} plan
{struct PullChain; relation L(a : Value); relation R(a : Value, b : Value);
    relation S(b : Value, c : Value); answer(a, c) : - L(a), R(a, b), S(b, c);
    relational
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
            L = observed_slice; R = observed_hash; S = observed_sorted; r0 =
            observed_slice; r1 = observed_hash; r2 = :: cq_example_providers
            :: borrowed_fields; r3 = observed_sorted; r4 = ::
            cq_example_providers :: borrowed_fields; r5 = ::
            cq_example_providers :: borrowed_fields;
        } indexes { i0 = observed_hash; i1 = observed_sorted; } lowerings
        {
            r0 = :: cq_example_providers :: rename; r1 = ::
            cq_example_providers :: rename; r2 = :: cq_example_providers ::
            indexed_join; r3 = :: cq_example_providers :: rename; r4 = ::
            cq_example_providers :: indexed_join; r5 = :: cq_example_providers
            :: project;
        }
    }} environment
{stored L at(__cq_store_L) operations observed_slice stored R at(__cq_store_R)
    operations observed_hash stored S at(__cq_store_S) operations
    observed_sorted stored r0 at(__cq_store_r0) operations observed_slice
    stored r1 at(__cq_store_r1) operations observed_hash} request {index i0}
```

Actual output (formatted where Rust syntax permits):

```rust
observe_reply! {
    resume
    {
        callback(:: mini_linq :: resume_physical) kind(1) state
        {
            plan
            {
                struct PullChain; relation L(a : Value); relation
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
                        L = observed_slice; R = observed_hash; S = observed_sorted;
                        r0 = observed_slice; r1 = observed_hash; r2 = ::
                        cq_example_providers :: borrowed_fields; r3 =
                        observed_sorted; r4 = :: cq_example_providers ::
                        borrowed_fields; r5 = :: cq_example_providers ::
                        borrowed_fields;
                    } indexes { i0 = observed_hash; i1 = observed_sorted; }
                    lowerings
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
                stored L at(__cq_store_L) operations observed_slice stored R
                at(__cq_store_R) operations observed_hash stored S
                at(__cq_store_S) operations observed_sorted stored r0
                at(__cq_store_r0) operations observed_slice stored r1
                at(__cq_store_r1) operations observed_hash
            } emitted
            {
                let __cq_store_L : & [Value] = L; let __cq_store_R : & :: std
                :: collections :: HashMap < Value, :: std :: vec :: Vec <
                Value > > = R; let __cq_store_S : & :: std :: collections ::
                BTreeMap < Value, :: std :: vec :: Vec < Value > > = S; let
                __cq_store_r0 : & [Value] = __cq_store_L; let __cq_store_r1 :
                & :: std :: collections :: HashMap < Value, :: std :: vec ::
                Vec < Value > > = __cq_store_R;
            } pending { index i0 }
            continuations(:: mini_linq :: resume_physical, mini_linq ::
            access)
        }
    } emitted
    {
        let __cq_index_i0 : & :: std :: collections :: HashMap < Value, :: std
        :: vec :: Vec < Value >> = __cq_store_r1;
    } result { indexed i0 at(__cq_index_i0) operations observed_hash }
}

```

---

## 2172007/018-resume_physical

[Raw input](raw/2172007/018-resume_physical.input.tokens) · [Raw output](raw/2172007/018-resume_physical.output.tokens)

Actual input:

```rust
resume
{plan
    {
        struct PullChain; relation L(a : Value); relation
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
                L = observed_slice; R = observed_hash; S = observed_sorted; r0
                = observed_slice; r1 = observed_hash; r2 = ::
                cq_example_providers :: borrowed_fields; r3 = observed_sorted;
                r4 = :: cq_example_providers :: borrowed_fields; r5 = ::
                cq_example_providers :: borrowed_fields;
            } indexes { i0 = observed_hash; i1 = observed_sorted; } lowerings
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
        stored L at(__cq_store_L) operations observed_slice stored R
        at(__cq_store_R) operations observed_hash stored S at(__cq_store_S)
        operations observed_sorted stored r0 at(__cq_store_r0) operations
        observed_slice stored r1 at(__cq_store_r1) operations observed_hash
    } emitted
    {
        let __cq_store_L : & [Value] = L; let __cq_store_R : & :: std ::
        collections :: HashMap < Value, :: std :: vec :: Vec < Value > > = R;
        let __cq_store_S : & :: std :: collections :: BTreeMap < Value, :: std
        :: vec :: Vec < Value > > = S; let __cq_store_r0 : & [Value] =
        __cq_store_L; let __cq_store_r1 : & :: std :: collections :: HashMap <
        Value, :: std :: vec :: Vec < Value > > = __cq_store_R;
    } pending { index i0 }
    continuations(:: mini_linq :: resume_physical, mini_linq :: access)}
emitted
{let __cq_index_i0 : & :: std :: collections :: HashMap < Value, :: std :: vec
    :: Vec < Value >> = __cq_store_r1;} result
{indexed i0 at(__cq_index_i0) operations observed_hash}
```

Actual output (formatted where Rust syntax permits):

```rust
::cq_example_providers::indexed_join! {
    reply_to(:: mini_linq :: resume_physical)
    {
        plan
        {
            struct PullChain; relation L(a : Value); relation
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
                    L = observed_slice; R = observed_hash; S = observed_sorted;
                    r0 = observed_slice; r1 = observed_hash; r2 = ::
                    cq_example_providers :: borrowed_fields; r3 =
                    observed_sorted; r4 = :: cq_example_providers ::
                    borrowed_fields; r5 = :: cq_example_providers ::
                    borrowed_fields;
                } indexes { i0 = observed_hash; i1 = observed_sorted; }
                lowerings
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
            stored L at(__cq_store_L) operations observed_slice stored R
            at(__cq_store_R) operations observed_hash stored S
            at(__cq_store_S) operations observed_sorted stored r0
            at(__cq_store_r0) operations observed_slice stored r1
            at(__cq_store_r1) operations observed_hash indexed i0
            at(__cq_index_i0) operations observed_hash
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
        struct PullChain; relation L(a : Value); relation
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
                L = observed_slice; R = observed_hash; S = observed_sorted; r0
                = observed_slice; r1 = observed_hash; r2 = ::
                cq_example_providers :: borrowed_fields; r3 = observed_sorted;
                r4 = :: cq_example_providers :: borrowed_fields; r5 = ::
                cq_example_providers :: borrowed_fields;
            } indexes { i0 = observed_hash; i1 = observed_sorted; } lowerings
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
        stored L at(__cq_store_L) operations observed_slice stored R
        at(__cq_store_R) operations observed_hash stored S at(__cq_store_S)
        operations observed_sorted stored r0 at(__cq_store_r0) operations
        observed_slice stored r1 at(__cq_store_r1) operations observed_hash
        indexed i0 at(__cq_index_i0) operations observed_hash
    } request { lower r2 }
}

```

---

## 2172007/019-cq-example-providers::indexed_join

[Raw input](raw/2172007/019-cq-example-providers::indexed_join.input.tokens) · [Raw output](raw/2172007/019-cq-example-providers::indexed_join.output.tokens)

Actual input:

```rust
reply_to(:: mini_linq :: resume_physical)
{
    plan
    {
        struct PullChain; relation L(a : Value); relation
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
                L = observed_slice; R = observed_hash; S = observed_sorted; r0
                = observed_slice; r1 = observed_hash; r2 = ::
                cq_example_providers :: borrowed_fields; r3 = observed_sorted;
                r4 = :: cq_example_providers :: borrowed_fields; r5 = ::
                cq_example_providers :: borrowed_fields;
            } indexes { i0 = observed_hash; i1 = observed_sorted; } lowerings
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
        stored L at(__cq_store_L) operations observed_slice stored R
        at(__cq_store_R) operations observed_hash stored S at(__cq_store_S)
        operations observed_sorted stored r0 at(__cq_store_r0) operations
        observed_slice stored r1 at(__cq_store_r1) operations observed_hash
        indexed i0 at(__cq_index_i0) operations observed_hash
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
    struct PullChain; relation L(a : Value); relation R(a : Value, b : Value);
    relation S(b : Value, c : Value); answer(a, c) : - L(a), R(a, b), S(b, c);
    relational
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
            L = observed_slice; R = observed_hash; S = observed_sorted; r0 =
            observed_slice; r1 = observed_hash; r2 = :: cq_example_providers
            :: borrowed_fields; r3 = observed_sorted; r4 = ::
            cq_example_providers :: borrowed_fields; r5 = ::
            cq_example_providers :: borrowed_fields;
        } indexes { i0 = observed_hash; i1 = observed_sorted; } lowerings
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
    stored L at(__cq_store_L) operations observed_slice stored R
    at(__cq_store_R) operations observed_hash stored S at(__cq_store_S)
    operations observed_sorted stored r0 at(__cq_store_r0) operations
    observed_slice stored r1 at(__cq_store_r1) operations observed_hash
    indexed i0 at(__cq_index_i0) operations observed_hash
} request { lower r2 }
```

Actual output (formatted where Rust syntax permits):

```rust
observed_slice! {
    reply_to(:: cq_example_providers :: indexed_join_rows)
    {
        reply_to(:: mini_linq :: resume_physical)
        {
            plan
            {
                struct PullChain; relation L(a : Value); relation
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
                        L = observed_slice; R = observed_hash; S = observed_sorted;
                        r0 = observed_slice; r1 = observed_hash; r2 = ::
                        cq_example_providers :: borrowed_fields; r3 =
                        observed_sorted; r4 = :: cq_example_providers ::
                        borrowed_fields; r5 = :: cq_example_providers ::
                        borrowed_fields;
                    } indexes { i0 = observed_hash; i1 = observed_sorted; }
                    lowerings
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
                stored L at(__cq_store_L) operations observed_slice stored R
                at(__cq_store_R) operations observed_hash stored S
                at(__cq_store_S) operations observed_sorted stored r0
                at(__cq_store_r0) operations observed_slice stored r1
                at(__cq_store_r1) operations observed_hash indexed i0
                at(__cq_index_i0) operations observed_hash
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
            struct PullChain; relation L(a : Value); relation
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
                    L = observed_slice; R = observed_hash; S = observed_sorted;
                    r0 = observed_slice; r1 = observed_hash; r2 = ::
                    cq_example_providers :: borrowed_fields; r3 =
                    observed_sorted; r4 = :: cq_example_providers ::
                    borrowed_fields; r5 = :: cq_example_providers ::
                    borrowed_fields;
                } indexes { i0 = observed_hash; i1 = observed_sorted; }
                lowerings
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
            stored L at(__cq_store_L) operations observed_slice stored R
            at(__cq_store_R) operations observed_hash stored S
            at(__cq_store_S) operations observed_sorted stored r0
            at(__cq_store_r0) operations observed_slice stored r1
            at(__cq_store_r1) operations observed_hash indexed i0
            at(__cq_index_i0) operations observed_hash
        } request { lower r2 }
    } plan
    {
        struct PullChain; relation L(a : Value); relation
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
                L = observed_slice; R = observed_hash; S = observed_sorted; r0
                = observed_slice; r1 = observed_hash; r2 = ::
                cq_example_providers :: borrowed_fields; r3 = observed_sorted;
                r4 = :: cq_example_providers :: borrowed_fields; r5 = ::
                cq_example_providers :: borrowed_fields;
            } indexes { i0 = observed_hash; i1 = observed_sorted; } lowerings
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
        stored L at(__cq_store_L) operations observed_slice stored R
        at(__cq_store_R) operations observed_hash stored S at(__cq_store_S)
        operations observed_sorted stored r0 at(__cq_store_r0) operations
        observed_slice stored r1 at(__cq_store_r1) operations observed_hash
        indexed i0 at(__cq_index_i0) operations observed_hash
    } request { rows r0 }
}

```

---

## 2172007/020-cq-example-providers::slice_rows

[Raw input](raw/2172007/020-cq-example-providers::slice_rows.input.tokens) · [Raw output](raw/2172007/020-cq-example-providers::slice_rows.output.tokens)

Actual input:

```rust
reply_to(observe_reply)
{
    callback(:: cq_example_providers :: indexed_join_rows) kind(0) state
    {reply_to(:: mini_linq :: resume_physical)
        {
            plan
            {
                struct PullChain; relation L(a : Value); relation
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
                        L = observed_slice; R = observed_hash; S = observed_sorted;
                        r0 = observed_slice; r1 = observed_hash; r2 = ::
                        cq_example_providers :: borrowed_fields; r3 =
                        observed_sorted; r4 = :: cq_example_providers ::
                        borrowed_fields; r5 = :: cq_example_providers ::
                        borrowed_fields;
                    } indexes { i0 = observed_hash; i1 = observed_sorted; }
                    lowerings
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
                stored L at(__cq_store_L) operations observed_slice stored R
                at(__cq_store_R) operations observed_hash stored S
                at(__cq_store_S) operations observed_sorted stored r0
                at(__cq_store_r0) operations observed_slice stored r1
                at(__cq_store_r1) operations observed_hash indexed i0
                at(__cq_index_i0) operations observed_hash
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
            struct PullChain; relation L(a : Value); relation
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
                    L = observed_slice; R = observed_hash; S = observed_sorted;
                    r0 = observed_slice; r1 = observed_hash; r2 = ::
                    cq_example_providers :: borrowed_fields; r3 =
                    observed_sorted; r4 = :: cq_example_providers ::
                    borrowed_fields; r5 = :: cq_example_providers ::
                    borrowed_fields;
                } indexes { i0 = observed_hash; i1 = observed_sorted; }
                lowerings
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
            stored L at(__cq_store_L) operations observed_slice stored R
            at(__cq_store_R) operations observed_hash stored S
            at(__cq_store_S) operations observed_sorted stored r0
            at(__cq_store_r0) operations observed_slice stored r1
            at(__cq_store_r1) operations observed_hash indexed i0
            at(__cq_index_i0) operations observed_hash
        } request { lower r2 }}
} plan
{struct PullChain; relation L(a : Value); relation R(a : Value, b : Value);
    relation S(b : Value, c : Value); answer(a, c) : - L(a), R(a, b), S(b, c);
    relational
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
            L = observed_slice; R = observed_hash; S = observed_sorted; r0 =
            observed_slice; r1 = observed_hash; r2 = :: cq_example_providers
            :: borrowed_fields; r3 = observed_sorted; r4 = ::
            cq_example_providers :: borrowed_fields; r5 = ::
            cq_example_providers :: borrowed_fields;
        } indexes { i0 = observed_hash; i1 = observed_sorted; } lowerings
        {
            r0 = :: cq_example_providers :: rename; r1 = ::
            cq_example_providers :: rename; r2 = :: cq_example_providers ::
            indexed_join; r3 = :: cq_example_providers :: rename; r4 = ::
            cq_example_providers :: indexed_join; r5 = :: cq_example_providers
            :: project;
        }
    }} environment
{stored L at(__cq_store_L) operations observed_slice stored R at(__cq_store_R)
    operations observed_hash stored S at(__cq_store_S) operations
    observed_sorted stored r0 at(__cq_store_r0) operations observed_slice
    stored r1 at(__cq_store_r1) operations observed_hash indexed i0
    at(__cq_index_i0) operations observed_hash} request {rows r0}
```

Actual output (formatted where Rust syntax permits):

```rust
observe_reply! {
    resume
    {
        callback(:: cq_example_providers :: indexed_join_rows) kind(0) state
        {
            reply_to(:: mini_linq :: resume_physical)
            {
                plan
                {
                    struct PullChain; relation L(a : Value); relation
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
                            L = observed_slice; R = observed_hash; S = observed_sorted;
                            r0 = observed_slice; r1 = observed_hash; r2 = ::
                            cq_example_providers :: borrowed_fields; r3 =
                            observed_sorted; r4 = :: cq_example_providers ::
                            borrowed_fields; r5 = :: cq_example_providers ::
                            borrowed_fields;
                        } indexes { i0 = observed_hash; i1 = observed_sorted; }
                        lowerings
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
                    stored L at(__cq_store_L) operations observed_slice stored R
                    at(__cq_store_R) operations observed_hash stored S
                    at(__cq_store_S) operations observed_sorted stored r0
                    at(__cq_store_r0) operations observed_slice stored r1
                    at(__cq_store_r1) operations observed_hash indexed i0
                    at(__cq_index_i0) operations observed_hash
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
                struct PullChain; relation L(a : Value); relation
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
                        L = observed_slice; R = observed_hash; S = observed_sorted;
                        r0 = observed_slice; r1 = observed_hash; r2 = ::
                        cq_example_providers :: borrowed_fields; r3 =
                        observed_sorted; r4 = :: cq_example_providers ::
                        borrowed_fields; r5 = :: cq_example_providers ::
                        borrowed_fields;
                    } indexes { i0 = observed_hash; i1 = observed_sorted; }
                    lowerings
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
                stored L at(__cq_store_L) operations observed_slice stored R
                at(__cq_store_R) operations observed_hash stored S
                at(__cq_store_S) operations observed_sorted stored r0
                at(__cq_store_r0) operations observed_slice stored r1
                at(__cq_store_r1) operations observed_hash indexed i0
                at(__cq_index_i0) operations observed_hash
            } request { lower r2 }
        }
    } source((__cq_store_r0).iter()) item(a) witness
    { row(a) fields { 0 = (a); } }
}

```

---

## 2172007/021-cq-example-providers::indexed_join_rows

[Raw input](raw/2172007/021-cq-example-providers::indexed_join_rows.input.tokens) · [Raw output](raw/2172007/021-cq-example-providers::indexed_join_rows.output.tokens)

Actual input:

```rust
resume
{reply_to(:: mini_linq :: resume_physical)
    {
        plan
        {
            struct PullChain; relation L(a : Value); relation
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
                    L = observed_slice; R = observed_hash; S = observed_sorted;
                    r0 = observed_slice; r1 = observed_hash; r2 = ::
                    cq_example_providers :: borrowed_fields; r3 =
                    observed_sorted; r4 = :: cq_example_providers ::
                    borrowed_fields; r5 = :: cq_example_providers ::
                    borrowed_fields;
                } indexes { i0 = observed_hash; i1 = observed_sorted; }
                lowerings
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
            stored L at(__cq_store_L) operations observed_slice stored R
            at(__cq_store_R) operations observed_hash stored S
            at(__cq_store_S) operations observed_sorted stored r0
            at(__cq_store_r0) operations observed_slice stored r1
            at(__cq_store_r1) operations observed_hash indexed i0
            at(__cq_index_i0) operations observed_hash
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
        struct PullChain; relation L(a : Value); relation
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
                L = observed_slice; R = observed_hash; S = observed_sorted; r0
                = observed_slice; r1 = observed_hash; r2 = ::
                cq_example_providers :: borrowed_fields; r3 = observed_sorted;
                r4 = :: cq_example_providers :: borrowed_fields; r5 = ::
                cq_example_providers :: borrowed_fields;
            } indexes { i0 = observed_hash; i1 = observed_sorted; } lowerings
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
        stored L at(__cq_store_L) operations observed_slice stored R
        at(__cq_store_R) operations observed_hash stored S at(__cq_store_S)
        operations observed_sorted stored r0 at(__cq_store_r0) operations
        observed_slice stored r1 at(__cq_store_r1) operations observed_hash
        indexed i0 at(__cq_index_i0) operations observed_hash
    } request { lower r2 }} source(observe(0, (__cq_store_r0).iter())) item(a)
witness {row(a) fields { 0 = (a); }}
```

Actual output (formatted where Rust syntax permits):

```rust
observed_hash! {
    reply_to(:: cq_example_providers :: indexed_join_matches)
    {
        request
        {
            reply_to(:: mini_linq :: resume_physical)
            {
                plan
                {
                    struct PullChain; relation L(a : Value); relation
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
                            L = observed_slice; R = observed_hash; S = observed_sorted;
                            r0 = observed_slice; r1 = observed_hash; r2 = ::
                            cq_example_providers :: borrowed_fields; r3 =
                            observed_sorted; r4 = :: cq_example_providers ::
                            borrowed_fields; r5 = :: cq_example_providers ::
                            borrowed_fields;
                        } indexes { i0 = observed_hash; i1 = observed_sorted; }
                        lowerings
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
                    stored L at(__cq_store_L) operations observed_slice stored R
                    at(__cq_store_R) operations observed_hash stored S
                    at(__cq_store_S) operations observed_sorted stored r0
                    at(__cq_store_r0) operations observed_slice stored r1
                    at(__cq_store_r1) operations observed_hash indexed i0
                    at(__cq_index_i0) operations observed_hash
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
                struct PullChain; relation L(a : Value); relation
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
                        L = observed_slice; R = observed_hash; S = observed_sorted;
                        r0 = observed_slice; r1 = observed_hash; r2 = ::
                        cq_example_providers :: borrowed_fields; r3 =
                        observed_sorted; r4 = :: cq_example_providers ::
                        borrowed_fields; r5 = :: cq_example_providers ::
                        borrowed_fields;
                    } indexes { i0 = observed_hash; i1 = observed_sorted; }
                    lowerings
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
                stored L at(__cq_store_L) operations observed_slice stored R
                at(__cq_store_R) operations observed_hash stored S
                at(__cq_store_S) operations observed_sorted stored r0
                at(__cq_store_r0) operations observed_slice stored r1
                at(__cq_store_r1) operations observed_hash indexed i0
                at(__cq_index_i0) operations observed_hash
            } request { lower r2 }
        } access { r0(a) => bind a <- (observe(0, (__cq_store_r0).iter())) }
        witness { row(a) fields { 0 = (a); } }
    } plan
    {
        struct PullChain; relation L(a : Value); relation
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
                L = observed_slice; R = observed_hash; S = observed_sorted; r0
                = observed_slice; r1 = observed_hash; r2 = ::
                cq_example_providers :: borrowed_fields; r3 = observed_sorted;
                r4 = :: cq_example_providers :: borrowed_fields; r5 = ::
                cq_example_providers :: borrowed_fields;
            } indexes { i0 = observed_hash; i1 = observed_sorted; } lowerings
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
        stored L at(__cq_store_L) operations observed_slice stored R
        at(__cq_store_R) operations observed_hash stored S at(__cq_store_S)
        operations observed_sorted stored r0 at(__cq_store_r0) operations
        observed_slice stored r1 at(__cq_store_r1) operations observed_hash
        indexed i0 at(__cq_index_i0) operations observed_hash
    } request { matches i0 values [(a)] }
}

```

---

## 2172007/022-cq-example-providers::hash_relation

[Raw input](raw/2172007/022-cq-example-providers::hash_relation.input.tokens) · [Raw output](raw/2172007/022-cq-example-providers::hash_relation.output.tokens)

Actual input:

```rust
reply_to(observe_reply)
{
    callback(:: cq_example_providers :: indexed_join_matches) kind(1) state
    {request
        {
            reply_to(:: mini_linq :: resume_physical)
            {
                plan
                {
                    struct PullChain; relation L(a : Value); relation
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
                            L = observed_slice; R = observed_hash; S = observed_sorted;
                            r0 = observed_slice; r1 = observed_hash; r2 = ::
                            cq_example_providers :: borrowed_fields; r3 =
                            observed_sorted; r4 = :: cq_example_providers ::
                            borrowed_fields; r5 = :: cq_example_providers ::
                            borrowed_fields;
                        } indexes { i0 = observed_hash; i1 = observed_sorted; }
                        lowerings
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
                    stored L at(__cq_store_L) operations observed_slice stored R
                    at(__cq_store_R) operations observed_hash stored S
                    at(__cq_store_S) operations observed_sorted stored r0
                    at(__cq_store_r0) operations observed_slice stored r1
                    at(__cq_store_r1) operations observed_hash indexed i0
                    at(__cq_index_i0) operations observed_hash
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
                struct PullChain; relation L(a : Value); relation
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
                        L = observed_slice; R = observed_hash; S = observed_sorted;
                        r0 = observed_slice; r1 = observed_hash; r2 = ::
                        cq_example_providers :: borrowed_fields; r3 =
                        observed_sorted; r4 = :: cq_example_providers ::
                        borrowed_fields; r5 = :: cq_example_providers ::
                        borrowed_fields;
                    } indexes { i0 = observed_hash; i1 = observed_sorted; }
                    lowerings
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
                stored L at(__cq_store_L) operations observed_slice stored R
                at(__cq_store_R) operations observed_hash stored S
                at(__cq_store_S) operations observed_sorted stored r0
                at(__cq_store_r0) operations observed_slice stored r1
                at(__cq_store_r1) operations observed_hash indexed i0
                at(__cq_index_i0) operations observed_hash
            } request { lower r2 }
        } access { r0(a) => bind a <- (observe(0, (__cq_store_r0).iter())) }
        witness { row(a) fields { 0 = (a); } }}
} plan
{struct PullChain; relation L(a : Value); relation R(a : Value, b : Value);
    relation S(b : Value, c : Value); answer(a, c) : - L(a), R(a, b), S(b, c);
    relational
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
            L = observed_slice; R = observed_hash; S = observed_sorted; r0 =
            observed_slice; r1 = observed_hash; r2 = :: cq_example_providers
            :: borrowed_fields; r3 = observed_sorted; r4 = ::
            cq_example_providers :: borrowed_fields; r5 = ::
            cq_example_providers :: borrowed_fields;
        } indexes { i0 = observed_hash; i1 = observed_sorted; } lowerings
        {
            r0 = :: cq_example_providers :: rename; r1 = ::
            cq_example_providers :: rename; r2 = :: cq_example_providers ::
            indexed_join; r3 = :: cq_example_providers :: rename; r4 = ::
            cq_example_providers :: indexed_join; r5 = :: cq_example_providers
            :: project;
        }
    }} environment
{stored L at(__cq_store_L) operations observed_slice stored R at(__cq_store_R)
    operations observed_hash stored S at(__cq_store_S) operations
    observed_sorted stored r0 at(__cq_store_r0) operations observed_slice
    stored r1 at(__cq_store_r1) operations observed_hash indexed i0
    at(__cq_index_i0) operations observed_hash} request
{matches i0 values [(a)]}
```

Actual output (formatted where Rust syntax permits):

```rust
observe_reply! {
    resume
    {
        callback(:: cq_example_providers :: indexed_join_matches) kind(1)
        state
        {
            request
            {
                reply_to(:: mini_linq :: resume_physical)
                {
                    plan
                    {
                        struct PullChain; relation L(a : Value); relation
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
                                L = observed_slice; R = observed_hash; S = observed_sorted;
                                r0 = observed_slice; r1 = observed_hash; r2 = ::
                                cq_example_providers :: borrowed_fields; r3 =
                                observed_sorted; r4 = :: cq_example_providers ::
                                borrowed_fields; r5 = :: cq_example_providers ::
                                borrowed_fields;
                            } indexes { i0 = observed_hash; i1 = observed_sorted; }
                            lowerings
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
                        stored L at(__cq_store_L) operations observed_slice stored R
                        at(__cq_store_R) operations observed_hash stored S
                        at(__cq_store_S) operations observed_sorted stored r0
                        at(__cq_store_r0) operations observed_slice stored r1
                        at(__cq_store_r1) operations observed_hash indexed i0
                        at(__cq_index_i0) operations observed_hash
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
                    struct PullChain; relation L(a : Value); relation
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
                            L = observed_slice; R = observed_hash; S = observed_sorted;
                            r0 = observed_slice; r1 = observed_hash; r2 = ::
                            cq_example_providers :: borrowed_fields; r3 =
                            observed_sorted; r4 = :: cq_example_providers ::
                            borrowed_fields; r5 = :: cq_example_providers ::
                            borrowed_fields;
                        } indexes { i0 = observed_hash; i1 = observed_sorted; }
                        lowerings
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
                    stored L at(__cq_store_L) operations observed_slice stored R
                    at(__cq_store_R) operations observed_hash stored S
                    at(__cq_store_S) operations observed_sorted stored r0
                    at(__cq_store_r0) operations observed_slice stored r1
                    at(__cq_store_r1) operations observed_hash indexed i0
                    at(__cq_index_i0) operations observed_hash
                } request { lower r2 }
            } access
            { r0(a) => bind a <- (observe(0, (__cq_store_r0).iter())) }
            witness { row(a) fields { 0 = (a); } }
        }
    } source((__cq_index_i0).get(a).into_iter().flatten()) item(b) witness
    { row((a, b,)) fields { 0 = (a); 1 = (b); } }
}

```

---

## 2172007/023-cq-example-providers::indexed_join_matches

[Raw input](raw/2172007/023-cq-example-providers::indexed_join_matches.input.tokens) · [Raw output](raw/2172007/023-cq-example-providers::indexed_join_matches.output.tokens)

Actual input:

```rust
resume
{request
    {
        reply_to(:: mini_linq :: resume_physical)
        {
            plan
            {
                struct PullChain; relation L(a : Value); relation
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
                        L = observed_slice; R = observed_hash; S = observed_sorted;
                        r0 = observed_slice; r1 = observed_hash; r2 = ::
                        cq_example_providers :: borrowed_fields; r3 =
                        observed_sorted; r4 = :: cq_example_providers ::
                        borrowed_fields; r5 = :: cq_example_providers ::
                        borrowed_fields;
                    } indexes { i0 = observed_hash; i1 = observed_sorted; }
                    lowerings
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
                stored L at(__cq_store_L) operations observed_slice stored R
                at(__cq_store_R) operations observed_hash stored S
                at(__cq_store_S) operations observed_sorted stored r0
                at(__cq_store_r0) operations observed_slice stored r1
                at(__cq_store_r1) operations observed_hash indexed i0
                at(__cq_index_i0) operations observed_hash
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
            struct PullChain; relation L(a : Value); relation
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
                    L = observed_slice; R = observed_hash; S = observed_sorted;
                    r0 = observed_slice; r1 = observed_hash; r2 = ::
                    cq_example_providers :: borrowed_fields; r3 =
                    observed_sorted; r4 = :: cq_example_providers ::
                    borrowed_fields; r5 = :: cq_example_providers ::
                    borrowed_fields;
                } indexes { i0 = observed_hash; i1 = observed_sorted; }
                lowerings
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
            stored L at(__cq_store_L) operations observed_slice stored R
            at(__cq_store_R) operations observed_hash stored S
            at(__cq_store_S) operations observed_sorted stored r0
            at(__cq_store_r0) operations observed_slice stored r1
            at(__cq_store_r1) operations observed_hash indexed i0
            at(__cq_index_i0) operations observed_hash
        } request { lower r2 }
    } access { r0(a) => bind a <- (observe(0, (__cq_store_r0).iter())) }
    witness { row(a) fields { 0 = (a); } }}
source(observe(1, (__cq_index_i0).get(a).into_iter().flatten())) item(b)
witness {row((a, b,)) fields { 0 = (a); 1 = (b); }}
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
                    struct PullChain; relation L(a : Value); relation
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
                            L = observed_slice; R = observed_hash; S = observed_sorted;
                            r0 = observed_slice; r1 = observed_hash; r2 = ::
                            cq_example_providers :: borrowed_fields; r3 =
                            observed_sorted; r4 = :: cq_example_providers ::
                            borrowed_fields; r5 = :: cq_example_providers ::
                            borrowed_fields;
                        } indexes { i0 = observed_hash; i1 = observed_sorted; }
                        lowerings
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
                    stored L at(__cq_store_L) operations observed_slice stored R
                    at(__cq_store_R) operations observed_hash stored S
                    at(__cq_store_S) operations observed_sorted stored r0
                    at(__cq_store_r0) operations observed_slice stored r1
                    at(__cq_store_r1) operations observed_hash indexed i0
                    at(__cq_index_i0) operations observed_hash
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
                struct PullChain; relation L(a : Value); relation
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
                        L = observed_slice; R = observed_hash; S = observed_sorted;
                        r0 = observed_slice; r1 = observed_hash; r2 = ::
                        cq_example_providers :: borrowed_fields; r3 =
                        observed_sorted; r4 = :: cq_example_providers ::
                        borrowed_fields; r5 = :: cq_example_providers ::
                        borrowed_fields;
                    } indexes { i0 = observed_hash; i1 = observed_sorted; }
                    lowerings
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
                stored L at(__cq_store_L) operations observed_slice stored R
                at(__cq_store_R) operations observed_hash stored S
                at(__cq_store_S) operations observed_sorted stored r0
                at(__cq_store_r0) operations observed_slice stored r1
                at(__cq_store_r1) operations observed_hash indexed i0
                at(__cq_index_i0) operations observed_hash
            } request { lower r2 }
        } access
        {
            r0(a) => bind a <- (observe(0, (__cq_store_r0).iter())), r1(a, b)
            => bind b <-
            (observe(1, (__cq_index_i0).get(a).into_iter().flatten()))
        } witness { row(a) fields { 0 = (a); } }
    } plan
    {
        struct PullChain; relation L(a : Value); relation
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
                L = observed_slice; R = observed_hash; S = observed_sorted; r0
                = observed_slice; r1 = observed_hash; r2 = ::
                cq_example_providers :: borrowed_fields; r3 = observed_sorted;
                r4 = :: cq_example_providers :: borrowed_fields; r5 = ::
                cq_example_providers :: borrowed_fields;
            } indexes { i0 = observed_hash; i1 = observed_sorted; } lowerings
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
        stored L at(__cq_store_L) operations observed_slice stored R
        at(__cq_store_R) operations observed_hash stored S at(__cq_store_S)
        operations observed_sorted stored r0 at(__cq_store_r0) operations
        observed_slice stored r1 at(__cq_store_r1) operations observed_hash
        indexed i0 at(__cq_index_i0) operations observed_hash
    } request
    {
        output r2 fields { 0 = (a); 1 = (b); } whole
        { r0 = (a); r1 = ((a, b,)); }
    }
}

```

---

## 2172007/024-cq-example-providers::borrowed_fields

[Raw input](raw/2172007/024-cq-example-providers::borrowed_fields.input.tokens) · [Raw output](raw/2172007/024-cq-example-providers::borrowed_fields.output.tokens)

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
                struct PullChain; relation L(a : Value); relation
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
                        L = observed_slice; R = observed_hash; S = observed_sorted;
                        r0 = observed_slice; r1 = observed_hash; r2 = ::
                        cq_example_providers :: borrowed_fields; r3 =
                        observed_sorted; r4 = :: cq_example_providers ::
                        borrowed_fields; r5 = :: cq_example_providers ::
                        borrowed_fields;
                    } indexes { i0 = observed_hash; i1 = observed_sorted; }
                    lowerings
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
                stored L at(__cq_store_L) operations observed_slice stored R
                at(__cq_store_R) operations observed_hash stored S
                at(__cq_store_S) operations observed_sorted stored r0
                at(__cq_store_r0) operations observed_slice stored r1
                at(__cq_store_r1) operations observed_hash indexed i0
                at(__cq_index_i0) operations observed_hash
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
            struct PullChain; relation L(a : Value); relation
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
                    L = observed_slice; R = observed_hash; S = observed_sorted;
                    r0 = observed_slice; r1 = observed_hash; r2 = ::
                    cq_example_providers :: borrowed_fields; r3 =
                    observed_sorted; r4 = :: cq_example_providers ::
                    borrowed_fields; r5 = :: cq_example_providers ::
                    borrowed_fields;
                } indexes { i0 = observed_hash; i1 = observed_sorted; }
                lowerings
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
            stored L at(__cq_store_L) operations observed_slice stored R
            at(__cq_store_R) operations observed_hash stored S
            at(__cq_store_S) operations observed_sorted stored r0
            at(__cq_store_r0) operations observed_slice stored r1
            at(__cq_store_r1) operations observed_hash indexed i0
            at(__cq_index_i0) operations observed_hash
        } request { lower r2 }
    } access
    {
        r0(a) => bind a <- (observe(0, (__cq_store_r0).iter())), r1(a, b) =>
        bind b <- (observe(1, (__cq_index_i0).get(a).into_iter().flatten()))
    } witness { row(a) fields { 0 = (a); } }
} plan
{
    struct PullChain; relation L(a : Value); relation R(a : Value, b : Value);
    relation S(b : Value, c : Value); answer(a, c) : - L(a), R(a, b), S(b, c);
    relational
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
            L = observed_slice; R = observed_hash; S = observed_sorted; r0 =
            observed_slice; r1 = observed_hash; r2 = :: cq_example_providers
            :: borrowed_fields; r3 = observed_sorted; r4 = ::
            cq_example_providers :: borrowed_fields; r5 = ::
            cq_example_providers :: borrowed_fields;
        } indexes { i0 = observed_hash; i1 = observed_sorted; } lowerings
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
    stored L at(__cq_store_L) operations observed_slice stored R
    at(__cq_store_R) operations observed_hash stored S at(__cq_store_S)
    operations observed_sorted stored r0 at(__cq_store_r0) operations
    observed_slice stored r1 at(__cq_store_r1) operations observed_hash
    indexed i0 at(__cq_index_i0) operations observed_hash
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
                    struct PullChain; relation L(a : Value); relation
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
                            L = observed_slice; R = observed_hash; S = observed_sorted;
                            r0 = observed_slice; r1 = observed_hash; r2 = ::
                            cq_example_providers :: borrowed_fields; r3 =
                            observed_sorted; r4 = :: cq_example_providers ::
                            borrowed_fields; r5 = :: cq_example_providers ::
                            borrowed_fields;
                        } indexes { i0 = observed_hash; i1 = observed_sorted; }
                        lowerings
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
                    stored L at(__cq_store_L) operations observed_slice stored R
                    at(__cq_store_R) operations observed_hash stored S
                    at(__cq_store_S) operations observed_sorted stored r0
                    at(__cq_store_r0) operations observed_slice stored r1
                    at(__cq_store_r1) operations observed_hash indexed i0
                    at(__cq_index_i0) operations observed_hash
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
                struct PullChain; relation L(a : Value); relation
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
                        L = observed_slice; R = observed_hash; S = observed_sorted;
                        r0 = observed_slice; r1 = observed_hash; r2 = ::
                        cq_example_providers :: borrowed_fields; r3 =
                        observed_sorted; r4 = :: cq_example_providers ::
                        borrowed_fields; r5 = :: cq_example_providers ::
                        borrowed_fields;
                    } indexes { i0 = observed_hash; i1 = observed_sorted; }
                    lowerings
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
                stored L at(__cq_store_L) operations observed_slice stored R
                at(__cq_store_R) operations observed_hash stored S
                at(__cq_store_S) operations observed_sorted stored r0
                at(__cq_store_r0) operations observed_slice stored r1
                at(__cq_store_r1) operations observed_hash indexed i0
                at(__cq_index_i0) operations observed_hash
            } request { lower r2 }
        } access
        {
            r0(a) => bind a <- (observe(0, (__cq_store_r0).iter())), r1(a, b)
            => bind b <-
            (observe(1, (__cq_index_i0).get(a).into_iter().flatten()))
        } witness { row(a) fields { 0 = (a); } }
    } witness { row((a, b,)) fields { 0 = (a); 1 = (b); } }
}

```

---

## 2172007/025-cq-example-providers::indexed_join_output

[Raw input](raw/2172007/025-cq-example-providers::indexed_join_output.input.tokens) · [Raw output](raw/2172007/025-cq-example-providers::indexed_join_output.output.tokens)

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
                struct PullChain; relation L(a : Value); relation
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
                        L = observed_slice; R = observed_hash; S = observed_sorted;
                        r0 = observed_slice; r1 = observed_hash; r2 = ::
                        cq_example_providers :: borrowed_fields; r3 =
                        observed_sorted; r4 = :: cq_example_providers ::
                        borrowed_fields; r5 = :: cq_example_providers ::
                        borrowed_fields;
                    } indexes { i0 = observed_hash; i1 = observed_sorted; }
                    lowerings
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
                stored L at(__cq_store_L) operations observed_slice stored R
                at(__cq_store_R) operations observed_hash stored S
                at(__cq_store_S) operations observed_sorted stored r0
                at(__cq_store_r0) operations observed_slice stored r1
                at(__cq_store_r1) operations observed_hash indexed i0
                at(__cq_index_i0) operations observed_hash
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
            struct PullChain; relation L(a : Value); relation
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
                    L = observed_slice; R = observed_hash; S = observed_sorted;
                    r0 = observed_slice; r1 = observed_hash; r2 = ::
                    cq_example_providers :: borrowed_fields; r3 =
                    observed_sorted; r4 = :: cq_example_providers ::
                    borrowed_fields; r5 = :: cq_example_providers ::
                    borrowed_fields;
                } indexes { i0 = observed_hash; i1 = observed_sorted; }
                lowerings
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
            stored L at(__cq_store_L) operations observed_slice stored R
            at(__cq_store_R) operations observed_hash stored S
            at(__cq_store_S) operations observed_sorted stored r0
            at(__cq_store_r0) operations observed_slice stored r1
            at(__cq_store_r1) operations observed_hash indexed i0
            at(__cq_index_i0) operations observed_hash
        } request { lower r2 }
    } access
    {
        r0(a) => bind a <- (observe(0, (__cq_store_r0).iter())), r1(a, b) =>
        bind b <- (observe(1, (__cq_index_i0).get(a).into_iter().flatten()))
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
            struct PullChain; relation L(a : Value); relation
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
                    L = observed_slice; R = observed_hash; S = observed_sorted;
                    r0 = observed_slice; r1 = observed_hash; r2 = ::
                    cq_example_providers :: borrowed_fields; r3 =
                    observed_sorted; r4 = :: cq_example_providers ::
                    borrowed_fields; r5 = :: cq_example_providers ::
                    borrowed_fields;
                } indexes { i0 = observed_hash; i1 = observed_sorted; }
                lowerings
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
            stored L at(__cq_store_L) operations observed_slice stored R
            at(__cq_store_R) operations observed_hash stored S
            at(__cq_store_S) operations observed_sorted stored r0
            at(__cq_store_r0) operations observed_slice stored r1
            at(__cq_store_r1) operations observed_hash indexed i0
            at(__cq_index_i0) operations observed_hash
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
            (observe(0, (__cq_store_r0).iter())), r1(a, b) => bind b <-
            (observe(1, (__cq_index_i0).get(a).into_iter().flatten()));
        }
    }
}

```

---

## 2172007/026-resume_physical

[Raw input](raw/2172007/026-resume_physical.input.tokens) · [Raw output](raw/2172007/026-resume_physical.output.tokens)

Actual input:

```rust
resume
{
    plan
    {
        struct PullChain; relation L(a : Value); relation
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
                L = observed_slice; R = observed_hash; S = observed_sorted; r0
                = observed_slice; r1 = observed_hash; r2 = ::
                cq_example_providers :: borrowed_fields; r3 = observed_sorted;
                r4 = :: cq_example_providers :: borrowed_fields; r5 = ::
                cq_example_providers :: borrowed_fields;
            } indexes { i0 = observed_hash; i1 = observed_sorted; } lowerings
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
        stored L at(__cq_store_L) operations observed_slice stored R
        at(__cq_store_R) operations observed_hash stored S at(__cq_store_S)
        operations observed_sorted stored r0 at(__cq_store_r0) operations
        observed_slice stored r1 at(__cq_store_r1) operations observed_hash
        indexed i0 at(__cq_index_i0) operations observed_hash
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
        r2(a, b) => ((a, b,)) : - r0(a) => bind a <-
        (observe(0, (__cq_store_r0).iter())), r1(a, b) => bind b <-
        (observe(1, (__cq_index_i0).get(a).into_iter().flatten()));
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
            struct PullChain; relation L(a : Value); relation
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
                    L = observed_slice; R = observed_hash; S = observed_sorted;
                    r0 = observed_slice; r1 = observed_hash; r2 = ::
                    cq_example_providers :: borrowed_fields; r3 =
                    observed_sorted; r4 = :: cq_example_providers ::
                    borrowed_fields; r5 = :: cq_example_providers ::
                    borrowed_fields;
                } indexes { i0 = observed_hash; i1 = observed_sorted; }
                lowerings
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
            stored L at(__cq_store_L) operations observed_slice stored R
            at(__cq_store_R) operations observed_hash stored S
            at(__cq_store_S) operations observed_sorted stored r0
            at(__cq_store_r0) operations observed_slice stored r1
            at(__cq_store_r1) operations observed_hash indexed i0
            at(__cq_index_i0) operations observed_hash region r2
            {
                r2(a, b) => ((a, b,)) : - r0(a) => bind a <-
                (observe(0, (__cq_store_r0).iter())), r1(a, b) => bind b <-
                (observe(1, (__cq_index_i0).get(a).into_iter().flatten()));
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
        struct PullChain; relation L(a : Value); relation
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
                L = observed_slice; R = observed_hash; S = observed_sorted; r0
                = observed_slice; r1 = observed_hash; r2 = ::
                cq_example_providers :: borrowed_fields; r3 = observed_sorted;
                r4 = :: cq_example_providers :: borrowed_fields; r5 = ::
                cq_example_providers :: borrowed_fields;
            } indexes { i0 = observed_hash; i1 = observed_sorted; } lowerings
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
        stored L at(__cq_store_L) operations observed_slice stored R
        at(__cq_store_R) operations observed_hash stored S at(__cq_store_S)
        operations observed_sorted stored r0 at(__cq_store_r0) operations
        observed_slice stored r1 at(__cq_store_r1) operations observed_hash
        indexed i0 at(__cq_index_i0) operations observed_hash region r2
        {
            r2(a, b) => ((a, b,)) : - r0(a) => bind a <-
            (observe(0, (__cq_store_r0).iter())), r1(a, b) => bind b <-
            (observe(1, (__cq_index_i0).get(a).into_iter().flatten()));
        }
    } request { lower r3 }
}

```

---

## 2172007/027-cq-example-providers::rename

[Raw input](raw/2172007/027-cq-example-providers::rename.input.tokens) · [Raw output](raw/2172007/027-cq-example-providers::rename.output.tokens)

Actual input:

```rust
reply_to(:: mini_linq :: resume_physical)
{
    plan
    {
        struct PullChain; relation L(a : Value); relation
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
                L = observed_slice; R = observed_hash; S = observed_sorted; r0
                = observed_slice; r1 = observed_hash; r2 = ::
                cq_example_providers :: borrowed_fields; r3 = observed_sorted;
                r4 = :: cq_example_providers :: borrowed_fields; r5 = ::
                cq_example_providers :: borrowed_fields;
            } indexes { i0 = observed_hash; i1 = observed_sorted; } lowerings
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
        stored L at(__cq_store_L) operations observed_slice stored R
        at(__cq_store_R) operations observed_hash stored S at(__cq_store_S)
        operations observed_sorted stored r0 at(__cq_store_r0) operations
        observed_slice stored r1 at(__cq_store_r1) operations observed_hash
        indexed i0 at(__cq_index_i0) operations observed_hash region r2
        {
            r2(a, b) => ((a, b,)) : - r0(a) => bind a <-
            (observe(0, (__cq_store_r0).iter())), r1(a, b) => bind b <-
            (observe(1, (__cq_index_i0).get(a).into_iter().flatten()));
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
    struct PullChain; relation L(a : Value); relation R(a : Value, b : Value);
    relation S(b : Value, c : Value); answer(a, c) : - L(a), R(a, b), S(b, c);
    relational
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
            L = observed_slice; R = observed_hash; S = observed_sorted; r0 =
            observed_slice; r1 = observed_hash; r2 = :: cq_example_providers
            :: borrowed_fields; r3 = observed_sorted; r4 = ::
            cq_example_providers :: borrowed_fields; r5 = ::
            cq_example_providers :: borrowed_fields;
        } indexes { i0 = observed_hash; i1 = observed_sorted; } lowerings
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
    stored L at(__cq_store_L) operations observed_slice stored R
    at(__cq_store_R) operations observed_hash stored S at(__cq_store_S)
    operations observed_sorted stored r0 at(__cq_store_r0) operations
    observed_slice stored r1 at(__cq_store_r1) operations observed_hash
    indexed i0 at(__cq_index_i0) operations observed_hash region r2
    {
        r2(a, b) => ((a, b,)) : - r0(a) => bind a <-
        (observe(0, (__cq_store_r0).iter())), r1(a, b) => bind b <-
        (observe(1, (__cq_index_i0).get(a).into_iter().flatten()));
    }
} request { lower r3 }
```

Actual output (formatted where Rust syntax permits):

```rust
observed_sorted! {
    reply_to(:: mini_linq :: resume_physical)
    {
        plan
        {
            struct PullChain; relation L(a : Value); relation
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
                    L = observed_slice; R = observed_hash; S = observed_sorted;
                    r0 = observed_slice; r1 = observed_hash; r2 = ::
                    cq_example_providers :: borrowed_fields; r3 =
                    observed_sorted; r4 = :: cq_example_providers ::
                    borrowed_fields; r5 = :: cq_example_providers ::
                    borrowed_fields;
                } indexes { i0 = observed_hash; i1 = observed_sorted; }
                lowerings
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
            stored L at(__cq_store_L) operations observed_slice stored R
            at(__cq_store_R) operations observed_hash stored S
            at(__cq_store_S) operations observed_sorted stored r0
            at(__cq_store_r0) operations observed_slice stored r1
            at(__cq_store_r1) operations observed_hash indexed i0
            at(__cq_index_i0) operations observed_hash region r2
            {
                r2(a, b) => ((a, b,)) : - r0(a) => bind a <-
                (observe(0, (__cq_store_r0).iter())), r1(a, b) => bind b <-
                (observe(1, (__cq_index_i0).get(a).into_iter().flatten()));
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
        struct PullChain; relation L(a : Value); relation
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
                L = observed_slice; R = observed_hash; S = observed_sorted; r0
                = observed_slice; r1 = observed_hash; r2 = ::
                cq_example_providers :: borrowed_fields; r3 = observed_sorted;
                r4 = :: cq_example_providers :: borrowed_fields; r5 = ::
                cq_example_providers :: borrowed_fields;
            } indexes { i0 = observed_hash; i1 = observed_sorted; } lowerings
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
        stored L at(__cq_store_L) operations observed_slice stored R
        at(__cq_store_R) operations observed_hash stored S at(__cq_store_S)
        operations observed_sorted stored r0 at(__cq_store_r0) operations
        observed_slice stored r1 at(__cq_store_r1) operations observed_hash
        indexed i0 at(__cq_index_i0) operations observed_hash region r2
        {
            r2(a, b) => ((a, b,)) : - r0(a) => bind a <-
            (observe(0, (__cq_store_r0).iter())), r1(a, b) => bind b <-
            (observe(1, (__cq_index_i0).get(a).into_iter().flatten()));
        }
    } request { input r3 from(__cq_store_S) }
}

```

---

## 2172007/028-cq-example-providers::sorted_relation

[Raw input](raw/2172007/028-cq-example-providers::sorted_relation.input.tokens) · [Raw output](raw/2172007/028-cq-example-providers::sorted_relation.output.tokens)

Actual input:

```rust
reply_to(observe_reply)
{
    callback(:: mini_linq :: resume_physical) kind(2) state
    {plan
        {
            struct PullChain; relation L(a : Value); relation
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
                    L = observed_slice; R = observed_hash; S = observed_sorted;
                    r0 = observed_slice; r1 = observed_hash; r2 = ::
                    cq_example_providers :: borrowed_fields; r3 =
                    observed_sorted; r4 = :: cq_example_providers ::
                    borrowed_fields; r5 = :: cq_example_providers ::
                    borrowed_fields;
                } indexes { i0 = observed_hash; i1 = observed_sorted; }
                lowerings
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
            stored L at(__cq_store_L) operations observed_slice stored R
            at(__cq_store_R) operations observed_hash stored S
            at(__cq_store_S) operations observed_sorted stored r0
            at(__cq_store_r0) operations observed_slice stored r1
            at(__cq_store_r1) operations observed_hash indexed i0
            at(__cq_index_i0) operations observed_hash region r2
            {
                r2(a, b) => ((a, b,)) : - r0(a) => bind a <-
                (observe(0, (__cq_store_r0).iter())), r1(a, b) => bind b <-
                (observe(1, (__cq_index_i0).get(a).into_iter().flatten()));
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
        continuations(:: mini_linq :: resume_physical, mini_linq :: access)}
} plan
{struct PullChain; relation L(a : Value); relation R(a : Value, b : Value);
    relation S(b : Value, c : Value); answer(a, c) : - L(a), R(a, b), S(b, c);
    relational
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
            L = observed_slice; R = observed_hash; S = observed_sorted; r0 =
            observed_slice; r1 = observed_hash; r2 = :: cq_example_providers
            :: borrowed_fields; r3 = observed_sorted; r4 = ::
            cq_example_providers :: borrowed_fields; r5 = ::
            cq_example_providers :: borrowed_fields;
        } indexes { i0 = observed_hash; i1 = observed_sorted; } lowerings
        {
            r0 = :: cq_example_providers :: rename; r1 = ::
            cq_example_providers :: rename; r2 = :: cq_example_providers ::
            indexed_join; r3 = :: cq_example_providers :: rename; r4 = ::
            cq_example_providers :: indexed_join; r5 = :: cq_example_providers
            :: project;
        }
    }} environment
{stored L at(__cq_store_L) operations observed_slice stored R at(__cq_store_R)
    operations observed_hash stored S at(__cq_store_S) operations
    observed_sorted stored r0 at(__cq_store_r0) operations observed_slice
    stored r1 at(__cq_store_r1) operations observed_hash indexed i0
    at(__cq_index_i0) operations observed_hash region r2
    {
        r2(a, b) => ((a, b,)) : - r0(a) => bind a <-
        (observe(0, (__cq_store_r0).iter())), r1(a, b) => bind b <-
        (observe(1, (__cq_index_i0).get(a).into_iter().flatten()));
    }} request {input r3 from(__cq_store_S)}
```

Actual output (formatted where Rust syntax permits):

```rust
observe_reply! {
    resume
    {
        callback(:: mini_linq :: resume_physical) kind(2) state
        {
            plan
            {
                struct PullChain; relation L(a : Value); relation
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
                        L = observed_slice; R = observed_hash; S = observed_sorted;
                        r0 = observed_slice; r1 = observed_hash; r2 = ::
                        cq_example_providers :: borrowed_fields; r3 =
                        observed_sorted; r4 = :: cq_example_providers ::
                        borrowed_fields; r5 = :: cq_example_providers ::
                        borrowed_fields;
                    } indexes { i0 = observed_hash; i1 = observed_sorted; }
                    lowerings
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
                stored L at(__cq_store_L) operations observed_slice stored R
                at(__cq_store_R) operations observed_hash stored S
                at(__cq_store_S) operations observed_sorted stored r0
                at(__cq_store_r0) operations observed_slice stored r1
                at(__cq_store_r1) operations observed_hash indexed i0
                at(__cq_index_i0) operations observed_hash region r2
                {
                    r2(a, b) => ((a, b,)) : - r0(a) => bind a <-
                    (observe(0, (__cq_store_r0).iter())), r1(a, b) => bind b <-
                    (observe(1, (__cq_index_i0).get(a).into_iter().flatten()));
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
                Value > > = __cq_store_r1;
            } pending { lower r3 }
            continuations(:: mini_linq :: resume_physical, mini_linq ::
            access)
        }
    } emitted
    {
        let __cq_store_r3 : & :: std :: collections :: BTreeMap < Value, ::
        std :: vec :: Vec < Value >> = __cq_store_S;
    } result { stored r3 at(__cq_store_r3) operations observed_sorted }
}

```

---

## 2172007/029-resume_physical

[Raw input](raw/2172007/029-resume_physical.input.tokens) · [Raw output](raw/2172007/029-resume_physical.output.tokens)

Actual input:

```rust
resume
{plan
    {
        struct PullChain; relation L(a : Value); relation
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
                L = observed_slice; R = observed_hash; S = observed_sorted; r0
                = observed_slice; r1 = observed_hash; r2 = ::
                cq_example_providers :: borrowed_fields; r3 = observed_sorted;
                r4 = :: cq_example_providers :: borrowed_fields; r5 = ::
                cq_example_providers :: borrowed_fields;
            } indexes { i0 = observed_hash; i1 = observed_sorted; } lowerings
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
        stored L at(__cq_store_L) operations observed_slice stored R
        at(__cq_store_R) operations observed_hash stored S at(__cq_store_S)
        operations observed_sorted stored r0 at(__cq_store_r0) operations
        observed_slice stored r1 at(__cq_store_r1) operations observed_hash
        indexed i0 at(__cq_index_i0) operations observed_hash region r2
        {
            r2(a, b) => ((a, b,)) : - r0(a) => bind a <-
            (observe(0, (__cq_store_r0).iter())), r1(a, b) => bind b <-
            (observe(1, (__cq_index_i0).get(a).into_iter().flatten()));
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
    continuations(:: mini_linq :: resume_physical, mini_linq :: access)}
emitted
{let __cq_store_r3 : & :: std :: collections :: BTreeMap < Value, :: std ::
    vec :: Vec < Value >> = __cq_store_S;} result
{stored r3 at(__cq_store_r3) operations observed_sorted}
```

Actual output (formatted where Rust syntax permits):

```rust
observed_sorted! {
    reply_to(:: mini_linq :: resume_physical)
    {
        plan
        {
            struct PullChain; relation L(a : Value); relation
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
                    L = observed_slice; R = observed_hash; S = observed_sorted;
                    r0 = observed_slice; r1 = observed_hash; r2 = ::
                    cq_example_providers :: borrowed_fields; r3 =
                    observed_sorted; r4 = :: cq_example_providers ::
                    borrowed_fields; r5 = :: cq_example_providers ::
                    borrowed_fields;
                } indexes { i0 = observed_hash; i1 = observed_sorted; }
                lowerings
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
            stored L at(__cq_store_L) operations observed_slice stored R
            at(__cq_store_R) operations observed_hash stored S
            at(__cq_store_S) operations observed_sorted stored r0
            at(__cq_store_r0) operations observed_slice stored r1
            at(__cq_store_r1) operations observed_hash indexed i0
            at(__cq_index_i0) operations observed_hash region r2
            {
                r2(a, b) => ((a, b,)) : - r0(a) => bind a <-
                (observe(0, (__cq_store_r0).iter())), r1(a, b) => bind b <-
                (observe(1, (__cq_index_i0).get(a).into_iter().flatten()));
            } stored r3 at(__cq_store_r3) operations observed_sorted
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
        struct PullChain; relation L(a : Value); relation
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
                L = observed_slice; R = observed_hash; S = observed_sorted; r0
                = observed_slice; r1 = observed_hash; r2 = ::
                cq_example_providers :: borrowed_fields; r3 = observed_sorted;
                r4 = :: cq_example_providers :: borrowed_fields; r5 = ::
                cq_example_providers :: borrowed_fields;
            } indexes { i0 = observed_hash; i1 = observed_sorted; } lowerings
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
        stored L at(__cq_store_L) operations observed_slice stored R
        at(__cq_store_R) operations observed_hash stored S at(__cq_store_S)
        operations observed_sorted stored r0 at(__cq_store_r0) operations
        observed_slice stored r1 at(__cq_store_r1) operations observed_hash
        indexed i0 at(__cq_index_i0) operations observed_hash region r2
        {
            r2(a, b) => ((a, b,)) : - r0(a) => bind a <-
            (observe(0, (__cq_store_r0).iter())), r1(a, b) => bind b <-
            (observe(1, (__cq_index_i0).get(a).into_iter().flatten()));
        } stored r3 at(__cq_store_r3) operations observed_sorted
    } request { index i1 }
}

```

---

## 2172007/030-cq-example-providers::sorted_relation

[Raw input](raw/2172007/030-cq-example-providers::sorted_relation.input.tokens) · [Raw output](raw/2172007/030-cq-example-providers::sorted_relation.output.tokens)

Actual input:

```rust
reply_to(observe_reply)
{
    callback(:: mini_linq :: resume_physical) kind(2) state
    {plan
        {
            struct PullChain; relation L(a : Value); relation
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
                    L = observed_slice; R = observed_hash; S = observed_sorted;
                    r0 = observed_slice; r1 = observed_hash; r2 = ::
                    cq_example_providers :: borrowed_fields; r3 =
                    observed_sorted; r4 = :: cq_example_providers ::
                    borrowed_fields; r5 = :: cq_example_providers ::
                    borrowed_fields;
                } indexes { i0 = observed_hash; i1 = observed_sorted; }
                lowerings
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
            stored L at(__cq_store_L) operations observed_slice stored R
            at(__cq_store_R) operations observed_hash stored S
            at(__cq_store_S) operations observed_sorted stored r0
            at(__cq_store_r0) operations observed_slice stored r1
            at(__cq_store_r1) operations observed_hash indexed i0
            at(__cq_index_i0) operations observed_hash region r2
            {
                r2(a, b) => ((a, b,)) : - r0(a) => bind a <-
                (observe(0, (__cq_store_r0).iter())), r1(a, b) => bind b <-
                (observe(1, (__cq_index_i0).get(a).into_iter().flatten()));
            } stored r3 at(__cq_store_r3) operations observed_sorted
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
        continuations(:: mini_linq :: resume_physical, mini_linq :: access)}
} plan
{struct PullChain; relation L(a : Value); relation R(a : Value, b : Value);
    relation S(b : Value, c : Value); answer(a, c) : - L(a), R(a, b), S(b, c);
    relational
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
            L = observed_slice; R = observed_hash; S = observed_sorted; r0 =
            observed_slice; r1 = observed_hash; r2 = :: cq_example_providers
            :: borrowed_fields; r3 = observed_sorted; r4 = ::
            cq_example_providers :: borrowed_fields; r5 = ::
            cq_example_providers :: borrowed_fields;
        } indexes { i0 = observed_hash; i1 = observed_sorted; } lowerings
        {
            r0 = :: cq_example_providers :: rename; r1 = ::
            cq_example_providers :: rename; r2 = :: cq_example_providers ::
            indexed_join; r3 = :: cq_example_providers :: rename; r4 = ::
            cq_example_providers :: indexed_join; r5 = :: cq_example_providers
            :: project;
        }
    }} environment
{stored L at(__cq_store_L) operations observed_slice stored R at(__cq_store_R)
    operations observed_hash stored S at(__cq_store_S) operations
    observed_sorted stored r0 at(__cq_store_r0) operations observed_slice
    stored r1 at(__cq_store_r1) operations observed_hash indexed i0
    at(__cq_index_i0) operations observed_hash region r2
    {
        r2(a, b) => ((a, b,)) : - r0(a) => bind a <-
        (observe(0, (__cq_store_r0).iter())), r1(a, b) => bind b <-
        (observe(1, (__cq_index_i0).get(a).into_iter().flatten()));
    } stored r3 at(__cq_store_r3) operations observed_sorted} request
{index i1}
```

Actual output (formatted where Rust syntax permits):

```rust
observe_reply! {
    resume
    {
        callback(:: mini_linq :: resume_physical) kind(2) state
        {
            plan
            {
                struct PullChain; relation L(a : Value); relation
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
                        L = observed_slice; R = observed_hash; S = observed_sorted;
                        r0 = observed_slice; r1 = observed_hash; r2 = ::
                        cq_example_providers :: borrowed_fields; r3 =
                        observed_sorted; r4 = :: cq_example_providers ::
                        borrowed_fields; r5 = :: cq_example_providers ::
                        borrowed_fields;
                    } indexes { i0 = observed_hash; i1 = observed_sorted; }
                    lowerings
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
                stored L at(__cq_store_L) operations observed_slice stored R
                at(__cq_store_R) operations observed_hash stored S
                at(__cq_store_S) operations observed_sorted stored r0
                at(__cq_store_r0) operations observed_slice stored r1
                at(__cq_store_r1) operations observed_hash indexed i0
                at(__cq_index_i0) operations observed_hash region r2
                {
                    r2(a, b) => ((a, b,)) : - r0(a) => bind a <-
                    (observe(0, (__cq_store_r0).iter())), r1(a, b) => bind b <-
                    (observe(1, (__cq_index_i0).get(a).into_iter().flatten()));
                } stored r3 at(__cq_store_r3) operations observed_sorted
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
                > > = __cq_store_S;
            } pending { index i1 }
            continuations(:: mini_linq :: resume_physical, mini_linq ::
            access)
        }
    } emitted
    {
        let __cq_index_i1 : & :: std :: collections :: BTreeMap < Value, ::
        std :: vec :: Vec < Value >> = __cq_store_r3;
    } result { indexed i1 at(__cq_index_i1) operations observed_sorted }
}

```

---

## 2172007/031-resume_physical

[Raw input](raw/2172007/031-resume_physical.input.tokens) · [Raw output](raw/2172007/031-resume_physical.output.tokens)

Actual input:

```rust
resume
{plan
    {
        struct PullChain; relation L(a : Value); relation
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
                L = observed_slice; R = observed_hash; S = observed_sorted; r0
                = observed_slice; r1 = observed_hash; r2 = ::
                cq_example_providers :: borrowed_fields; r3 = observed_sorted;
                r4 = :: cq_example_providers :: borrowed_fields; r5 = ::
                cq_example_providers :: borrowed_fields;
            } indexes { i0 = observed_hash; i1 = observed_sorted; } lowerings
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
        stored L at(__cq_store_L) operations observed_slice stored R
        at(__cq_store_R) operations observed_hash stored S at(__cq_store_S)
        operations observed_sorted stored r0 at(__cq_store_r0) operations
        observed_slice stored r1 at(__cq_store_r1) operations observed_hash
        indexed i0 at(__cq_index_i0) operations observed_hash region r2
        {
            r2(a, b) => ((a, b,)) : - r0(a) => bind a <-
            (observe(0, (__cq_store_r0).iter())), r1(a, b) => bind b <-
            (observe(1, (__cq_index_i0).get(a).into_iter().flatten()));
        } stored r3 at(__cq_store_r3) operations observed_sorted
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
    continuations(:: mini_linq :: resume_physical, mini_linq :: access)}
emitted
{let __cq_index_i1 : & :: std :: collections :: BTreeMap < Value, :: std ::
    vec :: Vec < Value >> = __cq_store_r3;} result
{indexed i1 at(__cq_index_i1) operations observed_sorted}
```

Actual output (formatted where Rust syntax permits):

```rust
::cq_example_providers::indexed_join! {
    reply_to(:: mini_linq :: resume_physical)
    {
        plan
        {
            struct PullChain; relation L(a : Value); relation
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
                    L = observed_slice; R = observed_hash; S = observed_sorted;
                    r0 = observed_slice; r1 = observed_hash; r2 = ::
                    cq_example_providers :: borrowed_fields; r3 =
                    observed_sorted; r4 = :: cq_example_providers ::
                    borrowed_fields; r5 = :: cq_example_providers ::
                    borrowed_fields;
                } indexes { i0 = observed_hash; i1 = observed_sorted; }
                lowerings
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
            stored L at(__cq_store_L) operations observed_slice stored R
            at(__cq_store_R) operations observed_hash stored S
            at(__cq_store_S) operations observed_sorted stored r0
            at(__cq_store_r0) operations observed_slice stored r1
            at(__cq_store_r1) operations observed_hash indexed i0
            at(__cq_index_i0) operations observed_hash region r2
            {
                r2(a, b) => ((a, b,)) : - r0(a) => bind a <-
                (observe(0, (__cq_store_r0).iter())), r1(a, b) => bind b <-
                (observe(1, (__cq_index_i0).get(a).into_iter().flatten()));
            } stored r3 at(__cq_store_r3) operations observed_sorted indexed
            i1 at(__cq_index_i1) operations observed_sorted
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
        struct PullChain; relation L(a : Value); relation
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
                L = observed_slice; R = observed_hash; S = observed_sorted; r0
                = observed_slice; r1 = observed_hash; r2 = ::
                cq_example_providers :: borrowed_fields; r3 = observed_sorted;
                r4 = :: cq_example_providers :: borrowed_fields; r5 = ::
                cq_example_providers :: borrowed_fields;
            } indexes { i0 = observed_hash; i1 = observed_sorted; } lowerings
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
        stored L at(__cq_store_L) operations observed_slice stored R
        at(__cq_store_R) operations observed_hash stored S at(__cq_store_S)
        operations observed_sorted stored r0 at(__cq_store_r0) operations
        observed_slice stored r1 at(__cq_store_r1) operations observed_hash
        indexed i0 at(__cq_index_i0) operations observed_hash region r2
        {
            r2(a, b) => ((a, b,)) : - r0(a) => bind a <-
            (observe(0, (__cq_store_r0).iter())), r1(a, b) => bind b <-
            (observe(1, (__cq_index_i0).get(a).into_iter().flatten()));
        } stored r3 at(__cq_store_r3) operations observed_sorted indexed i1
        at(__cq_index_i1) operations observed_sorted
    } request { lower r4 }
}

```

---

## 2172007/032-cq-example-providers::indexed_join

[Raw input](raw/2172007/032-cq-example-providers::indexed_join.input.tokens) · [Raw output](raw/2172007/032-cq-example-providers::indexed_join.output.tokens)

Actual input:

```rust
reply_to(:: mini_linq :: resume_physical)
{
    plan
    {
        struct PullChain; relation L(a : Value); relation
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
                L = observed_slice; R = observed_hash; S = observed_sorted; r0
                = observed_slice; r1 = observed_hash; r2 = ::
                cq_example_providers :: borrowed_fields; r3 = observed_sorted;
                r4 = :: cq_example_providers :: borrowed_fields; r5 = ::
                cq_example_providers :: borrowed_fields;
            } indexes { i0 = observed_hash; i1 = observed_sorted; } lowerings
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
        stored L at(__cq_store_L) operations observed_slice stored R
        at(__cq_store_R) operations observed_hash stored S at(__cq_store_S)
        operations observed_sorted stored r0 at(__cq_store_r0) operations
        observed_slice stored r1 at(__cq_store_r1) operations observed_hash
        indexed i0 at(__cq_index_i0) operations observed_hash region r2
        {
            r2(a, b) => ((a, b,)) : - r0(a) => bind a <-
            (observe(0, (__cq_store_r0).iter())), r1(a, b) => bind b <-
            (observe(1, (__cq_index_i0).get(a).into_iter().flatten()));
        } stored r3 at(__cq_store_r3) operations observed_sorted indexed i1
        at(__cq_index_i1) operations observed_sorted
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
    struct PullChain; relation L(a : Value); relation R(a : Value, b : Value);
    relation S(b : Value, c : Value); answer(a, c) : - L(a), R(a, b), S(b, c);
    relational
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
            L = observed_slice; R = observed_hash; S = observed_sorted; r0 =
            observed_slice; r1 = observed_hash; r2 = :: cq_example_providers
            :: borrowed_fields; r3 = observed_sorted; r4 = ::
            cq_example_providers :: borrowed_fields; r5 = ::
            cq_example_providers :: borrowed_fields;
        } indexes { i0 = observed_hash; i1 = observed_sorted; } lowerings
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
    stored L at(__cq_store_L) operations observed_slice stored R
    at(__cq_store_R) operations observed_hash stored S at(__cq_store_S)
    operations observed_sorted stored r0 at(__cq_store_r0) operations
    observed_slice stored r1 at(__cq_store_r1) operations observed_hash
    indexed i0 at(__cq_index_i0) operations observed_hash region r2
    {
        r2(a, b) => ((a, b,)) : - r0(a) => bind a <-
        (observe(0, (__cq_store_r0).iter())), r1(a, b) => bind b <-
        (observe(1, (__cq_index_i0).get(a).into_iter().flatten()));
    } stored r3 at(__cq_store_r3) operations observed_sorted indexed i1
    at(__cq_index_i1) operations observed_sorted
} request { lower r4 }
```

Actual output (formatted where Rust syntax permits):

```rust
observed_sorted! {
    reply_to(:: cq_example_providers :: indexed_join_matches)
    {
        request
        {
            reply_to(:: mini_linq :: resume_physical)
            {
                plan
                {
                    struct PullChain; relation L(a : Value); relation
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
                            L = observed_slice; R = observed_hash; S = observed_sorted;
                            r0 = observed_slice; r1 = observed_hash; r2 = ::
                            cq_example_providers :: borrowed_fields; r3 =
                            observed_sorted; r4 = :: cq_example_providers ::
                            borrowed_fields; r5 = :: cq_example_providers ::
                            borrowed_fields;
                        } indexes { i0 = observed_hash; i1 = observed_sorted; }
                        lowerings
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
                    stored L at(__cq_store_L) operations observed_slice stored R
                    at(__cq_store_R) operations observed_hash stored S
                    at(__cq_store_S) operations observed_sorted stored r0
                    at(__cq_store_r0) operations observed_slice stored r1
                    at(__cq_store_r1) operations observed_hash indexed i0
                    at(__cq_index_i0) operations observed_hash region r2
                    {
                        r2(a, b) => ((a, b,)) : - r0(a) => bind a <-
                        (observe(0, (__cq_store_r0).iter())), r1(a, b) => bind b <-
                        (observe(1, (__cq_index_i0).get(a).into_iter().flatten()));
                    } stored r3 at(__cq_store_r3) operations observed_sorted
                    indexed i1 at(__cq_index_i1) operations observed_sorted
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
                struct PullChain; relation L(a : Value); relation
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
                        L = observed_slice; R = observed_hash; S = observed_sorted;
                        r0 = observed_slice; r1 = observed_hash; r2 = ::
                        cq_example_providers :: borrowed_fields; r3 =
                        observed_sorted; r4 = :: cq_example_providers ::
                        borrowed_fields; r5 = :: cq_example_providers ::
                        borrowed_fields;
                    } indexes { i0 = observed_hash; i1 = observed_sorted; }
                    lowerings
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
                stored L at(__cq_store_L) operations observed_slice stored R
                at(__cq_store_R) operations observed_hash stored S
                at(__cq_store_S) operations observed_sorted stored r0
                at(__cq_store_r0) operations observed_slice stored r1
                at(__cq_store_r1) operations observed_hash indexed i0
                at(__cq_index_i0) operations observed_hash region r2
                {
                    r2(a, b) => ((a, b,)) : - r0(a) => bind a <-
                    (observe(0, (__cq_store_r0).iter())), r1(a, b) => bind b <-
                    (observe(1, (__cq_index_i0).get(a).into_iter().flatten()));
                } stored r3 at(__cq_store_r3) operations observed_sorted
                indexed i1 at(__cq_index_i1) operations observed_sorted
            } request { lower r4 }
        } access
        {
            r0(a) => bind a <- (observe(0, (__cq_store_r0).iter())), r1(a, b)
            => bind b <-
            (observe(1, (__cq_index_i0).get(a).into_iter().flatten()))
        } witness { row((a, b,)) fields { 0 = (a); 1 = (b); } }
    } plan
    {
        struct PullChain; relation L(a : Value); relation
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
                L = observed_slice; R = observed_hash; S = observed_sorted; r0
                = observed_slice; r1 = observed_hash; r2 = ::
                cq_example_providers :: borrowed_fields; r3 = observed_sorted;
                r4 = :: cq_example_providers :: borrowed_fields; r5 = ::
                cq_example_providers :: borrowed_fields;
            } indexes { i0 = observed_hash; i1 = observed_sorted; } lowerings
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
        stored L at(__cq_store_L) operations observed_slice stored R
        at(__cq_store_R) operations observed_hash stored S at(__cq_store_S)
        operations observed_sorted stored r0 at(__cq_store_r0) operations
        observed_slice stored r1 at(__cq_store_r1) operations observed_hash
        indexed i0 at(__cq_index_i0) operations observed_hash region r2
        {
            r2(a, b) => ((a, b,)) : - r0(a) => bind a <-
            (observe(0, (__cq_store_r0).iter())), r1(a, b) => bind b <-
            (observe(1, (__cq_index_i0).get(a).into_iter().flatten()));
        } stored r3 at(__cq_store_r3) operations observed_sorted indexed i1
        at(__cq_index_i1) operations observed_sorted
    } request { matches i1 values [(b)] }
}

```

---

## 2172007/033-cq-example-providers::sorted_relation

[Raw input](raw/2172007/033-cq-example-providers::sorted_relation.input.tokens) · [Raw output](raw/2172007/033-cq-example-providers::sorted_relation.output.tokens)

Actual input:

```rust
reply_to(observe_reply)
{
    callback(:: cq_example_providers :: indexed_join_matches) kind(2) state
    {request
        {
            reply_to(:: mini_linq :: resume_physical)
            {
                plan
                {
                    struct PullChain; relation L(a : Value); relation
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
                            L = observed_slice; R = observed_hash; S = observed_sorted;
                            r0 = observed_slice; r1 = observed_hash; r2 = ::
                            cq_example_providers :: borrowed_fields; r3 =
                            observed_sorted; r4 = :: cq_example_providers ::
                            borrowed_fields; r5 = :: cq_example_providers ::
                            borrowed_fields;
                        } indexes { i0 = observed_hash; i1 = observed_sorted; }
                        lowerings
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
                    stored L at(__cq_store_L) operations observed_slice stored R
                    at(__cq_store_R) operations observed_hash stored S
                    at(__cq_store_S) operations observed_sorted stored r0
                    at(__cq_store_r0) operations observed_slice stored r1
                    at(__cq_store_r1) operations observed_hash indexed i0
                    at(__cq_index_i0) operations observed_hash region r2
                    {
                        r2(a, b) => ((a, b,)) : - r0(a) => bind a <-
                        (observe(0, (__cq_store_r0).iter())), r1(a, b) => bind b <-
                        (observe(1, (__cq_index_i0).get(a).into_iter().flatten()));
                    } stored r3 at(__cq_store_r3) operations observed_sorted
                    indexed i1 at(__cq_index_i1) operations observed_sorted
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
                struct PullChain; relation L(a : Value); relation
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
                        L = observed_slice; R = observed_hash; S = observed_sorted;
                        r0 = observed_slice; r1 = observed_hash; r2 = ::
                        cq_example_providers :: borrowed_fields; r3 =
                        observed_sorted; r4 = :: cq_example_providers ::
                        borrowed_fields; r5 = :: cq_example_providers ::
                        borrowed_fields;
                    } indexes { i0 = observed_hash; i1 = observed_sorted; }
                    lowerings
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
                stored L at(__cq_store_L) operations observed_slice stored R
                at(__cq_store_R) operations observed_hash stored S
                at(__cq_store_S) operations observed_sorted stored r0
                at(__cq_store_r0) operations observed_slice stored r1
                at(__cq_store_r1) operations observed_hash indexed i0
                at(__cq_index_i0) operations observed_hash region r2
                {
                    r2(a, b) => ((a, b,)) : - r0(a) => bind a <-
                    (observe(0, (__cq_store_r0).iter())), r1(a, b) => bind b <-
                    (observe(1, (__cq_index_i0).get(a).into_iter().flatten()));
                } stored r3 at(__cq_store_r3) operations observed_sorted
                indexed i1 at(__cq_index_i1) operations observed_sorted
            } request { lower r4 }
        } access
        {
            r0(a) => bind a <- (observe(0, (__cq_store_r0).iter())), r1(a, b)
            => bind b <-
            (observe(1, (__cq_index_i0).get(a).into_iter().flatten()))
        } witness { row((a, b,)) fields { 0 = (a); 1 = (b); } }}
} plan
{struct PullChain; relation L(a : Value); relation R(a : Value, b : Value);
    relation S(b : Value, c : Value); answer(a, c) : - L(a), R(a, b), S(b, c);
    relational
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
            L = observed_slice; R = observed_hash; S = observed_sorted; r0 =
            observed_slice; r1 = observed_hash; r2 = :: cq_example_providers
            :: borrowed_fields; r3 = observed_sorted; r4 = ::
            cq_example_providers :: borrowed_fields; r5 = ::
            cq_example_providers :: borrowed_fields;
        } indexes { i0 = observed_hash; i1 = observed_sorted; } lowerings
        {
            r0 = :: cq_example_providers :: rename; r1 = ::
            cq_example_providers :: rename; r2 = :: cq_example_providers ::
            indexed_join; r3 = :: cq_example_providers :: rename; r4 = ::
            cq_example_providers :: indexed_join; r5 = :: cq_example_providers
            :: project;
        }
    }} environment
{stored L at(__cq_store_L) operations observed_slice stored R at(__cq_store_R)
    operations observed_hash stored S at(__cq_store_S) operations
    observed_sorted stored r0 at(__cq_store_r0) operations observed_slice
    stored r1 at(__cq_store_r1) operations observed_hash indexed i0
    at(__cq_index_i0) operations observed_hash region r2
    {
        r2(a, b) => ((a, b,)) : - r0(a) => bind a <-
        (observe(0, (__cq_store_r0).iter())), r1(a, b) => bind b <-
        (observe(1, (__cq_index_i0).get(a).into_iter().flatten()));
    } stored r3 at(__cq_store_r3) operations observed_sorted indexed i1
    at(__cq_index_i1) operations observed_sorted} request
{matches i1 values [(b)]}
```

Actual output (formatted where Rust syntax permits):

```rust
observe_reply! {
    resume
    {
        callback(:: cq_example_providers :: indexed_join_matches) kind(2)
        state
        {
            request
            {
                reply_to(:: mini_linq :: resume_physical)
                {
                    plan
                    {
                        struct PullChain; relation L(a : Value); relation
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
                                L = observed_slice; R = observed_hash; S = observed_sorted;
                                r0 = observed_slice; r1 = observed_hash; r2 = ::
                                cq_example_providers :: borrowed_fields; r3 =
                                observed_sorted; r4 = :: cq_example_providers ::
                                borrowed_fields; r5 = :: cq_example_providers ::
                                borrowed_fields;
                            } indexes { i0 = observed_hash; i1 = observed_sorted; }
                            lowerings
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
                        stored L at(__cq_store_L) operations observed_slice stored R
                        at(__cq_store_R) operations observed_hash stored S
                        at(__cq_store_S) operations observed_sorted stored r0
                        at(__cq_store_r0) operations observed_slice stored r1
                        at(__cq_store_r1) operations observed_hash indexed i0
                        at(__cq_index_i0) operations observed_hash region r2
                        {
                            r2(a, b) => ((a, b,)) : - r0(a) => bind a <-
                            (observe(0, (__cq_store_r0).iter())), r1(a, b) => bind b <-
                            (observe(1, (__cq_index_i0).get(a).into_iter().flatten()));
                        } stored r3 at(__cq_store_r3) operations observed_sorted
                        indexed i1 at(__cq_index_i1) operations observed_sorted
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
                    struct PullChain; relation L(a : Value); relation
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
                            L = observed_slice; R = observed_hash; S = observed_sorted;
                            r0 = observed_slice; r1 = observed_hash; r2 = ::
                            cq_example_providers :: borrowed_fields; r3 =
                            observed_sorted; r4 = :: cq_example_providers ::
                            borrowed_fields; r5 = :: cq_example_providers ::
                            borrowed_fields;
                        } indexes { i0 = observed_hash; i1 = observed_sorted; }
                        lowerings
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
                    stored L at(__cq_store_L) operations observed_slice stored R
                    at(__cq_store_R) operations observed_hash stored S
                    at(__cq_store_S) operations observed_sorted stored r0
                    at(__cq_store_r0) operations observed_slice stored r1
                    at(__cq_store_r1) operations observed_hash indexed i0
                    at(__cq_index_i0) operations observed_hash region r2
                    {
                        r2(a, b) => ((a, b,)) : - r0(a) => bind a <-
                        (observe(0, (__cq_store_r0).iter())), r1(a, b) => bind b <-
                        (observe(1, (__cq_index_i0).get(a).into_iter().flatten()));
                    } stored r3 at(__cq_store_r3) operations observed_sorted
                    indexed i1 at(__cq_index_i1) operations observed_sorted
                } request { lower r4 }
            } access
            {
                r0(a) => bind a <- (observe(0, (__cq_store_r0).iter())),
                r1(a, b) => bind b <-
                (observe(1, (__cq_index_i0).get(a).into_iter().flatten()))
            } witness { row((a, b,)) fields { 0 = (a); 1 = (b); } }
        }
    } source((__cq_index_i1).get(b).into_iter().flatten()) item(c) witness
    { row((b, c,)) fields { 0 = (b); 1 = (c); } }
}

```

---

## 2172007/034-cq-example-providers::indexed_join_matches

[Raw input](raw/2172007/034-cq-example-providers::indexed_join_matches.input.tokens) · [Raw output](raw/2172007/034-cq-example-providers::indexed_join_matches.output.tokens)

Actual input:

```rust
resume
{request
    {
        reply_to(:: mini_linq :: resume_physical)
        {
            plan
            {
                struct PullChain; relation L(a : Value); relation
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
                        L = observed_slice; R = observed_hash; S = observed_sorted;
                        r0 = observed_slice; r1 = observed_hash; r2 = ::
                        cq_example_providers :: borrowed_fields; r3 =
                        observed_sorted; r4 = :: cq_example_providers ::
                        borrowed_fields; r5 = :: cq_example_providers ::
                        borrowed_fields;
                    } indexes { i0 = observed_hash; i1 = observed_sorted; }
                    lowerings
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
                stored L at(__cq_store_L) operations observed_slice stored R
                at(__cq_store_R) operations observed_hash stored S
                at(__cq_store_S) operations observed_sorted stored r0
                at(__cq_store_r0) operations observed_slice stored r1
                at(__cq_store_r1) operations observed_hash indexed i0
                at(__cq_index_i0) operations observed_hash region r2
                {
                    r2(a, b) => ((a, b,)) : - r0(a) => bind a <-
                    (observe(0, (__cq_store_r0).iter())), r1(a, b) => bind b <-
                    (observe(1, (__cq_index_i0).get(a).into_iter().flatten()));
                } stored r3 at(__cq_store_r3) operations observed_sorted
                indexed i1 at(__cq_index_i1) operations observed_sorted
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
            struct PullChain; relation L(a : Value); relation
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
                    L = observed_slice; R = observed_hash; S = observed_sorted;
                    r0 = observed_slice; r1 = observed_hash; r2 = ::
                    cq_example_providers :: borrowed_fields; r3 =
                    observed_sorted; r4 = :: cq_example_providers ::
                    borrowed_fields; r5 = :: cq_example_providers ::
                    borrowed_fields;
                } indexes { i0 = observed_hash; i1 = observed_sorted; }
                lowerings
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
            stored L at(__cq_store_L) operations observed_slice stored R
            at(__cq_store_R) operations observed_hash stored S
            at(__cq_store_S) operations observed_sorted stored r0
            at(__cq_store_r0) operations observed_slice stored r1
            at(__cq_store_r1) operations observed_hash indexed i0
            at(__cq_index_i0) operations observed_hash region r2
            {
                r2(a, b) => ((a, b,)) : - r0(a) => bind a <-
                (observe(0, (__cq_store_r0).iter())), r1(a, b) => bind b <-
                (observe(1, (__cq_index_i0).get(a).into_iter().flatten()));
            } stored r3 at(__cq_store_r3) operations observed_sorted indexed
            i1 at(__cq_index_i1) operations observed_sorted
        } request { lower r4 }
    } access
    {
        r0(a) => bind a <- (observe(0, (__cq_store_r0).iter())), r1(a, b) =>
        bind b <- (observe(1, (__cq_index_i0).get(a).into_iter().flatten()))
    } witness { row((a, b,)) fields { 0 = (a); 1 = (b); } }}
source(observe(2, (__cq_index_i1).get(b).into_iter().flatten())) item(c)
witness {row((b, c,)) fields { 0 = (b); 1 = (c); }}
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
                    struct PullChain; relation L(a : Value); relation
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
                            L = observed_slice; R = observed_hash; S = observed_sorted;
                            r0 = observed_slice; r1 = observed_hash; r2 = ::
                            cq_example_providers :: borrowed_fields; r3 =
                            observed_sorted; r4 = :: cq_example_providers ::
                            borrowed_fields; r5 = :: cq_example_providers ::
                            borrowed_fields;
                        } indexes { i0 = observed_hash; i1 = observed_sorted; }
                        lowerings
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
                    stored L at(__cq_store_L) operations observed_slice stored R
                    at(__cq_store_R) operations observed_hash stored S
                    at(__cq_store_S) operations observed_sorted stored r0
                    at(__cq_store_r0) operations observed_slice stored r1
                    at(__cq_store_r1) operations observed_hash indexed i0
                    at(__cq_index_i0) operations observed_hash region r2
                    {
                        r2(a, b) => ((a, b,)) : - r0(a) => bind a <-
                        (observe(0, (__cq_store_r0).iter())), r1(a, b) => bind b <-
                        (observe(1, (__cq_index_i0).get(a).into_iter().flatten()));
                    } stored r3 at(__cq_store_r3) operations observed_sorted
                    indexed i1 at(__cq_index_i1) operations observed_sorted
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
                struct PullChain; relation L(a : Value); relation
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
                        L = observed_slice; R = observed_hash; S = observed_sorted;
                        r0 = observed_slice; r1 = observed_hash; r2 = ::
                        cq_example_providers :: borrowed_fields; r3 =
                        observed_sorted; r4 = :: cq_example_providers ::
                        borrowed_fields; r5 = :: cq_example_providers ::
                        borrowed_fields;
                    } indexes { i0 = observed_hash; i1 = observed_sorted; }
                    lowerings
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
                stored L at(__cq_store_L) operations observed_slice stored R
                at(__cq_store_R) operations observed_hash stored S
                at(__cq_store_S) operations observed_sorted stored r0
                at(__cq_store_r0) operations observed_slice stored r1
                at(__cq_store_r1) operations observed_hash indexed i0
                at(__cq_index_i0) operations observed_hash region r2
                {
                    r2(a, b) => ((a, b,)) : - r0(a) => bind a <-
                    (observe(0, (__cq_store_r0).iter())), r1(a, b) => bind b <-
                    (observe(1, (__cq_index_i0).get(a).into_iter().flatten()));
                } stored r3 at(__cq_store_r3) operations observed_sorted
                indexed i1 at(__cq_index_i1) operations observed_sorted
            } request { lower r4 }
        } access
        {
            r0(a) => bind a <- (observe(0, (__cq_store_r0).iter())), r1(a, b)
            => bind b <-
            (observe(1, (__cq_index_i0).get(a).into_iter().flatten())),
            r3(b, c) => bind c <-
            (observe(2, (__cq_index_i1).get(b).into_iter().flatten()))
        } witness { row((a, b,)) fields { 0 = (a); 1 = (b); } }
    } plan
    {
        struct PullChain; relation L(a : Value); relation
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
                L = observed_slice; R = observed_hash; S = observed_sorted; r0
                = observed_slice; r1 = observed_hash; r2 = ::
                cq_example_providers :: borrowed_fields; r3 = observed_sorted;
                r4 = :: cq_example_providers :: borrowed_fields; r5 = ::
                cq_example_providers :: borrowed_fields;
            } indexes { i0 = observed_hash; i1 = observed_sorted; } lowerings
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
        stored L at(__cq_store_L) operations observed_slice stored R
        at(__cq_store_R) operations observed_hash stored S at(__cq_store_S)
        operations observed_sorted stored r0 at(__cq_store_r0) operations
        observed_slice stored r1 at(__cq_store_r1) operations observed_hash
        indexed i0 at(__cq_index_i0) operations observed_hash region r2
        {
            r2(a, b) => ((a, b,)) : - r0(a) => bind a <-
            (observe(0, (__cq_store_r0).iter())), r1(a, b) => bind b <-
            (observe(1, (__cq_index_i0).get(a).into_iter().flatten()));
        } stored r3 at(__cq_store_r3) operations observed_sorted indexed i1
        at(__cq_index_i1) operations observed_sorted
    } request
    {
        output r4 fields { 0 = (a); 1 = (b); 2 = (c); } whole
        { r2 = ((a, b,)); r3 = ((b, c,)); }
    }
}

```

---

## 2172007/035-cq-example-providers::borrowed_fields

[Raw input](raw/2172007/035-cq-example-providers::borrowed_fields.input.tokens) · [Raw output](raw/2172007/035-cq-example-providers::borrowed_fields.output.tokens)

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
                struct PullChain; relation L(a : Value); relation
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
                        L = observed_slice; R = observed_hash; S = observed_sorted;
                        r0 = observed_slice; r1 = observed_hash; r2 = ::
                        cq_example_providers :: borrowed_fields; r3 =
                        observed_sorted; r4 = :: cq_example_providers ::
                        borrowed_fields; r5 = :: cq_example_providers ::
                        borrowed_fields;
                    } indexes { i0 = observed_hash; i1 = observed_sorted; }
                    lowerings
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
                stored L at(__cq_store_L) operations observed_slice stored R
                at(__cq_store_R) operations observed_hash stored S
                at(__cq_store_S) operations observed_sorted stored r0
                at(__cq_store_r0) operations observed_slice stored r1
                at(__cq_store_r1) operations observed_hash indexed i0
                at(__cq_index_i0) operations observed_hash region r2
                {
                    r2(a, b) => ((a, b,)) : - r0(a) => bind a <-
                    (observe(0, (__cq_store_r0).iter())), r1(a, b) => bind b <-
                    (observe(1, (__cq_index_i0).get(a).into_iter().flatten()));
                } stored r3 at(__cq_store_r3) operations observed_sorted
                indexed i1 at(__cq_index_i1) operations observed_sorted
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
            struct PullChain; relation L(a : Value); relation
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
                    L = observed_slice; R = observed_hash; S = observed_sorted;
                    r0 = observed_slice; r1 = observed_hash; r2 = ::
                    cq_example_providers :: borrowed_fields; r3 =
                    observed_sorted; r4 = :: cq_example_providers ::
                    borrowed_fields; r5 = :: cq_example_providers ::
                    borrowed_fields;
                } indexes { i0 = observed_hash; i1 = observed_sorted; }
                lowerings
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
            stored L at(__cq_store_L) operations observed_slice stored R
            at(__cq_store_R) operations observed_hash stored S
            at(__cq_store_S) operations observed_sorted stored r0
            at(__cq_store_r0) operations observed_slice stored r1
            at(__cq_store_r1) operations observed_hash indexed i0
            at(__cq_index_i0) operations observed_hash region r2
            {
                r2(a, b) => ((a, b,)) : - r0(a) => bind a <-
                (observe(0, (__cq_store_r0).iter())), r1(a, b) => bind b <-
                (observe(1, (__cq_index_i0).get(a).into_iter().flatten()));
            } stored r3 at(__cq_store_r3) operations observed_sorted indexed
            i1 at(__cq_index_i1) operations observed_sorted
        } request { lower r4 }
    } access
    {
        r0(a) => bind a <- (observe(0, (__cq_store_r0).iter())), r1(a, b) =>
        bind b <- (observe(1, (__cq_index_i0).get(a).into_iter().flatten())),
        r3(b, c) => bind c <-
        (observe(2, (__cq_index_i1).get(b).into_iter().flatten()))
    } witness { row((a, b,)) fields { 0 = (a); 1 = (b); } }
} plan
{
    struct PullChain; relation L(a : Value); relation R(a : Value, b : Value);
    relation S(b : Value, c : Value); answer(a, c) : - L(a), R(a, b), S(b, c);
    relational
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
            L = observed_slice; R = observed_hash; S = observed_sorted; r0 =
            observed_slice; r1 = observed_hash; r2 = :: cq_example_providers
            :: borrowed_fields; r3 = observed_sorted; r4 = ::
            cq_example_providers :: borrowed_fields; r5 = ::
            cq_example_providers :: borrowed_fields;
        } indexes { i0 = observed_hash; i1 = observed_sorted; } lowerings
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
    stored L at(__cq_store_L) operations observed_slice stored R
    at(__cq_store_R) operations observed_hash stored S at(__cq_store_S)
    operations observed_sorted stored r0 at(__cq_store_r0) operations
    observed_slice stored r1 at(__cq_store_r1) operations observed_hash
    indexed i0 at(__cq_index_i0) operations observed_hash region r2
    {
        r2(a, b) => ((a, b,)) : - r0(a) => bind a <-
        (observe(0, (__cq_store_r0).iter())), r1(a, b) => bind b <-
        (observe(1, (__cq_index_i0).get(a).into_iter().flatten()));
    } stored r3 at(__cq_store_r3) operations observed_sorted indexed i1
    at(__cq_index_i1) operations observed_sorted
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
                    struct PullChain; relation L(a : Value); relation
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
                            L = observed_slice; R = observed_hash; S = observed_sorted;
                            r0 = observed_slice; r1 = observed_hash; r2 = ::
                            cq_example_providers :: borrowed_fields; r3 =
                            observed_sorted; r4 = :: cq_example_providers ::
                            borrowed_fields; r5 = :: cq_example_providers ::
                            borrowed_fields;
                        } indexes { i0 = observed_hash; i1 = observed_sorted; }
                        lowerings
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
                    stored L at(__cq_store_L) operations observed_slice stored R
                    at(__cq_store_R) operations observed_hash stored S
                    at(__cq_store_S) operations observed_sorted stored r0
                    at(__cq_store_r0) operations observed_slice stored r1
                    at(__cq_store_r1) operations observed_hash indexed i0
                    at(__cq_index_i0) operations observed_hash region r2
                    {
                        r2(a, b) => ((a, b,)) : - r0(a) => bind a <-
                        (observe(0, (__cq_store_r0).iter())), r1(a, b) => bind b <-
                        (observe(1, (__cq_index_i0).get(a).into_iter().flatten()));
                    } stored r3 at(__cq_store_r3) operations observed_sorted
                    indexed i1 at(__cq_index_i1) operations observed_sorted
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
                struct PullChain; relation L(a : Value); relation
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
                        L = observed_slice; R = observed_hash; S = observed_sorted;
                        r0 = observed_slice; r1 = observed_hash; r2 = ::
                        cq_example_providers :: borrowed_fields; r3 =
                        observed_sorted; r4 = :: cq_example_providers ::
                        borrowed_fields; r5 = :: cq_example_providers ::
                        borrowed_fields;
                    } indexes { i0 = observed_hash; i1 = observed_sorted; }
                    lowerings
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
                stored L at(__cq_store_L) operations observed_slice stored R
                at(__cq_store_R) operations observed_hash stored S
                at(__cq_store_S) operations observed_sorted stored r0
                at(__cq_store_r0) operations observed_slice stored r1
                at(__cq_store_r1) operations observed_hash indexed i0
                at(__cq_index_i0) operations observed_hash region r2
                {
                    r2(a, b) => ((a, b,)) : - r0(a) => bind a <-
                    (observe(0, (__cq_store_r0).iter())), r1(a, b) => bind b <-
                    (observe(1, (__cq_index_i0).get(a).into_iter().flatten()));
                } stored r3 at(__cq_store_r3) operations observed_sorted
                indexed i1 at(__cq_index_i1) operations observed_sorted
            } request { lower r4 }
        } access
        {
            r0(a) => bind a <- (observe(0, (__cq_store_r0).iter())), r1(a, b)
            => bind b <-
            (observe(1, (__cq_index_i0).get(a).into_iter().flatten())),
            r3(b, c) => bind c <-
            (observe(2, (__cq_index_i1).get(b).into_iter().flatten()))
        } witness { row((a, b,)) fields { 0 = (a); 1 = (b); } }
    } witness { row((a, b, c,)) fields { 0 = (a); 1 = (b); 2 = (c); } }
}

```

---

## 2172007/036-cq-example-providers::indexed_join_output

[Raw input](raw/2172007/036-cq-example-providers::indexed_join_output.input.tokens) · [Raw output](raw/2172007/036-cq-example-providers::indexed_join_output.output.tokens)

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
                struct PullChain; relation L(a : Value); relation
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
                        L = observed_slice; R = observed_hash; S = observed_sorted;
                        r0 = observed_slice; r1 = observed_hash; r2 = ::
                        cq_example_providers :: borrowed_fields; r3 =
                        observed_sorted; r4 = :: cq_example_providers ::
                        borrowed_fields; r5 = :: cq_example_providers ::
                        borrowed_fields;
                    } indexes { i0 = observed_hash; i1 = observed_sorted; }
                    lowerings
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
                stored L at(__cq_store_L) operations observed_slice stored R
                at(__cq_store_R) operations observed_hash stored S
                at(__cq_store_S) operations observed_sorted stored r0
                at(__cq_store_r0) operations observed_slice stored r1
                at(__cq_store_r1) operations observed_hash indexed i0
                at(__cq_index_i0) operations observed_hash region r2
                {
                    r2(a, b) => ((a, b,)) : - r0(a) => bind a <-
                    (observe(0, (__cq_store_r0).iter())), r1(a, b) => bind b <-
                    (observe(1, (__cq_index_i0).get(a).into_iter().flatten()));
                } stored r3 at(__cq_store_r3) operations observed_sorted
                indexed i1 at(__cq_index_i1) operations observed_sorted
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
            struct PullChain; relation L(a : Value); relation
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
                    L = observed_slice; R = observed_hash; S = observed_sorted;
                    r0 = observed_slice; r1 = observed_hash; r2 = ::
                    cq_example_providers :: borrowed_fields; r3 =
                    observed_sorted; r4 = :: cq_example_providers ::
                    borrowed_fields; r5 = :: cq_example_providers ::
                    borrowed_fields;
                } indexes { i0 = observed_hash; i1 = observed_sorted; }
                lowerings
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
            stored L at(__cq_store_L) operations observed_slice stored R
            at(__cq_store_R) operations observed_hash stored S
            at(__cq_store_S) operations observed_sorted stored r0
            at(__cq_store_r0) operations observed_slice stored r1
            at(__cq_store_r1) operations observed_hash indexed i0
            at(__cq_index_i0) operations observed_hash region r2
            {
                r2(a, b) => ((a, b,)) : - r0(a) => bind a <-
                (observe(0, (__cq_store_r0).iter())), r1(a, b) => bind b <-
                (observe(1, (__cq_index_i0).get(a).into_iter().flatten()));
            } stored r3 at(__cq_store_r3) operations observed_sorted indexed
            i1 at(__cq_index_i1) operations observed_sorted
        } request { lower r4 }
    } access
    {
        r0(a) => bind a <- (observe(0, (__cq_store_r0).iter())), r1(a, b) =>
        bind b <- (observe(1, (__cq_index_i0).get(a).into_iter().flatten())),
        r3(b, c) => bind c <-
        (observe(2, (__cq_index_i1).get(b).into_iter().flatten()))
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
            struct PullChain; relation L(a : Value); relation
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
                    L = observed_slice; R = observed_hash; S = observed_sorted;
                    r0 = observed_slice; r1 = observed_hash; r2 = ::
                    cq_example_providers :: borrowed_fields; r3 =
                    observed_sorted; r4 = :: cq_example_providers ::
                    borrowed_fields; r5 = :: cq_example_providers ::
                    borrowed_fields;
                } indexes { i0 = observed_hash; i1 = observed_sorted; }
                lowerings
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
            stored L at(__cq_store_L) operations observed_slice stored R
            at(__cq_store_R) operations observed_hash stored S
            at(__cq_store_S) operations observed_sorted stored r0
            at(__cq_store_r0) operations observed_slice stored r1
            at(__cq_store_r1) operations observed_hash indexed i0
            at(__cq_index_i0) operations observed_hash region r2
            {
                r2(a, b) => ((a, b,)) : - r0(a) => bind a <-
                (observe(0, (__cq_store_r0).iter())), r1(a, b) => bind b <-
                (observe(1, (__cq_index_i0).get(a).into_iter().flatten()));
            } stored r3 at(__cq_store_r3) operations observed_sorted indexed
            i1 at(__cq_index_i1) operations observed_sorted
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
            (observe(0, (__cq_store_r0).iter())), r1(a, b) => bind b <-
            (observe(1, (__cq_index_i0).get(a).into_iter().flatten())),
            r3(b, c) => bind c <-
            (observe(2, (__cq_index_i1).get(b).into_iter().flatten()));
        }
    }
}

```

---

## 2172007/037-resume_physical

[Raw input](raw/2172007/037-resume_physical.input.tokens) · [Raw output](raw/2172007/037-resume_physical.output.tokens)

Actual input:

```rust
resume
{
    plan
    {
        struct PullChain; relation L(a : Value); relation
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
                L = observed_slice; R = observed_hash; S = observed_sorted; r0
                = observed_slice; r1 = observed_hash; r2 = ::
                cq_example_providers :: borrowed_fields; r3 = observed_sorted;
                r4 = :: cq_example_providers :: borrowed_fields; r5 = ::
                cq_example_providers :: borrowed_fields;
            } indexes { i0 = observed_hash; i1 = observed_sorted; } lowerings
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
        stored L at(__cq_store_L) operations observed_slice stored R
        at(__cq_store_R) operations observed_hash stored S at(__cq_store_S)
        operations observed_sorted stored r0 at(__cq_store_r0) operations
        observed_slice stored r1 at(__cq_store_r1) operations observed_hash
        indexed i0 at(__cq_index_i0) operations observed_hash region r2
        {
            r2(a, b) => ((a, b,)) : - r0(a) => bind a <-
            (observe(0, (__cq_store_r0).iter())), r1(a, b) => bind b <-
            (observe(1, (__cq_index_i0).get(a).into_iter().flatten()));
        } stored r3 at(__cq_store_r3) operations observed_sorted indexed i1
        at(__cq_index_i1) operations observed_sorted
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
        (observe(0, (__cq_store_r0).iter())), r1(a, b) => bind b <-
        (observe(1, (__cq_index_i0).get(a).into_iter().flatten())), r3(b, c)
        => bind c <-
        (observe(2, (__cq_index_i1).get(b).into_iter().flatten()));
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
            struct PullChain; relation L(a : Value); relation
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
                    L = observed_slice; R = observed_hash; S = observed_sorted;
                    r0 = observed_slice; r1 = observed_hash; r2 = ::
                    cq_example_providers :: borrowed_fields; r3 =
                    observed_sorted; r4 = :: cq_example_providers ::
                    borrowed_fields; r5 = :: cq_example_providers ::
                    borrowed_fields;
                } indexes { i0 = observed_hash; i1 = observed_sorted; }
                lowerings
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
            stored L at(__cq_store_L) operations observed_slice stored R
            at(__cq_store_R) operations observed_hash stored S
            at(__cq_store_S) operations observed_sorted stored r0
            at(__cq_store_r0) operations observed_slice stored r1
            at(__cq_store_r1) operations observed_hash indexed i0
            at(__cq_index_i0) operations observed_hash region r2
            {
                r2(a, b) => ((a, b,)) : - r0(a) => bind a <-
                (observe(0, (__cq_store_r0).iter())), r1(a, b) => bind b <-
                (observe(1, (__cq_index_i0).get(a).into_iter().flatten()));
            } stored r3 at(__cq_store_r3) operations observed_sorted indexed
            i1 at(__cq_index_i1) operations observed_sorted region r4
            {
                r4(a, b, c) => ((a, b, c,)) : - r0(a) => bind a <-
                (observe(0, (__cq_store_r0).iter())), r1(a, b) => bind b <-
                (observe(1, (__cq_index_i0).get(a).into_iter().flatten())),
                r3(b, c) => bind c <-
                (observe(2, (__cq_index_i1).get(b).into_iter().flatten()));
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
        struct PullChain; relation L(a : Value); relation
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
                L = observed_slice; R = observed_hash; S = observed_sorted; r0
                = observed_slice; r1 = observed_hash; r2 = ::
                cq_example_providers :: borrowed_fields; r3 = observed_sorted;
                r4 = :: cq_example_providers :: borrowed_fields; r5 = ::
                cq_example_providers :: borrowed_fields;
            } indexes { i0 = observed_hash; i1 = observed_sorted; } lowerings
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
        stored L at(__cq_store_L) operations observed_slice stored R
        at(__cq_store_R) operations observed_hash stored S at(__cq_store_S)
        operations observed_sorted stored r0 at(__cq_store_r0) operations
        observed_slice stored r1 at(__cq_store_r1) operations observed_hash
        indexed i0 at(__cq_index_i0) operations observed_hash region r2
        {
            r2(a, b) => ((a, b,)) : - r0(a) => bind a <-
            (observe(0, (__cq_store_r0).iter())), r1(a, b) => bind b <-
            (observe(1, (__cq_index_i0).get(a).into_iter().flatten()));
        } stored r3 at(__cq_store_r3) operations observed_sorted indexed i1
        at(__cq_index_i1) operations observed_sorted region r4
        {
            r4(a, b, c) => ((a, b, c,)) : - r0(a) => bind a <-
            (observe(0, (__cq_store_r0).iter())), r1(a, b) => bind b <-
            (observe(1, (__cq_index_i0).get(a).into_iter().flatten())),
            r3(b, c) => bind c <-
            (observe(2, (__cq_index_i1).get(b).into_iter().flatten()));
        }
    } request { lower r5 }
}

```

---

## 2172007/038-cq-example-providers::project

[Raw input](raw/2172007/038-cq-example-providers::project.input.tokens) · [Raw output](raw/2172007/038-cq-example-providers::project.output.tokens)

Actual input:

```rust
reply_to(:: mini_linq :: resume_physical)
{
    plan
    {
        struct PullChain; relation L(a : Value); relation
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
                L = observed_slice; R = observed_hash; S = observed_sorted; r0
                = observed_slice; r1 = observed_hash; r2 = ::
                cq_example_providers :: borrowed_fields; r3 = observed_sorted;
                r4 = :: cq_example_providers :: borrowed_fields; r5 = ::
                cq_example_providers :: borrowed_fields;
            } indexes { i0 = observed_hash; i1 = observed_sorted; } lowerings
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
        stored L at(__cq_store_L) operations observed_slice stored R
        at(__cq_store_R) operations observed_hash stored S at(__cq_store_S)
        operations observed_sorted stored r0 at(__cq_store_r0) operations
        observed_slice stored r1 at(__cq_store_r1) operations observed_hash
        indexed i0 at(__cq_index_i0) operations observed_hash region r2
        {
            r2(a, b) => ((a, b,)) : - r0(a) => bind a <-
            (observe(0, (__cq_store_r0).iter())), r1(a, b) => bind b <-
            (observe(1, (__cq_index_i0).get(a).into_iter().flatten()));
        } stored r3 at(__cq_store_r3) operations observed_sorted indexed i1
        at(__cq_index_i1) operations observed_sorted region r4
        {
            r4(a, b, c) => ((a, b, c,)) : - r0(a) => bind a <-
            (observe(0, (__cq_store_r0).iter())), r1(a, b) => bind b <-
            (observe(1, (__cq_index_i0).get(a).into_iter().flatten())),
            r3(b, c) => bind c <-
            (observe(2, (__cq_index_i1).get(b).into_iter().flatten()));
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
    struct PullChain; relation L(a : Value); relation R(a : Value, b : Value);
    relation S(b : Value, c : Value); answer(a, c) : - L(a), R(a, b), S(b, c);
    relational
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
            L = observed_slice; R = observed_hash; S = observed_sorted; r0 =
            observed_slice; r1 = observed_hash; r2 = :: cq_example_providers
            :: borrowed_fields; r3 = observed_sorted; r4 = ::
            cq_example_providers :: borrowed_fields; r5 = ::
            cq_example_providers :: borrowed_fields;
        } indexes { i0 = observed_hash; i1 = observed_sorted; } lowerings
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
    stored L at(__cq_store_L) operations observed_slice stored R
    at(__cq_store_R) operations observed_hash stored S at(__cq_store_S)
    operations observed_sorted stored r0 at(__cq_store_r0) operations
    observed_slice stored r1 at(__cq_store_r1) operations observed_hash
    indexed i0 at(__cq_index_i0) operations observed_hash region r2
    {
        r2(a, b) => ((a, b,)) : - r0(a) => bind a <-
        (observe(0, (__cq_store_r0).iter())), r1(a, b) => bind b <-
        (observe(1, (__cq_index_i0).get(a).into_iter().flatten()));
    } stored r3 at(__cq_store_r3) operations observed_sorted indexed i1
    at(__cq_index_i1) operations observed_sorted region r4
    {
        r4(a, b, c) => ((a, b, c,)) : - r0(a) => bind a <-
        (observe(0, (__cq_store_r0).iter())), r1(a, b) => bind b <-
        (observe(1, (__cq_index_i0).get(a).into_iter().flatten())), r3(b, c)
        => bind c <-
        (observe(2, (__cq_index_i1).get(b).into_iter().flatten()));
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
                    struct PullChain; relation L(a : Value); relation
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
                            L = observed_slice; R = observed_hash; S = observed_sorted;
                            r0 = observed_slice; r1 = observed_hash; r2 = ::
                            cq_example_providers :: borrowed_fields; r3 =
                            observed_sorted; r4 = :: cq_example_providers ::
                            borrowed_fields; r5 = :: cq_example_providers ::
                            borrowed_fields;
                        } indexes { i0 = observed_hash; i1 = observed_sorted; }
                        lowerings
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
                    stored L at(__cq_store_L) operations observed_slice stored R
                    at(__cq_store_R) operations observed_hash stored S
                    at(__cq_store_S) operations observed_sorted stored r0
                    at(__cq_store_r0) operations observed_slice stored r1
                    at(__cq_store_r1) operations observed_hash indexed i0
                    at(__cq_index_i0) operations observed_hash region r2
                    {
                        r2(a, b) => ((a, b,)) : - r0(a) => bind a <-
                        (observe(0, (__cq_store_r0).iter())), r1(a, b) => bind b <-
                        (observe(1, (__cq_index_i0).get(a).into_iter().flatten()));
                    } stored r3 at(__cq_store_r3) operations observed_sorted
                    indexed i1 at(__cq_index_i1) operations observed_sorted
                    region r4
                    {
                        r4(a, b, c) => ((a, b, c,)) : - r0(a) => bind a <-
                        (observe(0, (__cq_store_r0).iter())), r1(a, b) => bind b <-
                        (observe(1, (__cq_index_i0).get(a).into_iter().flatten())),
                        r3(b, c) => bind c <-
                        (observe(2, (__cq_index_i1).get(b).into_iter().flatten()));
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
                struct PullChain; relation L(a : Value); relation
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
                        L = observed_slice; R = observed_hash; S = observed_sorted;
                        r0 = observed_slice; r1 = observed_hash; r2 = ::
                        cq_example_providers :: borrowed_fields; r3 =
                        observed_sorted; r4 = :: cq_example_providers ::
                        borrowed_fields; r5 = :: cq_example_providers ::
                        borrowed_fields;
                    } indexes { i0 = observed_hash; i1 = observed_sorted; }
                    lowerings
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
                stored L at(__cq_store_L) operations observed_slice stored R
                at(__cq_store_R) operations observed_hash stored S
                at(__cq_store_S) operations observed_sorted stored r0
                at(__cq_store_r0) operations observed_slice stored r1
                at(__cq_store_r1) operations observed_hash indexed i0
                at(__cq_index_i0) operations observed_hash region r2
                {
                    r2(a, b) => ((a, b,)) : - r0(a) => bind a <-
                    (observe(0, (__cq_store_r0).iter())), r1(a, b) => bind b <-
                    (observe(1, (__cq_index_i0).get(a).into_iter().flatten()));
                } stored r3 at(__cq_store_r3) operations observed_sorted
                indexed i1 at(__cq_index_i1) operations observed_sorted region
                r4
                {
                    r4(a, b, c) => ((a, b, c,)) : - r0(a) => bind a <-
                    (observe(0, (__cq_store_r0).iter())), r1(a, b) => bind b <-
                    (observe(1, (__cq_index_i0).get(a).into_iter().flatten())),
                    r3(b, c) => bind c <-
                    (observe(2, (__cq_index_i1).get(b).into_iter().flatten()));
                }
            } request { lower r5 }
        } access
        {
            r0(a) => bind a <- (observe(0, (__cq_store_r0).iter())), r1(a, b)
            => bind b <-
            (observe(1, (__cq_index_i0).get(a).into_iter().flatten())),
            r3(b, c) => bind c <-
            (observe(2, (__cq_index_i1).get(b).into_iter().flatten()))
        } witness { row((a, b, c,)) fields { 0 = (a); 1 = (b); 2 = (c); } }
    } plan
    {
        struct PullChain; relation L(a : Value); relation
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
                L = observed_slice; R = observed_hash; S = observed_sorted; r0
                = observed_slice; r1 = observed_hash; r2 = ::
                cq_example_providers :: borrowed_fields; r3 = observed_sorted;
                r4 = :: cq_example_providers :: borrowed_fields; r5 = ::
                cq_example_providers :: borrowed_fields;
            } indexes { i0 = observed_hash; i1 = observed_sorted; } lowerings
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
        stored L at(__cq_store_L) operations observed_slice stored R
        at(__cq_store_R) operations observed_hash stored S at(__cq_store_S)
        operations observed_sorted stored r0 at(__cq_store_r0) operations
        observed_slice stored r1 at(__cq_store_r1) operations observed_hash
        indexed i0 at(__cq_index_i0) operations observed_hash region r2
        {
            r2(a, b) => ((a, b,)) : - r0(a) => bind a <-
            (observe(0, (__cq_store_r0).iter())), r1(a, b) => bind b <-
            (observe(1, (__cq_index_i0).get(a).into_iter().flatten()));
        } stored r3 at(__cq_store_r3) operations observed_sorted indexed i1
        at(__cq_index_i1) operations observed_sorted region r4
        {
            r4(a, b, c) => ((a, b, c,)) : - r0(a) => bind a <-
            (observe(0, (__cq_store_r0).iter())), r1(a, b) => bind b <-
            (observe(1, (__cq_index_i0).get(a).into_iter().flatten())),
            r3(b, c) => bind c <-
            (observe(2, (__cq_index_i1).get(b).into_iter().flatten()));
        }
    } request
    { output r5 fields { 0 = (a); 1 = (c); } whole { r4 = ((a, b, c,)); } }
}

```

---

## 2172007/039-cq-example-providers::borrowed_fields

[Raw input](raw/2172007/039-cq-example-providers::borrowed_fields.input.tokens) · [Raw output](raw/2172007/039-cq-example-providers::borrowed_fields.output.tokens)

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
                struct PullChain; relation L(a : Value); relation
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
                        L = observed_slice; R = observed_hash; S = observed_sorted;
                        r0 = observed_slice; r1 = observed_hash; r2 = ::
                        cq_example_providers :: borrowed_fields; r3 =
                        observed_sorted; r4 = :: cq_example_providers ::
                        borrowed_fields; r5 = :: cq_example_providers ::
                        borrowed_fields;
                    } indexes { i0 = observed_hash; i1 = observed_sorted; }
                    lowerings
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
                stored L at(__cq_store_L) operations observed_slice stored R
                at(__cq_store_R) operations observed_hash stored S
                at(__cq_store_S) operations observed_sorted stored r0
                at(__cq_store_r0) operations observed_slice stored r1
                at(__cq_store_r1) operations observed_hash indexed i0
                at(__cq_index_i0) operations observed_hash region r2
                {
                    r2(a, b) => ((a, b,)) : - r0(a) => bind a <-
                    (observe(0, (__cq_store_r0).iter())), r1(a, b) => bind b <-
                    (observe(1, (__cq_index_i0).get(a).into_iter().flatten()));
                } stored r3 at(__cq_store_r3) operations observed_sorted
                indexed i1 at(__cq_index_i1) operations observed_sorted region
                r4
                {
                    r4(a, b, c) => ((a, b, c,)) : - r0(a) => bind a <-
                    (observe(0, (__cq_store_r0).iter())), r1(a, b) => bind b <-
                    (observe(1, (__cq_index_i0).get(a).into_iter().flatten())),
                    r3(b, c) => bind c <-
                    (observe(2, (__cq_index_i1).get(b).into_iter().flatten()));
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
            struct PullChain; relation L(a : Value); relation
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
                    L = observed_slice; R = observed_hash; S = observed_sorted;
                    r0 = observed_slice; r1 = observed_hash; r2 = ::
                    cq_example_providers :: borrowed_fields; r3 =
                    observed_sorted; r4 = :: cq_example_providers ::
                    borrowed_fields; r5 = :: cq_example_providers ::
                    borrowed_fields;
                } indexes { i0 = observed_hash; i1 = observed_sorted; }
                lowerings
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
            stored L at(__cq_store_L) operations observed_slice stored R
            at(__cq_store_R) operations observed_hash stored S
            at(__cq_store_S) operations observed_sorted stored r0
            at(__cq_store_r0) operations observed_slice stored r1
            at(__cq_store_r1) operations observed_hash indexed i0
            at(__cq_index_i0) operations observed_hash region r2
            {
                r2(a, b) => ((a, b,)) : - r0(a) => bind a <-
                (observe(0, (__cq_store_r0).iter())), r1(a, b) => bind b <-
                (observe(1, (__cq_index_i0).get(a).into_iter().flatten()));
            } stored r3 at(__cq_store_r3) operations observed_sorted indexed
            i1 at(__cq_index_i1) operations observed_sorted region r4
            {
                r4(a, b, c) => ((a, b, c,)) : - r0(a) => bind a <-
                (observe(0, (__cq_store_r0).iter())), r1(a, b) => bind b <-
                (observe(1, (__cq_index_i0).get(a).into_iter().flatten())),
                r3(b, c) => bind c <-
                (observe(2, (__cq_index_i1).get(b).into_iter().flatten()));
            }
        } request { lower r5 }
    } access
    {
        r0(a) => bind a <- (observe(0, (__cq_store_r0).iter())), r1(a, b) =>
        bind b <- (observe(1, (__cq_index_i0).get(a).into_iter().flatten())),
        r3(b, c) => bind c <-
        (observe(2, (__cq_index_i1).get(b).into_iter().flatten()))
    } witness { row((a, b, c,)) fields { 0 = (a); 1 = (b); 2 = (c); } }
} plan
{
    struct PullChain; relation L(a : Value); relation R(a : Value, b : Value);
    relation S(b : Value, c : Value); answer(a, c) : - L(a), R(a, b), S(b, c);
    relational
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
            L = observed_slice; R = observed_hash; S = observed_sorted; r0 =
            observed_slice; r1 = observed_hash; r2 = :: cq_example_providers
            :: borrowed_fields; r3 = observed_sorted; r4 = ::
            cq_example_providers :: borrowed_fields; r5 = ::
            cq_example_providers :: borrowed_fields;
        } indexes { i0 = observed_hash; i1 = observed_sorted; } lowerings
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
    stored L at(__cq_store_L) operations observed_slice stored R
    at(__cq_store_R) operations observed_hash stored S at(__cq_store_S)
    operations observed_sorted stored r0 at(__cq_store_r0) operations
    observed_slice stored r1 at(__cq_store_r1) operations observed_hash
    indexed i0 at(__cq_index_i0) operations observed_hash region r2
    {
        r2(a, b) => ((a, b,)) : - r0(a) => bind a <-
        (observe(0, (__cq_store_r0).iter())), r1(a, b) => bind b <-
        (observe(1, (__cq_index_i0).get(a).into_iter().flatten()));
    } stored r3 at(__cq_store_r3) operations observed_sorted indexed i1
    at(__cq_index_i1) operations observed_sorted region r4
    {
        r4(a, b, c) => ((a, b, c,)) : - r0(a) => bind a <-
        (observe(0, (__cq_store_r0).iter())), r1(a, b) => bind b <-
        (observe(1, (__cq_index_i0).get(a).into_iter().flatten())), r3(b, c)
        => bind c <-
        (observe(2, (__cq_index_i1).get(b).into_iter().flatten()));
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
                    struct PullChain; relation L(a : Value); relation
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
                            L = observed_slice; R = observed_hash; S = observed_sorted;
                            r0 = observed_slice; r1 = observed_hash; r2 = ::
                            cq_example_providers :: borrowed_fields; r3 =
                            observed_sorted; r4 = :: cq_example_providers ::
                            borrowed_fields; r5 = :: cq_example_providers ::
                            borrowed_fields;
                        } indexes { i0 = observed_hash; i1 = observed_sorted; }
                        lowerings
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
                    stored L at(__cq_store_L) operations observed_slice stored R
                    at(__cq_store_R) operations observed_hash stored S
                    at(__cq_store_S) operations observed_sorted stored r0
                    at(__cq_store_r0) operations observed_slice stored r1
                    at(__cq_store_r1) operations observed_hash indexed i0
                    at(__cq_index_i0) operations observed_hash region r2
                    {
                        r2(a, b) => ((a, b,)) : - r0(a) => bind a <-
                        (observe(0, (__cq_store_r0).iter())), r1(a, b) => bind b <-
                        (observe(1, (__cq_index_i0).get(a).into_iter().flatten()));
                    } stored r3 at(__cq_store_r3) operations observed_sorted
                    indexed i1 at(__cq_index_i1) operations observed_sorted
                    region r4
                    {
                        r4(a, b, c) => ((a, b, c,)) : - r0(a) => bind a <-
                        (observe(0, (__cq_store_r0).iter())), r1(a, b) => bind b <-
                        (observe(1, (__cq_index_i0).get(a).into_iter().flatten())),
                        r3(b, c) => bind c <-
                        (observe(2, (__cq_index_i1).get(b).into_iter().flatten()));
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
                struct PullChain; relation L(a : Value); relation
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
                        L = observed_slice; R = observed_hash; S = observed_sorted;
                        r0 = observed_slice; r1 = observed_hash; r2 = ::
                        cq_example_providers :: borrowed_fields; r3 =
                        observed_sorted; r4 = :: cq_example_providers ::
                        borrowed_fields; r5 = :: cq_example_providers ::
                        borrowed_fields;
                    } indexes { i0 = observed_hash; i1 = observed_sorted; }
                    lowerings
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
                stored L at(__cq_store_L) operations observed_slice stored R
                at(__cq_store_R) operations observed_hash stored S
                at(__cq_store_S) operations observed_sorted stored r0
                at(__cq_store_r0) operations observed_slice stored r1
                at(__cq_store_r1) operations observed_hash indexed i0
                at(__cq_index_i0) operations observed_hash region r2
                {
                    r2(a, b) => ((a, b,)) : - r0(a) => bind a <-
                    (observe(0, (__cq_store_r0).iter())), r1(a, b) => bind b <-
                    (observe(1, (__cq_index_i0).get(a).into_iter().flatten()));
                } stored r3 at(__cq_store_r3) operations observed_sorted
                indexed i1 at(__cq_index_i1) operations observed_sorted region
                r4
                {
                    r4(a, b, c) => ((a, b, c,)) : - r0(a) => bind a <-
                    (observe(0, (__cq_store_r0).iter())), r1(a, b) => bind b <-
                    (observe(1, (__cq_index_i0).get(a).into_iter().flatten())),
                    r3(b, c) => bind c <-
                    (observe(2, (__cq_index_i1).get(b).into_iter().flatten()));
                }
            } request { lower r5 }
        } access
        {
            r0(a) => bind a <- (observe(0, (__cq_store_r0).iter())), r1(a, b)
            => bind b <-
            (observe(1, (__cq_index_i0).get(a).into_iter().flatten())),
            r3(b, c) => bind c <-
            (observe(2, (__cq_index_i1).get(b).into_iter().flatten()))
        } witness { row((a, b, c,)) fields { 0 = (a); 1 = (b); 2 = (c); } }
    } witness { row((a, c,)) fields { 0 = (a); 1 = (c); } }
}

```

---

## 2172007/040-cq-example-providers::project_output

[Raw input](raw/2172007/040-cq-example-providers::project_output.input.tokens) · [Raw output](raw/2172007/040-cq-example-providers::project_output.output.tokens)

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
                struct PullChain; relation L(a : Value); relation
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
                        L = observed_slice; R = observed_hash; S = observed_sorted;
                        r0 = observed_slice; r1 = observed_hash; r2 = ::
                        cq_example_providers :: borrowed_fields; r3 =
                        observed_sorted; r4 = :: cq_example_providers ::
                        borrowed_fields; r5 = :: cq_example_providers ::
                        borrowed_fields;
                    } indexes { i0 = observed_hash; i1 = observed_sorted; }
                    lowerings
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
                stored L at(__cq_store_L) operations observed_slice stored R
                at(__cq_store_R) operations observed_hash stored S
                at(__cq_store_S) operations observed_sorted stored r0
                at(__cq_store_r0) operations observed_slice stored r1
                at(__cq_store_r1) operations observed_hash indexed i0
                at(__cq_index_i0) operations observed_hash region r2
                {
                    r2(a, b) => ((a, b,)) : - r0(a) => bind a <-
                    (observe(0, (__cq_store_r0).iter())), r1(a, b) => bind b <-
                    (observe(1, (__cq_index_i0).get(a).into_iter().flatten()));
                } stored r3 at(__cq_store_r3) operations observed_sorted
                indexed i1 at(__cq_index_i1) operations observed_sorted region
                r4
                {
                    r4(a, b, c) => ((a, b, c,)) : - r0(a) => bind a <-
                    (observe(0, (__cq_store_r0).iter())), r1(a, b) => bind b <-
                    (observe(1, (__cq_index_i0).get(a).into_iter().flatten())),
                    r3(b, c) => bind c <-
                    (observe(2, (__cq_index_i1).get(b).into_iter().flatten()));
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
            struct PullChain; relation L(a : Value); relation
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
                    L = observed_slice; R = observed_hash; S = observed_sorted;
                    r0 = observed_slice; r1 = observed_hash; r2 = ::
                    cq_example_providers :: borrowed_fields; r3 =
                    observed_sorted; r4 = :: cq_example_providers ::
                    borrowed_fields; r5 = :: cq_example_providers ::
                    borrowed_fields;
                } indexes { i0 = observed_hash; i1 = observed_sorted; }
                lowerings
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
            stored L at(__cq_store_L) operations observed_slice stored R
            at(__cq_store_R) operations observed_hash stored S
            at(__cq_store_S) operations observed_sorted stored r0
            at(__cq_store_r0) operations observed_slice stored r1
            at(__cq_store_r1) operations observed_hash indexed i0
            at(__cq_index_i0) operations observed_hash region r2
            {
                r2(a, b) => ((a, b,)) : - r0(a) => bind a <-
                (observe(0, (__cq_store_r0).iter())), r1(a, b) => bind b <-
                (observe(1, (__cq_index_i0).get(a).into_iter().flatten()));
            } stored r3 at(__cq_store_r3) operations observed_sorted indexed
            i1 at(__cq_index_i1) operations observed_sorted region r4
            {
                r4(a, b, c) => ((a, b, c,)) : - r0(a) => bind a <-
                (observe(0, (__cq_store_r0).iter())), r1(a, b) => bind b <-
                (observe(1, (__cq_index_i0).get(a).into_iter().flatten())),
                r3(b, c) => bind c <-
                (observe(2, (__cq_index_i1).get(b).into_iter().flatten()));
            }
        } request { lower r5 }
    } access
    {
        r0(a) => bind a <- (observe(0, (__cq_store_r0).iter())), r1(a, b) =>
        bind b <- (observe(1, (__cq_index_i0).get(a).into_iter().flatten())),
        r3(b, c) => bind c <-
        (observe(2, (__cq_index_i1).get(b).into_iter().flatten()))
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
            struct PullChain; relation L(a : Value); relation
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
                    L = observed_slice; R = observed_hash; S = observed_sorted;
                    r0 = observed_slice; r1 = observed_hash; r2 = ::
                    cq_example_providers :: borrowed_fields; r3 =
                    observed_sorted; r4 = :: cq_example_providers ::
                    borrowed_fields; r5 = :: cq_example_providers ::
                    borrowed_fields;
                } indexes { i0 = observed_hash; i1 = observed_sorted; }
                lowerings
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
            stored L at(__cq_store_L) operations observed_slice stored R
            at(__cq_store_R) operations observed_hash stored S
            at(__cq_store_S) operations observed_sorted stored r0
            at(__cq_store_r0) operations observed_slice stored r1
            at(__cq_store_r1) operations observed_hash indexed i0
            at(__cq_index_i0) operations observed_hash region r2
            {
                r2(a, b) => ((a, b,)) : - r0(a) => bind a <-
                (observe(0, (__cq_store_r0).iter())), r1(a, b) => bind b <-
                (observe(1, (__cq_index_i0).get(a).into_iter().flatten()));
            } stored r3 at(__cq_store_r3) operations observed_sorted indexed
            i1 at(__cq_index_i1) operations observed_sorted region r4
            {
                r4(a, b, c) => ((a, b, c,)) : - r0(a) => bind a <-
                (observe(0, (__cq_store_r0).iter())), r1(a, b) => bind b <-
                (observe(1, (__cq_index_i0).get(a).into_iter().flatten())),
                r3(b, c) => bind c <-
                (observe(2, (__cq_index_i1).get(b).into_iter().flatten()));
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
            (observe(0, (__cq_store_r0).iter())), r1(a, b) => bind b <-
            (observe(1, (__cq_index_i0).get(a).into_iter().flatten())),
            r3(b, c) => bind c <-
            (observe(2, (__cq_index_i1).get(b).into_iter().flatten()));
        }
    }
}

```

---

## 2172007/041-resume_physical

[Raw input](raw/2172007/041-resume_physical.input.tokens) · [Raw output](raw/2172007/041-resume_physical.output.tokens)

Actual input:

```rust
resume
{
    plan
    {
        struct PullChain; relation L(a : Value); relation
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
                L = observed_slice; R = observed_hash; S = observed_sorted; r0
                = observed_slice; r1 = observed_hash; r2 = ::
                cq_example_providers :: borrowed_fields; r3 = observed_sorted;
                r4 = :: cq_example_providers :: borrowed_fields; r5 = ::
                cq_example_providers :: borrowed_fields;
            } indexes { i0 = observed_hash; i1 = observed_sorted; } lowerings
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
        stored L at(__cq_store_L) operations observed_slice stored R
        at(__cq_store_R) operations observed_hash stored S at(__cq_store_S)
        operations observed_sorted stored r0 at(__cq_store_r0) operations
        observed_slice stored r1 at(__cq_store_r1) operations observed_hash
        indexed i0 at(__cq_index_i0) operations observed_hash region r2
        {
            r2(a, b) => ((a, b,)) : - r0(a) => bind a <-
            (observe(0, (__cq_store_r0).iter())), r1(a, b) => bind b <-
            (observe(1, (__cq_index_i0).get(a).into_iter().flatten()));
        } stored r3 at(__cq_store_r3) operations observed_sorted indexed i1
        at(__cq_index_i1) operations observed_sorted region r4
        {
            r4(a, b, c) => ((a, b, c,)) : - r0(a) => bind a <-
            (observe(0, (__cq_store_r0).iter())), r1(a, b) => bind b <-
            (observe(1, (__cq_index_i0).get(a).into_iter().flatten())),
            r3(b, c) => bind c <-
            (observe(2, (__cq_index_i1).get(b).into_iter().flatten()));
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
        r5(a, c) => ((a, c,)) : - r0(a) => bind a <-
        (observe(0, (__cq_store_r0).iter())), r1(a, b) => bind b <-
        (observe(1, (__cq_index_i0).get(a).into_iter().flatten())), r3(b, c)
        => bind c <-
        (observe(2, (__cq_index_i1).get(b).into_iter().flatten()));
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
        (observe(0, (__cq_store_r0).iter())), r1(a, b) => bind b <-
        (observe(1, (__cq_index_i0).get(a).into_iter().flatten())), r3(b, c)
        => bind c <-
        (observe(2, (__cq_index_i1).get(b).into_iter().flatten()));
    }
}
```

---

## 2172007/042-access_to_distinct_iter

[Raw input](raw/2172007/042-access_to_distinct_iter.input.tokens) · [Raw output](raw/2172007/042-access_to_distinct_iter.output.tokens)

Actual input:

```rust
answer(a, c) => ((a, c,)) : - r0(a) => bind a <-
(observe(0, (__cq_store_r0).iter())), r1(a, b) => bind b <-
(observe(1, (__cq_index_i0).get(a).into_iter().flatten())), r3(b, c) => bind c
<- (observe(2, (__cq_index_i1).get(b).into_iter().flatten()));
```

Actual output (formatted where Rust syntax permits):

```rust
mini_linq::pipeline_to_distinct_iter! {
    iter0 = scan a in (observe(0, (__cq_store_r0).iter())) yield (a,); iter1 =
    join iter0 as (a,) with b in
    (observe(1, (__cq_index_i0).get(a).into_iter().flatten())) yield (a, b,);
    iter2 = join iter1 as (a, b,) with c in
    (observe(2, (__cq_index_i1).get(b).into_iter().flatten())) yield
    (a, b, c,); iter3 = project iter2 as (a, b, c,) yield ((a, c,)); iter4 =
    distinct iter3; return iter4;
}

```

---

## 2172007/043-pipeline_to_distinct_iter

[Raw input](raw/2172007/043-pipeline_to_distinct_iter.input.tokens) · [Raw output](raw/2172007/043-pipeline_to_distinct_iter.output.tokens)

Actual input:

```rust
iter0 = scan a in (observe(0, (__cq_store_r0).iter())) yield (a,); iter1 =
join iter0 as (a,) with b in
(observe(1, (__cq_index_i0).get(a).into_iter().flatten())) yield (a, b,);
iter2 = join iter1 as (a, b,) with c in
(observe(2, (__cq_index_i1).get(b).into_iter().flatten())) yield (a, b, c,);
iter3 = project iter2 as (a, b, c,) yield ((a, c,)); iter4 = distinct iter3;
return iter4;
```

Actual output (formatted where Rust syntax permits):

```rust
:: core :: iter :: Iterator ::
flatten(:: core :: iter ::
once_with(move | |
{
    let iter0 = :: core :: iter :: Iterator ::
    map(:: core :: iter :: IntoIterator ::
    into_iter(observe(0, (__cq_store_r0).iter())), move | a | (a,),); let
    iter1 = :: core :: iter :: Iterator ::
    flat_map(iter0, move | (a,) |
    {
        :: core :: iter :: Iterator ::
        map(:: core :: iter :: IntoIterator ::
        into_iter(observe(1, (__cq_index_i0).get(a).into_iter().flatten())),
        move | b |
        { let a = :: core :: clone :: Clone :: clone(& a); (a, b,) },)
    },); let iter2 = :: core :: iter :: Iterator ::
    flat_map(iter1, move | (a, b,) |
    {
        :: core :: iter :: Iterator ::
        map(:: core :: iter :: IntoIterator ::
        into_iter(observe(2, (__cq_index_i1).get(b).into_iter().flatten())),
        move | c |
        {
            let a = :: core :: clone :: Clone :: clone(& a); let b = :: core
            :: clone :: Clone :: clone(& b); (a, b, c,)
        },)
    },); let iter3 = :: core :: iter :: Iterator ::
    map(iter2, move | #[allow(unused_variables)] (a, b, c,) | (a, c,),); let
    iter4 =
    {
        let mut seen = :: std :: collections :: HashSet :: new(); :: core ::
        iter :: Iterator ::
        filter(iter3, move | row |
        seen.insert(:: core :: clone :: Clone :: clone(row)),)
    }; iter4
}))
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
   Compiling cq-ir v0.1.0 (/tmp/mini-linq-bind-pull-20260919/workspace/crates/cq-ir)
   Compiling cq-physical-selection v0.1.0 (/tmp/mini-linq-bind-pull-20260919/workspace/crates/cq-physical-selection)
   Compiling cq-access-lowering v0.1.0 (/tmp/mini-linq-bind-pull-20260919/workspace/crates/cq-access-lowering)
   Compiling cq-index-policy v0.1.0 (/tmp/mini-linq-bind-pull-20260919/workspace/crates/cq-index-policy)
   Compiling cq-physical-lowering v0.1.0 (/tmp/mini-linq-bind-pull-20260919/workspace/crates/cq-physical-lowering)
   Compiling cq-pipeline-lowering v0.1.0 (/tmp/mini-linq-bind-pull-20260919/workspace/crates/cq-pipeline-lowering)
   Compiling cq-query-lowering v0.1.0 (/tmp/mini-linq-bind-pull-20260919/workspace/crates/cq-query-lowering)
   Compiling mini-linq v0.1.0 (/tmp/mini-linq-bind-pull-20260919/workspace/crates/mini-linq)
   Compiling cq-example-providers v0.1.0 (/tmp/mini-linq-bind-pull-20260919/workspace/crates/cq-example-providers)
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
  --> crates/mini-linq/tests/physical_pull.rs:15:1
   |
15 | / thread_local! {
16 | |     // Each provider records source evaluations and successful row polls.
17 | |     static OBSERVATIONS: Cell<[[usize; 2]; 3]> = const { Cell::new([[0; 2]; 3]) };
18 | | }
   | |_^
   |
   = note: expanding `thread_local! { static OBSERVATIONS: Cell<[[usize; 2]; 3]> = const { Cell::new([[0; 2]; 3]) }; }`
   = note: to `$crate :: thread :: local_impl :: thread_local_process_attrs!
           ([] []; static OBSERVATIONS: Cell<[[usize; 2]; 3]> = const
           { Cell::new([[0; 2]; 3]) };);`
   = note: expanding `thread_local_process_attrs! { [] []; static OBSERVATIONS: Cell<[[usize; 2]; 3]> = const
           { Cell::new([[0; 2]; 3]) }; }`
   = note: to ` const OBSERVATIONS : $crate :: thread :: LocalKey < Cell<[[usize; 2]; 3]> > =
           $crate :: thread :: local_impl :: thread_local_inner!
           (@ key Cell<[[usize; 2]; 3]>, , const { Cell::new([[0; 2]; 3]) });`
   = note: expanding `thread_local_inner! { @ key Cell<[[usize; 2]; 3]>, , const { Cell::new([[0; 2]; 3]) } }`
   = note: to `{
               const __RUST_STD_INTERNAL_INIT : Cell<[[usize; 2]; 3]> =
               { Cell::new([[0; 2]; 3]) }; unsafe
               {
                   $crate :: thread :: LocalKey ::
                   new(const
                   {
                       if $crate :: mem :: needs_drop :: < Cell<[[usize; 2]; 3]> > ()
                       {
                           | _ |
                           {
                               #[thread_local] static __RUST_STD_INTERNAL_VAL : $crate ::
                               thread :: local_impl :: EagerStorage < Cell<[[usize; 2]; 3]>
                               > = $crate :: thread :: local_impl :: EagerStorage ::
                               new(__RUST_STD_INTERNAL_INIT); __RUST_STD_INTERNAL_VAL.get()
                           }
                       } else
                       {
                           | _ |
                           {
                               #[thread_local] static __RUST_STD_INTERNAL_VAL :
                               Cell<[[usize; 2]; 3]> = __RUST_STD_INTERNAL_INIT; &
                               __RUST_STD_INTERNAL_VAL
                           }
                       }
                   })
               }
           }`

note: trace_macro
   --> crates/mini-linq/tests/physical_pull.rs:127:5
    |
127 | /     ::mini_linq::cq_to_ra_written_order! {
128 | |         struct PullChain;
129 | |         relation L(a: Value);
130 | |         relation R(a: Value, b: Value);
131 | |         relation S(b: Value, c: Value);
132 | |         answer(a, c) :- L(a), R(a, b), S(b, c);
133 | |     }
    | |_____^
    |
    = note: expanding `explicit_policy! { struct PullChain; relation L(a : Value); relation R(a : Value, b : Value);
            relation S(b : Value, c : Value); answer(a, c) : - L(a), R(a, b), S(b, c);
            relational
            {
                r0 = rename L { a - > a }; r1 = rename R { a - > a, b - > b }; r2 =
                natural_join r0 with r1; r3 = rename S { b - > b, c - > c }; r4 =
                natural_join r2 with r3; r5 = project r4 keep { a, c }; output r5 as
                answer(a, c);
            } indexes { i0 = @ r1 [0]; i1 = @ r3 [0]; } uses
            { r2.right = i0; r4.right = i1; } }`
    = note: to `:: mini_linq :: given_physical_bindings!
            {
                struct PullChain; relation L(a : Value); relation R(a : Value, b : Value);
                relation S(b : Value, c : Value); answer(a, c) : - L(a), R(a, b), S(b, c);
                relational
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
                        L = observed_slice; R = observed_hash; S = observed_sorted; r0 =
                        observed_slice; r1 = observed_hash; r2 = :: cq_example_providers
                        :: borrowed_fields; r3 = observed_sorted; r4 = ::
                        cq_example_providers :: borrowed_fields; r5 = ::
                        cq_example_providers :: borrowed_fields;
                    } indexes { i0 = observed_hash; i1 = observed_sorted; } lowerings
                    {
                        r0 = :: cq_example_providers :: rename; r1 = ::
                        cq_example_providers :: rename; r2 = :: cq_example_providers ::
                        indexed_join; r3 = :: cq_example_providers :: rename; r4 = ::
                        cq_example_providers :: indexed_join; r5 = :: cq_example_providers
                        :: project;
                    }
                }
            }`
    = note: expanding `observed_slice! { reply_to(:: mini_linq :: resume_physical)
            {
                plan
                {
                    struct PullChain; relation L(a : Value); relation
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
                            L = observed_slice; R = observed_hash; S = observed_sorted; r0
                            = observed_slice; r1 = observed_hash; r2 = ::
                            cq_example_providers :: borrowed_fields; r3 = observed_sorted;
                            r4 = :: cq_example_providers :: borrowed_fields; r5 = ::
                            cq_example_providers :: borrowed_fields;
                        } indexes { i0 = observed_hash; i1 = observed_sorted; } lowerings
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
                struct PullChain; relation L(a : Value); relation R(a : Value, b : Value);
                relation S(b : Value, c : Value); answer(a, c) : - L(a), R(a, b), S(b, c);
                relational
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
                        L = observed_slice; R = observed_hash; S = observed_sorted; r0 =
                        observed_slice; r1 = observed_hash; r2 = :: cq_example_providers
                        :: borrowed_fields; r3 = observed_sorted; r4 = ::
                        cq_example_providers :: borrowed_fields; r5 = ::
                        cq_example_providers :: borrowed_fields;
                    } indexes { i0 = observed_hash; i1 = observed_sorted; } lowerings
                    {
                        r0 = :: cq_example_providers :: rename; r1 = ::
                        cq_example_providers :: rename; r2 = :: cq_example_providers ::
                        indexed_join; r3 = :: cq_example_providers :: rename; r4 = ::
                        cq_example_providers :: indexed_join; r5 = :: cq_example_providers
                        :: project;
                    }
                }
            } environment {} request { input L from(L) } }`
    = note: to `observe_provider!
            {provider(:: cq_example_providers :: slice_rows) kind(0)
                reply_to(:: mini_linq :: resume_physical)
                {
                    plan
                    {
                        struct PullChain; relation L(a : Value); relation
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
                                L = observed_slice; R = observed_hash; S = observed_sorted;
                                r0 = observed_slice; r1 = observed_hash; r2 = ::
                                cq_example_providers :: borrowed_fields; r3 =
                                observed_sorted; r4 = :: cq_example_providers ::
                                borrowed_fields; r5 = :: cq_example_providers ::
                                borrowed_fields;
                            } indexes { i0 = observed_hash; i1 = observed_sorted; }
                            lowerings
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
                    struct PullChain; relation L(a : Value); relation
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
                            L = observed_slice; R = observed_hash; S = observed_sorted; r0
                            = observed_slice; r1 = observed_hash; r2 = ::
                            cq_example_providers :: borrowed_fields; r3 = observed_sorted;
                            r4 = :: cq_example_providers :: borrowed_fields; r5 = ::
                            cq_example_providers :: borrowed_fields;
                        } indexes { i0 = observed_hash; i1 = observed_sorted; } lowerings
                        {
                            r0 = :: cq_example_providers :: rename; r1 = ::
                            cq_example_providers :: rename; r2 = :: cq_example_providers
                            :: indexed_join; r3 = :: cq_example_providers :: rename; r4 =
                            :: cq_example_providers :: indexed_join; r5 = ::
                            cq_example_providers :: project;
                        }
                    }
                } environment {} request { input L from(L) }}`
    = note: expanding `observe_provider! { provider(:: cq_example_providers :: slice_rows) kind(0)
            reply_to(:: mini_linq :: resume_physical)
            {
                plan
                {
                    struct PullChain; relation L(a : Value); relation
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
                            L = observed_slice; R = observed_hash; S = observed_sorted; r0
                            = observed_slice; r1 = observed_hash; r2 = ::
                            cq_example_providers :: borrowed_fields; r3 = observed_sorted;
                            r4 = :: cq_example_providers :: borrowed_fields; r5 = ::
                            cq_example_providers :: borrowed_fields;
                        } indexes { i0 = observed_hash; i1 = observed_sorted; } lowerings
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
                struct PullChain; relation L(a : Value); relation R(a : Value, b : Value);
                relation S(b : Value, c : Value); answer(a, c) : - L(a), R(a, b), S(b, c);
                relational
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
                        L = observed_slice; R = observed_hash; S = observed_sorted; r0 =
                        observed_slice; r1 = observed_hash; r2 = :: cq_example_providers
                        :: borrowed_fields; r3 = observed_sorted; r4 = ::
                        cq_example_providers :: borrowed_fields; r5 = ::
                        cq_example_providers :: borrowed_fields;
                    } indexes { i0 = observed_hash; i1 = observed_sorted; } lowerings
                    {
                        r0 = :: cq_example_providers :: rename; r1 = ::
                        cq_example_providers :: rename; r2 = :: cq_example_providers ::
                        indexed_join; r3 = :: cq_example_providers :: rename; r4 = ::
                        cq_example_providers :: indexed_join; r5 = :: cq_example_providers
                        :: project;
                    }
                }
            } environment {} request { input L from(L) } }`
    = note: to `:: cq_example_providers :: slice_rows !
            {
                reply_to(observe_reply)
                {
                    callback(:: mini_linq :: resume_physical) kind(0) state
                    {plan
                        {
                            struct PullChain; relation L(a : Value); relation
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
                                    L = observed_slice; R = observed_hash; S = observed_sorted;
                                    r0 = observed_slice; r1 = observed_hash; r2 = ::
                                    cq_example_providers :: borrowed_fields; r3 =
                                    observed_sorted; r4 = :: cq_example_providers ::
                                    borrowed_fields; r5 = :: cq_example_providers ::
                                    borrowed_fields;
                                } indexes { i0 = observed_hash; i1 = observed_sorted; }
                                lowerings
                                {
                                    r0 = :: cq_example_providers :: rename; r1 = ::
                                    cq_example_providers :: rename; r2 = :: cq_example_providers
                                    :: indexed_join; r3 = :: cq_example_providers :: rename; r4
                                    = :: cq_example_providers :: indexed_join; r5 = ::
                                    cq_example_providers :: project;
                                }
                            }
                        } environment {} emitted {} pending { input L from(L) }
                        continuations(:: mini_linq :: resume_physical, mini_linq ::
                        access)}
                } plan
                {struct PullChain; relation L(a : Value); relation
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
                            L = observed_slice; R = observed_hash; S = observed_sorted; r0
                            = observed_slice; r1 = observed_hash; r2 = ::
                            cq_example_providers :: borrowed_fields; r3 = observed_sorted;
                            r4 = :: cq_example_providers :: borrowed_fields; r5 = ::
                            cq_example_providers :: borrowed_fields;
                        } indexes { i0 = observed_hash; i1 = observed_sorted; } lowerings
                        {
                            r0 = :: cq_example_providers :: rename; r1 = ::
                            cq_example_providers :: rename; r2 = :: cq_example_providers
                            :: indexed_join; r3 = :: cq_example_providers :: rename; r4 =
                            :: cq_example_providers :: indexed_join; r5 = ::
                            cq_example_providers :: project;
                        }
                    }} environment {} request {input L from(L)}
            }`
    = note: expanding `observe_reply! { resume
            {
                callback(:: mini_linq :: resume_physical) kind(0) state
                {
                    plan
                    {
                        struct PullChain; relation L(a : Value); relation
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
                                L = observed_slice; R = observed_hash; S = observed_sorted;
                                r0 = observed_slice; r1 = observed_hash; r2 = ::
                                cq_example_providers :: borrowed_fields; r3 =
                                observed_sorted; r4 = :: cq_example_providers ::
                                borrowed_fields; r5 = :: cq_example_providers ::
                                borrowed_fields;
                            } indexes { i0 = observed_hash; i1 = observed_sorted; }
                            lowerings
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
                }
            } emitted { let __cq_store_L : & [Value] = L; } result
            { stored L at(__cq_store_L) operations observed_slice } }`
    = note: to `:: mini_linq :: resume_physical !
            {
                resume
                {plan
                    {
                        struct PullChain; relation L(a : Value); relation
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
                                L = observed_slice; R = observed_hash; S = observed_sorted;
                                r0 = observed_slice; r1 = observed_hash; r2 = ::
                                cq_example_providers :: borrowed_fields; r3 =
                                observed_sorted; r4 = :: cq_example_providers ::
                                borrowed_fields; r5 = :: cq_example_providers ::
                                borrowed_fields;
                            } indexes { i0 = observed_hash; i1 = observed_sorted; }
                            lowerings
                            {
                                r0 = :: cq_example_providers :: rename; r1 = ::
                                cq_example_providers :: rename; r2 = :: cq_example_providers
                                :: indexed_join; r3 = :: cq_example_providers :: rename; r4
                                = :: cq_example_providers :: indexed_join; r5 = ::
                                cq_example_providers :: project;
                            }
                        }
                    } environment {} emitted {} pending { input L from(L) }
                    continuations(:: mini_linq :: resume_physical, mini_linq :: access)}
                emitted {let __cq_store_L : & [Value] = L;} result
                {stored L at(__cq_store_L) operations observed_slice}
            }`
    = note: expanding `observed_hash! { reply_to(:: mini_linq :: resume_physical)
            {
                plan
                {
                    struct PullChain; relation L(a : Value); relation
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
                            L = observed_slice; R = observed_hash; S = observed_sorted; r0
                            = observed_slice; r1 = observed_hash; r2 = ::
                            cq_example_providers :: borrowed_fields; r3 = observed_sorted;
                            r4 = :: cq_example_providers :: borrowed_fields; r5 = ::
                            cq_example_providers :: borrowed_fields;
                        } indexes { i0 = observed_hash; i1 = observed_sorted; } lowerings
                        {
                            r0 = :: cq_example_providers :: rename; r1 = ::
                            cq_example_providers :: rename; r2 = :: cq_example_providers
                            :: indexed_join; r3 = :: cq_example_providers :: rename; r4 =
                            :: cq_example_providers :: indexed_join; r5 = ::
                            cq_example_providers :: project;
                        }
                    }
                } environment { stored L at(__cq_store_L) operations observed_slice }
                emitted { let __cq_store_L : & [Value] = L; } pending { input R from(R) }
                continuations(:: mini_linq :: resume_physical, mini_linq :: access)
            } plan
            {
                struct PullChain; relation L(a : Value); relation R(a : Value, b : Value);
                relation S(b : Value, c : Value); answer(a, c) : - L(a), R(a, b), S(b, c);
                relational
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
                        L = observed_slice; R = observed_hash; S = observed_sorted; r0 =
                        observed_slice; r1 = observed_hash; r2 = :: cq_example_providers
                        :: borrowed_fields; r3 = observed_sorted; r4 = ::
                        cq_example_providers :: borrowed_fields; r5 = ::
                        cq_example_providers :: borrowed_fields;
                    } indexes { i0 = observed_hash; i1 = observed_sorted; } lowerings
                    {
                        r0 = :: cq_example_providers :: rename; r1 = ::
                        cq_example_providers :: rename; r2 = :: cq_example_providers ::
                        indexed_join; r3 = :: cq_example_providers :: rename; r4 = ::
                        cq_example_providers :: indexed_join; r5 = :: cq_example_providers
                        :: project;
                    }
                }
            } environment { stored L at(__cq_store_L) operations observed_slice } request
            { input R from(R) } }`
    = note: to `observe_provider!
            {provider(:: cq_example_providers :: hash_relation) kind(1)
                reply_to(:: mini_linq :: resume_physical)
                {
                    plan
                    {
                        struct PullChain; relation L(a : Value); relation
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
                                L = observed_slice; R = observed_hash; S = observed_sorted;
                                r0 = observed_slice; r1 = observed_hash; r2 = ::
                                cq_example_providers :: borrowed_fields; r3 =
                                observed_sorted; r4 = :: cq_example_providers ::
                                borrowed_fields; r5 = :: cq_example_providers ::
                                borrowed_fields;
                            } indexes { i0 = observed_hash; i1 = observed_sorted; }
                            lowerings
                            {
                                r0 = :: cq_example_providers :: rename; r1 = ::
                                cq_example_providers :: rename; r2 = :: cq_example_providers
                                :: indexed_join; r3 = :: cq_example_providers :: rename; r4
                                = :: cq_example_providers :: indexed_join; r5 = ::
                                cq_example_providers :: project;
                            }
                        }
                    } environment { stored L at(__cq_store_L) operations observed_slice }
                    emitted { let __cq_store_L : & [Value] = L; } pending
                    { input R from(R) }
                    continuations(:: mini_linq :: resume_physical, mini_linq :: access)
                } plan
                {
                    struct PullChain; relation L(a : Value); relation
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
                            L = observed_slice; R = observed_hash; S = observed_sorted; r0
                            = observed_slice; r1 = observed_hash; r2 = ::
                            cq_example_providers :: borrowed_fields; r3 = observed_sorted;
                            r4 = :: cq_example_providers :: borrowed_fields; r5 = ::
                            cq_example_providers :: borrowed_fields;
                        } indexes { i0 = observed_hash; i1 = observed_sorted; } lowerings
                        {
                            r0 = :: cq_example_providers :: rename; r1 = ::
                            cq_example_providers :: rename; r2 = :: cq_example_providers
                            :: indexed_join; r3 = :: cq_example_providers :: rename; r4 =
                            :: cq_example_providers :: indexed_join; r5 = ::
                            cq_example_providers :: project;
                        }
                    }
                } environment { stored L at(__cq_store_L) operations observed_slice }
                request { input R from(R) }}`
    = note: expanding `observe_provider! { provider(:: cq_example_providers :: hash_relation) kind(1)
            reply_to(:: mini_linq :: resume_physical)
            {
                plan
                {
                    struct PullChain; relation L(a : Value); relation
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
                            L = observed_slice; R = observed_hash; S = observed_sorted; r0
                            = observed_slice; r1 = observed_hash; r2 = ::
                            cq_example_providers :: borrowed_fields; r3 = observed_sorted;
                            r4 = :: cq_example_providers :: borrowed_fields; r5 = ::
                            cq_example_providers :: borrowed_fields;
                        } indexes { i0 = observed_hash; i1 = observed_sorted; } lowerings
                        {
                            r0 = :: cq_example_providers :: rename; r1 = ::
                            cq_example_providers :: rename; r2 = :: cq_example_providers
                            :: indexed_join; r3 = :: cq_example_providers :: rename; r4 =
                            :: cq_example_providers :: indexed_join; r5 = ::
                            cq_example_providers :: project;
                        }
                    }
                } environment { stored L at(__cq_store_L) operations observed_slice }
                emitted { let __cq_store_L : & [Value] = L; } pending { input R from(R) }
                continuations(:: mini_linq :: resume_physical, mini_linq :: access)
            } plan
            {
                struct PullChain; relation L(a : Value); relation R(a : Value, b : Value);
                relation S(b : Value, c : Value); answer(a, c) : - L(a), R(a, b), S(b, c);
                relational
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
                        L = observed_slice; R = observed_hash; S = observed_sorted; r0 =
                        observed_slice; r1 = observed_hash; r2 = :: cq_example_providers
                        :: borrowed_fields; r3 = observed_sorted; r4 = ::
                        cq_example_providers :: borrowed_fields; r5 = ::
                        cq_example_providers :: borrowed_fields;
                    } indexes { i0 = observed_hash; i1 = observed_sorted; } lowerings
                    {
                        r0 = :: cq_example_providers :: rename; r1 = ::
                        cq_example_providers :: rename; r2 = :: cq_example_providers ::
                        indexed_join; r3 = :: cq_example_providers :: rename; r4 = ::
                        cq_example_providers :: indexed_join; r5 = :: cq_example_providers
                        :: project;
                    }
                }
            } environment { stored L at(__cq_store_L) operations observed_slice } request
            { input R from(R) } }`
    = note: to `:: cq_example_providers :: hash_relation !
            {
                reply_to(observe_reply)
                {
                    callback(:: mini_linq :: resume_physical) kind(1) state
                    {plan
                        {
                            struct PullChain; relation L(a : Value); relation
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
                                    L = observed_slice; R = observed_hash; S = observed_sorted;
                                    r0 = observed_slice; r1 = observed_hash; r2 = ::
                                    cq_example_providers :: borrowed_fields; r3 =
                                    observed_sorted; r4 = :: cq_example_providers ::
                                    borrowed_fields; r5 = :: cq_example_providers ::
                                    borrowed_fields;
                                } indexes { i0 = observed_hash; i1 = observed_sorted; }
                                lowerings
                                {
                                    r0 = :: cq_example_providers :: rename; r1 = ::
                                    cq_example_providers :: rename; r2 = :: cq_example_providers
                                    :: indexed_join; r3 = :: cq_example_providers :: rename; r4
                                    = :: cq_example_providers :: indexed_join; r5 = ::
                                    cq_example_providers :: project;
                                }
                            }
                        } environment
                        { stored L at(__cq_store_L) operations observed_slice } emitted
                        { let __cq_store_L : & [Value] = L; } pending { input R from(R) }
                        continuations(:: mini_linq :: resume_physical, mini_linq ::
                        access)}
                } plan
                {struct PullChain; relation L(a : Value); relation
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
                            L = observed_slice; R = observed_hash; S = observed_sorted; r0
                            = observed_slice; r1 = observed_hash; r2 = ::
                            cq_example_providers :: borrowed_fields; r3 = observed_sorted;
                            r4 = :: cq_example_providers :: borrowed_fields; r5 = ::
                            cq_example_providers :: borrowed_fields;
                        } indexes { i0 = observed_hash; i1 = observed_sorted; } lowerings
                        {
                            r0 = :: cq_example_providers :: rename; r1 = ::
                            cq_example_providers :: rename; r2 = :: cq_example_providers
                            :: indexed_join; r3 = :: cq_example_providers :: rename; r4 =
                            :: cq_example_providers :: indexed_join; r5 = ::
                            cq_example_providers :: project;
                        }
                    }} environment {stored L at(__cq_store_L) operations observed_slice}
                request {input R from(R)}
            }`
    = note: expanding `observe_reply! { resume
            {
                callback(:: mini_linq :: resume_physical) kind(1) state
                {
                    plan
                    {
                        struct PullChain; relation L(a : Value); relation
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
                                L = observed_slice; R = observed_hash; S = observed_sorted;
                                r0 = observed_slice; r1 = observed_hash; r2 = ::
                                cq_example_providers :: borrowed_fields; r3 =
                                observed_sorted; r4 = :: cq_example_providers ::
                                borrowed_fields; r5 = :: cq_example_providers ::
                                borrowed_fields;
                            } indexes { i0 = observed_hash; i1 = observed_sorted; }
                            lowerings
                            {
                                r0 = :: cq_example_providers :: rename; r1 = ::
                                cq_example_providers :: rename; r2 = :: cq_example_providers
                                :: indexed_join; r3 = :: cq_example_providers :: rename; r4
                                = :: cq_example_providers :: indexed_join; r5 = ::
                                cq_example_providers :: project;
                            }
                        }
                    } environment { stored L at(__cq_store_L) operations observed_slice }
                    emitted { let __cq_store_L : & [Value] = L; } pending
                    { input R from(R) }
                    continuations(:: mini_linq :: resume_physical, mini_linq :: access)
                }
            } emitted
            {
                let __cq_store_R : & :: std :: collections :: HashMap < Value, :: std ::
                vec :: Vec < Value >> = R;
            } result { stored R at(__cq_store_R) operations observed_hash } }`
    = note: to `:: mini_linq :: resume_physical !
            {
                resume
                {plan
                    {
                        struct PullChain; relation L(a : Value); relation
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
                                L = observed_slice; R = observed_hash; S = observed_sorted;
                                r0 = observed_slice; r1 = observed_hash; r2 = ::
                                cq_example_providers :: borrowed_fields; r3 =
                                observed_sorted; r4 = :: cq_example_providers ::
                                borrowed_fields; r5 = :: cq_example_providers ::
                                borrowed_fields;
                            } indexes { i0 = observed_hash; i1 = observed_sorted; }
                            lowerings
                            {
                                r0 = :: cq_example_providers :: rename; r1 = ::
                                cq_example_providers :: rename; r2 = :: cq_example_providers
                                :: indexed_join; r3 = :: cq_example_providers :: rename; r4
                                = :: cq_example_providers :: indexed_join; r5 = ::
                                cq_example_providers :: project;
                            }
                        }
                    } environment { stored L at(__cq_store_L) operations observed_slice }
                    emitted { let __cq_store_L : & [Value] = L; } pending
                    { input R from(R) }
                    continuations(:: mini_linq :: resume_physical, mini_linq :: access)}
                emitted
                {let __cq_store_R : & :: std :: collections :: HashMap < Value, :: std ::
                    vec :: Vec < Value >> = R;} result
                {stored R at(__cq_store_R) operations observed_hash}
            }`
    = note: expanding `observed_sorted! { reply_to(:: mini_linq :: resume_physical)
            {
                plan
                {
                    struct PullChain; relation L(a : Value); relation
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
                            L = observed_slice; R = observed_hash; S = observed_sorted; r0
                            = observed_slice; r1 = observed_hash; r2 = ::
                            cq_example_providers :: borrowed_fields; r3 = observed_sorted;
                            r4 = :: cq_example_providers :: borrowed_fields; r5 = ::
                            cq_example_providers :: borrowed_fields;
                        } indexes { i0 = observed_hash; i1 = observed_sorted; } lowerings
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
                    stored L at(__cq_store_L) operations observed_slice stored R
                    at(__cq_store_R) operations observed_hash
                } emitted
                {
                    let __cq_store_L : & [Value] = L; let __cq_store_R : & :: std ::
                    collections :: HashMap < Value, :: std :: vec :: Vec < Value > > = R;
                } pending { input S from(S) }
                continuations(:: mini_linq :: resume_physical, mini_linq :: access)
            } plan
            {
                struct PullChain; relation L(a : Value); relation R(a : Value, b : Value);
                relation S(b : Value, c : Value); answer(a, c) : - L(a), R(a, b), S(b, c);
                relational
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
                        L = observed_slice; R = observed_hash; S = observed_sorted; r0 =
                        observed_slice; r1 = observed_hash; r2 = :: cq_example_providers
                        :: borrowed_fields; r3 = observed_sorted; r4 = ::
                        cq_example_providers :: borrowed_fields; r5 = ::
                        cq_example_providers :: borrowed_fields;
                    } indexes { i0 = observed_hash; i1 = observed_sorted; } lowerings
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
                stored L at(__cq_store_L) operations observed_slice stored R
                at(__cq_store_R) operations observed_hash
            } request { input S from(S) } }`
    = note: to `observe_provider!
            {provider(:: cq_example_providers :: sorted_relation) kind(2)
                reply_to(:: mini_linq :: resume_physical)
                {
                    plan
                    {
                        struct PullChain; relation L(a : Value); relation
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
                                L = observed_slice; R = observed_hash; S = observed_sorted;
                                r0 = observed_slice; r1 = observed_hash; r2 = ::
                                cq_example_providers :: borrowed_fields; r3 =
                                observed_sorted; r4 = :: cq_example_providers ::
                                borrowed_fields; r5 = :: cq_example_providers ::
                                borrowed_fields;
                            } indexes { i0 = observed_hash; i1 = observed_sorted; }
                            lowerings
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
                        stored L at(__cq_store_L) operations observed_slice stored R
                        at(__cq_store_R) operations observed_hash
                    } emitted
                    {
                        let __cq_store_L : & [Value] = L; let __cq_store_R : & :: std ::
                        collections :: HashMap < Value, :: std :: vec :: Vec < Value > > =
                        R;
                    } pending { input S from(S) }
                    continuations(:: mini_linq :: resume_physical, mini_linq :: access)
                } plan
                {
                    struct PullChain; relation L(a : Value); relation
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
                            L = observed_slice; R = observed_hash; S = observed_sorted; r0
                            = observed_slice; r1 = observed_hash; r2 = ::
                            cq_example_providers :: borrowed_fields; r3 = observed_sorted;
                            r4 = :: cq_example_providers :: borrowed_fields; r5 = ::
                            cq_example_providers :: borrowed_fields;
                        } indexes { i0 = observed_hash; i1 = observed_sorted; } lowerings
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
                    stored L at(__cq_store_L) operations observed_slice stored R
                    at(__cq_store_R) operations observed_hash
                } request { input S from(S) }}`
    = note: expanding `observe_provider! { provider(:: cq_example_providers :: sorted_relation) kind(2)
            reply_to(:: mini_linq :: resume_physical)
            {
                plan
                {
                    struct PullChain; relation L(a : Value); relation
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
                            L = observed_slice; R = observed_hash; S = observed_sorted; r0
                            = observed_slice; r1 = observed_hash; r2 = ::
                            cq_example_providers :: borrowed_fields; r3 = observed_sorted;
                            r4 = :: cq_example_providers :: borrowed_fields; r5 = ::
                            cq_example_providers :: borrowed_fields;
                        } indexes { i0 = observed_hash; i1 = observed_sorted; } lowerings
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
                    stored L at(__cq_store_L) operations observed_slice stored R
                    at(__cq_store_R) operations observed_hash
                } emitted
                {
                    let __cq_store_L : & [Value] = L; let __cq_store_R : & :: std ::
                    collections :: HashMap < Value, :: std :: vec :: Vec < Value > > = R;
                } pending { input S from(S) }
                continuations(:: mini_linq :: resume_physical, mini_linq :: access)
            } plan
            {
                struct PullChain; relation L(a : Value); relation R(a : Value, b : Value);
                relation S(b : Value, c : Value); answer(a, c) : - L(a), R(a, b), S(b, c);
                relational
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
                        L = observed_slice; R = observed_hash; S = observed_sorted; r0 =
                        observed_slice; r1 = observed_hash; r2 = :: cq_example_providers
                        :: borrowed_fields; r3 = observed_sorted; r4 = ::
                        cq_example_providers :: borrowed_fields; r5 = ::
                        cq_example_providers :: borrowed_fields;
                    } indexes { i0 = observed_hash; i1 = observed_sorted; } lowerings
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
                stored L at(__cq_store_L) operations observed_slice stored R
                at(__cq_store_R) operations observed_hash
            } request { input S from(S) } }`
    = note: to `:: cq_example_providers :: sorted_relation !
            {
                reply_to(observe_reply)
                {
                    callback(:: mini_linq :: resume_physical) kind(2) state
                    {plan
                        {
                            struct PullChain; relation L(a : Value); relation
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
                                    L = observed_slice; R = observed_hash; S = observed_sorted;
                                    r0 = observed_slice; r1 = observed_hash; r2 = ::
                                    cq_example_providers :: borrowed_fields; r3 =
                                    observed_sorted; r4 = :: cq_example_providers ::
                                    borrowed_fields; r5 = :: cq_example_providers ::
                                    borrowed_fields;
                                } indexes { i0 = observed_hash; i1 = observed_sorted; }
                                lowerings
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
                            stored L at(__cq_store_L) operations observed_slice stored R
                            at(__cq_store_R) operations observed_hash
                        } emitted
                        {
                            let __cq_store_L : & [Value] = L; let __cq_store_R : & :: std
                            :: collections :: HashMap < Value, :: std :: vec :: Vec <
                            Value > > = R;
                        } pending { input S from(S) }
                        continuations(:: mini_linq :: resume_physical, mini_linq ::
                        access)}
                } plan
                {struct PullChain; relation L(a : Value); relation
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
                            L = observed_slice; R = observed_hash; S = observed_sorted; r0
                            = observed_slice; r1 = observed_hash; r2 = ::
                            cq_example_providers :: borrowed_fields; r3 = observed_sorted;
                            r4 = :: cq_example_providers :: borrowed_fields; r5 = ::
                            cq_example_providers :: borrowed_fields;
                        } indexes { i0 = observed_hash; i1 = observed_sorted; } lowerings
                        {
                            r0 = :: cq_example_providers :: rename; r1 = ::
                            cq_example_providers :: rename; r2 = :: cq_example_providers
                            :: indexed_join; r3 = :: cq_example_providers :: rename; r4 =
                            :: cq_example_providers :: indexed_join; r5 = ::
                            cq_example_providers :: project;
                        }
                    }} environment
                {stored L at(__cq_store_L) operations observed_slice stored R
                    at(__cq_store_R) operations observed_hash} request {input S from(S)}
            }`
    = note: expanding `observe_reply! { resume
            {
                callback(:: mini_linq :: resume_physical) kind(2) state
                {
                    plan
                    {
                        struct PullChain; relation L(a : Value); relation
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
                                L = observed_slice; R = observed_hash; S = observed_sorted;
                                r0 = observed_slice; r1 = observed_hash; r2 = ::
                                cq_example_providers :: borrowed_fields; r3 =
                                observed_sorted; r4 = :: cq_example_providers ::
                                borrowed_fields; r5 = :: cq_example_providers ::
                                borrowed_fields;
                            } indexes { i0 = observed_hash; i1 = observed_sorted; }
                            lowerings
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
                        stored L at(__cq_store_L) operations observed_slice stored R
                        at(__cq_store_R) operations observed_hash
                    } emitted
                    {
                        let __cq_store_L : & [Value] = L; let __cq_store_R : & :: std ::
                        collections :: HashMap < Value, :: std :: vec :: Vec < Value > > =
                        R;
                    } pending { input S from(S) }
                    continuations(:: mini_linq :: resume_physical, mini_linq :: access)
                }
            } emitted
            {
                let __cq_store_S : & :: std :: collections :: BTreeMap < Value, :: std ::
                vec :: Vec < Value >> = S;
            } result { stored S at(__cq_store_S) operations observed_sorted } }`
    = note: to `:: mini_linq :: resume_physical !
            {
                resume
                {plan
                    {
                        struct PullChain; relation L(a : Value); relation
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
                                L = observed_slice; R = observed_hash; S = observed_sorted;
                                r0 = observed_slice; r1 = observed_hash; r2 = ::
                                cq_example_providers :: borrowed_fields; r3 =
                                observed_sorted; r4 = :: cq_example_providers ::
                                borrowed_fields; r5 = :: cq_example_providers ::
                                borrowed_fields;
                            } indexes { i0 = observed_hash; i1 = observed_sorted; }
                            lowerings
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
                        stored L at(__cq_store_L) operations observed_slice stored R
                        at(__cq_store_R) operations observed_hash
                    } emitted
                    {
                        let __cq_store_L : & [Value] = L; let __cq_store_R : & :: std ::
                        collections :: HashMap < Value, :: std :: vec :: Vec < Value > > =
                        R;
                    } pending { input S from(S) }
                    continuations(:: mini_linq :: resume_physical, mini_linq :: access)}
                emitted
                {let __cq_store_S : & :: std :: collections :: BTreeMap < Value, :: std ::
                    vec :: Vec < Value >> = S;} result
                {stored S at(__cq_store_S) operations observed_sorted}
            }`
    = note: expanding `observed_slice! { reply_to(:: mini_linq :: resume_physical)
            {
                plan
                {
                    struct PullChain; relation L(a : Value); relation
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
                            L = observed_slice; R = observed_hash; S = observed_sorted; r0
                            = observed_slice; r1 = observed_hash; r2 = ::
                            cq_example_providers :: borrowed_fields; r3 = observed_sorted;
                            r4 = :: cq_example_providers :: borrowed_fields; r5 = ::
                            cq_example_providers :: borrowed_fields;
                        } indexes { i0 = observed_hash; i1 = observed_sorted; } lowerings
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
                    stored L at(__cq_store_L) operations observed_slice stored R
                    at(__cq_store_R) operations observed_hash stored S at(__cq_store_S)
                    operations observed_sorted
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
                struct PullChain; relation L(a : Value); relation R(a : Value, b : Value);
                relation S(b : Value, c : Value); answer(a, c) : - L(a), R(a, b), S(b, c);
                relational
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
                        L = observed_slice; R = observed_hash; S = observed_sorted; r0 =
                        observed_slice; r1 = observed_hash; r2 = :: cq_example_providers
                        :: borrowed_fields; r3 = observed_sorted; r4 = ::
                        cq_example_providers :: borrowed_fields; r5 = ::
                        cq_example_providers :: borrowed_fields;
                    } indexes { i0 = observed_hash; i1 = observed_sorted; } lowerings
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
                stored L at(__cq_store_L) operations observed_slice stored R
                at(__cq_store_R) operations observed_hash stored S at(__cq_store_S)
                operations observed_sorted
            } request { input r0 from(__cq_store_L) } }`
    = note: to `observe_provider!
            {provider(:: cq_example_providers :: slice_rows) kind(0)
                reply_to(:: mini_linq :: resume_physical)
                {
                    plan
                    {
                        struct PullChain; relation L(a : Value); relation
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
                                L = observed_slice; R = observed_hash; S = observed_sorted;
                                r0 = observed_slice; r1 = observed_hash; r2 = ::
                                cq_example_providers :: borrowed_fields; r3 =
                                observed_sorted; r4 = :: cq_example_providers ::
                                borrowed_fields; r5 = :: cq_example_providers ::
                                borrowed_fields;
                            } indexes { i0 = observed_hash; i1 = observed_sorted; }
                            lowerings
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
                        stored L at(__cq_store_L) operations observed_slice stored R
                        at(__cq_store_R) operations observed_hash stored S
                        at(__cq_store_S) operations observed_sorted
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
                    struct PullChain; relation L(a : Value); relation
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
                            L = observed_slice; R = observed_hash; S = observed_sorted; r0
                            = observed_slice; r1 = observed_hash; r2 = ::
                            cq_example_providers :: borrowed_fields; r3 = observed_sorted;
                            r4 = :: cq_example_providers :: borrowed_fields; r5 = ::
                            cq_example_providers :: borrowed_fields;
                        } indexes { i0 = observed_hash; i1 = observed_sorted; } lowerings
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
                    stored L at(__cq_store_L) operations observed_slice stored R
                    at(__cq_store_R) operations observed_hash stored S at(__cq_store_S)
                    operations observed_sorted
                } request { input r0 from(__cq_store_L) }}`
    = note: expanding `observe_provider! { provider(:: cq_example_providers :: slice_rows) kind(0)
            reply_to(:: mini_linq :: resume_physical)
            {
                plan
                {
                    struct PullChain; relation L(a : Value); relation
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
                            L = observed_slice; R = observed_hash; S = observed_sorted; r0
                            = observed_slice; r1 = observed_hash; r2 = ::
                            cq_example_providers :: borrowed_fields; r3 = observed_sorted;
                            r4 = :: cq_example_providers :: borrowed_fields; r5 = ::
                            cq_example_providers :: borrowed_fields;
                        } indexes { i0 = observed_hash; i1 = observed_sorted; } lowerings
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
                    stored L at(__cq_store_L) operations observed_slice stored R
                    at(__cq_store_R) operations observed_hash stored S at(__cq_store_S)
                    operations observed_sorted
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
                struct PullChain; relation L(a : Value); relation R(a : Value, b : Value);
                relation S(b : Value, c : Value); answer(a, c) : - L(a), R(a, b), S(b, c);
                relational
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
                        L = observed_slice; R = observed_hash; S = observed_sorted; r0 =
                        observed_slice; r1 = observed_hash; r2 = :: cq_example_providers
                        :: borrowed_fields; r3 = observed_sorted; r4 = ::
                        cq_example_providers :: borrowed_fields; r5 = ::
                        cq_example_providers :: borrowed_fields;
                    } indexes { i0 = observed_hash; i1 = observed_sorted; } lowerings
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
                stored L at(__cq_store_L) operations observed_slice stored R
                at(__cq_store_R) operations observed_hash stored S at(__cq_store_S)
                operations observed_sorted
            } request { input r0 from(__cq_store_L) } }`
    = note: to `:: cq_example_providers :: slice_rows !
            {
                reply_to(observe_reply)
                {
                    callback(:: mini_linq :: resume_physical) kind(0) state
                    {plan
                        {
                            struct PullChain; relation L(a : Value); relation
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
                                    L = observed_slice; R = observed_hash; S = observed_sorted;
                                    r0 = observed_slice; r1 = observed_hash; r2 = ::
                                    cq_example_providers :: borrowed_fields; r3 =
                                    observed_sorted; r4 = :: cq_example_providers ::
                                    borrowed_fields; r5 = :: cq_example_providers ::
                                    borrowed_fields;
                                } indexes { i0 = observed_hash; i1 = observed_sorted; }
                                lowerings
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
                            stored L at(__cq_store_L) operations observed_slice stored R
                            at(__cq_store_R) operations observed_hash stored S
                            at(__cq_store_S) operations observed_sorted
                        } emitted
                        {
                            let __cq_store_L : & [Value] = L; let __cq_store_R : & :: std
                            :: collections :: HashMap < Value, :: std :: vec :: Vec <
                            Value > > = R; let __cq_store_S : & :: std :: collections ::
                            BTreeMap < Value, :: std :: vec :: Vec < Value > > = S;
                        } pending { lower r0 }
                        continuations(:: mini_linq :: resume_physical, mini_linq ::
                        access)}
                } plan
                {struct PullChain; relation L(a : Value); relation
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
                            L = observed_slice; R = observed_hash; S = observed_sorted; r0
                            = observed_slice; r1 = observed_hash; r2 = ::
                            cq_example_providers :: borrowed_fields; r3 = observed_sorted;
                            r4 = :: cq_example_providers :: borrowed_fields; r5 = ::
                            cq_example_providers :: borrowed_fields;
                        } indexes { i0 = observed_hash; i1 = observed_sorted; } lowerings
                        {
                            r0 = :: cq_example_providers :: rename; r1 = ::
                            cq_example_providers :: rename; r2 = :: cq_example_providers
                            :: indexed_join; r3 = :: cq_example_providers :: rename; r4 =
                            :: cq_example_providers :: indexed_join; r5 = ::
                            cq_example_providers :: project;
                        }
                    }} environment
                {stored L at(__cq_store_L) operations observed_slice stored R
                    at(__cq_store_R) operations observed_hash stored S at(__cq_store_S)
                    operations observed_sorted} request {input r0 from(__cq_store_L)}
            }`
    = note: expanding `observe_reply! { resume
            {
                callback(:: mini_linq :: resume_physical) kind(0) state
                {
                    plan
                    {
                        struct PullChain; relation L(a : Value); relation
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
                                L = observed_slice; R = observed_hash; S = observed_sorted;
                                r0 = observed_slice; r1 = observed_hash; r2 = ::
                                cq_example_providers :: borrowed_fields; r3 =
                                observed_sorted; r4 = :: cq_example_providers ::
                                borrowed_fields; r5 = :: cq_example_providers ::
                                borrowed_fields;
                            } indexes { i0 = observed_hash; i1 = observed_sorted; }
                            lowerings
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
                        stored L at(__cq_store_L) operations observed_slice stored R
                        at(__cq_store_R) operations observed_hash stored S
                        at(__cq_store_S) operations observed_sorted
                    } emitted
                    {
                        let __cq_store_L : & [Value] = L; let __cq_store_R : & :: std ::
                        collections :: HashMap < Value, :: std :: vec :: Vec < Value > > =
                        R; let __cq_store_S : & :: std :: collections :: BTreeMap < Value,
                        :: std :: vec :: Vec < Value > > = S;
                    } pending { lower r0 }
                    continuations(:: mini_linq :: resume_physical, mini_linq :: access)
                }
            } emitted { let __cq_store_r0 : & [Value] = __cq_store_L; } result
            { stored r0 at(__cq_store_r0) operations observed_slice } }`
    = note: to `:: mini_linq :: resume_physical !
            {
                resume
                {plan
                    {
                        struct PullChain; relation L(a : Value); relation
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
                                L = observed_slice; R = observed_hash; S = observed_sorted;
                                r0 = observed_slice; r1 = observed_hash; r2 = ::
                                cq_example_providers :: borrowed_fields; r3 =
                                observed_sorted; r4 = :: cq_example_providers ::
                                borrowed_fields; r5 = :: cq_example_providers ::
                                borrowed_fields;
                            } indexes { i0 = observed_hash; i1 = observed_sorted; }
                            lowerings
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
                        stored L at(__cq_store_L) operations observed_slice stored R
                        at(__cq_store_R) operations observed_hash stored S
                        at(__cq_store_S) operations observed_sorted
                    } emitted
                    {
                        let __cq_store_L : & [Value] = L; let __cq_store_R : & :: std ::
                        collections :: HashMap < Value, :: std :: vec :: Vec < Value > > =
                        R; let __cq_store_S : & :: std :: collections :: BTreeMap < Value,
                        :: std :: vec :: Vec < Value > > = S;
                    } pending { lower r0 }
                    continuations(:: mini_linq :: resume_physical, mini_linq :: access)}
                emitted {let __cq_store_r0 : & [Value] = __cq_store_L;} result
                {stored r0 at(__cq_store_r0) operations observed_slice}
            }`
    = note: expanding `observed_hash! { reply_to(:: mini_linq :: resume_physical)
            {
                plan
                {
                    struct PullChain; relation L(a : Value); relation
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
                            L = observed_slice; R = observed_hash; S = observed_sorted; r0
                            = observed_slice; r1 = observed_hash; r2 = ::
                            cq_example_providers :: borrowed_fields; r3 = observed_sorted;
                            r4 = :: cq_example_providers :: borrowed_fields; r5 = ::
                            cq_example_providers :: borrowed_fields;
                        } indexes { i0 = observed_hash; i1 = observed_sorted; } lowerings
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
                    stored L at(__cq_store_L) operations observed_slice stored R
                    at(__cq_store_R) operations observed_hash stored S at(__cq_store_S)
                    operations observed_sorted stored r0 at(__cq_store_r0) operations
                    observed_slice
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
                struct PullChain; relation L(a : Value); relation R(a : Value, b : Value);
                relation S(b : Value, c : Value); answer(a, c) : - L(a), R(a, b), S(b, c);
                relational
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
                        L = observed_slice; R = observed_hash; S = observed_sorted; r0 =
                        observed_slice; r1 = observed_hash; r2 = :: cq_example_providers
                        :: borrowed_fields; r3 = observed_sorted; r4 = ::
                        cq_example_providers :: borrowed_fields; r5 = ::
                        cq_example_providers :: borrowed_fields;
                    } indexes { i0 = observed_hash; i1 = observed_sorted; } lowerings
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
                stored L at(__cq_store_L) operations observed_slice stored R
                at(__cq_store_R) operations observed_hash stored S at(__cq_store_S)
                operations observed_sorted stored r0 at(__cq_store_r0) operations
                observed_slice
            } request { input r1 from(__cq_store_R) } }`
    = note: to `observe_provider!
            {provider(:: cq_example_providers :: hash_relation) kind(1)
                reply_to(:: mini_linq :: resume_physical)
                {
                    plan
                    {
                        struct PullChain; relation L(a : Value); relation
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
                                L = observed_slice; R = observed_hash; S = observed_sorted;
                                r0 = observed_slice; r1 = observed_hash; r2 = ::
                                cq_example_providers :: borrowed_fields; r3 =
                                observed_sorted; r4 = :: cq_example_providers ::
                                borrowed_fields; r5 = :: cq_example_providers ::
                                borrowed_fields;
                            } indexes { i0 = observed_hash; i1 = observed_sorted; }
                            lowerings
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
                        stored L at(__cq_store_L) operations observed_slice stored R
                        at(__cq_store_R) operations observed_hash stored S
                        at(__cq_store_S) operations observed_sorted stored r0
                        at(__cq_store_r0) operations observed_slice
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
                    struct PullChain; relation L(a : Value); relation
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
                            L = observed_slice; R = observed_hash; S = observed_sorted; r0
                            = observed_slice; r1 = observed_hash; r2 = ::
                            cq_example_providers :: borrowed_fields; r3 = observed_sorted;
                            r4 = :: cq_example_providers :: borrowed_fields; r5 = ::
                            cq_example_providers :: borrowed_fields;
                        } indexes { i0 = observed_hash; i1 = observed_sorted; } lowerings
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
                    stored L at(__cq_store_L) operations observed_slice stored R
                    at(__cq_store_R) operations observed_hash stored S at(__cq_store_S)
                    operations observed_sorted stored r0 at(__cq_store_r0) operations
                    observed_slice
                } request { input r1 from(__cq_store_R) }}`
    = note: expanding `observe_provider! { provider(:: cq_example_providers :: hash_relation) kind(1)
            reply_to(:: mini_linq :: resume_physical)
            {
                plan
                {
                    struct PullChain; relation L(a : Value); relation
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
                            L = observed_slice; R = observed_hash; S = observed_sorted; r0
                            = observed_slice; r1 = observed_hash; r2 = ::
                            cq_example_providers :: borrowed_fields; r3 = observed_sorted;
                            r4 = :: cq_example_providers :: borrowed_fields; r5 = ::
                            cq_example_providers :: borrowed_fields;
                        } indexes { i0 = observed_hash; i1 = observed_sorted; } lowerings
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
                    stored L at(__cq_store_L) operations observed_slice stored R
                    at(__cq_store_R) operations observed_hash stored S at(__cq_store_S)
                    operations observed_sorted stored r0 at(__cq_store_r0) operations
                    observed_slice
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
                struct PullChain; relation L(a : Value); relation R(a : Value, b : Value);
                relation S(b : Value, c : Value); answer(a, c) : - L(a), R(a, b), S(b, c);
                relational
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
                        L = observed_slice; R = observed_hash; S = observed_sorted; r0 =
                        observed_slice; r1 = observed_hash; r2 = :: cq_example_providers
                        :: borrowed_fields; r3 = observed_sorted; r4 = ::
                        cq_example_providers :: borrowed_fields; r5 = ::
                        cq_example_providers :: borrowed_fields;
                    } indexes { i0 = observed_hash; i1 = observed_sorted; } lowerings
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
                stored L at(__cq_store_L) operations observed_slice stored R
                at(__cq_store_R) operations observed_hash stored S at(__cq_store_S)
                operations observed_sorted stored r0 at(__cq_store_r0) operations
                observed_slice
            } request { input r1 from(__cq_store_R) } }`
    = note: to `:: cq_example_providers :: hash_relation !
            {
                reply_to(observe_reply)
                {
                    callback(:: mini_linq :: resume_physical) kind(1) state
                    {plan
                        {
                            struct PullChain; relation L(a : Value); relation
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
                                    L = observed_slice; R = observed_hash; S = observed_sorted;
                                    r0 = observed_slice; r1 = observed_hash; r2 = ::
                                    cq_example_providers :: borrowed_fields; r3 =
                                    observed_sorted; r4 = :: cq_example_providers ::
                                    borrowed_fields; r5 = :: cq_example_providers ::
                                    borrowed_fields;
                                } indexes { i0 = observed_hash; i1 = observed_sorted; }
                                lowerings
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
                            stored L at(__cq_store_L) operations observed_slice stored R
                            at(__cq_store_R) operations observed_hash stored S
                            at(__cq_store_S) operations observed_sorted stored r0
                            at(__cq_store_r0) operations observed_slice
                        } emitted
                        {
                            let __cq_store_L : & [Value] = L; let __cq_store_R : & :: std
                            :: collections :: HashMap < Value, :: std :: vec :: Vec <
                            Value > > = R; let __cq_store_S : & :: std :: collections ::
                            BTreeMap < Value, :: std :: vec :: Vec < Value > > = S; let
                            __cq_store_r0 : & [Value] = __cq_store_L;
                        } pending { lower r1 }
                        continuations(:: mini_linq :: resume_physical, mini_linq ::
                        access)}
                } plan
                {struct PullChain; relation L(a : Value); relation
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
                            L = observed_slice; R = observed_hash; S = observed_sorted; r0
                            = observed_slice; r1 = observed_hash; r2 = ::
                            cq_example_providers :: borrowed_fields; r3 = observed_sorted;
                            r4 = :: cq_example_providers :: borrowed_fields; r5 = ::
                            cq_example_providers :: borrowed_fields;
                        } indexes { i0 = observed_hash; i1 = observed_sorted; } lowerings
                        {
                            r0 = :: cq_example_providers :: rename; r1 = ::
                            cq_example_providers :: rename; r2 = :: cq_example_providers
                            :: indexed_join; r3 = :: cq_example_providers :: rename; r4 =
                            :: cq_example_providers :: indexed_join; r5 = ::
                            cq_example_providers :: project;
                        }
                    }} environment
                {stored L at(__cq_store_L) operations observed_slice stored R
                    at(__cq_store_R) operations observed_hash stored S at(__cq_store_S)
                    operations observed_sorted stored r0 at(__cq_store_r0) operations
                    observed_slice} request {input r1 from(__cq_store_R)}
            }`
    = note: expanding `observe_reply! { resume
            {
                callback(:: mini_linq :: resume_physical) kind(1) state
                {
                    plan
                    {
                        struct PullChain; relation L(a : Value); relation
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
                                L = observed_slice; R = observed_hash; S = observed_sorted;
                                r0 = observed_slice; r1 = observed_hash; r2 = ::
                                cq_example_providers :: borrowed_fields; r3 =
                                observed_sorted; r4 = :: cq_example_providers ::
                                borrowed_fields; r5 = :: cq_example_providers ::
                                borrowed_fields;
                            } indexes { i0 = observed_hash; i1 = observed_sorted; }
                            lowerings
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
                        stored L at(__cq_store_L) operations observed_slice stored R
                        at(__cq_store_R) operations observed_hash stored S
                        at(__cq_store_S) operations observed_sorted stored r0
                        at(__cq_store_r0) operations observed_slice
                    } emitted
                    {
                        let __cq_store_L : & [Value] = L; let __cq_store_R : & :: std ::
                        collections :: HashMap < Value, :: std :: vec :: Vec < Value > > =
                        R; let __cq_store_S : & :: std :: collections :: BTreeMap < Value,
                        :: std :: vec :: Vec < Value > > = S; let __cq_store_r0 : &
                        [Value] = __cq_store_L;
                    } pending { lower r1 }
                    continuations(:: mini_linq :: resume_physical, mini_linq :: access)
                }
            } emitted
            {
                let __cq_store_r1 : & :: std :: collections :: HashMap < Value, :: std ::
                vec :: Vec < Value >> = __cq_store_R;
            } result { stored r1 at(__cq_store_r1) operations observed_hash } }`
    = note: to `:: mini_linq :: resume_physical !
            {
                resume
                {plan
                    {
                        struct PullChain; relation L(a : Value); relation
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
                                L = observed_slice; R = observed_hash; S = observed_sorted;
                                r0 = observed_slice; r1 = observed_hash; r2 = ::
                                cq_example_providers :: borrowed_fields; r3 =
                                observed_sorted; r4 = :: cq_example_providers ::
                                borrowed_fields; r5 = :: cq_example_providers ::
                                borrowed_fields;
                            } indexes { i0 = observed_hash; i1 = observed_sorted; }
                            lowerings
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
                        stored L at(__cq_store_L) operations observed_slice stored R
                        at(__cq_store_R) operations observed_hash stored S
                        at(__cq_store_S) operations observed_sorted stored r0
                        at(__cq_store_r0) operations observed_slice
                    } emitted
                    {
                        let __cq_store_L : & [Value] = L; let __cq_store_R : & :: std ::
                        collections :: HashMap < Value, :: std :: vec :: Vec < Value > > =
                        R; let __cq_store_S : & :: std :: collections :: BTreeMap < Value,
                        :: std :: vec :: Vec < Value > > = S; let __cq_store_r0 : &
                        [Value] = __cq_store_L;
                    } pending { lower r1 }
                    continuations(:: mini_linq :: resume_physical, mini_linq :: access)}
                emitted
                {let __cq_store_r1 : & :: std :: collections :: HashMap < Value, :: std ::
                    vec :: Vec < Value >> = __cq_store_R;} result
                {stored r1 at(__cq_store_r1) operations observed_hash}
            }`
    = note: expanding `observed_hash! { reply_to(:: mini_linq :: resume_physical)
            {
                plan
                {
                    struct PullChain; relation L(a : Value); relation
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
                            L = observed_slice; R = observed_hash; S = observed_sorted; r0
                            = observed_slice; r1 = observed_hash; r2 = ::
                            cq_example_providers :: borrowed_fields; r3 = observed_sorted;
                            r4 = :: cq_example_providers :: borrowed_fields; r5 = ::
                            cq_example_providers :: borrowed_fields;
                        } indexes { i0 = observed_hash; i1 = observed_sorted; } lowerings
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
                    stored L at(__cq_store_L) operations observed_slice stored R
                    at(__cq_store_R) operations observed_hash stored S at(__cq_store_S)
                    operations observed_sorted stored r0 at(__cq_store_r0) operations
                    observed_slice stored r1 at(__cq_store_r1) operations observed_hash
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
                struct PullChain; relation L(a : Value); relation R(a : Value, b : Value);
                relation S(b : Value, c : Value); answer(a, c) : - L(a), R(a, b), S(b, c);
                relational
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
                        L = observed_slice; R = observed_hash; S = observed_sorted; r0 =
                        observed_slice; r1 = observed_hash; r2 = :: cq_example_providers
                        :: borrowed_fields; r3 = observed_sorted; r4 = ::
                        cq_example_providers :: borrowed_fields; r5 = ::
                        cq_example_providers :: borrowed_fields;
                    } indexes { i0 = observed_hash; i1 = observed_sorted; } lowerings
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
                stored L at(__cq_store_L) operations observed_slice stored R
                at(__cq_store_R) operations observed_hash stored S at(__cq_store_S)
                operations observed_sorted stored r0 at(__cq_store_r0) operations
                observed_slice stored r1 at(__cq_store_r1) operations observed_hash
            } request { index i0 } }`
    = note: to `observe_provider!
            {provider(:: cq_example_providers :: hash_relation) kind(1)
                reply_to(:: mini_linq :: resume_physical)
                {
                    plan
                    {
                        struct PullChain; relation L(a : Value); relation
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
                                L = observed_slice; R = observed_hash; S = observed_sorted;
                                r0 = observed_slice; r1 = observed_hash; r2 = ::
                                cq_example_providers :: borrowed_fields; r3 =
                                observed_sorted; r4 = :: cq_example_providers ::
                                borrowed_fields; r5 = :: cq_example_providers ::
                                borrowed_fields;
                            } indexes { i0 = observed_hash; i1 = observed_sorted; }
                            lowerings
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
                        stored L at(__cq_store_L) operations observed_slice stored R
                        at(__cq_store_R) operations observed_hash stored S
                        at(__cq_store_S) operations observed_sorted stored r0
                        at(__cq_store_r0) operations observed_slice stored r1
                        at(__cq_store_r1) operations observed_hash
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
                    struct PullChain; relation L(a : Value); relation
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
                            L = observed_slice; R = observed_hash; S = observed_sorted; r0
                            = observed_slice; r1 = observed_hash; r2 = ::
                            cq_example_providers :: borrowed_fields; r3 = observed_sorted;
                            r4 = :: cq_example_providers :: borrowed_fields; r5 = ::
                            cq_example_providers :: borrowed_fields;
                        } indexes { i0 = observed_hash; i1 = observed_sorted; } lowerings
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
                    stored L at(__cq_store_L) operations observed_slice stored R
                    at(__cq_store_R) operations observed_hash stored S at(__cq_store_S)
                    operations observed_sorted stored r0 at(__cq_store_r0) operations
                    observed_slice stored r1 at(__cq_store_r1) operations observed_hash
                } request { index i0 }}`
    = note: expanding `observe_provider! { provider(:: cq_example_providers :: hash_relation) kind(1)
            reply_to(:: mini_linq :: resume_physical)
            {
                plan
                {
                    struct PullChain; relation L(a : Value); relation
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
                            L = observed_slice; R = observed_hash; S = observed_sorted; r0
                            = observed_slice; r1 = observed_hash; r2 = ::
                            cq_example_providers :: borrowed_fields; r3 = observed_sorted;
                            r4 = :: cq_example_providers :: borrowed_fields; r5 = ::
                            cq_example_providers :: borrowed_fields;
                        } indexes { i0 = observed_hash; i1 = observed_sorted; } lowerings
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
                    stored L at(__cq_store_L) operations observed_slice stored R
                    at(__cq_store_R) operations observed_hash stored S at(__cq_store_S)
                    operations observed_sorted stored r0 at(__cq_store_r0) operations
                    observed_slice stored r1 at(__cq_store_r1) operations observed_hash
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
                struct PullChain; relation L(a : Value); relation R(a : Value, b : Value);
                relation S(b : Value, c : Value); answer(a, c) : - L(a), R(a, b), S(b, c);
                relational
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
                        L = observed_slice; R = observed_hash; S = observed_sorted; r0 =
                        observed_slice; r1 = observed_hash; r2 = :: cq_example_providers
                        :: borrowed_fields; r3 = observed_sorted; r4 = ::
                        cq_example_providers :: borrowed_fields; r5 = ::
                        cq_example_providers :: borrowed_fields;
                    } indexes { i0 = observed_hash; i1 = observed_sorted; } lowerings
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
                stored L at(__cq_store_L) operations observed_slice stored R
                at(__cq_store_R) operations observed_hash stored S at(__cq_store_S)
                operations observed_sorted stored r0 at(__cq_store_r0) operations
                observed_slice stored r1 at(__cq_store_r1) operations observed_hash
            } request { index i0 } }`
    = note: to `:: cq_example_providers :: hash_relation !
            {
                reply_to(observe_reply)
                {
                    callback(:: mini_linq :: resume_physical) kind(1) state
                    {plan
                        {
                            struct PullChain; relation L(a : Value); relation
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
                                    L = observed_slice; R = observed_hash; S = observed_sorted;
                                    r0 = observed_slice; r1 = observed_hash; r2 = ::
                                    cq_example_providers :: borrowed_fields; r3 =
                                    observed_sorted; r4 = :: cq_example_providers ::
                                    borrowed_fields; r5 = :: cq_example_providers ::
                                    borrowed_fields;
                                } indexes { i0 = observed_hash; i1 = observed_sorted; }
                                lowerings
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
                            stored L at(__cq_store_L) operations observed_slice stored R
                            at(__cq_store_R) operations observed_hash stored S
                            at(__cq_store_S) operations observed_sorted stored r0
                            at(__cq_store_r0) operations observed_slice stored r1
                            at(__cq_store_r1) operations observed_hash
                        } emitted
                        {
                            let __cq_store_L : & [Value] = L; let __cq_store_R : & :: std
                            :: collections :: HashMap < Value, :: std :: vec :: Vec <
                            Value > > = R; let __cq_store_S : & :: std :: collections ::
                            BTreeMap < Value, :: std :: vec :: Vec < Value > > = S; let
                            __cq_store_r0 : & [Value] = __cq_store_L; let __cq_store_r1 :
                            & :: std :: collections :: HashMap < Value, :: std :: vec ::
                            Vec < Value > > = __cq_store_R;
                        } pending { index i0 }
                        continuations(:: mini_linq :: resume_physical, mini_linq ::
                        access)}
                } plan
                {struct PullChain; relation L(a : Value); relation
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
                            L = observed_slice; R = observed_hash; S = observed_sorted; r0
                            = observed_slice; r1 = observed_hash; r2 = ::
                            cq_example_providers :: borrowed_fields; r3 = observed_sorted;
                            r4 = :: cq_example_providers :: borrowed_fields; r5 = ::
                            cq_example_providers :: borrowed_fields;
                        } indexes { i0 = observed_hash; i1 = observed_sorted; } lowerings
                        {
                            r0 = :: cq_example_providers :: rename; r1 = ::
                            cq_example_providers :: rename; r2 = :: cq_example_providers
                            :: indexed_join; r3 = :: cq_example_providers :: rename; r4 =
                            :: cq_example_providers :: indexed_join; r5 = ::
                            cq_example_providers :: project;
                        }
                    }} environment
                {stored L at(__cq_store_L) operations observed_slice stored R
                    at(__cq_store_R) operations observed_hash stored S at(__cq_store_S)
                    operations observed_sorted stored r0 at(__cq_store_r0) operations
                    observed_slice stored r1 at(__cq_store_r1) operations observed_hash}
                request {index i0}
            }`
    = note: expanding `observe_reply! { resume
            {
                callback(:: mini_linq :: resume_physical) kind(1) state
                {
                    plan
                    {
                        struct PullChain; relation L(a : Value); relation
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
                                L = observed_slice; R = observed_hash; S = observed_sorted;
                                r0 = observed_slice; r1 = observed_hash; r2 = ::
                                cq_example_providers :: borrowed_fields; r3 =
                                observed_sorted; r4 = :: cq_example_providers ::
                                borrowed_fields; r5 = :: cq_example_providers ::
                                borrowed_fields;
                            } indexes { i0 = observed_hash; i1 = observed_sorted; }
                            lowerings
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
                        stored L at(__cq_store_L) operations observed_slice stored R
                        at(__cq_store_R) operations observed_hash stored S
                        at(__cq_store_S) operations observed_sorted stored r0
                        at(__cq_store_r0) operations observed_slice stored r1
                        at(__cq_store_r1) operations observed_hash
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
                }
            } emitted
            {
                let __cq_index_i0 : & :: std :: collections :: HashMap < Value, :: std ::
                vec :: Vec < Value >> = __cq_store_r1;
            } result { indexed i0 at(__cq_index_i0) operations observed_hash } }`
    = note: to `:: mini_linq :: resume_physical !
            {
                resume
                {plan
                    {
                        struct PullChain; relation L(a : Value); relation
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
                                L = observed_slice; R = observed_hash; S = observed_sorted;
                                r0 = observed_slice; r1 = observed_hash; r2 = ::
                                cq_example_providers :: borrowed_fields; r3 =
                                observed_sorted; r4 = :: cq_example_providers ::
                                borrowed_fields; r5 = :: cq_example_providers ::
                                borrowed_fields;
                            } indexes { i0 = observed_hash; i1 = observed_sorted; }
                            lowerings
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
                        stored L at(__cq_store_L) operations observed_slice stored R
                        at(__cq_store_R) operations observed_hash stored S
                        at(__cq_store_S) operations observed_sorted stored r0
                        at(__cq_store_r0) operations observed_slice stored r1
                        at(__cq_store_r1) operations observed_hash
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
                    continuations(:: mini_linq :: resume_physical, mini_linq :: access)}
                emitted
                {let __cq_index_i0 : & :: std :: collections :: HashMap < Value, :: std ::
                    vec :: Vec < Value >> = __cq_store_r1;} result
                {indexed i0 at(__cq_index_i0) operations observed_hash}
            }`
    = note: expanding `observed_slice! { reply_to(:: cq_example_providers :: indexed_join_rows)
            {
                reply_to(:: mini_linq :: resume_physical)
                {
                    plan
                    {
                        struct PullChain; relation L(a : Value); relation
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
                                L = observed_slice; R = observed_hash; S = observed_sorted;
                                r0 = observed_slice; r1 = observed_hash; r2 = ::
                                cq_example_providers :: borrowed_fields; r3 =
                                observed_sorted; r4 = :: cq_example_providers ::
                                borrowed_fields; r5 = :: cq_example_providers ::
                                borrowed_fields;
                            } indexes { i0 = observed_hash; i1 = observed_sorted; }
                            lowerings
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
                        stored L at(__cq_store_L) operations observed_slice stored R
                        at(__cq_store_R) operations observed_hash stored S
                        at(__cq_store_S) operations observed_sorted stored r0
                        at(__cq_store_r0) operations observed_slice stored r1
                        at(__cq_store_r1) operations observed_hash indexed i0
                        at(__cq_index_i0) operations observed_hash
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
                    struct PullChain; relation L(a : Value); relation
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
                            L = observed_slice; R = observed_hash; S = observed_sorted; r0
                            = observed_slice; r1 = observed_hash; r2 = ::
                            cq_example_providers :: borrowed_fields; r3 = observed_sorted;
                            r4 = :: cq_example_providers :: borrowed_fields; r5 = ::
                            cq_example_providers :: borrowed_fields;
                        } indexes { i0 = observed_hash; i1 = observed_sorted; } lowerings
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
                    stored L at(__cq_store_L) operations observed_slice stored R
                    at(__cq_store_R) operations observed_hash stored S at(__cq_store_S)
                    operations observed_sorted stored r0 at(__cq_store_r0) operations
                    observed_slice stored r1 at(__cq_store_r1) operations observed_hash
                    indexed i0 at(__cq_index_i0) operations observed_hash
                } request { lower r2 }
            } plan
            {
                struct PullChain; relation L(a : Value); relation R(a : Value, b : Value);
                relation S(b : Value, c : Value); answer(a, c) : - L(a), R(a, b), S(b, c);
                relational
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
                        L = observed_slice; R = observed_hash; S = observed_sorted; r0 =
                        observed_slice; r1 = observed_hash; r2 = :: cq_example_providers
                        :: borrowed_fields; r3 = observed_sorted; r4 = ::
                        cq_example_providers :: borrowed_fields; r5 = ::
                        cq_example_providers :: borrowed_fields;
                    } indexes { i0 = observed_hash; i1 = observed_sorted; } lowerings
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
                stored L at(__cq_store_L) operations observed_slice stored R
                at(__cq_store_R) operations observed_hash stored S at(__cq_store_S)
                operations observed_sorted stored r0 at(__cq_store_r0) operations
                observed_slice stored r1 at(__cq_store_r1) operations observed_hash
                indexed i0 at(__cq_index_i0) operations observed_hash
            } request { rows r0 } }`
    = note: to `observe_provider!
            {provider(:: cq_example_providers :: slice_rows) kind(0)
                reply_to(:: cq_example_providers :: indexed_join_rows)
                {
                    reply_to(:: mini_linq :: resume_physical)
                    {
                        plan
                        {
                            struct PullChain; relation L(a : Value); relation
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
                                    L = observed_slice; R = observed_hash; S = observed_sorted;
                                    r0 = observed_slice; r1 = observed_hash; r2 = ::
                                    cq_example_providers :: borrowed_fields; r3 =
                                    observed_sorted; r4 = :: cq_example_providers ::
                                    borrowed_fields; r5 = :: cq_example_providers ::
                                    borrowed_fields;
                                } indexes { i0 = observed_hash; i1 = observed_sorted; }
                                lowerings
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
                            stored L at(__cq_store_L) operations observed_slice stored R
                            at(__cq_store_R) operations observed_hash stored S
                            at(__cq_store_S) operations observed_sorted stored r0
                            at(__cq_store_r0) operations observed_slice stored r1
                            at(__cq_store_r1) operations observed_hash indexed i0
                            at(__cq_index_i0) operations observed_hash
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
                        struct PullChain; relation L(a : Value); relation
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
                                L = observed_slice; R = observed_hash; S = observed_sorted;
                                r0 = observed_slice; r1 = observed_hash; r2 = ::
                                cq_example_providers :: borrowed_fields; r3 =
                                observed_sorted; r4 = :: cq_example_providers ::
                                borrowed_fields; r5 = :: cq_example_providers ::
                                borrowed_fields;
                            } indexes { i0 = observed_hash; i1 = observed_sorted; }
                            lowerings
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
                        stored L at(__cq_store_L) operations observed_slice stored R
                        at(__cq_store_R) operations observed_hash stored S
                        at(__cq_store_S) operations observed_sorted stored r0
                        at(__cq_store_r0) operations observed_slice stored r1
                        at(__cq_store_r1) operations observed_hash indexed i0
                        at(__cq_index_i0) operations observed_hash
                    } request { lower r2 }
                } plan
                {
                    struct PullChain; relation L(a : Value); relation
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
                            L = observed_slice; R = observed_hash; S = observed_sorted; r0
                            = observed_slice; r1 = observed_hash; r2 = ::
                            cq_example_providers :: borrowed_fields; r3 = observed_sorted;
                            r4 = :: cq_example_providers :: borrowed_fields; r5 = ::
                            cq_example_providers :: borrowed_fields;
                        } indexes { i0 = observed_hash; i1 = observed_sorted; } lowerings
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
                    stored L at(__cq_store_L) operations observed_slice stored R
                    at(__cq_store_R) operations observed_hash stored S at(__cq_store_S)
                    operations observed_sorted stored r0 at(__cq_store_r0) operations
                    observed_slice stored r1 at(__cq_store_r1) operations observed_hash
                    indexed i0 at(__cq_index_i0) operations observed_hash
                } request { rows r0 }}`
    = note: expanding `observe_provider! { provider(:: cq_example_providers :: slice_rows) kind(0)
            reply_to(:: cq_example_providers :: indexed_join_rows)
            {
                reply_to(:: mini_linq :: resume_physical)
                {
                    plan
                    {
                        struct PullChain; relation L(a : Value); relation
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
                                L = observed_slice; R = observed_hash; S = observed_sorted;
                                r0 = observed_slice; r1 = observed_hash; r2 = ::
                                cq_example_providers :: borrowed_fields; r3 =
                                observed_sorted; r4 = :: cq_example_providers ::
                                borrowed_fields; r5 = :: cq_example_providers ::
                                borrowed_fields;
                            } indexes { i0 = observed_hash; i1 = observed_sorted; }
                            lowerings
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
                        stored L at(__cq_store_L) operations observed_slice stored R
                        at(__cq_store_R) operations observed_hash stored S
                        at(__cq_store_S) operations observed_sorted stored r0
                        at(__cq_store_r0) operations observed_slice stored r1
                        at(__cq_store_r1) operations observed_hash indexed i0
                        at(__cq_index_i0) operations observed_hash
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
                    struct PullChain; relation L(a : Value); relation
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
                            L = observed_slice; R = observed_hash; S = observed_sorted; r0
                            = observed_slice; r1 = observed_hash; r2 = ::
                            cq_example_providers :: borrowed_fields; r3 = observed_sorted;
                            r4 = :: cq_example_providers :: borrowed_fields; r5 = ::
                            cq_example_providers :: borrowed_fields;
                        } indexes { i0 = observed_hash; i1 = observed_sorted; } lowerings
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
                    stored L at(__cq_store_L) operations observed_slice stored R
                    at(__cq_store_R) operations observed_hash stored S at(__cq_store_S)
                    operations observed_sorted stored r0 at(__cq_store_r0) operations
                    observed_slice stored r1 at(__cq_store_r1) operations observed_hash
                    indexed i0 at(__cq_index_i0) operations observed_hash
                } request { lower r2 }
            } plan
            {
                struct PullChain; relation L(a : Value); relation R(a : Value, b : Value);
                relation S(b : Value, c : Value); answer(a, c) : - L(a), R(a, b), S(b, c);
                relational
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
                        L = observed_slice; R = observed_hash; S = observed_sorted; r0 =
                        observed_slice; r1 = observed_hash; r2 = :: cq_example_providers
                        :: borrowed_fields; r3 = observed_sorted; r4 = ::
                        cq_example_providers :: borrowed_fields; r5 = ::
                        cq_example_providers :: borrowed_fields;
                    } indexes { i0 = observed_hash; i1 = observed_sorted; } lowerings
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
                stored L at(__cq_store_L) operations observed_slice stored R
                at(__cq_store_R) operations observed_hash stored S at(__cq_store_S)
                operations observed_sorted stored r0 at(__cq_store_r0) operations
                observed_slice stored r1 at(__cq_store_r1) operations observed_hash
                indexed i0 at(__cq_index_i0) operations observed_hash
            } request { rows r0 } }`
    = note: to `:: cq_example_providers :: slice_rows !
            {
                reply_to(observe_reply)
                {
                    callback(:: cq_example_providers :: indexed_join_rows) kind(0) state
                    {reply_to(:: mini_linq :: resume_physical)
                        {
                            plan
                            {
                                struct PullChain; relation L(a : Value); relation
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
                                        L = observed_slice; R = observed_hash; S = observed_sorted;
                                        r0 = observed_slice; r1 = observed_hash; r2 = ::
                                        cq_example_providers :: borrowed_fields; r3 =
                                        observed_sorted; r4 = :: cq_example_providers ::
                                        borrowed_fields; r5 = :: cq_example_providers ::
                                        borrowed_fields;
                                    } indexes { i0 = observed_hash; i1 = observed_sorted; }
                                    lowerings
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
                                stored L at(__cq_store_L) operations observed_slice stored R
                                at(__cq_store_R) operations observed_hash stored S
                                at(__cq_store_S) operations observed_sorted stored r0
                                at(__cq_store_r0) operations observed_slice stored r1
                                at(__cq_store_r1) operations observed_hash indexed i0
                                at(__cq_index_i0) operations observed_hash
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
                            struct PullChain; relation L(a : Value); relation
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
                                    L = observed_slice; R = observed_hash; S = observed_sorted;
                                    r0 = observed_slice; r1 = observed_hash; r2 = ::
                                    cq_example_providers :: borrowed_fields; r3 =
                                    observed_sorted; r4 = :: cq_example_providers ::
                                    borrowed_fields; r5 = :: cq_example_providers ::
                                    borrowed_fields;
                                } indexes { i0 = observed_hash; i1 = observed_sorted; }
                                lowerings
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
                            stored L at(__cq_store_L) operations observed_slice stored R
                            at(__cq_store_R) operations observed_hash stored S
                            at(__cq_store_S) operations observed_sorted stored r0
                            at(__cq_store_r0) operations observed_slice stored r1
                            at(__cq_store_r1) operations observed_hash indexed i0
                            at(__cq_index_i0) operations observed_hash
                        } request { lower r2 }}
                } plan
                {struct PullChain; relation L(a : Value); relation
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
                            L = observed_slice; R = observed_hash; S = observed_sorted; r0
                            = observed_slice; r1 = observed_hash; r2 = ::
                            cq_example_providers :: borrowed_fields; r3 = observed_sorted;
                            r4 = :: cq_example_providers :: borrowed_fields; r5 = ::
                            cq_example_providers :: borrowed_fields;
                        } indexes { i0 = observed_hash; i1 = observed_sorted; } lowerings
                        {
                            r0 = :: cq_example_providers :: rename; r1 = ::
                            cq_example_providers :: rename; r2 = :: cq_example_providers
                            :: indexed_join; r3 = :: cq_example_providers :: rename; r4 =
                            :: cq_example_providers :: indexed_join; r5 = ::
                            cq_example_providers :: project;
                        }
                    }} environment
                {stored L at(__cq_store_L) operations observed_slice stored R
                    at(__cq_store_R) operations observed_hash stored S at(__cq_store_S)
                    operations observed_sorted stored r0 at(__cq_store_r0) operations
                    observed_slice stored r1 at(__cq_store_r1) operations observed_hash
                    indexed i0 at(__cq_index_i0) operations observed_hash} request
                {rows r0}
            }`
    = note: expanding `observe_reply! { resume
            {
                callback(:: cq_example_providers :: indexed_join_rows) kind(0) state
                {
                    reply_to(:: mini_linq :: resume_physical)
                    {
                        plan
                        {
                            struct PullChain; relation L(a : Value); relation
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
                                    L = observed_slice; R = observed_hash; S = observed_sorted;
                                    r0 = observed_slice; r1 = observed_hash; r2 = ::
                                    cq_example_providers :: borrowed_fields; r3 =
                                    observed_sorted; r4 = :: cq_example_providers ::
                                    borrowed_fields; r5 = :: cq_example_providers ::
                                    borrowed_fields;
                                } indexes { i0 = observed_hash; i1 = observed_sorted; }
                                lowerings
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
                            stored L at(__cq_store_L) operations observed_slice stored R
                            at(__cq_store_R) operations observed_hash stored S
                            at(__cq_store_S) operations observed_sorted stored r0
                            at(__cq_store_r0) operations observed_slice stored r1
                            at(__cq_store_r1) operations observed_hash indexed i0
                            at(__cq_index_i0) operations observed_hash
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
                        struct PullChain; relation L(a : Value); relation
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
                                L = observed_slice; R = observed_hash; S = observed_sorted;
                                r0 = observed_slice; r1 = observed_hash; r2 = ::
                                cq_example_providers :: borrowed_fields; r3 =
                                observed_sorted; r4 = :: cq_example_providers ::
                                borrowed_fields; r5 = :: cq_example_providers ::
                                borrowed_fields;
                            } indexes { i0 = observed_hash; i1 = observed_sorted; }
                            lowerings
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
                        stored L at(__cq_store_L) operations observed_slice stored R
                        at(__cq_store_R) operations observed_hash stored S
                        at(__cq_store_S) operations observed_sorted stored r0
                        at(__cq_store_r0) operations observed_slice stored r1
                        at(__cq_store_r1) operations observed_hash indexed i0
                        at(__cq_index_i0) operations observed_hash
                    } request { lower r2 }
                }
            } source((__cq_store_r0).iter()) item(a) witness
            { row(a) fields { 0 = (a); } } }`
    = note: to `:: cq_example_providers :: indexed_join_rows !
            {
                resume
                {reply_to(:: mini_linq :: resume_physical)
                    {
                        plan
                        {
                            struct PullChain; relation L(a : Value); relation
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
                                    L = observed_slice; R = observed_hash; S = observed_sorted;
                                    r0 = observed_slice; r1 = observed_hash; r2 = ::
                                    cq_example_providers :: borrowed_fields; r3 =
                                    observed_sorted; r4 = :: cq_example_providers ::
                                    borrowed_fields; r5 = :: cq_example_providers ::
                                    borrowed_fields;
                                } indexes { i0 = observed_hash; i1 = observed_sorted; }
                                lowerings
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
                            stored L at(__cq_store_L) operations observed_slice stored R
                            at(__cq_store_R) operations observed_hash stored S
                            at(__cq_store_S) operations observed_sorted stored r0
                            at(__cq_store_r0) operations observed_slice stored r1
                            at(__cq_store_r1) operations observed_hash indexed i0
                            at(__cq_index_i0) operations observed_hash
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
                        struct PullChain; relation L(a : Value); relation
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
                                L = observed_slice; R = observed_hash; S = observed_sorted;
                                r0 = observed_slice; r1 = observed_hash; r2 = ::
                                cq_example_providers :: borrowed_fields; r3 =
                                observed_sorted; r4 = :: cq_example_providers ::
                                borrowed_fields; r5 = :: cq_example_providers ::
                                borrowed_fields;
                            } indexes { i0 = observed_hash; i1 = observed_sorted; }
                            lowerings
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
                        stored L at(__cq_store_L) operations observed_slice stored R
                        at(__cq_store_R) operations observed_hash stored S
                        at(__cq_store_S) operations observed_sorted stored r0
                        at(__cq_store_r0) operations observed_slice stored r1
                        at(__cq_store_r1) operations observed_hash indexed i0
                        at(__cq_index_i0) operations observed_hash
                    } request { lower r2 }} source(observe(0, (__cq_store_r0).iter()))
                item(a) witness {row(a) fields { 0 = (a); }}
            }`
    = note: expanding `observed_hash! { reply_to(:: cq_example_providers :: indexed_join_matches)
            {
                request
                {
                    reply_to(:: mini_linq :: resume_physical)
                    {
                        plan
                        {
                            struct PullChain; relation L(a : Value); relation
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
                                    L = observed_slice; R = observed_hash; S = observed_sorted;
                                    r0 = observed_slice; r1 = observed_hash; r2 = ::
                                    cq_example_providers :: borrowed_fields; r3 =
                                    observed_sorted; r4 = :: cq_example_providers ::
                                    borrowed_fields; r5 = :: cq_example_providers ::
                                    borrowed_fields;
                                } indexes { i0 = observed_hash; i1 = observed_sorted; }
                                lowerings
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
                            stored L at(__cq_store_L) operations observed_slice stored R
                            at(__cq_store_R) operations observed_hash stored S
                            at(__cq_store_S) operations observed_sorted stored r0
                            at(__cq_store_r0) operations observed_slice stored r1
                            at(__cq_store_r1) operations observed_hash indexed i0
                            at(__cq_index_i0) operations observed_hash
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
                        struct PullChain; relation L(a : Value); relation
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
                                L = observed_slice; R = observed_hash; S = observed_sorted;
                                r0 = observed_slice; r1 = observed_hash; r2 = ::
                                cq_example_providers :: borrowed_fields; r3 =
                                observed_sorted; r4 = :: cq_example_providers ::
                                borrowed_fields; r5 = :: cq_example_providers ::
                                borrowed_fields;
                            } indexes { i0 = observed_hash; i1 = observed_sorted; }
                            lowerings
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
                        stored L at(__cq_store_L) operations observed_slice stored R
                        at(__cq_store_R) operations observed_hash stored S
                        at(__cq_store_S) operations observed_sorted stored r0
                        at(__cq_store_r0) operations observed_slice stored r1
                        at(__cq_store_r1) operations observed_hash indexed i0
                        at(__cq_index_i0) operations observed_hash
                    } request { lower r2 }
                } access { r0(a) => bind a <- (observe(0, (__cq_store_r0).iter())) }
                witness { row(a) fields { 0 = (a); } }
            } plan
            {
                struct PullChain; relation L(a : Value); relation R(a : Value, b : Value);
                relation S(b : Value, c : Value); answer(a, c) : - L(a), R(a, b), S(b, c);
                relational
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
                        L = observed_slice; R = observed_hash; S = observed_sorted; r0 =
                        observed_slice; r1 = observed_hash; r2 = :: cq_example_providers
                        :: borrowed_fields; r3 = observed_sorted; r4 = ::
                        cq_example_providers :: borrowed_fields; r5 = ::
                        cq_example_providers :: borrowed_fields;
                    } indexes { i0 = observed_hash; i1 = observed_sorted; } lowerings
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
                stored L at(__cq_store_L) operations observed_slice stored R
                at(__cq_store_R) operations observed_hash stored S at(__cq_store_S)
                operations observed_sorted stored r0 at(__cq_store_r0) operations
                observed_slice stored r1 at(__cq_store_r1) operations observed_hash
                indexed i0 at(__cq_index_i0) operations observed_hash
            } request { matches i0 values [(a)] } }`
    = note: to `observe_provider!
            {provider(:: cq_example_providers :: hash_relation) kind(1)
                reply_to(:: cq_example_providers :: indexed_join_matches)
                {
                    request
                    {
                        reply_to(:: mini_linq :: resume_physical)
                        {
                            plan
                            {
                                struct PullChain; relation L(a : Value); relation
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
                                        L = observed_slice; R = observed_hash; S = observed_sorted;
                                        r0 = observed_slice; r1 = observed_hash; r2 = ::
                                        cq_example_providers :: borrowed_fields; r3 =
                                        observed_sorted; r4 = :: cq_example_providers ::
                                        borrowed_fields; r5 = :: cq_example_providers ::
                                        borrowed_fields;
                                    } indexes { i0 = observed_hash; i1 = observed_sorted; }
                                    lowerings
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
                                stored L at(__cq_store_L) operations observed_slice stored R
                                at(__cq_store_R) operations observed_hash stored S
                                at(__cq_store_S) operations observed_sorted stored r0
                                at(__cq_store_r0) operations observed_slice stored r1
                                at(__cq_store_r1) operations observed_hash indexed i0
                                at(__cq_index_i0) operations observed_hash
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
                            struct PullChain; relation L(a : Value); relation
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
                                    L = observed_slice; R = observed_hash; S = observed_sorted;
                                    r0 = observed_slice; r1 = observed_hash; r2 = ::
                                    cq_example_providers :: borrowed_fields; r3 =
                                    observed_sorted; r4 = :: cq_example_providers ::
                                    borrowed_fields; r5 = :: cq_example_providers ::
                                    borrowed_fields;
                                } indexes { i0 = observed_hash; i1 = observed_sorted; }
                                lowerings
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
                            stored L at(__cq_store_L) operations observed_slice stored R
                            at(__cq_store_R) operations observed_hash stored S
                            at(__cq_store_S) operations observed_sorted stored r0
                            at(__cq_store_r0) operations observed_slice stored r1
                            at(__cq_store_r1) operations observed_hash indexed i0
                            at(__cq_index_i0) operations observed_hash
                        } request { lower r2 }
                    } access { r0(a) => bind a <- (observe(0, (__cq_store_r0).iter())) }
                    witness { row(a) fields { 0 = (a); } }
                } plan
                {
                    struct PullChain; relation L(a : Value); relation
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
                            L = observed_slice; R = observed_hash; S = observed_sorted; r0
                            = observed_slice; r1 = observed_hash; r2 = ::
                            cq_example_providers :: borrowed_fields; r3 = observed_sorted;
                            r4 = :: cq_example_providers :: borrowed_fields; r5 = ::
                            cq_example_providers :: borrowed_fields;
                        } indexes { i0 = observed_hash; i1 = observed_sorted; } lowerings
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
                    stored L at(__cq_store_L) operations observed_slice stored R
                    at(__cq_store_R) operations observed_hash stored S at(__cq_store_S)
                    operations observed_sorted stored r0 at(__cq_store_r0) operations
                    observed_slice stored r1 at(__cq_store_r1) operations observed_hash
                    indexed i0 at(__cq_index_i0) operations observed_hash
                } request { matches i0 values [(a)] }}`
    = note: expanding `observe_provider! { provider(:: cq_example_providers :: hash_relation) kind(1)
            reply_to(:: cq_example_providers :: indexed_join_matches)
            {
                request
                {
                    reply_to(:: mini_linq :: resume_physical)
                    {
                        plan
                        {
                            struct PullChain; relation L(a : Value); relation
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
                                    L = observed_slice; R = observed_hash; S = observed_sorted;
                                    r0 = observed_slice; r1 = observed_hash; r2 = ::
                                    cq_example_providers :: borrowed_fields; r3 =
                                    observed_sorted; r4 = :: cq_example_providers ::
                                    borrowed_fields; r5 = :: cq_example_providers ::
                                    borrowed_fields;
                                } indexes { i0 = observed_hash; i1 = observed_sorted; }
                                lowerings
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
                            stored L at(__cq_store_L) operations observed_slice stored R
                            at(__cq_store_R) operations observed_hash stored S
                            at(__cq_store_S) operations observed_sorted stored r0
                            at(__cq_store_r0) operations observed_slice stored r1
                            at(__cq_store_r1) operations observed_hash indexed i0
                            at(__cq_index_i0) operations observed_hash
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
                        struct PullChain; relation L(a : Value); relation
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
                                L = observed_slice; R = observed_hash; S = observed_sorted;
                                r0 = observed_slice; r1 = observed_hash; r2 = ::
                                cq_example_providers :: borrowed_fields; r3 =
                                observed_sorted; r4 = :: cq_example_providers ::
                                borrowed_fields; r5 = :: cq_example_providers ::
                                borrowed_fields;
                            } indexes { i0 = observed_hash; i1 = observed_sorted; }
                            lowerings
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
                        stored L at(__cq_store_L) operations observed_slice stored R
                        at(__cq_store_R) operations observed_hash stored S
                        at(__cq_store_S) operations observed_sorted stored r0
                        at(__cq_store_r0) operations observed_slice stored r1
                        at(__cq_store_r1) operations observed_hash indexed i0
                        at(__cq_index_i0) operations observed_hash
                    } request { lower r2 }
                } access { r0(a) => bind a <- (observe(0, (__cq_store_r0).iter())) }
                witness { row(a) fields { 0 = (a); } }
            } plan
            {
                struct PullChain; relation L(a : Value); relation R(a : Value, b : Value);
                relation S(b : Value, c : Value); answer(a, c) : - L(a), R(a, b), S(b, c);
                relational
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
                        L = observed_slice; R = observed_hash; S = observed_sorted; r0 =
                        observed_slice; r1 = observed_hash; r2 = :: cq_example_providers
                        :: borrowed_fields; r3 = observed_sorted; r4 = ::
                        cq_example_providers :: borrowed_fields; r5 = ::
                        cq_example_providers :: borrowed_fields;
                    } indexes { i0 = observed_hash; i1 = observed_sorted; } lowerings
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
                stored L at(__cq_store_L) operations observed_slice stored R
                at(__cq_store_R) operations observed_hash stored S at(__cq_store_S)
                operations observed_sorted stored r0 at(__cq_store_r0) operations
                observed_slice stored r1 at(__cq_store_r1) operations observed_hash
                indexed i0 at(__cq_index_i0) operations observed_hash
            } request { matches i0 values [(a)] } }`
    = note: to `:: cq_example_providers :: hash_relation !
            {
                reply_to(observe_reply)
                {
                    callback(:: cq_example_providers :: indexed_join_matches) kind(1)
                    state
                    {request
                        {
                            reply_to(:: mini_linq :: resume_physical)
                            {
                                plan
                                {
                                    struct PullChain; relation L(a : Value); relation
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
                                            L = observed_slice; R = observed_hash; S = observed_sorted;
                                            r0 = observed_slice; r1 = observed_hash; r2 = ::
                                            cq_example_providers :: borrowed_fields; r3 =
                                            observed_sorted; r4 = :: cq_example_providers ::
                                            borrowed_fields; r5 = :: cq_example_providers ::
                                            borrowed_fields;
                                        } indexes { i0 = observed_hash; i1 = observed_sorted; }
                                        lowerings
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
                                    stored L at(__cq_store_L) operations observed_slice stored R
                                    at(__cq_store_R) operations observed_hash stored S
                                    at(__cq_store_S) operations observed_sorted stored r0
                                    at(__cq_store_r0) operations observed_slice stored r1
                                    at(__cq_store_r1) operations observed_hash indexed i0
                                    at(__cq_index_i0) operations observed_hash
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
                                struct PullChain; relation L(a : Value); relation
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
                                        L = observed_slice; R = observed_hash; S = observed_sorted;
                                        r0 = observed_slice; r1 = observed_hash; r2 = ::
                                        cq_example_providers :: borrowed_fields; r3 =
                                        observed_sorted; r4 = :: cq_example_providers ::
                                        borrowed_fields; r5 = :: cq_example_providers ::
                                        borrowed_fields;
                                    } indexes { i0 = observed_hash; i1 = observed_sorted; }
                                    lowerings
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
                                stored L at(__cq_store_L) operations observed_slice stored R
                                at(__cq_store_R) operations observed_hash stored S
                                at(__cq_store_S) operations observed_sorted stored r0
                                at(__cq_store_r0) operations observed_slice stored r1
                                at(__cq_store_r1) operations observed_hash indexed i0
                                at(__cq_index_i0) operations observed_hash
                            } request { lower r2 }
                        } access
                        { r0(a) => bind a <- (observe(0, (__cq_store_r0).iter())) }
                        witness { row(a) fields { 0 = (a); } }}
                } plan
                {struct PullChain; relation L(a : Value); relation
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
                            L = observed_slice; R = observed_hash; S = observed_sorted; r0
                            = observed_slice; r1 = observed_hash; r2 = ::
                            cq_example_providers :: borrowed_fields; r3 = observed_sorted;
                            r4 = :: cq_example_providers :: borrowed_fields; r5 = ::
                            cq_example_providers :: borrowed_fields;
                        } indexes { i0 = observed_hash; i1 = observed_sorted; } lowerings
                        {
                            r0 = :: cq_example_providers :: rename; r1 = ::
                            cq_example_providers :: rename; r2 = :: cq_example_providers
                            :: indexed_join; r3 = :: cq_example_providers :: rename; r4 =
                            :: cq_example_providers :: indexed_join; r5 = ::
                            cq_example_providers :: project;
                        }
                    }} environment
                {stored L at(__cq_store_L) operations observed_slice stored R
                    at(__cq_store_R) operations observed_hash stored S at(__cq_store_S)
                    operations observed_sorted stored r0 at(__cq_store_r0) operations
                    observed_slice stored r1 at(__cq_store_r1) operations observed_hash
                    indexed i0 at(__cq_index_i0) operations observed_hash} request
                {matches i0 values [(a)]}
            }`
    = note: expanding `observe_reply! { resume
            {
                callback(:: cq_example_providers :: indexed_join_matches) kind(1) state
                {
                    request
                    {
                        reply_to(:: mini_linq :: resume_physical)
                        {
                            plan
                            {
                                struct PullChain; relation L(a : Value); relation
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
                                        L = observed_slice; R = observed_hash; S = observed_sorted;
                                        r0 = observed_slice; r1 = observed_hash; r2 = ::
                                        cq_example_providers :: borrowed_fields; r3 =
                                        observed_sorted; r4 = :: cq_example_providers ::
                                        borrowed_fields; r5 = :: cq_example_providers ::
                                        borrowed_fields;
                                    } indexes { i0 = observed_hash; i1 = observed_sorted; }
                                    lowerings
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
                                stored L at(__cq_store_L) operations observed_slice stored R
                                at(__cq_store_R) operations observed_hash stored S
                                at(__cq_store_S) operations observed_sorted stored r0
                                at(__cq_store_r0) operations observed_slice stored r1
                                at(__cq_store_r1) operations observed_hash indexed i0
                                at(__cq_index_i0) operations observed_hash
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
                            struct PullChain; relation L(a : Value); relation
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
                                    L = observed_slice; R = observed_hash; S = observed_sorted;
                                    r0 = observed_slice; r1 = observed_hash; r2 = ::
                                    cq_example_providers :: borrowed_fields; r3 =
                                    observed_sorted; r4 = :: cq_example_providers ::
                                    borrowed_fields; r5 = :: cq_example_providers ::
                                    borrowed_fields;
                                } indexes { i0 = observed_hash; i1 = observed_sorted; }
                                lowerings
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
                            stored L at(__cq_store_L) operations observed_slice stored R
                            at(__cq_store_R) operations observed_hash stored S
                            at(__cq_store_S) operations observed_sorted stored r0
                            at(__cq_store_r0) operations observed_slice stored r1
                            at(__cq_store_r1) operations observed_hash indexed i0
                            at(__cq_index_i0) operations observed_hash
                        } request { lower r2 }
                    } access { r0(a) => bind a <- (observe(0, (__cq_store_r0).iter())) }
                    witness { row(a) fields { 0 = (a); } }
                }
            } source((__cq_index_i0).get(a).into_iter().flatten()) item(b) witness
            { row((a, b,)) fields { 0 = (a); 1 = (b); } } }`
    = note: to `:: cq_example_providers :: indexed_join_matches !
            {
                resume
                {request
                    {
                        reply_to(:: mini_linq :: resume_physical)
                        {
                            plan
                            {
                                struct PullChain; relation L(a : Value); relation
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
                                        L = observed_slice; R = observed_hash; S = observed_sorted;
                                        r0 = observed_slice; r1 = observed_hash; r2 = ::
                                        cq_example_providers :: borrowed_fields; r3 =
                                        observed_sorted; r4 = :: cq_example_providers ::
                                        borrowed_fields; r5 = :: cq_example_providers ::
                                        borrowed_fields;
                                    } indexes { i0 = observed_hash; i1 = observed_sorted; }
                                    lowerings
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
                                stored L at(__cq_store_L) operations observed_slice stored R
                                at(__cq_store_R) operations observed_hash stored S
                                at(__cq_store_S) operations observed_sorted stored r0
                                at(__cq_store_r0) operations observed_slice stored r1
                                at(__cq_store_r1) operations observed_hash indexed i0
                                at(__cq_index_i0) operations observed_hash
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
                            struct PullChain; relation L(a : Value); relation
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
                                    L = observed_slice; R = observed_hash; S = observed_sorted;
                                    r0 = observed_slice; r1 = observed_hash; r2 = ::
                                    cq_example_providers :: borrowed_fields; r3 =
                                    observed_sorted; r4 = :: cq_example_providers ::
                                    borrowed_fields; r5 = :: cq_example_providers ::
                                    borrowed_fields;
                                } indexes { i0 = observed_hash; i1 = observed_sorted; }
                                lowerings
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
                            stored L at(__cq_store_L) operations observed_slice stored R
                            at(__cq_store_R) operations observed_hash stored S
                            at(__cq_store_S) operations observed_sorted stored r0
                            at(__cq_store_r0) operations observed_slice stored r1
                            at(__cq_store_r1) operations observed_hash indexed i0
                            at(__cq_index_i0) operations observed_hash
                        } request { lower r2 }
                    } access { r0(a) => bind a <- (observe(0, (__cq_store_r0).iter())) }
                    witness { row(a) fields { 0 = (a); } }}
                source(observe(1, (__cq_index_i0).get(a).into_iter().flatten())) item(b)
                witness {row((a, b,)) fields { 0 = (a); 1 = (b); }}
            }`
    = note: expanding `observed_sorted! { reply_to(:: mini_linq :: resume_physical)
            {
                plan
                {
                    struct PullChain; relation L(a : Value); relation
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
                            L = observed_slice; R = observed_hash; S = observed_sorted; r0
                            = observed_slice; r1 = observed_hash; r2 = ::
                            cq_example_providers :: borrowed_fields; r3 = observed_sorted;
                            r4 = :: cq_example_providers :: borrowed_fields; r5 = ::
                            cq_example_providers :: borrowed_fields;
                        } indexes { i0 = observed_hash; i1 = observed_sorted; } lowerings
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
                    stored L at(__cq_store_L) operations observed_slice stored R
                    at(__cq_store_R) operations observed_hash stored S at(__cq_store_S)
                    operations observed_sorted stored r0 at(__cq_store_r0) operations
                    observed_slice stored r1 at(__cq_store_r1) operations observed_hash
                    indexed i0 at(__cq_index_i0) operations observed_hash region r2
                    {
                        r2(a, b) => ((a, b,)) : - r0(a) => bind a <-
                        (observe(0, (__cq_store_r0).iter())), r1(a, b) => bind b <-
                        (observe(1, (__cq_index_i0).get(a).into_iter().flatten()));
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
                struct PullChain; relation L(a : Value); relation R(a : Value, b : Value);
                relation S(b : Value, c : Value); answer(a, c) : - L(a), R(a, b), S(b, c);
                relational
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
                        L = observed_slice; R = observed_hash; S = observed_sorted; r0 =
                        observed_slice; r1 = observed_hash; r2 = :: cq_example_providers
                        :: borrowed_fields; r3 = observed_sorted; r4 = ::
                        cq_example_providers :: borrowed_fields; r5 = ::
                        cq_example_providers :: borrowed_fields;
                    } indexes { i0 = observed_hash; i1 = observed_sorted; } lowerings
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
                stored L at(__cq_store_L) operations observed_slice stored R
                at(__cq_store_R) operations observed_hash stored S at(__cq_store_S)
                operations observed_sorted stored r0 at(__cq_store_r0) operations
                observed_slice stored r1 at(__cq_store_r1) operations observed_hash
                indexed i0 at(__cq_index_i0) operations observed_hash region r2
                {
                    r2(a, b) => ((a, b,)) : - r0(a) => bind a <-
                    (observe(0, (__cq_store_r0).iter())), r1(a, b) => bind b <-
                    (observe(1, (__cq_index_i0).get(a).into_iter().flatten()));
                }
            } request { input r3 from(__cq_store_S) } }`
    = note: to `observe_provider!
            {provider(:: cq_example_providers :: sorted_relation) kind(2)
                reply_to(:: mini_linq :: resume_physical)
                {
                    plan
                    {
                        struct PullChain; relation L(a : Value); relation
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
                                L = observed_slice; R = observed_hash; S = observed_sorted;
                                r0 = observed_slice; r1 = observed_hash; r2 = ::
                                cq_example_providers :: borrowed_fields; r3 =
                                observed_sorted; r4 = :: cq_example_providers ::
                                borrowed_fields; r5 = :: cq_example_providers ::
                                borrowed_fields;
                            } indexes { i0 = observed_hash; i1 = observed_sorted; }
                            lowerings
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
                        stored L at(__cq_store_L) operations observed_slice stored R
                        at(__cq_store_R) operations observed_hash stored S
                        at(__cq_store_S) operations observed_sorted stored r0
                        at(__cq_store_r0) operations observed_slice stored r1
                        at(__cq_store_r1) operations observed_hash indexed i0
                        at(__cq_index_i0) operations observed_hash region r2
                        {
                            r2(a, b) => ((a, b,)) : - r0(a) => bind a <-
                            (observe(0, (__cq_store_r0).iter())), r1(a, b) => bind b <-
                            (observe(1, (__cq_index_i0).get(a).into_iter().flatten()));
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
                    struct PullChain; relation L(a : Value); relation
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
                            L = observed_slice; R = observed_hash; S = observed_sorted; r0
                            = observed_slice; r1 = observed_hash; r2 = ::
                            cq_example_providers :: borrowed_fields; r3 = observed_sorted;
                            r4 = :: cq_example_providers :: borrowed_fields; r5 = ::
                            cq_example_providers :: borrowed_fields;
                        } indexes { i0 = observed_hash; i1 = observed_sorted; } lowerings
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
                    stored L at(__cq_store_L) operations observed_slice stored R
                    at(__cq_store_R) operations observed_hash stored S at(__cq_store_S)
                    operations observed_sorted stored r0 at(__cq_store_r0) operations
                    observed_slice stored r1 at(__cq_store_r1) operations observed_hash
                    indexed i0 at(__cq_index_i0) operations observed_hash region r2
                    {
                        r2(a, b) => ((a, b,)) : - r0(a) => bind a <-
                        (observe(0, (__cq_store_r0).iter())), r1(a, b) => bind b <-
                        (observe(1, (__cq_index_i0).get(a).into_iter().flatten()));
                    }
                } request { input r3 from(__cq_store_S) }}`
    = note: expanding `observe_provider! { provider(:: cq_example_providers :: sorted_relation) kind(2)
            reply_to(:: mini_linq :: resume_physical)
            {
                plan
                {
                    struct PullChain; relation L(a : Value); relation
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
                            L = observed_slice; R = observed_hash; S = observed_sorted; r0
                            = observed_slice; r1 = observed_hash; r2 = ::
                            cq_example_providers :: borrowed_fields; r3 = observed_sorted;
                            r4 = :: cq_example_providers :: borrowed_fields; r5 = ::
                            cq_example_providers :: borrowed_fields;
                        } indexes { i0 = observed_hash; i1 = observed_sorted; } lowerings
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
                    stored L at(__cq_store_L) operations observed_slice stored R
                    at(__cq_store_R) operations observed_hash stored S at(__cq_store_S)
                    operations observed_sorted stored r0 at(__cq_store_r0) operations
                    observed_slice stored r1 at(__cq_store_r1) operations observed_hash
                    indexed i0 at(__cq_index_i0) operations observed_hash region r2
                    {
                        r2(a, b) => ((a, b,)) : - r0(a) => bind a <-
                        (observe(0, (__cq_store_r0).iter())), r1(a, b) => bind b <-
                        (observe(1, (__cq_index_i0).get(a).into_iter().flatten()));
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
                struct PullChain; relation L(a : Value); relation R(a : Value, b : Value);
                relation S(b : Value, c : Value); answer(a, c) : - L(a), R(a, b), S(b, c);
                relational
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
                        L = observed_slice; R = observed_hash; S = observed_sorted; r0 =
                        observed_slice; r1 = observed_hash; r2 = :: cq_example_providers
                        :: borrowed_fields; r3 = observed_sorted; r4 = ::
                        cq_example_providers :: borrowed_fields; r5 = ::
                        cq_example_providers :: borrowed_fields;
                    } indexes { i0 = observed_hash; i1 = observed_sorted; } lowerings
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
                stored L at(__cq_store_L) operations observed_slice stored R
                at(__cq_store_R) operations observed_hash stored S at(__cq_store_S)
                operations observed_sorted stored r0 at(__cq_store_r0) operations
                observed_slice stored r1 at(__cq_store_r1) operations observed_hash
                indexed i0 at(__cq_index_i0) operations observed_hash region r2
                {
                    r2(a, b) => ((a, b,)) : - r0(a) => bind a <-
                    (observe(0, (__cq_store_r0).iter())), r1(a, b) => bind b <-
                    (observe(1, (__cq_index_i0).get(a).into_iter().flatten()));
                }
            } request { input r3 from(__cq_store_S) } }`
    = note: to `:: cq_example_providers :: sorted_relation !
            {
                reply_to(observe_reply)
                {
                    callback(:: mini_linq :: resume_physical) kind(2) state
                    {plan
                        {
                            struct PullChain; relation L(a : Value); relation
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
                                    L = observed_slice; R = observed_hash; S = observed_sorted;
                                    r0 = observed_slice; r1 = observed_hash; r2 = ::
                                    cq_example_providers :: borrowed_fields; r3 =
                                    observed_sorted; r4 = :: cq_example_providers ::
                                    borrowed_fields; r5 = :: cq_example_providers ::
                                    borrowed_fields;
                                } indexes { i0 = observed_hash; i1 = observed_sorted; }
                                lowerings
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
                            stored L at(__cq_store_L) operations observed_slice stored R
                            at(__cq_store_R) operations observed_hash stored S
                            at(__cq_store_S) operations observed_sorted stored r0
                            at(__cq_store_r0) operations observed_slice stored r1
                            at(__cq_store_r1) operations observed_hash indexed i0
                            at(__cq_index_i0) operations observed_hash region r2
                            {
                                r2(a, b) => ((a, b,)) : - r0(a) => bind a <-
                                (observe(0, (__cq_store_r0).iter())), r1(a, b) => bind b <-
                                (observe(1, (__cq_index_i0).get(a).into_iter().flatten()));
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
                            Value > > = __cq_store_r1;
                        } pending { lower r3 }
                        continuations(:: mini_linq :: resume_physical, mini_linq ::
                        access)}
                } plan
                {struct PullChain; relation L(a : Value); relation
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
                            L = observed_slice; R = observed_hash; S = observed_sorted; r0
                            = observed_slice; r1 = observed_hash; r2 = ::
                            cq_example_providers :: borrowed_fields; r3 = observed_sorted;
                            r4 = :: cq_example_providers :: borrowed_fields; r5 = ::
                            cq_example_providers :: borrowed_fields;
                        } indexes { i0 = observed_hash; i1 = observed_sorted; } lowerings
                        {
                            r0 = :: cq_example_providers :: rename; r1 = ::
                            cq_example_providers :: rename; r2 = :: cq_example_providers
                            :: indexed_join; r3 = :: cq_example_providers :: rename; r4 =
                            :: cq_example_providers :: indexed_join; r5 = ::
                            cq_example_providers :: project;
                        }
                    }} environment
                {stored L at(__cq_store_L) operations observed_slice stored R
                    at(__cq_store_R) operations observed_hash stored S at(__cq_store_S)
                    operations observed_sorted stored r0 at(__cq_store_r0) operations
                    observed_slice stored r1 at(__cq_store_r1) operations observed_hash
                    indexed i0 at(__cq_index_i0) operations observed_hash region r2
                    {
                        r2(a, b) => ((a, b,)) : - r0(a) => bind a <-
                        (observe(0, (__cq_store_r0).iter())), r1(a, b) => bind b <-
                        (observe(1, (__cq_index_i0).get(a).into_iter().flatten()));
                    }} request {input r3 from(__cq_store_S)}
            }`
    = note: expanding `observe_reply! { resume
            {
                callback(:: mini_linq :: resume_physical) kind(2) state
                {
                    plan
                    {
                        struct PullChain; relation L(a : Value); relation
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
                                L = observed_slice; R = observed_hash; S = observed_sorted;
                                r0 = observed_slice; r1 = observed_hash; r2 = ::
                                cq_example_providers :: borrowed_fields; r3 =
                                observed_sorted; r4 = :: cq_example_providers ::
                                borrowed_fields; r5 = :: cq_example_providers ::
                                borrowed_fields;
                            } indexes { i0 = observed_hash; i1 = observed_sorted; }
                            lowerings
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
                        stored L at(__cq_store_L) operations observed_slice stored R
                        at(__cq_store_R) operations observed_hash stored S
                        at(__cq_store_S) operations observed_sorted stored r0
                        at(__cq_store_r0) operations observed_slice stored r1
                        at(__cq_store_r1) operations observed_hash indexed i0
                        at(__cq_index_i0) operations observed_hash region r2
                        {
                            r2(a, b) => ((a, b,)) : - r0(a) => bind a <-
                            (observe(0, (__cq_store_r0).iter())), r1(a, b) => bind b <-
                            (observe(1, (__cq_index_i0).get(a).into_iter().flatten()));
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
                }
            } emitted
            {
                let __cq_store_r3 : & :: std :: collections :: BTreeMap < Value, :: std ::
                vec :: Vec < Value >> = __cq_store_S;
            } result { stored r3 at(__cq_store_r3) operations observed_sorted } }`
    = note: to `:: mini_linq :: resume_physical !
            {
                resume
                {plan
                    {
                        struct PullChain; relation L(a : Value); relation
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
                                L = observed_slice; R = observed_hash; S = observed_sorted;
                                r0 = observed_slice; r1 = observed_hash; r2 = ::
                                cq_example_providers :: borrowed_fields; r3 =
                                observed_sorted; r4 = :: cq_example_providers ::
                                borrowed_fields; r5 = :: cq_example_providers ::
                                borrowed_fields;
                            } indexes { i0 = observed_hash; i1 = observed_sorted; }
                            lowerings
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
                        stored L at(__cq_store_L) operations observed_slice stored R
                        at(__cq_store_R) operations observed_hash stored S
                        at(__cq_store_S) operations observed_sorted stored r0
                        at(__cq_store_r0) operations observed_slice stored r1
                        at(__cq_store_r1) operations observed_hash indexed i0
                        at(__cq_index_i0) operations observed_hash region r2
                        {
                            r2(a, b) => ((a, b,)) : - r0(a) => bind a <-
                            (observe(0, (__cq_store_r0).iter())), r1(a, b) => bind b <-
                            (observe(1, (__cq_index_i0).get(a).into_iter().flatten()));
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
                    continuations(:: mini_linq :: resume_physical, mini_linq :: access)}
                emitted
                {let __cq_store_r3 : & :: std :: collections :: BTreeMap < Value, :: std
                    :: vec :: Vec < Value >> = __cq_store_S;} result
                {stored r3 at(__cq_store_r3) operations observed_sorted}
            }`
    = note: expanding `observed_sorted! { reply_to(:: mini_linq :: resume_physical)
            {
                plan
                {
                    struct PullChain; relation L(a : Value); relation
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
                            L = observed_slice; R = observed_hash; S = observed_sorted; r0
                            = observed_slice; r1 = observed_hash; r2 = ::
                            cq_example_providers :: borrowed_fields; r3 = observed_sorted;
                            r4 = :: cq_example_providers :: borrowed_fields; r5 = ::
                            cq_example_providers :: borrowed_fields;
                        } indexes { i0 = observed_hash; i1 = observed_sorted; } lowerings
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
                    stored L at(__cq_store_L) operations observed_slice stored R
                    at(__cq_store_R) operations observed_hash stored S at(__cq_store_S)
                    operations observed_sorted stored r0 at(__cq_store_r0) operations
                    observed_slice stored r1 at(__cq_store_r1) operations observed_hash
                    indexed i0 at(__cq_index_i0) operations observed_hash region r2
                    {
                        r2(a, b) => ((a, b,)) : - r0(a) => bind a <-
                        (observe(0, (__cq_store_r0).iter())), r1(a, b) => bind b <-
                        (observe(1, (__cq_index_i0).get(a).into_iter().flatten()));
                    } stored r3 at(__cq_store_r3) operations observed_sorted
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
                struct PullChain; relation L(a : Value); relation R(a : Value, b : Value);
                relation S(b : Value, c : Value); answer(a, c) : - L(a), R(a, b), S(b, c);
                relational
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
                        L = observed_slice; R = observed_hash; S = observed_sorted; r0 =
                        observed_slice; r1 = observed_hash; r2 = :: cq_example_providers
                        :: borrowed_fields; r3 = observed_sorted; r4 = ::
                        cq_example_providers :: borrowed_fields; r5 = ::
                        cq_example_providers :: borrowed_fields;
                    } indexes { i0 = observed_hash; i1 = observed_sorted; } lowerings
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
                stored L at(__cq_store_L) operations observed_slice stored R
                at(__cq_store_R) operations observed_hash stored S at(__cq_store_S)
                operations observed_sorted stored r0 at(__cq_store_r0) operations
                observed_slice stored r1 at(__cq_store_r1) operations observed_hash
                indexed i0 at(__cq_index_i0) operations observed_hash region r2
                {
                    r2(a, b) => ((a, b,)) : - r0(a) => bind a <-
                    (observe(0, (__cq_store_r0).iter())), r1(a, b) => bind b <-
                    (observe(1, (__cq_index_i0).get(a).into_iter().flatten()));
                } stored r3 at(__cq_store_r3) operations observed_sorted
            } request { index i1 } }`
    = note: to `observe_provider!
            {provider(:: cq_example_providers :: sorted_relation) kind(2)
                reply_to(:: mini_linq :: resume_physical)
                {
                    plan
                    {
                        struct PullChain; relation L(a : Value); relation
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
                                L = observed_slice; R = observed_hash; S = observed_sorted;
                                r0 = observed_slice; r1 = observed_hash; r2 = ::
                                cq_example_providers :: borrowed_fields; r3 =
                                observed_sorted; r4 = :: cq_example_providers ::
                                borrowed_fields; r5 = :: cq_example_providers ::
                                borrowed_fields;
                            } indexes { i0 = observed_hash; i1 = observed_sorted; }
                            lowerings
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
                        stored L at(__cq_store_L) operations observed_slice stored R
                        at(__cq_store_R) operations observed_hash stored S
                        at(__cq_store_S) operations observed_sorted stored r0
                        at(__cq_store_r0) operations observed_slice stored r1
                        at(__cq_store_r1) operations observed_hash indexed i0
                        at(__cq_index_i0) operations observed_hash region r2
                        {
                            r2(a, b) => ((a, b,)) : - r0(a) => bind a <-
                            (observe(0, (__cq_store_r0).iter())), r1(a, b) => bind b <-
                            (observe(1, (__cq_index_i0).get(a).into_iter().flatten()));
                        } stored r3 at(__cq_store_r3) operations observed_sorted
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
                    struct PullChain; relation L(a : Value); relation
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
                            L = observed_slice; R = observed_hash; S = observed_sorted; r0
                            = observed_slice; r1 = observed_hash; r2 = ::
                            cq_example_providers :: borrowed_fields; r3 = observed_sorted;
                            r4 = :: cq_example_providers :: borrowed_fields; r5 = ::
                            cq_example_providers :: borrowed_fields;
                        } indexes { i0 = observed_hash; i1 = observed_sorted; } lowerings
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
                    stored L at(__cq_store_L) operations observed_slice stored R
                    at(__cq_store_R) operations observed_hash stored S at(__cq_store_S)
                    operations observed_sorted stored r0 at(__cq_store_r0) operations
                    observed_slice stored r1 at(__cq_store_r1) operations observed_hash
                    indexed i0 at(__cq_index_i0) operations observed_hash region r2
                    {
                        r2(a, b) => ((a, b,)) : - r0(a) => bind a <-
                        (observe(0, (__cq_store_r0).iter())), r1(a, b) => bind b <-
                        (observe(1, (__cq_index_i0).get(a).into_iter().flatten()));
                    } stored r3 at(__cq_store_r3) operations observed_sorted
                } request { index i1 }}`
    = note: expanding `observe_provider! { provider(:: cq_example_providers :: sorted_relation) kind(2)
            reply_to(:: mini_linq :: resume_physical)
            {
                plan
                {
                    struct PullChain; relation L(a : Value); relation
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
                            L = observed_slice; R = observed_hash; S = observed_sorted; r0
                            = observed_slice; r1 = observed_hash; r2 = ::
                            cq_example_providers :: borrowed_fields; r3 = observed_sorted;
                            r4 = :: cq_example_providers :: borrowed_fields; r5 = ::
                            cq_example_providers :: borrowed_fields;
                        } indexes { i0 = observed_hash; i1 = observed_sorted; } lowerings
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
                    stored L at(__cq_store_L) operations observed_slice stored R
                    at(__cq_store_R) operations observed_hash stored S at(__cq_store_S)
                    operations observed_sorted stored r0 at(__cq_store_r0) operations
                    observed_slice stored r1 at(__cq_store_r1) operations observed_hash
                    indexed i0 at(__cq_index_i0) operations observed_hash region r2
                    {
                        r2(a, b) => ((a, b,)) : - r0(a) => bind a <-
                        (observe(0, (__cq_store_r0).iter())), r1(a, b) => bind b <-
                        (observe(1, (__cq_index_i0).get(a).into_iter().flatten()));
                    } stored r3 at(__cq_store_r3) operations observed_sorted
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
                struct PullChain; relation L(a : Value); relation R(a : Value, b : Value);
                relation S(b : Value, c : Value); answer(a, c) : - L(a), R(a, b), S(b, c);
                relational
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
                        L = observed_slice; R = observed_hash; S = observed_sorted; r0 =
                        observed_slice; r1 = observed_hash; r2 = :: cq_example_providers
                        :: borrowed_fields; r3 = observed_sorted; r4 = ::
                        cq_example_providers :: borrowed_fields; r5 = ::
                        cq_example_providers :: borrowed_fields;
                    } indexes { i0 = observed_hash; i1 = observed_sorted; } lowerings
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
                stored L at(__cq_store_L) operations observed_slice stored R
                at(__cq_store_R) operations observed_hash stored S at(__cq_store_S)
                operations observed_sorted stored r0 at(__cq_store_r0) operations
                observed_slice stored r1 at(__cq_store_r1) operations observed_hash
                indexed i0 at(__cq_index_i0) operations observed_hash region r2
                {
                    r2(a, b) => ((a, b,)) : - r0(a) => bind a <-
                    (observe(0, (__cq_store_r0).iter())), r1(a, b) => bind b <-
                    (observe(1, (__cq_index_i0).get(a).into_iter().flatten()));
                } stored r3 at(__cq_store_r3) operations observed_sorted
            } request { index i1 } }`
    = note: to `:: cq_example_providers :: sorted_relation !
            {
                reply_to(observe_reply)
                {
                    callback(:: mini_linq :: resume_physical) kind(2) state
                    {plan
                        {
                            struct PullChain; relation L(a : Value); relation
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
                                    L = observed_slice; R = observed_hash; S = observed_sorted;
                                    r0 = observed_slice; r1 = observed_hash; r2 = ::
                                    cq_example_providers :: borrowed_fields; r3 =
                                    observed_sorted; r4 = :: cq_example_providers ::
                                    borrowed_fields; r5 = :: cq_example_providers ::
                                    borrowed_fields;
                                } indexes { i0 = observed_hash; i1 = observed_sorted; }
                                lowerings
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
                            stored L at(__cq_store_L) operations observed_slice stored R
                            at(__cq_store_R) operations observed_hash stored S
                            at(__cq_store_S) operations observed_sorted stored r0
                            at(__cq_store_r0) operations observed_slice stored r1
                            at(__cq_store_r1) operations observed_hash indexed i0
                            at(__cq_index_i0) operations observed_hash region r2
                            {
                                r2(a, b) => ((a, b,)) : - r0(a) => bind a <-
                                (observe(0, (__cq_store_r0).iter())), r1(a, b) => bind b <-
                                (observe(1, (__cq_index_i0).get(a).into_iter().flatten()));
                            } stored r3 at(__cq_store_r3) operations observed_sorted
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
                            > > = __cq_store_S;
                        } pending { index i1 }
                        continuations(:: mini_linq :: resume_physical, mini_linq ::
                        access)}
                } plan
                {struct PullChain; relation L(a : Value); relation
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
                            L = observed_slice; R = observed_hash; S = observed_sorted; r0
                            = observed_slice; r1 = observed_hash; r2 = ::
                            cq_example_providers :: borrowed_fields; r3 = observed_sorted;
                            r4 = :: cq_example_providers :: borrowed_fields; r5 = ::
                            cq_example_providers :: borrowed_fields;
                        } indexes { i0 = observed_hash; i1 = observed_sorted; } lowerings
                        {
                            r0 = :: cq_example_providers :: rename; r1 = ::
                            cq_example_providers :: rename; r2 = :: cq_example_providers
                            :: indexed_join; r3 = :: cq_example_providers :: rename; r4 =
                            :: cq_example_providers :: indexed_join; r5 = ::
                            cq_example_providers :: project;
                        }
                    }} environment
                {stored L at(__cq_store_L) operations observed_slice stored R
                    at(__cq_store_R) operations observed_hash stored S at(__cq_store_S)
                    operations observed_sorted stored r0 at(__cq_store_r0) operations
                    observed_slice stored r1 at(__cq_store_r1) operations observed_hash
                    indexed i0 at(__cq_index_i0) operations observed_hash region r2
                    {
                        r2(a, b) => ((a, b,)) : - r0(a) => bind a <-
                        (observe(0, (__cq_store_r0).iter())), r1(a, b) => bind b <-
                        (observe(1, (__cq_index_i0).get(a).into_iter().flatten()));
                    } stored r3 at(__cq_store_r3) operations observed_sorted} request
                {index i1}
            }`
    = note: expanding `observe_reply! { resume
            {
                callback(:: mini_linq :: resume_physical) kind(2) state
                {
                    plan
                    {
                        struct PullChain; relation L(a : Value); relation
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
                                L = observed_slice; R = observed_hash; S = observed_sorted;
                                r0 = observed_slice; r1 = observed_hash; r2 = ::
                                cq_example_providers :: borrowed_fields; r3 =
                                observed_sorted; r4 = :: cq_example_providers ::
                                borrowed_fields; r5 = :: cq_example_providers ::
                                borrowed_fields;
                            } indexes { i0 = observed_hash; i1 = observed_sorted; }
                            lowerings
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
                        stored L at(__cq_store_L) operations observed_slice stored R
                        at(__cq_store_R) operations observed_hash stored S
                        at(__cq_store_S) operations observed_sorted stored r0
                        at(__cq_store_r0) operations observed_slice stored r1
                        at(__cq_store_r1) operations observed_hash indexed i0
                        at(__cq_index_i0) operations observed_hash region r2
                        {
                            r2(a, b) => ((a, b,)) : - r0(a) => bind a <-
                            (observe(0, (__cq_store_r0).iter())), r1(a, b) => bind b <-
                            (observe(1, (__cq_index_i0).get(a).into_iter().flatten()));
                        } stored r3 at(__cq_store_r3) operations observed_sorted
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
                }
            } emitted
            {
                let __cq_index_i1 : & :: std :: collections :: BTreeMap < Value, :: std ::
                vec :: Vec < Value >> = __cq_store_r3;
            } result { indexed i1 at(__cq_index_i1) operations observed_sorted } }`
    = note: to `:: mini_linq :: resume_physical !
            {
                resume
                {plan
                    {
                        struct PullChain; relation L(a : Value); relation
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
                                L = observed_slice; R = observed_hash; S = observed_sorted;
                                r0 = observed_slice; r1 = observed_hash; r2 = ::
                                cq_example_providers :: borrowed_fields; r3 =
                                observed_sorted; r4 = :: cq_example_providers ::
                                borrowed_fields; r5 = :: cq_example_providers ::
                                borrowed_fields;
                            } indexes { i0 = observed_hash; i1 = observed_sorted; }
                            lowerings
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
                        stored L at(__cq_store_L) operations observed_slice stored R
                        at(__cq_store_R) operations observed_hash stored S
                        at(__cq_store_S) operations observed_sorted stored r0
                        at(__cq_store_r0) operations observed_slice stored r1
                        at(__cq_store_r1) operations observed_hash indexed i0
                        at(__cq_index_i0) operations observed_hash region r2
                        {
                            r2(a, b) => ((a, b,)) : - r0(a) => bind a <-
                            (observe(0, (__cq_store_r0).iter())), r1(a, b) => bind b <-
                            (observe(1, (__cq_index_i0).get(a).into_iter().flatten()));
                        } stored r3 at(__cq_store_r3) operations observed_sorted
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
                    continuations(:: mini_linq :: resume_physical, mini_linq :: access)}
                emitted
                {let __cq_index_i1 : & :: std :: collections :: BTreeMap < Value, :: std
                    :: vec :: Vec < Value >> = __cq_store_r3;} result
                {indexed i1 at(__cq_index_i1) operations observed_sorted}
            }`
    = note: expanding `observed_sorted! { reply_to(:: cq_example_providers :: indexed_join_matches)
            {
                request
                {
                    reply_to(:: mini_linq :: resume_physical)
                    {
                        plan
                        {
                            struct PullChain; relation L(a : Value); relation
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
                                    L = observed_slice; R = observed_hash; S = observed_sorted;
                                    r0 = observed_slice; r1 = observed_hash; r2 = ::
                                    cq_example_providers :: borrowed_fields; r3 =
                                    observed_sorted; r4 = :: cq_example_providers ::
                                    borrowed_fields; r5 = :: cq_example_providers ::
                                    borrowed_fields;
                                } indexes { i0 = observed_hash; i1 = observed_sorted; }
                                lowerings
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
                            stored L at(__cq_store_L) operations observed_slice stored R
                            at(__cq_store_R) operations observed_hash stored S
                            at(__cq_store_S) operations observed_sorted stored r0
                            at(__cq_store_r0) operations observed_slice stored r1
                            at(__cq_store_r1) operations observed_hash indexed i0
                            at(__cq_index_i0) operations observed_hash region r2
                            {
                                r2(a, b) => ((a, b,)) : - r0(a) => bind a <-
                                (observe(0, (__cq_store_r0).iter())), r1(a, b) => bind b <-
                                (observe(1, (__cq_index_i0).get(a).into_iter().flatten()));
                            } stored r3 at(__cq_store_r3) operations observed_sorted
                            indexed i1 at(__cq_index_i1) operations observed_sorted
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
                        struct PullChain; relation L(a : Value); relation
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
                                L = observed_slice; R = observed_hash; S = observed_sorted;
                                r0 = observed_slice; r1 = observed_hash; r2 = ::
                                cq_example_providers :: borrowed_fields; r3 =
                                observed_sorted; r4 = :: cq_example_providers ::
                                borrowed_fields; r5 = :: cq_example_providers ::
                                borrowed_fields;
                            } indexes { i0 = observed_hash; i1 = observed_sorted; }
                            lowerings
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
                        stored L at(__cq_store_L) operations observed_slice stored R
                        at(__cq_store_R) operations observed_hash stored S
                        at(__cq_store_S) operations observed_sorted stored r0
                        at(__cq_store_r0) operations observed_slice stored r1
                        at(__cq_store_r1) operations observed_hash indexed i0
                        at(__cq_index_i0) operations observed_hash region r2
                        {
                            r2(a, b) => ((a, b,)) : - r0(a) => bind a <-
                            (observe(0, (__cq_store_r0).iter())), r1(a, b) => bind b <-
                            (observe(1, (__cq_index_i0).get(a).into_iter().flatten()));
                        } stored r3 at(__cq_store_r3) operations observed_sorted indexed
                        i1 at(__cq_index_i1) operations observed_sorted
                    } request { lower r4 }
                } access
                {
                    r0(a) => bind a <- (observe(0, (__cq_store_r0).iter())), r1(a, b) =>
                    bind b <- (observe(1, (__cq_index_i0).get(a).into_iter().flatten()))
                } witness { row((a, b,)) fields { 0 = (a); 1 = (b); } }
            } plan
            {
                struct PullChain; relation L(a : Value); relation R(a : Value, b : Value);
                relation S(b : Value, c : Value); answer(a, c) : - L(a), R(a, b), S(b, c);
                relational
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
                        L = observed_slice; R = observed_hash; S = observed_sorted; r0 =
                        observed_slice; r1 = observed_hash; r2 = :: cq_example_providers
                        :: borrowed_fields; r3 = observed_sorted; r4 = ::
                        cq_example_providers :: borrowed_fields; r5 = ::
                        cq_example_providers :: borrowed_fields;
                    } indexes { i0 = observed_hash; i1 = observed_sorted; } lowerings
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
                stored L at(__cq_store_L) operations observed_slice stored R
                at(__cq_store_R) operations observed_hash stored S at(__cq_store_S)
                operations observed_sorted stored r0 at(__cq_store_r0) operations
                observed_slice stored r1 at(__cq_store_r1) operations observed_hash
                indexed i0 at(__cq_index_i0) operations observed_hash region r2
                {
                    r2(a, b) => ((a, b,)) : - r0(a) => bind a <-
                    (observe(0, (__cq_store_r0).iter())), r1(a, b) => bind b <-
                    (observe(1, (__cq_index_i0).get(a).into_iter().flatten()));
                } stored r3 at(__cq_store_r3) operations observed_sorted indexed i1
                at(__cq_index_i1) operations observed_sorted
            } request { matches i1 values [(b)] } }`
    = note: to `observe_provider!
            {provider(:: cq_example_providers :: sorted_relation) kind(2)
                reply_to(:: cq_example_providers :: indexed_join_matches)
                {
                    request
                    {
                        reply_to(:: mini_linq :: resume_physical)
                        {
                            plan
                            {
                                struct PullChain; relation L(a : Value); relation
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
                                        L = observed_slice; R = observed_hash; S = observed_sorted;
                                        r0 = observed_slice; r1 = observed_hash; r2 = ::
                                        cq_example_providers :: borrowed_fields; r3 =
                                        observed_sorted; r4 = :: cq_example_providers ::
                                        borrowed_fields; r5 = :: cq_example_providers ::
                                        borrowed_fields;
                                    } indexes { i0 = observed_hash; i1 = observed_sorted; }
                                    lowerings
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
                                stored L at(__cq_store_L) operations observed_slice stored R
                                at(__cq_store_R) operations observed_hash stored S
                                at(__cq_store_S) operations observed_sorted stored r0
                                at(__cq_store_r0) operations observed_slice stored r1
                                at(__cq_store_r1) operations observed_hash indexed i0
                                at(__cq_index_i0) operations observed_hash region r2
                                {
                                    r2(a, b) => ((a, b,)) : - r0(a) => bind a <-
                                    (observe(0, (__cq_store_r0).iter())), r1(a, b) => bind b <-
                                    (observe(1, (__cq_index_i0).get(a).into_iter().flatten()));
                                } stored r3 at(__cq_store_r3) operations observed_sorted
                                indexed i1 at(__cq_index_i1) operations observed_sorted
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
                            struct PullChain; relation L(a : Value); relation
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
                                    L = observed_slice; R = observed_hash; S = observed_sorted;
                                    r0 = observed_slice; r1 = observed_hash; r2 = ::
                                    cq_example_providers :: borrowed_fields; r3 =
                                    observed_sorted; r4 = :: cq_example_providers ::
                                    borrowed_fields; r5 = :: cq_example_providers ::
                                    borrowed_fields;
                                } indexes { i0 = observed_hash; i1 = observed_sorted; }
                                lowerings
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
                            stored L at(__cq_store_L) operations observed_slice stored R
                            at(__cq_store_R) operations observed_hash stored S
                            at(__cq_store_S) operations observed_sorted stored r0
                            at(__cq_store_r0) operations observed_slice stored r1
                            at(__cq_store_r1) operations observed_hash indexed i0
                            at(__cq_index_i0) operations observed_hash region r2
                            {
                                r2(a, b) => ((a, b,)) : - r0(a) => bind a <-
                                (observe(0, (__cq_store_r0).iter())), r1(a, b) => bind b <-
                                (observe(1, (__cq_index_i0).get(a).into_iter().flatten()));
                            } stored r3 at(__cq_store_r3) operations observed_sorted
                            indexed i1 at(__cq_index_i1) operations observed_sorted
                        } request { lower r4 }
                    } access
                    {
                        r0(a) => bind a <- (observe(0, (__cq_store_r0).iter())), r1(a, b)
                        => bind b <-
                        (observe(1, (__cq_index_i0).get(a).into_iter().flatten()))
                    } witness { row((a, b,)) fields { 0 = (a); 1 = (b); } }
                } plan
                {
                    struct PullChain; relation L(a : Value); relation
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
                            L = observed_slice; R = observed_hash; S = observed_sorted; r0
                            = observed_slice; r1 = observed_hash; r2 = ::
                            cq_example_providers :: borrowed_fields; r3 = observed_sorted;
                            r4 = :: cq_example_providers :: borrowed_fields; r5 = ::
                            cq_example_providers :: borrowed_fields;
                        } indexes { i0 = observed_hash; i1 = observed_sorted; } lowerings
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
                    stored L at(__cq_store_L) operations observed_slice stored R
                    at(__cq_store_R) operations observed_hash stored S at(__cq_store_S)
                    operations observed_sorted stored r0 at(__cq_store_r0) operations
                    observed_slice stored r1 at(__cq_store_r1) operations observed_hash
                    indexed i0 at(__cq_index_i0) operations observed_hash region r2
                    {
                        r2(a, b) => ((a, b,)) : - r0(a) => bind a <-
                        (observe(0, (__cq_store_r0).iter())), r1(a, b) => bind b <-
                        (observe(1, (__cq_index_i0).get(a).into_iter().flatten()));
                    } stored r3 at(__cq_store_r3) operations observed_sorted indexed i1
                    at(__cq_index_i1) operations observed_sorted
                } request { matches i1 values [(b)] }}`
    = note: expanding `observe_provider! { provider(:: cq_example_providers :: sorted_relation) kind(2)
            reply_to(:: cq_example_providers :: indexed_join_matches)
            {
                request
                {
                    reply_to(:: mini_linq :: resume_physical)
                    {
                        plan
                        {
                            struct PullChain; relation L(a : Value); relation
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
                                    L = observed_slice; R = observed_hash; S = observed_sorted;
                                    r0 = observed_slice; r1 = observed_hash; r2 = ::
                                    cq_example_providers :: borrowed_fields; r3 =
                                    observed_sorted; r4 = :: cq_example_providers ::
                                    borrowed_fields; r5 = :: cq_example_providers ::
                                    borrowed_fields;
                                } indexes { i0 = observed_hash; i1 = observed_sorted; }
                                lowerings
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
                            stored L at(__cq_store_L) operations observed_slice stored R
                            at(__cq_store_R) operations observed_hash stored S
                            at(__cq_store_S) operations observed_sorted stored r0
                            at(__cq_store_r0) operations observed_slice stored r1
                            at(__cq_store_r1) operations observed_hash indexed i0
                            at(__cq_index_i0) operations observed_hash region r2
                            {
                                r2(a, b) => ((a, b,)) : - r0(a) => bind a <-
                                (observe(0, (__cq_store_r0).iter())), r1(a, b) => bind b <-
                                (observe(1, (__cq_index_i0).get(a).into_iter().flatten()));
                            } stored r3 at(__cq_store_r3) operations observed_sorted
                            indexed i1 at(__cq_index_i1) operations observed_sorted
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
                        struct PullChain; relation L(a : Value); relation
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
                                L = observed_slice; R = observed_hash; S = observed_sorted;
                                r0 = observed_slice; r1 = observed_hash; r2 = ::
                                cq_example_providers :: borrowed_fields; r3 =
                                observed_sorted; r4 = :: cq_example_providers ::
                                borrowed_fields; r5 = :: cq_example_providers ::
                                borrowed_fields;
                            } indexes { i0 = observed_hash; i1 = observed_sorted; }
                            lowerings
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
                        stored L at(__cq_store_L) operations observed_slice stored R
                        at(__cq_store_R) operations observed_hash stored S
                        at(__cq_store_S) operations observed_sorted stored r0
                        at(__cq_store_r0) operations observed_slice stored r1
                        at(__cq_store_r1) operations observed_hash indexed i0
                        at(__cq_index_i0) operations observed_hash region r2
                        {
                            r2(a, b) => ((a, b,)) : - r0(a) => bind a <-
                            (observe(0, (__cq_store_r0).iter())), r1(a, b) => bind b <-
                            (observe(1, (__cq_index_i0).get(a).into_iter().flatten()));
                        } stored r3 at(__cq_store_r3) operations observed_sorted indexed
                        i1 at(__cq_index_i1) operations observed_sorted
                    } request { lower r4 }
                } access
                {
                    r0(a) => bind a <- (observe(0, (__cq_store_r0).iter())), r1(a, b) =>
                    bind b <- (observe(1, (__cq_index_i0).get(a).into_iter().flatten()))
                } witness { row((a, b,)) fields { 0 = (a); 1 = (b); } }
            } plan
            {
                struct PullChain; relation L(a : Value); relation R(a : Value, b : Value);
                relation S(b : Value, c : Value); answer(a, c) : - L(a), R(a, b), S(b, c);
                relational
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
                        L = observed_slice; R = observed_hash; S = observed_sorted; r0 =
                        observed_slice; r1 = observed_hash; r2 = :: cq_example_providers
                        :: borrowed_fields; r3 = observed_sorted; r4 = ::
                        cq_example_providers :: borrowed_fields; r5 = ::
                        cq_example_providers :: borrowed_fields;
                    } indexes { i0 = observed_hash; i1 = observed_sorted; } lowerings
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
                stored L at(__cq_store_L) operations observed_slice stored R
                at(__cq_store_R) operations observed_hash stored S at(__cq_store_S)
                operations observed_sorted stored r0 at(__cq_store_r0) operations
                observed_slice stored r1 at(__cq_store_r1) operations observed_hash
                indexed i0 at(__cq_index_i0) operations observed_hash region r2
                {
                    r2(a, b) => ((a, b,)) : - r0(a) => bind a <-
                    (observe(0, (__cq_store_r0).iter())), r1(a, b) => bind b <-
                    (observe(1, (__cq_index_i0).get(a).into_iter().flatten()));
                } stored r3 at(__cq_store_r3) operations observed_sorted indexed i1
                at(__cq_index_i1) operations observed_sorted
            } request { matches i1 values [(b)] } }`
    = note: to `:: cq_example_providers :: sorted_relation !
            {
                reply_to(observe_reply)
                {
                    callback(:: cq_example_providers :: indexed_join_matches) kind(2)
                    state
                    {request
                        {
                            reply_to(:: mini_linq :: resume_physical)
                            {
                                plan
                                {
                                    struct PullChain; relation L(a : Value); relation
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
                                            L = observed_slice; R = observed_hash; S = observed_sorted;
                                            r0 = observed_slice; r1 = observed_hash; r2 = ::
                                            cq_example_providers :: borrowed_fields; r3 =
                                            observed_sorted; r4 = :: cq_example_providers ::
                                            borrowed_fields; r5 = :: cq_example_providers ::
                                            borrowed_fields;
                                        } indexes { i0 = observed_hash; i1 = observed_sorted; }
                                        lowerings
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
                                    stored L at(__cq_store_L) operations observed_slice stored R
                                    at(__cq_store_R) operations observed_hash stored S
                                    at(__cq_store_S) operations observed_sorted stored r0
                                    at(__cq_store_r0) operations observed_slice stored r1
                                    at(__cq_store_r1) operations observed_hash indexed i0
                                    at(__cq_index_i0) operations observed_hash region r2
                                    {
                                        r2(a, b) => ((a, b,)) : - r0(a) => bind a <-
                                        (observe(0, (__cq_store_r0).iter())), r1(a, b) => bind b <-
                                        (observe(1, (__cq_index_i0).get(a).into_iter().flatten()));
                                    } stored r3 at(__cq_store_r3) operations observed_sorted
                                    indexed i1 at(__cq_index_i1) operations observed_sorted
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
                                struct PullChain; relation L(a : Value); relation
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
                                        L = observed_slice; R = observed_hash; S = observed_sorted;
                                        r0 = observed_slice; r1 = observed_hash; r2 = ::
                                        cq_example_providers :: borrowed_fields; r3 =
                                        observed_sorted; r4 = :: cq_example_providers ::
                                        borrowed_fields; r5 = :: cq_example_providers ::
                                        borrowed_fields;
                                    } indexes { i0 = observed_hash; i1 = observed_sorted; }
                                    lowerings
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
                                stored L at(__cq_store_L) operations observed_slice stored R
                                at(__cq_store_R) operations observed_hash stored S
                                at(__cq_store_S) operations observed_sorted stored r0
                                at(__cq_store_r0) operations observed_slice stored r1
                                at(__cq_store_r1) operations observed_hash indexed i0
                                at(__cq_index_i0) operations observed_hash region r2
                                {
                                    r2(a, b) => ((a, b,)) : - r0(a) => bind a <-
                                    (observe(0, (__cq_store_r0).iter())), r1(a, b) => bind b <-
                                    (observe(1, (__cq_index_i0).get(a).into_iter().flatten()));
                                } stored r3 at(__cq_store_r3) operations observed_sorted
                                indexed i1 at(__cq_index_i1) operations observed_sorted
                            } request { lower r4 }
                        } access
                        {
                            r0(a) => bind a <- (observe(0, (__cq_store_r0).iter())),
                            r1(a, b) => bind b <-
                            (observe(1, (__cq_index_i0).get(a).into_iter().flatten()))
                        } witness { row((a, b,)) fields { 0 = (a); 1 = (b); } }}
                } plan
                {struct PullChain; relation L(a : Value); relation
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
                            L = observed_slice; R = observed_hash; S = observed_sorted; r0
                            = observed_slice; r1 = observed_hash; r2 = ::
                            cq_example_providers :: borrowed_fields; r3 = observed_sorted;
                            r4 = :: cq_example_providers :: borrowed_fields; r5 = ::
                            cq_example_providers :: borrowed_fields;
                        } indexes { i0 = observed_hash; i1 = observed_sorted; } lowerings
                        {
                            r0 = :: cq_example_providers :: rename; r1 = ::
                            cq_example_providers :: rename; r2 = :: cq_example_providers
                            :: indexed_join; r3 = :: cq_example_providers :: rename; r4 =
                            :: cq_example_providers :: indexed_join; r5 = ::
                            cq_example_providers :: project;
                        }
                    }} environment
                {stored L at(__cq_store_L) operations observed_slice stored R
                    at(__cq_store_R) operations observed_hash stored S at(__cq_store_S)
                    operations observed_sorted stored r0 at(__cq_store_r0) operations
                    observed_slice stored r1 at(__cq_store_r1) operations observed_hash
                    indexed i0 at(__cq_index_i0) operations observed_hash region r2
                    {
                        r2(a, b) => ((a, b,)) : - r0(a) => bind a <-
                        (observe(0, (__cq_store_r0).iter())), r1(a, b) => bind b <-
                        (observe(1, (__cq_index_i0).get(a).into_iter().flatten()));
                    } stored r3 at(__cq_store_r3) operations observed_sorted indexed i1
                    at(__cq_index_i1) operations observed_sorted} request
                {matches i1 values [(b)]}
            }`
    = note: expanding `observe_reply! { resume
            {
                callback(:: cq_example_providers :: indexed_join_matches) kind(2) state
                {
                    request
                    {
                        reply_to(:: mini_linq :: resume_physical)
                        {
                            plan
                            {
                                struct PullChain; relation L(a : Value); relation
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
                                        L = observed_slice; R = observed_hash; S = observed_sorted;
                                        r0 = observed_slice; r1 = observed_hash; r2 = ::
                                        cq_example_providers :: borrowed_fields; r3 =
                                        observed_sorted; r4 = :: cq_example_providers ::
                                        borrowed_fields; r5 = :: cq_example_providers ::
                                        borrowed_fields;
                                    } indexes { i0 = observed_hash; i1 = observed_sorted; }
                                    lowerings
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
                                stored L at(__cq_store_L) operations observed_slice stored R
                                at(__cq_store_R) operations observed_hash stored S
                                at(__cq_store_S) operations observed_sorted stored r0
                                at(__cq_store_r0) operations observed_slice stored r1
                                at(__cq_store_r1) operations observed_hash indexed i0
                                at(__cq_index_i0) operations observed_hash region r2
                                {
                                    r2(a, b) => ((a, b,)) : - r0(a) => bind a <-
                                    (observe(0, (__cq_store_r0).iter())), r1(a, b) => bind b <-
                                    (observe(1, (__cq_index_i0).get(a).into_iter().flatten()));
                                } stored r3 at(__cq_store_r3) operations observed_sorted
                                indexed i1 at(__cq_index_i1) operations observed_sorted
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
                            struct PullChain; relation L(a : Value); relation
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
                                    L = observed_slice; R = observed_hash; S = observed_sorted;
                                    r0 = observed_slice; r1 = observed_hash; r2 = ::
                                    cq_example_providers :: borrowed_fields; r3 =
                                    observed_sorted; r4 = :: cq_example_providers ::
                                    borrowed_fields; r5 = :: cq_example_providers ::
                                    borrowed_fields;
                                } indexes { i0 = observed_hash; i1 = observed_sorted; }
                                lowerings
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
                            stored L at(__cq_store_L) operations observed_slice stored R
                            at(__cq_store_R) operations observed_hash stored S
                            at(__cq_store_S) operations observed_sorted stored r0
                            at(__cq_store_r0) operations observed_slice stored r1
                            at(__cq_store_r1) operations observed_hash indexed i0
                            at(__cq_index_i0) operations observed_hash region r2
                            {
                                r2(a, b) => ((a, b,)) : - r0(a) => bind a <-
                                (observe(0, (__cq_store_r0).iter())), r1(a, b) => bind b <-
                                (observe(1, (__cq_index_i0).get(a).into_iter().flatten()));
                            } stored r3 at(__cq_store_r3) operations observed_sorted
                            indexed i1 at(__cq_index_i1) operations observed_sorted
                        } request { lower r4 }
                    } access
                    {
                        r0(a) => bind a <- (observe(0, (__cq_store_r0).iter())), r1(a, b)
                        => bind b <-
                        (observe(1, (__cq_index_i0).get(a).into_iter().flatten()))
                    } witness { row((a, b,)) fields { 0 = (a); 1 = (b); } }
                }
            } source((__cq_index_i1).get(b).into_iter().flatten()) item(c) witness
            { row((b, c,)) fields { 0 = (b); 1 = (c); } } }`
    = note: to `:: cq_example_providers :: indexed_join_matches !
            {
                resume
                {request
                    {
                        reply_to(:: mini_linq :: resume_physical)
                        {
                            plan
                            {
                                struct PullChain; relation L(a : Value); relation
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
                                        L = observed_slice; R = observed_hash; S = observed_sorted;
                                        r0 = observed_slice; r1 = observed_hash; r2 = ::
                                        cq_example_providers :: borrowed_fields; r3 =
                                        observed_sorted; r4 = :: cq_example_providers ::
                                        borrowed_fields; r5 = :: cq_example_providers ::
                                        borrowed_fields;
                                    } indexes { i0 = observed_hash; i1 = observed_sorted; }
                                    lowerings
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
                                stored L at(__cq_store_L) operations observed_slice stored R
                                at(__cq_store_R) operations observed_hash stored S
                                at(__cq_store_S) operations observed_sorted stored r0
                                at(__cq_store_r0) operations observed_slice stored r1
                                at(__cq_store_r1) operations observed_hash indexed i0
                                at(__cq_index_i0) operations observed_hash region r2
                                {
                                    r2(a, b) => ((a, b,)) : - r0(a) => bind a <-
                                    (observe(0, (__cq_store_r0).iter())), r1(a, b) => bind b <-
                                    (observe(1, (__cq_index_i0).get(a).into_iter().flatten()));
                                } stored r3 at(__cq_store_r3) operations observed_sorted
                                indexed i1 at(__cq_index_i1) operations observed_sorted
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
                            struct PullChain; relation L(a : Value); relation
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
                                    L = observed_slice; R = observed_hash; S = observed_sorted;
                                    r0 = observed_slice; r1 = observed_hash; r2 = ::
                                    cq_example_providers :: borrowed_fields; r3 =
                                    observed_sorted; r4 = :: cq_example_providers ::
                                    borrowed_fields; r5 = :: cq_example_providers ::
                                    borrowed_fields;
                                } indexes { i0 = observed_hash; i1 = observed_sorted; }
                                lowerings
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
                            stored L at(__cq_store_L) operations observed_slice stored R
                            at(__cq_store_R) operations observed_hash stored S
                            at(__cq_store_S) operations observed_sorted stored r0
                            at(__cq_store_r0) operations observed_slice stored r1
                            at(__cq_store_r1) operations observed_hash indexed i0
                            at(__cq_index_i0) operations observed_hash region r2
                            {
                                r2(a, b) => ((a, b,)) : - r0(a) => bind a <-
                                (observe(0, (__cq_store_r0).iter())), r1(a, b) => bind b <-
                                (observe(1, (__cq_index_i0).get(a).into_iter().flatten()));
                            } stored r3 at(__cq_store_r3) operations observed_sorted
                            indexed i1 at(__cq_index_i1) operations observed_sorted
                        } request { lower r4 }
                    } access
                    {
                        r0(a) => bind a <- (observe(0, (__cq_store_r0).iter())), r1(a, b)
                        => bind b <-
                        (observe(1, (__cq_index_i0).get(a).into_iter().flatten()))
                    } witness { row((a, b,)) fields { 0 = (a); 1 = (b); } }}
                source(observe(2, (__cq_index_i1).get(b).into_iter().flatten())) item(c)
                witness {row((b, c,)) fields { 0 = (b); 1 = (c); }}
            }`
    = note: this note originates in the macro `::mini_linq::cq_to_ra_written_order` (in Nightly builds, run with -Z macro-backtrace for more info)

note: trace_macro
   --> crates/mini-linq/tests/physical_pull.rs:162:20
    |
162 |         (Value(1), vec![Value(10), Value(11)]),
    |                    ^^^^^^^^^^^^^^^^^^^^^^^^^^
    |
    = note: expanding `vec! { Value(10), Value(11) }`
    = note: to `$crate :: boxed ::
            box_assume_init_into_vec_unsafe($crate :: intrinsics ::
            write_box_via_move($crate :: boxed :: Box :: new_uninit(),
            [Value(10), Value(11)]))`

note: trace_macro
   --> crates/mini-linq/tests/physical_pull.rs:163:20
    |
163 |         (Value(2), vec![Value(20)]),
    |                    ^^^^^^^^^^^^^^^
    |
    = note: expanding `vec! { Value(20) }`
    = note: to `$crate :: boxed ::
            box_assume_init_into_vec_unsafe($crate :: intrinsics ::
            write_box_via_move($crate :: boxed :: Box :: new_uninit(), [Value(20)]))`

note: trace_macro
   --> crates/mini-linq/tests/physical_pull.rs:166:21
    |
166 |         (Value(10), vec![Value(100), Value(101)]),
    |                     ^^^^^^^^^^^^^^^^^^^^^^^^^^^^
    |
    = note: expanding `vec! { Value(100), Value(101) }`
    = note: to `$crate :: boxed ::
            box_assume_init_into_vec_unsafe($crate :: intrinsics ::
            write_box_via_move($crate :: boxed :: Box :: new_uninit(),
            [Value(100), Value(101)]))`

note: trace_macro
   --> crates/mini-linq/tests/physical_pull.rs:167:21
    |
167 |         (Value(11), vec![Value(110)]),
    |                     ^^^^^^^^^^^^^^^^
    |
    = note: expanding `vec! { Value(110) }`
    = note: to `$crate :: boxed ::
            box_assume_init_into_vec_unsafe($crate :: intrinsics ::
            write_box_via_move($crate :: boxed :: Box :: new_uninit(), [Value(110)]))`

note: trace_macro
   --> crates/mini-linq/tests/physical_pull.rs:168:21
    |
168 |         (Value(20), vec![Value(200)]),
    |                     ^^^^^^^^^^^^^^^^
    |
    = note: expanding `vec! { Value(200) }`
    = note: to `$crate :: boxed ::
            box_assume_init_into_vec_unsafe($crate :: intrinsics ::
            write_box_via_move($crate :: boxed :: Box :: new_uninit(), [Value(200)]))`

note: trace_macro
   --> crates/mini-linq/tests/physical_pull.rs:172:5
    |
172 |     assert_eq!(OBSERVATIONS.get(), [[0; 2]; 3]);
    |     ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^
    |
    = note: expanding `assert_eq! { OBSERVATIONS.get(), [[0; 2]; 3] }`
    = note: to `match (& OBSERVATIONS.get(), & [[0; 2]; 3])
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
   --> crates/mini-linq/tests/physical_pull.rs:174:5
    |
174 |     assert_eq!(OBSERVATIONS.get(), [[0; 2]; 3]);
    |     ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^
    |
    = note: expanding `assert_eq! { OBSERVATIONS.get(), [[0; 2]; 3] }`
    = note: to `match (& OBSERVATIONS.get(), & [[0; 2]; 3])
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
   --> crates/mini-linq/tests/physical_pull.rs:176:5
    |
176 |     assert_eq!(OBSERVATIONS.get(), [[1, 1]; 3]);
    |     ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^
    |
    = note: expanding `assert_eq! { OBSERVATIONS.get(), [[1, 1]; 3] }`
    = note: to `match (& OBSERVATIONS.get(), & [[1, 1]; 3])
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
   --> crates/mini-linq/tests/physical_pull.rs:178:5
    |
178 |     assert_eq!(cursor.next(), Some((&left[0], &right[&Value(10)][1])));
    |     ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^
    |
    = note: expanding `assert_eq! { cursor.next(), Some((&left[0], &right[&Value(10)][1])) }`
    = note: to `match (& cursor.next(), & Some((&left[0], &right[&Value(10)][1])))
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
   --> crates/mini-linq/tests/physical_pull.rs:179:5
    |
179 |     assert_eq!(OBSERVATIONS.get(), [[1, 1], [1, 1], [1, 2]]);
    |     ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^
    |
    = note: expanding `assert_eq! { OBSERVATIONS.get(), [[1, 1], [1, 1], [1, 2]] }`
    = note: to `match (& OBSERVATIONS.get(), & [[1, 1], [1, 1], [1, 2]])
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
   --> crates/mini-linq/tests/physical_pull.rs:186:20
    |
186 |         (Value(1), vec![Value(10), Value(10), Value(11)]),
    |                    ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^
    |
    = note: expanding `vec! { Value(10), Value(10), Value(11) }`
    = note: to `$crate :: boxed ::
            box_assume_init_into_vec_unsafe($crate :: intrinsics ::
            write_box_via_move($crate :: boxed :: Box :: new_uninit(),
            [Value(10), Value(10), Value(11)]))`

note: trace_macro
   --> crates/mini-linq/tests/physical_pull.rs:187:20
    |
187 |         (Value(2), vec![Value(20)]),
    |                    ^^^^^^^^^^^^^^^
    |
    = note: expanding `vec! { Value(20) }`
    = note: to `$crate :: boxed ::
            box_assume_init_into_vec_unsafe($crate :: intrinsics ::
            write_box_via_move($crate :: boxed :: Box :: new_uninit(), [Value(20)]))`

note: trace_macro
   --> crates/mini-linq/tests/physical_pull.rs:190:21
    |
190 |         (Value(10), vec![Value(100), Value(101), Value(101)]),
    |                     ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^
    |
    = note: expanding `vec! { Value(100), Value(101), Value(101) }`
    = note: to `$crate :: boxed ::
            box_assume_init_into_vec_unsafe($crate :: intrinsics ::
            write_box_via_move($crate :: boxed :: Box :: new_uninit(),
            [Value(100), Value(101), Value(101)]))`

note: trace_macro
   --> crates/mini-linq/tests/physical_pull.rs:191:21
    |
191 |         (Value(11), vec![Value(100)]),
    |                     ^^^^^^^^^^^^^^^^
    |
    = note: expanding `vec! { Value(100) }`
    = note: to `$crate :: boxed ::
            box_assume_init_into_vec_unsafe($crate :: intrinsics ::
            write_box_via_move($crate :: boxed :: Box :: new_uninit(), [Value(100)]))`

note: trace_macro
   --> crates/mini-linq/tests/physical_pull.rs:192:21
    |
192 |         (Value(20), vec![Value(200)]),
    |                     ^^^^^^^^^^^^^^^^
    |
    = note: expanding `vec! { Value(200) }`
    = note: to `$crate :: boxed ::
            box_assume_init_into_vec_unsafe($crate :: intrinsics ::
            write_box_via_move($crate :: boxed :: Box :: new_uninit(), [Value(200)]))`

note: trace_macro
   --> crates/mini-linq/tests/physical_pull.rs:196:5
    |
196 |     assert_eq!(actual, oracle(&left, &middle, &right));
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
   --> crates/mini-linq/tests/physical_pull.rs:197:5
    |
197 |     assert_eq!(rows.len(), actual.len());
    |     ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^
    |
    = note: expanding `assert_eq! { rows.len(), actual.len() }`
    = note: to `match (& rows.len(), & actual.len())
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
   --> crates/mini-linq/tests/physical_pull.rs:233:9
    |
233 |         assert_eq!(actual, oracle(&left, &middle, &right));
    |         ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^
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
   --> crates/mini-linq/tests/physical_pull.rs:234:9
    |
234 |         assert_eq!(rows.len(), actual.len());
    |         ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^
    |
    = note: expanding `assert_eq! { rows.len(), actual.len() }`
    = note: to `match (& rows.len(), & actual.len())
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
   --> crates/mini-linq/tests/physical_pull.rs:243:10
    |
243 |           &format!(
    |  __________^
244 | |             "{}\n{}",
245 | |             include_str!("fixtures/physical-pull/expected.ra"),
246 | |             include_str!("fixtures/physical-pull/expected.indexes"),
247 | |         ),
    | |_________^
    |
    = note: expanding `format! { "{}\n{}", include_str!("fixtures/physical-pull/expected.ra"),
            include_str!("fixtures/physical-pull/expected.indexes"), }`
    = note: to `$crate :: __export ::
            must_use({
                $crate :: fmt ::
                format($crate :: __export :: format_args!
                ("{}\n{}", include_str!("fixtures/physical-pull/expected.ra"),
                include_str!("fixtures/physical-pull/expected.indexes"),))
            })`

    Finished `dev` profile [unoptimized + debuginfo] target(s) in 5.80s

```
