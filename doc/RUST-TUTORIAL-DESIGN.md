# Rust tutorial design for MiniLinq

The Rust material should make the compiler architecture feel inevitable. Start
with concrete values, borrows, and iterator objects; only then name syntax
trees, proc macros, and pass contracts. The central bridge is:

> A program can be represented as a value, transformed into another value,
> and finally emitted as an ordinary Rust expression.

Triangle is the running example. Its complete program at every stage is a
checked-in scenario: [CQ](../test-support/relational/scenarios/triangle.cq),
[VO](../test-support/relational/scenarios/triangle.vo),
[RA](../test-support/relational/scenarios/triangle.ra),
[IX](../test-support/relational/scenarios/triangle.ix),
[Physical](../test-support/relational/scenarios/triangle.physical),
[policy](../test-support/relational/scenarios/triangle.policy), and
[Structures](../test-support/relational/scenarios/triangle.structures). The
`.ix` and `.structures` files hold only the `indexes { … }` and
`structures { … }` suffixes; [`Scenario::ix` and
`Scenario::structured`](../test-support/relational/scenarios.rs) append them to
the RA and Physical programs they extend.

## Learning outcomes

After the tutorial, students should be able to explain and implement:

1. the difference between a caller-owned host relation, a structure the
   printed block builds over borrowed rows, and a lazy query cursor;
2. why a proc-macro function accepts `TokenStream` while compiler passes accept
   typed IR programs;
3. how typed construction, `quote!`, and `syn::parse2` produce the next
   language;
4. why an RA `rN` is a logical set while a Physical `pipe` result is a cursor
   and a `let` result is retained storage;
5. how Rust's `map`, `flat_map`, and `filter` encode a pull pipeline;
6. why the final `syn::Expr` is the concrete target rather than a
   pretty-printing hint; and
7. where the printed program borrows, where a host key type forces a clone,
   and where the query head computes owned values.

## Sequence 1: the caller owns the relations

A declaration is a schema whose column types are real Rust types:

```text
relation Road(src: City, dst: City);
```

It contains no facts, and the compiler generates no `build` or `run` API. The
caller writes an ordinary function whose parameters are the relation values,
then places a macro invocation in its body. The expansion is one Rust block
expression evaluated in that scope, naming each relation as a local variable
([composition example](../examples/composition/src/main.rs)):

```rust
#[allow(non_snake_case)]
fn query<'a>(
    L: &'a [(i32,)],
    R: &'a HashMap<(i32,), Vec<(i32,)>>,
    S: &'a BTreeMap<(i32,), Vec<(i32,)>>,
) -> HashSet<(&'a i32, &'a i32, &'a i32)> {
    ::core::iter::Iterator::collect(::mini_linq::index_demands! {
        relational {
            relation L(a: i32);
            relation R(a: i32, b: i32);
            relation S(b: i32, c: i32);
            l = rename L { a -> a };
            r = rename R { a -> a, b -> b };
            s = rename S { b -> b, c -> c };
            j0 = natural_join l with r;
            j1 = natural_join j0 with s;
            output j1 as answer(a, b, c);
        }
    })
}
```

The caller's query-independent policy, appended by the local macro the
example binds as `mini_linq::physical_policy`, states each host's shape. An unkeyed
host (`L;`) is read as `L.iter()` over full-row tuples. A keyed host
(`R keyed [a];`) is a map `Map<(key…,), Coll<(rest…,)>>` in the declared column
order, probed in place. There is no input normalization step: the host value
is used exactly as supplied.

Ask students to predict what happens if `query` returns the lazy iterator
instead of collecting it, and then if `L` is passed by value. The borrow
checker, not a MiniLinq rule, explains both answers.

## Sequence 2: built structures borrow host rows

When the policy says a relation is only scanned but a probe needs a key,
Structures binds a structure for that read. Triangle's policy declares `R`,
`S`, and `T` unkeyed, so its Structures suffix is:

```text
structures {
    R;
    S;
    T;
    r1 = ::cq_storage::HashIndex<_, _> [b];
    r3 = ::cq_storage::HashIndex<_, _> [a, c];
}
```

