# Executable provider realization protocol

Status: implemented for the host-expression domain below. The public stages
remain `RA → IX → Physical → Rust[Access]`; these messages are administrative
continuations within realization. Other existing routes are not thereby
certified conforming.

`cq-physical-lowering` schedules calls using the paths already recorded in
`P = (IX, Φ, Λ)`. It preserves `P` and has no concrete provider dependency or
provider-name cases. Executable external macros generate their own Rust
statements, source expressions, and result witnesses. Selected RA rules assemble
Access clauses. The core does not interpret Rust templates
returned by another DSL.

## Driver requests and replies

The implemented syntax is in `cq-ir::provider_protocol`, using derive-based
parsing and printing. `P` is the complete Physical payload. `Γ` is an ordered
list of established bindings. The braces following `reply_to` and `resume`
contain opaque continuation tokens, interpreted only by their owner.

```text
Request<T> ::= reply_to (MacroPath) { state }
               plan { P }
               environment { Result* }
               request { T }
Task       ::= input r from (RustExpr)
             | index i
             | lower r
Reply      ::= resume { state }
               emitted { RustStmt* }
               result { Result }
Result     ::= stored r at (RustExpr) operations MacroPath
             | indexed i at (RustExpr) operations MacroPath
             | region r { Access }
```

`input r` is sent to `Φ_R(r)`, `index i` to `Φ_I(i)`, and `lower r` to
`Λ(r)`. A selected lowerer requests result construction from `Φ_R(r)`.
The common driver handles only these three tasks. `Request<T>` also transports
an external family's typed tasks; adding such a family does not add a case to
the common driver.

The driver orders tasks as follows:

1. Establish each used host input once, in first-Rename-occurrence order.
2. Visit the already ordered RA definitions. Immediately before a definition,
   establish each index named by its explicit IX use that has not yet been
   established. Its exact source version must already be stored.
3. Invoke that definition's recorded lowering. Continue with the next definition.

Its serialized continuation is:

```text
State ::= plan { P }
          environment { Result* }
          emitted { RustStmt* }
          pending { Task }
          continuations (ResumeMacroPath, AccessMacroPath)
```

On resumption the driver checks Physical structure, checks that the environment
is a prefix of this task order, and checks the pending task and reply identity.
An input requires `stored`, an index requires `indexed`, and an RA lowering may
return `stored` or `region`. The index identity remains attached to the exact
`D(i) = (r,C)` through unchanged `P`; relation and index identifiers occupy
separate namespaces. These are structural checks, not authentication of an
external macro or proof of its representation law.

The driver appends emitted statements in reply order, without hoisting access
expressions. A trailing value expression in `emitted` rejects rather than
silently receiving a semicolon. It never changes `Φ` or `Λ`. Unsupported
combinations reject during expansion or Rust type checking.

The operations path names an executable macro for the established
representation. It can be the provider itself. The core forwards the path
without inspecting its spelling. The provider owes the law connecting that
macro and its emitted value to the selected relation or exact index.

A `region` is Access syntax for a computation, not an iterable storage value.
An index request whose source is a region rejects. A producer returning `stored`
must explicitly establish the selected storage with emitted code. There is no
implicit `collect`, clone, or materialization fallback. The supplied join
lowerer can extend an existing region on its left; its right index must be
established from stored input.

Borrowing is expressed by returned Rust expressions and statements. Rust checks
their lifetimes; provider laws additionally require them to denote the recorded
immutable version. No `Clone`, `Copy`, universal `&Row`, owner ID, or runtime
relation wrapper is required by the generic driver.

## Exact indexed binary-join family

The supplied external examples use these optional messages from
`cq-ir::provider_protocol::exact`:

```text
Task          ::= rows r
                | matches i values [(RustExpr), ...]
                | output r fields { (c = (RustExpr);)* }
                           whole { (source = (RustExpr);)* }
IteratorReply ::= resume { state }
                  source (RustExpr) item (RustPat)
                  witness { row (RustExpr)
                            fields { (c = (RustExpr);)* } }
WitnessReply  ::= resume { state }
                  witness { row (RustExpr)
                            fields { (c = (RustExpr);)* } }
```

`rows` enumerates the current stored relation. `matches` receives borrowed
logical field expressions in the recorded index-column order and applies the
index's encoding. Rows and Matches return `IteratorReply`: the expression
enumerates matching items, the pattern describes their bindings, and the witness
maps logical column ordinals to Rust expressions. The selected RA rule adds the
occurrence label and assembles `=> bind PAT <- (EXPR)`. The provider supplies no
runtime query continuation, Access clauses, or nested join. For a map bucket, the payload can
supply only the second physical column: the first is already supplied by the
lookup argument. This implements the paper's `dec_i(k,w)` obligation.

`output` supplies every field in the result's logical heading. `whole` records
available operand witnesses by exact source-version name; it may be empty.
An output provider may reuse a witness only under its representation law.
The supplied `borrowed_fields` constructs a tuple from the supplied borrowed
fields and ignores the optional whole witnesses. It emits no collection and
clones no payload. Output returns `WitnessReply`, which cannot contribute a
source or Access clause.

The witness's `row` is compiler metadata. It is evaluated only where generated
Rust actually uses it. A lookup depending on an outer row remains a source under
that preceding Access binding. `bind` denotes dependent enumeration; it does not
choose eager execution. Pull consumers defer source evaluation until demand;
the eager consumer emits loops. The named Pipeline IR remains unchanged, with
query-composition `flat_map` introduced only by its final Rust lowering. These messages disappear
during realization; no additional
retained physical-operation IR is introduced. Block pruning and trie traversal
need their own admitted operations and laws; the exact family does not imply
those capabilities.

## Implemented host-expression domain

