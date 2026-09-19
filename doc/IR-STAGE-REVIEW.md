# IR boundary review

The specification is [IX and Physical](IX-PHYSICAL-REDESIGN.md); executable
provider messages are specified [separately](PROVIDER-PROTOCOL.md).
A parser accepting a program does not mean every selected implementation admits
that program.

| Boundary | Information introduced | Information preserved / excluded |
|---|---|---|
| CQ → RA | Chosen logical operators and dependency graph | Input schemas, Rust leaves and output meaning; no storage selection |
| RA → IX | Named exact-version index demands and each join's index reference | Entire RA unchanged; no provider or allocation |
| IX → Physical | Provider for each relation/index and lowering rule for each RA definition | Entire IX unchanged; choices are opaque external Rust paths |
| Physical → Rust[Access] | Selected storage aliases/construction and concrete Rust access expressions | Follow recorded choices; no re-selection, hidden materialization or index-demand analysis |
| Access → selected consumer | Execution behavior of already chosen Rust leaves | Independently specified endpoint contract; push/pull placement remains unresolved |

## IX is complete index analysis

For each NaturalJoin or Semijoin, IX records the exact right relation version and
the sorted logical ordinals of its joined columns. Identical `(version, columns)`
demands share one identity. Empty joined-column sets still have an explicit
unit-column index demand. Every indexed position names its index:

```text
indexes { i0 = @r[0]; i1 = @s[0]; }
uses { j0.right = i0; j1.right = i1; }
```

Structural checks reject missing/wrong-version uses, unused definitions,
duplicate descriptors, and invalid ordinals. Physical rules translate the
written logical ordinals through the selected representation; they do not
rediscover which indexes the RA plan needs.

## Physical freezes three independent choices

Physical is the unchanged IX payload plus `relations`, `indexes`, and
`lowerings` tables. These tables cover referenced host inputs and all RA
results, every IX index, and every RA definition, respectively. Host inputs
have no lowering entry. The current one-name relation table rejects a host
input/result name collision.

A path identifies executable external code; its spelling does not certify a
capability. Structural selection checks domains. Selected implementations and
Rust type checking establish admission; their denotational laws require
separate reasoning and tests. Recording the selected lowering is necessary
because a representation alone does not determine an access algorithm.

## Realization has administrative states, not an extra query IR

The driver keeps Physical unchanged, records established bindings, and issues
the selected Input/Index/Lower requests in dependency order. Replies establish
`stored`, `indexed`, or `region` bindings, with identities and kinds checked.
An index denotes its exact IX source/version. A region is Access syntax,
not a Rust collection; an index request over a region is rejected today.

External exact-join operations return Access fragments with a witness and an
ordinal-to-expression field map. The selected output provider constructs the
carrier from those interpreted fields. The driver does not understand hash,
sorted, block, or trie names. Callbacks are visible macro expansions inside
this one realization computation.

The final region's logical head must equal the normalized RA heading in sorted
order, and the output metadata must match that order. The realizer changes the
predicate name to the source output name and invokes the caller's `access!`.
General output permutation/computed heads and a final stored result require
additional selected behavior; they are not silently adapted.

## Verified scope and remaining gaps

The implemented provider family supports direct input Rename, binary indexed
NaturalJoin, and terminal Project over borrowed scalar slices and two-column
hash/sorted maps. The
map's existing first-column index is reused; a different logical order produced
by Rename is supported. Tests include non-Clone payloads, mixed providers,
caller-defined output macros, invalid assignments, and actual macro captures.
The selected Project rule accepts only the last RA definition named by output;
intermediate projection rejects before dropped names can leak into later joins.
The [full-CQ trace](ix-physical/query-trace/review.md) exercises this terminal rule;
the [direct-RA trace](ix-physical/realization-trace/review.md) remains separate.

The logical planner still constructs broader RA, including YA semijoins. This
does not imply physical YA support. Block/trie access, semijoin lowerers,
intermediate materialization, shared lazy indexes, Select/Extend, nonterminal
projection, and
generated host APIs are not supplied by the new example providers.

Existing Access → Pipeline → Rust and Access → eager Rust implementations have
separate observation contracts. Their independent tests remain useful. Their
availability does not settle when execution-model selection should occur in
the new planning architecture.