The printer binds each structure at the top of the block through
`cq_storage::Build`, from `((key…,), (rest…,))` pairs of **references** in
sorted column order, then emits a `cq_storage::require` guard naming the trait
every read on it uses. A [realization
fixture](../passes/cq-physical-realization/tests/realization.rs) pins this
shape:

```rust
let __cq_r_on_a = &{
    use ::cq_storage::Build as _;
    ::cq_storage::HashIndex::build(::core::iter::Iterator::map(
        ::core::iter::Iterator::map(R.iter(), move |(b, a,)| (a, b,)),
        |(a, b,)| ((a,), (b,))
    ))
};
::cq_storage::require::seek(__cq_r_on_a);
```

Nothing is cloned: the structure is keyed by `(&T, …)` and stores reference
payloads. The only clone the printer ever emits is the owned key used to probe
a keyed host map, because that map's key type is `(T, …)`. Whether the chosen
type actually implements the required traits is rustc's check at the guard.

`cq_storage::HashIndex` defers bucket construction until its first lookup;
`HashSet` and `BTreeMap` build eagerly. Construction timing belongs to the
selected structure type, not to the printer.

## Sequence 3: a cursor borrows storage

Use a small owner and a method returning an iterator:

```rust
struct Rows {
    values: Vec<(City, City,)>,
}

impl Rows {
    fn query(&self) -> impl Iterator<Item = &(City, City,)> + '_ {
        self.values.iter()
    }
}
```

The `+ '_` ties the returned cursor to `&self`. The cursor may be resumed while
the owner is alive, but it cannot outlive storage. Calling `query()` creates an
ordinary compiled value with a hidden concrete type; it does not materialize
the values and it does not create a thread or task.

Then show partial consumption:

```rust
fn first_two(rows: &Rows) -> Vec<&(City, City,)> {
    let mut cursor = rows.query();
    [cursor.next(), cursor.next()].into_iter().flatten().collect()
}
```

The iterator object holds resumable state. There is no hidden outer `for`
loop; the cursor itself is the whole result-set protocol.

## Sequence 4: the head decides ownership

Rows flow between operators as tuples of `&T` host columns plus any owned
values an `extend` computed. The head does not add clones:

- a bare head variable forwards its borrowed column, so
  `answer(c, a)` over heading `[a, c]` prints
  `move |(a, c,)| (c, a,)` and yields `(&T, &T)`;
- a computed head expression such as `answer(a + c)` is evaluated per row
  with every column in scope as `&T`, and its value is whatever that Rust
  expression produces.

Owned output therefore requires an authored operation: a computed head
expression, or a caller-side copy such as the composition example's
`.map(|(a, b, c)| (*a, *b, *c))` for display.

## Sequence 5: programs become typed values

Only after ownership is concrete, introduce the value that represents a
logical plan. An RA definition is a result name and one operator:

```rust
pub struct Definition {
    pub result: syn::Ident,
    pub eq_token: syn::Token![=],
    pub operator: Operator,
    pub semi_token: syn::Token![;],
}
```

`cq_lang_ra::contract::check` infers each result's attribute-set heading from
the declarations, complete renames, and earlier results; the syntax carries no
duplicate schema:

```text
r2 = natural_join r0 with r1;
```

This is not Rust execution. `r2` denotes a set relation; shared attributes and
natural-join equality follow from the intersection of the inferred headings.

`r0 = unit;` denotes the relation containing one empty binding. Elaboration
seeds the prefix with it whenever a guard or binding precedes the first atom,
including the constant bindings normalization introduces:

```text
relational {
    relation R(a: i32, b: i32);
    r0 = unit;
    r1 = extend r0 with { __cq_0_1: i32 } value ({ let __cq_0_1: i32 = 5; (__cq_0_1,) });
    r2 = rename R { a -> x, b -> __cq_0_1 };
    r3 = natural_join r1 with r2;
    r4 = project r3 keep { x };
    output r4 as answer(x);
}
```

Physical reuses the `rN` names but adds a mode to each definition:

```text
r2 = pipe probe[b] r0, r1;
```

