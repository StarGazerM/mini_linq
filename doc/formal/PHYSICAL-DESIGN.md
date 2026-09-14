# Physical choices through Access refinement

[physical_design.tex](physical_design.tex) builds the supplement. Its
[architecture discussion](architecture_placement.tex) identifies existing compiler
boundaries; [access_refinement.tex](access_refinement.tex) defines the implemented
native operations, representation laws, 14 rules and two algorithms. The
[design record](../ACCESS-REFINEMENT.md), [module map](../COMPILER-STRUCTURE.md) and
[single final boundary audit](../IR-STAGE-REVIEW.md) connect the theory to the code.

The canonical graph is CQ → RA → IX → Rust[Access] → Rust[Pipeline] → Rust.
The configured direct edge is RA → Rust[Access], with layouts, current versions,
selected providers and resource sharing in a compiler-local environment Γ.
There is no serialized physical Context or empty index forwarding pass.

A semijoin lowers to candidate traversal followed by exact membership and current
selection. Row scans discard metadata tokens. Pruned scans expand local Access
into Blocks → metadata guard → current BlockRows. Key construction projects the
current RHS through Access; its reuse identity is current RHS, ordered separator,
key-provider path and collection family, independent of candidate scan choice.
Zone bounds permit conservative rejection, while exact full-key membership
preserves semijoin semantics. The rule applies to candidates plus residual together.

Hash and nested witnesses implement the same Build/Probe interface with different
construction and probe timing. The support-preserving stream endpoint avoids a
terminal Distinct where a set sink or conservative accumulation tolerates repeated
keys. It preserves the pre-Distinct sequence, so unique provider traversal stays
unique; support equality alone is insufficient for uniqueness-sensitive consumers. It leaves the canonical distinct iterator
endpoint intact and introduces no new language.

## Rule and implementation correspondence

[physical_rules.tsv](physical_rules.tsv) maps each AF rule to the algorithm that
uses it, its actual source function, and a bounded-model function where applicable.
The global [rules.tsv](rules.tsv) contains 140 baseline rules and these 14 physical
rules. [check.py](check.py) checks inventories, source markers, matching algorithm
mentions, model-function links and source fingerprints. A successful inventory
check does not prove a semantic rule or the adequacy of Rust expansion.

[physical_model.py](physical_model.py) checks 98,415 current-view candidate cases
and 1,280 witness cases over a finite two-coordinate domain. Both operand masks
vary independently of immutable payload; block sizes and paired key layouts vary,
including empty and permuted separators. A pairwise oracle checks exact answers.
Targeted cases cover correlated keys, signed `i64` boundaries, empty resources,
stream support and the failure of stale-version exact resources. These counts
identify bounded semantic evidence, not all input data or generated Rust.

## Static information and input data

The implementation selects macro paths explicitly. Relation versions, layouts,
logical separators and legal RA schedules are static. Actual keys, current masks
and coordinate ranges are runtime data. The paper's
[native Access witness](example/src/bin/access_representations.rs) additionally
shows an ordinary runtime branch selecting exact bitmap or hash storage; its
cutoff is illustrative, not an optimizer.

The literature comparison retains the distinction between exact semijoin
reduction, conservative predicate transfer and nested/factorized result semantics.
This implementation provides exact reduction with optional zone rejection.
Bloom transfer scheduling, automatic cost search, a capability catalogue and a
new nested IR are not implemented or specified as pending overlay commands.
The direct array implementation accepts `i64` Unit/Rename/Project/Semijoin/
NaturalJoin plans and variable-only output of at most twelve fields. Its formal
preservation argument is conditional on lawful providers and host operations.

## Reproduction

```sh
make -C doc/formal check
make -C doc/formal model
make -C doc/formal pdf
make -C doc/formal example
cargo test -p cq-storage -p cq-compiler -p mini-linq --lib --tests --locked
cargo test -p mini-linq --test heterogeneous --release --locked
```

The source fingerprints are refreshed only after source review and consolidated
validation. [snapshot.txt](snapshot.txt) records that gate; an unchanged old
fingerprint is a deliberate drift diagnostic during an active refactor.
