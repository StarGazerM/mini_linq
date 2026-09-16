# Nested execution directly from named RA

`nested_pull!` and `nested_push!` are an exploratory pair of whole-query
realizations in `cq-realize`. They consume the same existing RA and select
query control before generating storage traversal. They use no IX, Access,
Pipeline, additional IR or runtime relational interpreter. Existing hash and
configured-relation implementations remain comparison baselines; this experiment
does not complete their migration or replace them with compatibility aliases.

The implementation entry points are [nested_pull.rs](../crates/cq-realize/src/nested_pull.rs)
and [nested_push.rs](../crates/cq-realize/src/nested_push.rs). The end-to-end
[example](../examples/ra-execution/main.rs) selects both from one logical query.

## Domain and borrowed API

The input is checked named RA containing Unit, Rename, Project, NaturalJoin and
Semijoin, with a variable-only output. Output order and repeated variables are
preserved. Select, Extend and arbitrary output expressions are rejected. The
realization consumes the supplied algebra; proving it represents an annotated
CQ belongs to the preceding planner.

Only per-input bindings are accepted:

```text
inputs { R => provider_path; S => another_provider; }
```

Each binding must implement `type (T, ...)`, `scan (&input_storage)` and
`field (row) (column)`. Scans must be finite and repeatable, represent the same
input relation throughout execution, and expose field references tied to the
borrowed caller-owned storage. A reference into temporary scan state is
insufficient. `Input` is not invoked and no local selected-view owner is created.
Tuple hash/sorted providers meet this direct scan contract. The stock block
provider's view-based Scan is not automatically compatible with `&BlockStore`.

For input lifetime `'a` and logical output types `(T0, T1, ...)`, the generated
APIs have these shapes:

```rust
Pull::iter(&r, &s) // impl Iterator<Item = (&'a T0, &'a T1, ...)> + 'a
Push::for_each(&r, &s, consumer) // consumer: impl FnMut((&'a T0, &'a T1, ...))
```

Arguments follow input declaration order. Both APIs produce distinct answers
by value. A consumer may retain those references while the original inputs remain
borrowed; no payload ownership or Clone bound is introduced by these lowerings.
HashSet distinctness requires lawful equality and hashing of the output fields.
Join comparisons require lawful equality on shared fields. Rust still checks
the concrete types, tuple traits and lifetimes of the chosen providers.

Inputs and relevant equality/hash behavior must stay stable for the whole
traversal, including consumer calls. Shared borrowing does not prove this law
for arbitrary interior-mutability implementations. Equal projected values may
come from different input addresses; distinctness chooses a representative, with
no promise of pointer identity or identical enumeration order between branches.

## Support refinement

An intermediate row associates each logical attribute with an original field
reference. Pull represents it as a tuple in logical-heading order; push carries
borrowed field expressions in its compiler-local binding map. Projection and
joining retain these references, without copying payloads or borrowing temporary
intermediate tuples. Let `erase` read their values, and let `E(r)` denote the
finite support enumeration for a named result `r`:

| RA operator | Support enumeration |
| --- | --- |
| Unit | One empty tuple. |
| Rename R | Scan R and map fields to the renamed heading. |
| Project U(r) | Restrict each row of E(r) to U. |
| NaturalJoin(l,r) | For every left row, enumerate right rows and emit each pair agreeing on **all** shared fields. |
| Semijoin(l,r) | Emit a left occurrence only if the right enumeration has a compatible row; stop the existence search at its first witness. |

The induction invariant is:

```text
support(erase(E(r))) = [[r]]
```

The table maps to the operator cases of `Lowering::relation` in `nested_pull.rs`
and `Producer::produce` in `nested_push.rs`. These are alternative compilation
functions over the same RA operator type, not two additional IRs.

The base cases follow provider and Unit laws. Projection preserves support by
restriction; nested equality tests establish natural join; an existence test
establishes semijoin. These positive operations depend on support rather than
witness multiplicity. Intermediate projection may therefore emit duplicates
without needing a local distinctness set. Empty shared headings give Cartesian
product or right nonemptiness; projection to no fields yields support `{()}`
exactly when its input is nonempty.