A `pipe` result is a lazy stream read exactly once; a `let` result is retained
so it can be keyed or read repeatedly; an `alias` is a host read in place. The
mode, not the name, says whether a result is a cursor or storage.

## Sequence 6: construct typed values, not strings

`syn` gives the compiler typed Rust-backed syntax values. A pass may build
its target struct directly, as `cq_index_policy::compile` builds each
`ix::IndexDefinition`, or use `quote!` with `syn::parse2` where tokens are the
natural form, particularly for Rust expressions:

```rust
fn make_identifier(number: usize) -> syn::Ident {
    quote::format_ident!("r{number}")
}

fn make_expression(left: &syn::Ident, right: &syn::Ident) -> syn::Expr {
    syn::parse2(quote::quote!((#left, #right)))
        .expect("the quoted tuple expression is valid Rust")
}
```

Do not build a formatted string and parse it. Strings lose spans, make
identifier hygiene obscure, and create a second untyped surface grammar.

Every pass borrows its source program and returns its complete target
program; realization returns a `syn::Expr`. Tests may wrap that expression in a
Rust file, but the pass itself produces only the expression.

## Sequence 7: why proc macros take TokenStream

Rust fixes the ABI of a function-like proc macro:

```rust
#[proc_macro]
pub fn example(input: proc_macro::TokenStream) -> proc_macro::TokenStream {
    let expression: syn::Expr = match syn::parse(input) {
        Ok(expression) => expression,
        Err(error) => return error.into_compile_error().into(),
    };
    quote::quote!(#expression).into()
}
```

This does not imply that the query compiler should accept a token stream.
Token transport belongs at the Rust language boundary. Each facade entry
parses one source program, calls typed functions, and forwards the result to
the next macro through a path the caller resolves (usually by aliasing a local
module as `mini_linq`):

```text
mini_linq! { CQ }                     cq_lang_cq::contract::check, then emit
  -> mini_linq::cq_to_ra! { CQ }      caller picks cq_to_ra / cq_to_ra_generic /
                                      cq_to_ra_hypergraph: normalize, order,
                                      [reduce], elaborate
  -> mini_linq::choose_index_policy! { RA }
                                      caller binds to index_demands!
  -> mini_linq::physical_policy! { IX }
                                      caller macro appends `policy { … }` and
                                      invokes lower_physical!
  -> mini_linq::select_structures! { Physical policy { … } }
  -> mini_linq::structures_to_rust! { Structures policy { … } }
                                      prints through mini_linq::distinct_state!
  -> one Rust block expression
```

There is no extra public `expand_*` wrapper because each proc-macro entry
already owns the only token-level work.

## Sequence 8: local contracts and correspondence checks

A parser answers whether tokens match a grammar. A local contract answers
whether a program is meaningful inside its language. A correspondence check
answers whether a target program implements its source:

```text
contract::check(&program)                -> syn::Result<Analysis>
pass::compile(&source[, options])        -> syn::Result<Target>
pass::check(&source, &target[, options]) -> syn::Result<()>
```