`realize_physical!` takes a complete Physical module as a Rust expression.
The module retains CQ schemas and RA output metadata, but this entry does not
generate a `struct` or a `run` method. Each used host relation identifier must
already denote a Rust expression in the caller's scope. The selected provider
checks its concrete type through emitted Rust annotations.

The first external package, `cq-example-providers`, supplies:

| Macro | Admitted representation or rule |
|---|---|
| `slice_rows!` | A borrowed unary slice `&[T]`. |
| `hash_relation!` | A borrowed two-column `HashMap<K, Vec<V>>`. |
| `sorted_relation!` | A borrowed two-column `BTreeMap<K, Vec<V>>`. |
| `borrowed_fields!` | The selected output carrier, a tuple of borrowed logical fields. |

The selected operator rules are exported by `mini-linq` and implemented in
`cq-physical-lowering`. They consume the provider protocol but contain no
concrete collection implementation:

| Macro | Admitted rule |
|---|---|
| `rename!` | A direct Rename of a declared host input; asks the selected result provider to establish its representation. |
| `indexed_join!` | NaturalJoin with an explicit IX use, an exact matching RHS index, and a stored or normalized Access-region LHS. |
| `project!` | Terminal Project preserving source bindings and selecting only its output fields. |

The map index providers reuse their existing first physical column index. They
translate that position through Rename into the result's logical heading and
check it against the already recorded IX demand. They do not discover join
columns, create another index, or fall back to scanning. Another indexed column,
a composite/empty demand, or a mismatched concrete map representation rejects.
The supplied operator family admits terminal Project but does not implement Unit,
intermediate Project, Select, Extend,
Semijoin, reusable intermediate construction, zone pruning, or WCOJ.

Every returned region must satisfy Access's binding contract and expose exactly
the normalized RA heading in logical column order. At completion the final
binding must be a region, and the declared output must have the same normalized
order. Other output layouts reject. The driver changes only the head predicate
name to the declared output name, then emits:

```text
{ emitted-statements... AccessMacroPath! { final-Access-region } }
```

The Access consumer is explicitly supplied by composition. The integration test
selects the existing set-loop consumer; this does not settle push/pull placement
or select a default consumer for other programs.

## Executed heterogeneous sequence

The integration query in
`crates/mini-linq/tests/physical_realization.rs` has a slice `L`, a hash map `R`,
a B-tree map `S`, direct Rename results `l,r,s`, then
`j0 = natural_join l with r` and `j1 = natural_join j0 with s`.
The driver's task order is exactly:

```text
input L, input R, input S,
lower l, lower r, lower s,
index i0, lower j0,
index i1, lower j1
```

This lists tasks, not every macro expansion. Each Rename calls its selected
result provider. The first join requests `rows l`, then `matches i0`, then
`output j0`. The second extends `j0`'s existing Access body, requests `matches
i1`, then `output j1`. Each operation has a separate callback; completed
Rust[Access] must not be presented as `realize_physical!`'s immediate output.

The integrated tests compare borrowed results with independent relational
enumeration, use fields implementing neither `Clone` nor `Copy`, check source
addresses, and test incompatible relation/index assignments and index columns.
Generic driver tests additionally cover malformed replies, task order, and
unsupported region indexing. These validate the admitted domain, not arbitrary
external providers or all remaining compilation routes.

The local two-output fixtures
in `doc/ix-physical/` remain authored design expectations; the implemented entry
consumes a complete module with one declared final output.

## Distinct state and exact-set sinks

Execution endpoints require explicit caller bindings, independently of input
relation providers:

```rust
mod wiring {
    pub use ::mini_linq::*;
    pub use cq_example_providers::{
        ordered_distinct as distinct_state,
        ordered_set_sink as set_sink,
    };
}
use wiring as mini_linq;
```

`Distinct` remains a semantic Pipeline operator. Its lowering emits:

```rust
let mut seen = mini_linq::distinct_state! { operator unique; new };
// Inside the lazy filter; row is borrowed, then yielded unchanged if accepted:
mini_linq::distinct_state! { operator unique; insert(seen, row) }
```

The `insert` result must be true exactly on the first occurrence of an equal
value. The provider owns retained representation and copying. It must not consume
the candidate, poll future input, or change value equality. `new` is deferred
until the first iterator poll. An ordered set implementation still emits first
occurrences in input order; it does not sort the stream.

Set endpoints use `set_sink!` with `new`, `insert(state, value)`, and
`finish(state)`. Insert consumes a value; finish consumes the state and returns
an exact set of supplied values. This law authorizes folding terminal Distinct
into the sink. A bag sink is not admissible. No collection type or Clone bound
is imposed by the sink lowering. Hash and ordered examples choose HashSet and
BTreeSet; callers can supply other lawful implementations.

Every request begins `operator name;`. For Pipeline it is the Distinct result
stream name; for direct Access set loops it is the output relation name. These
names are local to an invocation. An alias selects one family for that scope;
a caller-written routing macro can select different providers by operator name.
Missing bindings fail macro resolution. Support and existence endpoints omit
terminal distinct state entirely. Provider laws remain obligations; choosing a
macro path does not mechanically prove equality or set semantics.

| Supplied provider | Retained representation | Value requirements |
|---|---|---|
| `hash_distinct!` | HashSet of cloned values | Clone + Eq + Hash |
| `ordered_distinct!` | BTreeSet of cloned values | Clone + Ord |
| `hash_set_sink!` | HashSet of owned values | Eq + Hash |
| `ordered_set_sink!` | BTreeSet of owned values | Ord |

These requirements belong to those implementations. The integration tests also
use an external distinct provider retaining only injective integer keys for
non-Clone payloads, and a vector-backed sink returning a boxed custom result.