Both hosts map the output variables in authored order, then deduplicate that
final borrowed tuple with a HashSet. Under the provider, value and lifetime
premises above, completed iteration and completed push delivery contain each
logical output tuple exactly once. The relation denotation does not prescribe
which equal reference representative is retained.

Pull emits nested lazy adapters and short-circuit existence searches. Creating
its iterator does not enumerate rows; advancing it performs only the traversal
needed to find the next distinct answer or exhaustion. A single advance may
still scan many witnesses. Push emits nested loops and calls its consumer when
the final seen-set admits a new answer. It exhausts its computation normally;
`for_each` does not provide a consumer-controlled early-stop protocol.

Shared DAG uses are recomputed from stable inputs. The code contains no
materialized intermediate relation, but final distinctness retains one borrowed
tuple per emitted answer, and pull retains traversal state. Recomputation can
increase work and generated code substantially. These are nested algorithms,
without per-join algorithm selection, zone pruning, statistics changes or cost
planning.

## Usage and scope checks

Run the complete example with only its selected implementations:

```sh
cargo run -p mini-linq --no-default-features \
  --features graph,nested-push,nested-pull,tuple-hash-rows,tuple-sorted-rows \
  --example ra-execution
```

The example uses local `choose_indexes` wrappers to add the per-input bindings
and dispatch planned RA directly to `nested_pull!` or `nested_push!`. That
continuation name is inherited from the existing graph planner; it does not
cause an index pass to run. Changing this wrapper selects the whole realization,
rather than replacing an inner Access macro after materialization.

Its pull wrapper has this form; the push module substitutes `nested_push!`:

```rust
macro_rules! realize {
    ($($ra:tt)*) => { ::mini_linq::nested_pull! {
        inputs {
            Facts => ::mini_linq::tuple_hash_rows;
            Tags => ::mini_linq::tuple_hash_rows;
            Allowed => ::mini_linq::tuple_sorted_rows;
        }
        $($ra)*
    } };
}
use realize as choose_indexes;
```

The example's routing module exposes that alias to `graph_query!`. The shared
CQ is `answer(value, tag) :- Facts(key, value), Tags(key, tag), Allowed(key)`.
After compilation, the two modules expose these calls:

```rust
let facts = HashSet::from([(1, "alpha".to_owned()), (2, "beta".to_owned())]);
let tags = HashSet::from([(1, 10), (1, 20), (2, 30)]);
let allowed = BTreeSet::from([(1,)]);
let pulled: HashSet<_> = pull::Star::iter(&facts, &tags, &allowed).collect();
let mut pushed = HashSet::new();
push::Star::for_each(&facts, &tags, &allowed, |row| { pushed.insert(row); });
assert_eq!(pulled, pushed);
```

Both sets contain borrowed answers `("alpha", 10)` and `("alpha", 20)` by
value. `HashSet` and `BTreeSet` are the standard-library collections imported
by the complete example.

Acceptance checks compare completed value sets and distinctness against an
independent nested oracle, including duplicate-producing projection, empty
inputs/headings, semijoins, shared definitions and repeated output variables.
Demand checks distinguish iterator construction, partial polling and exhaustion.
Ownership checks use non-Clone fields and retained borrowed answers. Unsupported
operations or temporary-borrow providers must fail explicitly. These checks do
not establish arbitrary effects, provider laws or asymptotic improvements.

Capture every actual procedural-macro invocation of the example with:

```sh
python3 tools/trace_macros.py --example ra-execution \
  --features graph,nested-push,nested-pull,tuple-hash-rows,tuple-sorted-rows \
  --out target/ra-execution-trace
```

The output directory must be new. The tool instruments an isolated source copy,
records each input/output TokenStream, and returns the original expansion to
rustc. Its index links individual calls, with raw tokens retained alongside
formatted reading copies; it does not reconstruct the pass chain through typed
compiler APIs. Source hashes verify that the original compiler was unchanged
during capture.