`compile` checks its source contract (CQ's `Analysis` is `()`), constructs
the target, and ends by checking the target contract. Errors are span-aware
`syn::Error`s in debug and release builds; the proc-macro entry emits them as
`compile_error!`. `compile` does not call `check`: tests and tools invoke
`pass::check` explicitly, and it derives its verdict independently rather
than re-running the compiler.

Keep a compiler's output conventions separate from what its checker accepts.
`cq_elaborate::compile` emits renames at first mention and numbers results
`r0, r1, …`; the pass tests pin that exact output against the scenario `.ra`
file. `cq_elaborate::check` accepts any RA program with the VO program's
declarations and head whose erased kernel equals that CQ's kernel.
`cq_index_policy::check` is exact: the RA must be preserved unchanged, and the
demands must be the first-use descriptors `@rN[ordinals]` named `i0, i1, …`,
where the ordinals index the right operand's sorted heading. A join sharing no
attribute records an empty key, as in
[`cartesian.ix`](../test-support/relational/scenarios/cartesian.ix).

Realization has no `check`. Its evidence is oracle tests over executed output,
the `cq_storage::require` trait guards rustc checks, and the expansion
fixtures; re-parsing the printed block into a shadow plan would duplicate the
printer.

## Sequence 9: ordinary iterators express pull

Introduce the physical operations through values students already know:

```rust
fn joined<'a>(
    left: &'a [(City,)],
    right: &'a std::collections::HashMap<(&'a City,), Vec<(&'a City,)>>,
) -> impl Iterator<Item = (&'a City, &'a City)> + 'a {
    left.iter().flat_map(move |(x,)| {
        right
            .get(&(x,))
            .into_iter()
            .flatten()
            .map(move |&(y,)| (x, y))
    })
}
```

The `flat_map` closure retains the left binding while the inner cursor
produces rows. It is the Rust-native bind-shaped operation for a parameterized
nested loop. Rust's standard library does not define a general `Monad` trait,
and this compiler does not need one.

The Triangle Physical program states the operator and dataflow structure
before any Rust spelling:

```text
physical {
    relation R(first: i32, second: i32);
    relation S(first: i32, second: i32);
    relation T(first: i32, second: i32);
    r0 = alias rename R { first -> a, second -> b };
    r1 = let rename S { first -> b, second -> c };
    r2 = pipe probe[b] r0, r1;
    r3 = let rename T { first -> c, second -> a };
    r4 = pipe probe[a, c] r2, r3;
    r5 = pipe canon project r4 keep { a, b, c };
    return r5 as answer(a, b, c);
}
```

Structures appends the bindings Γ naming the value that answers each keyed
read (Sequence 2), and `cq_physical_realization::compile` prints them as
`map`, `flat_map`, and `filter` over `cq_storage` traits. The printer invents
no join or intermediate result.

## Sequence 10: laziness and distinct state

A program over hosts alone evaluates to the lazy iterator itself. A program
that binds any built structure collects its answers inside the block and
evaluates to `Vec::into_iter`, because the structures live only for the
block. Triangle under its policy is the second kind; the composition example
is the first.

Building the outer iterator constructs closure and adapter state. Advancing
it with `next()` or `any()` performs query work without collecting a result;
`collect()` is one possible consumer, not the trigger for execution.

CQ projection has set semantics, while a join cursor may enumerate several
witnesses for the same answer. Physical therefore flags a returned result
`canon` unless uniqueness is inferred. The printer deduplicates it through
the caller's provider, requesting `mini_linq::distinct_state! { operator r5;
new }` once and `insert(__cq_seen, __cq_row)` per row. With
`cq_state_providers::hash_distinct` this is equivalent to:

```rust
fn distinct_rows<'a>(
    rows: impl Iterator<Item = (&'a City, &'a City)>,
) -> impl Iterator<Item = (&'a City, &'a City)> {
    let mut seen = std::collections::HashSet::new();
    rows.filter(move |row| seen.insert(std::clone::Clone::clone(row)))
}
```

The filter is still lazy, but memory grows with first-seen answers. The clone
copies a tuple of references, not host data.

## Mapping Rust topics to homework passes

| Rust idea | Compiler use |
|---|---|
| borrowed typed syntax | every `compile(&Source, …)` input |
| `Vec`, `BTreeSet`, and maps as compiler state | clause binding, first-use demand order, name lookup |
| typed struct construction | target programs such as `ix::Program` |
| `quote!` and `syn::parse2` | Rust expressions and the printed block |
| borrows | host reads, structure builds, and all query traversal |
| `impl Iterator + '_` | cursor borrowing caller hosts |
| `flat_map` | `probe`: parameterized nested-loop join |
| closure capture | saved left bindings and distinct state |
| `syn::Expr` | realization's concrete Rust target |
| proc-macro `TokenStream` | language boundary only |

## Suggested guided labs

### Lab A: ownership trace

Give students Triangle's Physical, policy, and Structures programs and ask
them to predict the printed block, marking:

- which host each read borrows, and why no host is consumed;
- which reads need a built structure, and over which borrowed rows;
- why `r3` is keyed on both of `T`'s columns; and
- why no `Clone` appears, and which policy change would introduce one.

