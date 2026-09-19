## 2172007/039-cq-example-providers::borrowed_fields

[Raw input](039-cq-example-providers::borrowed_fields.input.tokens) · [Raw output](039-cq-example-providers::borrowed_fields.output.tokens)

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
