# Rust tutorial design for MiniLinq

The Rust material should make the compiler architecture feel inevitable. Start
with concrete values, moves, borrows, and iterator objects; only then name
syntax trees, proc macros, and pass contracts. The central bridge is:

> A program can be represented as a value, transformed into another value,
> and finally emitted as an ordinary Rust expression.

The [complete Triangle expansion](DSL.md#complete-triangle-expansion) is the
running example. It shows the whole program after every transformation, so no
concept depends on an invisible compiler stage.

## Learning outcomes

After the tutorial, students should be able to explain and implement:

1. the difference between an input iterator, an owned relation, and a borrowed
   query cursor;
2. why a proc-macro function accepts `TokenStream` while compiler passes accept
   typed IR syntax;
3. how `syn::parse2` and `quote!` turn values into the next typed language;
4. why `rN` is a logical relation result and `iterN` is a lazy cursor result;
5. how Rust's `once`, `map`, `flat_map`, and `filter` encode a pull pipeline;
6. why the final `syn::Expr` is the concrete target IR rather than a
   pretty-printing hint; and
7. where ownership permits a move, requires a borrow, or deliberately clones
   fields into an additional physical object.

## Sequence 1: a public input is a one-shot value

Begin with a normal generic function:

```text
relation Road(src: City, dst: City);
```

This declaration is a schema: its two entries are real Rust column types. It
does not contain road facts. The host supplies a particular `Road` relation
instance to the generated `build` or `run` argument, whose row type is the
always-trailing tuple `(City, City,)`.

```rust
type City = String;

fn normalize(
    input: impl IntoIterator<Item = (City, City,)>,
) -> std::collections::HashSet<(City, City,)> {
    input.into_iter().collect()
}
```

`IntoIterator` is not a fixed collection type. It names a value that can be
consumed to produce rows. Calling `into_iter()` takes that value. This matters
because the generated `Program::build` must consume each used public input
exactly once, regardless of whether the caller supplied a `Vec`, array, set,
or user-defined iterator.

Normalizing to `HashSet<Row>` establishes set semantics before storage layout
is chosen. It also creates a borrowable intermediate owner from which several
physical structures may be constructed.

Ask students to predict the result of trying to call `input.into_iter()` a
second time. The compiler error introduces ownership as an observable design
constraint, not an abstract rule.

## Sequence 2: move versus clone during storage construction

A single physical owner can consume an owned normalized relation:

```rust
fn row_store(
    relation: std::collections::HashSet<(City, City,)>,
) -> Vec<(City, City,)> {
    relation.into_iter().collect()
}
```

A full-row key remains a `HashSet<Row>`, so the correct construction is just a
move:

```rust
fn full_index(
    relation: std::collections::HashSet<(City, City,)>,
) -> std::collections::HashSet<(City, City,)> {
    relation
}
```

Recollecting that same set would perform work with no representation change.
This is the first practical compiler lesson: generated Rust should respect the
host language's ownership model rather than mechanically clone every input.

If one relation needs two physical objects, the earlier builder must borrow
while creating owned values for the additional object:

```rust
fn proper_index(
    relation: &std::collections::HashSet<(City, City,)>,
) -> std::collections::HashMap<(City,), Vec<(City,)>> {
    let mut index = std::collections::HashMap::<(City,), Vec<(City,)>>::new();
    for (key, payload,) in relation.iter() {
        index
            .entry((std::clone::Clone::clone(key),))
            .or_default()
            .push((std::clone::Clone::clone(payload),));
    }
    index
}
```

The field clones here are deliberate: the new `HashMap` owns a separate key
and payload representation. After that borrowing builder finishes, the final
consumer may move the original set. Distinguish these build-time clones from
query traversal, where stored tuples remain borrowed.

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

This matches database-driver language: building the result cursor is a query
step, while creating physical state happened in `Program::build`.

Then show partial consumption:

```rust
fn first_two(rows: &Rows) -> Vec<(City, City,)> {
    let mut cursor = rows.query();
    let first = cursor.next().cloned();
    let second = cursor.next().cloned();
    [first, second].into_iter().flatten().collect()
}
```

The iterator object holds resumable state. There is no outer `for` loop hidden
in the surrounding application; the cursor itself is the whole result-set
protocol.

## Sequence 4: source access should borrow stored tuples

A generated scan should be:

```rust
fn scan(rows: &[(City, City,)]) -> impl Iterator<Item = &(City, City,)> {
    rows.iter()
}
```

An indexed lookup can also remain borrowed:

```rust
fn lookup<'a>(
    index: &'a std::collections::HashMap<(City,), Vec<(City,)>>,
    key: &City,
) -> impl Iterator<Item = &'a (City,)> {
    index.get(&(std::clone::Clone::clone(key),)).into_iter().flatten()
}
```

Appending `.cloned()` would produce an owned tuple for every candidate. The
baseline instead borrows all stored rows and payloads, clones only the fields
needed to construct an owned lookup key, and creates one owned result tuple at
successful projection.

Rust match ergonomics explain the generated patterns. For a borrowed scan,
the body of

```rust
fn enumerate(rows: &[(City, City,)]) -> Vec<(City, City,)> {
    let mut output = Vec::new();
    for (x, y,) in rows.iter() {
        output.push((
            std::clone::Clone::clone(x),
            std::clone::Clone::clone(y),
        ));
    }
    output
}
```

receives `x: &City` and `y: &City`; cloning those selected fields constructs
the owned output row.

## Sequence 5: programs become typed values

Only after ownership is concrete, introduce a small value that represents a
query plan:

```rust
struct Definition {
    result: syn::Ident,
    operator: Operator,
}

type Heading = std::collections::BTreeSet<String>;
```

A definition serializes its result identity and operator. The RelationalPlan
checker infers its attribute-set heading from complete Renames and earlier
results; it does not duplicate an ordered schema in the syntax:

```text
r2 = natural_join r0 with r1;
```

This is not yet Rust execution. `r2` denotes a set relation, and later logical
operators may name it. Shared attributes and natural-join equality follow from
the intersection of the inferred headings.

The supplied definition `r0 = unit;` is the useful zero-heading exception. It
denotes a relation containing one empty binding, analogous to an identity
value from which a global logical clause can proceed. The positive basic pass
does not emit it; Optional HW4 uses it before an uncorrelated negation or
aggregation.

Contrast it with a physical cursor identity:

```text
iter1 = join iter0 as (x, y,) with (z,) in (source) yield (x, y, z,);
```

`iter1` denotes a lazy Rust-compatible stream. The input pattern and binding
record describe the iterator item passed across one physical operator boundary.
Using separate names prevents `r2` from being mistaken for an allocated
intermediate relation or `iter1` from being confused with set semantics.

## Sequence 6: parse and quote values, not strings

`syn` gives the compiler typed Rust-backed syntax values. `quote!` is a typed
construction aid, and `syn::parse2` ensures the constructed tokens belong to
the target language:

```rust
fn make_identifier(number: usize) -> syn::Ident {
    quote::format_ident!("rel{number}")
}

fn make_expression(left: &syn::Ident, right: &syn::Ident) -> syn::Expr {
    syn::parse2(quote::quote!((#left, #right)))
        .expect("the quoted tuple expression is valid Rust")
}
```

Do not build a formatted string and parse it. Strings lose spans, make
identifier hygiene obscure, and create a second untyped surface grammar.

The student pass follows the same pattern at a larger scale:

```rust
fn preserve_expression(source: &syn::Expr) -> syn::Expr {
    syn::parse2(quote::quote!(#source))
        .expect("an existing expression is still valid Rust")
}
```

In the real pass, the target is a nominal MiniLinq IR or complete `syn::File`,
not merely an expression. The important principle is unchanged: borrow the
input syntax value, construct the complete next value, then parse into the
declared target type.

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

This does not imply that the query compiler itself should accept a token
stream. Token transport belongs at the Rust language boundary. MiniLinq's
direct entries parse exactly one source type, call one typed compiler function,
and route or emit the returned type:

```text
mini_linq(TokenStream)
  -> parse cq::Module
  -> compile_cq(&cq::Module)
  -> quote choose_indexes! { target }
```

The same pattern repeats for RelationalPlan, IndexRequirements,
RustAccessPlan, and IteratorPipeline. There is no extra public `expand_*`
wrapper because the proc-macro entry already owns the only token-level work.

## Sequence 8: local contracts and edge contracts

A parser answers whether tokens match a grammar. A local contract answers
whether an object is meaningful inside a language. The two APIs make that
choice explicit:

```text
contract::check(&value)       -> syn::Result<()>
contract::well_formed(&value) -> bool
```

The public typed pass calls its source's `contract::check` before lowering and
returns a span-aware `syn::Error` for invalid input in both debug and release
builds. The proc-macro entry forwards that error as `compile_error!` without
repeating the semantic check. Debug postconditions separately check the
specified source-to-target relation; disabling them does not disable source
validation. Tests and tools can also call the diagnostic form directly.

An edge contract is stronger than target well-formedness. For example, any
locally valid compiler-produced RelationalPlan is not enough: Basic HW1 must
produce one complete Rename per source occurrence, source-order left-deep
NaturalJoins, the final Project, output metadata, and `rN` numbering for its
source CQ. Basic HW2 must preserve that exact plan while adding exactly the
canonical keys inferred from heading intersections and Rename inversion.

The physical target is a complete Rust AST, so tests inspect its individual
Rust items, storage builders, local macro, and runtime behavior. Re-parsing the
whole `syn::File` into a second shadow plan would duplicate the compiler.

## Sequence 9: ordinary iterators express pull

Introduce the physical operations through values students already know:

```rust
fn joined<'a>(
    left: &'a [(City,)],
    right: &'a std::collections::HashMap<(City,), Vec<(City,)>>,
) -> impl Iterator<Item = (City, City,)> + 'a {
    left.iter().flat_map(move |(x,)| {
        right
            .get(&(std::clone::Clone::clone(x),))
            .into_iter()
            .flatten()
            .map(move |(y,)| {
                (
                    std::clone::Clone::clone(x),
                    std::clone::Clone::clone(y),
                )
            })
    })
}
```

The `flat_map` closure retains the left value while the inner cursor produces
rows. It is the Rust-native bind-shaped operation for a parameterized nested
loop. `std::iter::once` plays the pure/return-shaped role when a standalone
empty binding is needed. Rust's standard library does not define a general
higher-kinded `Monad` or monad-transformer trait, and this compiler does not
need one.

The explicit IteratorPipeline appears before these concrete combinators:

```text
iter0 = scan (x,) in (left.iter()) yield (x,);
iter1 = join iter0 as (x,) with (y,) in (right_source) yield (x, y,);
iter2 = project iter1 as (x, y,) yield ((x.clone(), y.clone(),));
iter3 = distinct iter2;
return iter3;
```

That plan states the physical operator and dataflow structure, but not the
host-language spelling. `cq_pipeline::iterator::compile` is the semantic lowering
that maps it to `map`, `flat_map`, and `filter`. This keeps the final emitter
honest: it does not invent a join or intermediate result while printing.

## Sequence 10: root laziness and distinct state

If the source expression itself can perform work, merely creating adapters
outside a closure may evaluate it too early. The supplied Rust lowering wraps
all named locals in:

```rust
fn deferred() -> impl Iterator<Item = i32> {
    Iterator::flatten(std::iter::once_with(move || {
        let iter0 = std::iter::once(1);
        let iter1 = Iterator::map(iter0, |value| value + 1);
        iter1
    }))
}
```

The closure is invoked only when the outer iterator is first advanced. The
complete Triangle final residual uses the same shape and declares one
`let iterN` for every operator.

CQ projection has set semantics, while a join cursor may enumerate several
witnesses for the same result. IteratorPipeline therefore includes an explicit
`Distinct` after `Project`. Its Rust implementation owns state:

```rust
fn distinct_rows(
    rows: impl Iterator<Item = (City, City,)>,
) -> impl Iterator<Item = (City, City,)> {
    let mut seen = std::collections::HashSet::new();
    rows.filter(move |row| seen.insert(std::clone::Clone::clone(row)))
}
```

The filter is still lazy, but memory grows with first-seen results. It clones
the owned result tuple into `seen`; this is not a clone of an input row or an
index payload. `Storage::materialize` only collects and sorts this already
distinct result cursor.

## Mapping Rust topics to homework passes

| Rust idea | Compiler use |
|---|---|
| borrowed typed syntax | every `compile_*(&Source)` input |
| `Vec`, `BTreeSet`, and maps as compiler state | left-to-right relation binding, stable requirement union, name lookup |
| `quote!` interpolation | complete target construction |
| `syn::parse2` | target grammar check and typed return value |
| moves | final ownership-taking storage consumer |
| borrows | earlier builders and all query traversal |
| `impl Iterator + '_` | cursor borrowing generated storage |
| `flat_map` | binary parameterized nested-loop join |
| closure capture | saved left bindings and distinct state |
| `syn::File` and `syn::Expr` | concrete Rust target IRs |
| proc-macro `TokenStream` | language boundary only |

## Suggested guided labs

### Lab A: ownership trace

Give students the Triangle `build` method and ask them to mark:

- where each public input is consumed;
- which normalized relation each builder borrows or moves;
- why the full-key index is a direct move; and
- why query-time `.cloned()` would be unnecessary.

### Lab B: logical exhaustiveness

In the complete RelationalPlan, ask why fully bound `T(z, x)` remains a
NaturalJoin while its physical access becomes membership, and have students
calculate every inferred attribute-set heading by hand.

### Lab C: two namespaces

Place the Triangle RelationalPlan and IteratorPipeline side by side. Ask which
names denote sets, which denote cursors, where duplicate derivations can arise,
and why only the cursor side needs explicit `Distinct`.

### Lab D: inspect one compiler layer at a time

Run `cargo xtask explain triangle`, then open the six generated `.rs` files
under `target/mini-linq-explain/triangle/`. Use the editor's compare command on
adjacent files to inspect them. They are syntax/diff snapshots outside Cargo's
crate graph, not Rust Analyzer type-checked modules. Compare:

1. `mini_linq!` to RelationalPlan;
2. RelationalPlan to the index overlay;
3. the complete staged Rust file;
4. RustAccessPlan to IteratorPipeline; and
5. the final named-local Rust expression.

Students should compare each result with the six authoritative DSL artifacts,
not inspect only the final expansion. They can stop early with, for example,
`cargo xtask explain triangle --through relational-plan`; a later failure
leaves the successful earlier artifacts and an `ERROR.md` diagnostic.

Rust Analyzer's **Expand macro recursively at caret** is only a final-code
view. It cannot pause after an individual pass, and `workload_query!` directly
calls the first three typed functions rather than exposing their results as
nested macros. Do not re-expand its virtual `[EXPANSION].rs` buffer. For an
optional implementation-level debugging exercise, set a breakpoint in the
relevant pass and use Rust Analyzer's test CodeLens with CodeLLDB; the course
does not require that debugger.

### Lab E: partial cursor consumption

Build Triangle storage, call `query()`, request one row, pause, request a
second row, and then drop the cursor. Instrument a source expression to verify
that constructing the cursor performs zero source work and that dropping it
does not scan remaining rows.

## Common misconceptions to surface

- **“A proc macro takes tokens, so the compiler pass should too.”** Rust fixes
  the outer ABI; the internal compiler should use typed values.
- **“IndexRequirements can replace the logical plan.”** The overlay must carry
  the complete logical dataflow because later physical lowering now consumes
  that plan.
- **“`r2` is an intermediate table.”** It is a set-valued mapping in logical
  algebra. No materialization decision exists at that layer.
- **“`iter2` is a `for` loop.”** It is a lazy cursor value with resumable state.
- **“IteratorPipeline already means `flat_map`.”** It names the operator edge;
  the next pass chooses the exact Rust combinator.
- **“Quoting is code generation.”** Once the final target is `syn::Expr`, the
  quoting step is pure token emission. Semantic work must already be done.
- **“Laziness means indexes are built on first `next()`.”** All physical
  storage is eagerly built by `Program::build`; only query traversal is lazy.
- **“Materialization provides set semantics.”** Explicit cursor `Distinct`
  does; materialization only collects and sorts.
- **“Async makes pull more lazy.”** Async adds suspension for waiting, not the
  demand-driven iterator semantics used by finite in-memory execution.

## Optional Homework 4 transition

After the basic tutorial, Optional HW4 asks students to change semantic IR
design rather than only generated Rust. They extend the CQ body, then add
logical `AntiSemijoin` and `AggregateApply` operators before deciding indexes
and concrete accesses. Numbered comments point to each required syntax,
contract, pass, and test region.

Staff already supplies logical `Unit`. When a global extended clause appears
before a positive atom, students use that empty-heading singleton as the left
input. Correlation is inferred from heading intersection, so a global clause
has an empty intersection rather than a serialized `()` key. AntiSemijoin may
preserve that empty heading; the following global aggregate appends its owned
output. The empty intersection creates no index entry.

Students cumulatively edit the production grammar and passes on that
checkpoint. The `hw4` feature gates only the optional test targets; it does
not change the meaning of `mini_linq!` or select an execution backend. This
reinforces the same lesson as the baseline: new semantics belong in an
explicit typed language and pass, not in a hidden configuration-dependent
printer.