### Lab B: logical exhaustiveness

In the Triangle RA program, ask why fully bound `T(c, a)` remains a
`natural_join` while its Physical read becomes `probe[a, c]` with an empty
payload, and have students calculate every inferred heading by hand.

### Lab C: two namespaces

Place the Triangle RA and Physical programs side by side. Ask which names
denote sets, which are `pipe` cursors or `let` storage, where duplicate
derivations can arise, and why only the returned result needs `canon`.

### Lab D: inspect one compiler layer at a time

Run `cargo xtask explain triangle`, then open the generated `.tokens` files
under `target/mini-linq-explain/triangle/`. Use the editor's compare command
on adjacent files. They are syntax/diff snapshots outside Cargo's crate graph,
not type-checked modules. Compare:

1. `01-cq.tokens` with `02-relational-plan.tokens` (RA); and
2. `02-relational-plan.tokens` with `03-index-requirements.tokens` (IX).

The stepper stops at IX; read the Physical and Structures stages from the
scenario files and the printed Rust from the
[full-chain test](../facade/mini-linq/tests/physical_query.rs). Students can
stop early with `cargo xtask explain triangle --through relational-plan`; a
later failure leaves the earlier artifacts and an `ERROR.md` diagnostic.

Rust Analyzer's **Expand macro recursively at caret** is only a final-code
view. It cannot pause after an individual pass, and `workload_query!` only
captures the CQ source and a `module()` factory; `xtask` invokes the typed
planning passes. For optional implementation-level debugging, set a
breakpoint in the relevant pass and use Rust Analyzer's test CodeLens with
CodeLLDB; the course does not require that debugger.

### Lab E: partial cursor consumption

Change the composition example to return its lazy iterator, request one row,
pause, request a second, and drop the cursor. Wrap `L` in a host type whose
`iter()` counts advanced rows to show that constructing the cursor scans
nothing and dropping it scans no further. Repeat with Triangle's policy and
observe that collection inside the block removes this effect.

## Common misconceptions to surface

- **“A proc macro takes tokens, so the compiler pass should too.”** Rust fixes
  the outer ABI; the internal compiler uses typed programs.
- **“IX can replace RA.”** An `ix::Program` is the complete RA program plus
  its `indexes { … }` block, because physical lowering consumes both.
- **“`r2` is an intermediate table.”** In RA it is a set-valued result; no
  materialization decision exists at that layer.
- **“A `pipe` is a `for` loop.”** It is a lazy cursor value with resumable
  state.
- **“An iterator runs only when collected.”** `next()` and short-circuit
  consumers also execute it.
- **“A Physical `probe` already means `flat_map`.”** It names the operator;
  the printer chooses the exact Rust combinators.
- **“Quoting is code generation.”** Semantic decisions are already fixed in
  Structures; printing only chooses Rust spelling.
- **“The compiler owns the input.”** The caller owns every host; the printed
  block only borrows.
- **“Materialization provides set semantics.”** The `canon` flag and the
  caller's distinct-state provider do; collection only stores.
- **“Async makes pull more lazy.”** Async adds suspension for waiting, not the
  demand-driven iterator semantics used by finite in-memory execution.

## Optional Homework 4 transition

After the basic tutorial, [Optional HW4](HW4-OPTIONAL.md) asks students to
change semantic IR design rather than only generated Rust. The CQ parser
already accepts negation and aggregation body items, but the baseline CQ
contract rejects them. Students make them valid, then add logical
`AntiSemijoin` and `AggregateApply` operators before deciding indexes and
physical reads.

When a global extended clause appears before a positive atom, students use
the supplied `unit` as the left input. Correlation is inferred from heading
intersection, so a global clause has an empty intersection. Where the
extension derives a demand for it, the IX rule records the empty key `[]`,
exactly as the baseline already does for a cartesian join.

The `hw4` feature gates only the optional test targets; it does not change the
meaning of `mini_linq!` or select an execution backend. New semantics belong
in an explicit typed language and pass, not in a configuration-dependent
printer.
