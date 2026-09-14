# Block storage and Access implementation

The block route is one physical implementation of existing RA. Its semantics and
choice points are specified in [Access refinement](ACCESS-REFINEMENT.md), with
[formal rules](formal/access_refinement.tex) and a
[complete boundary review](IR-STAGE-REVIEW.md).

`cq-storage` holds data structures and operation traits. It has no runtime
relation wrapper, semijoin function or query interpreter. Generated Rust owns
storage and selection values; native provider macros access them through traits.

| Interface | Minimal responsibility |
| --- | --- |
| `BlockRead` | Store identity and descriptor enumeration |
| `BlockDescriptor` | Owning store, block identity and payload rows |
| `CoordinateBounds` | Conservative coordinate bounds, separate from payload access |
| `BuildBlocks` | Pack a unique owned row set into blocks |
| `BlockSelection` | Membership, insertion, nonempty blocks and selected cardinality |
| `MakeSelection` | All/empty selections for a particular store |
| `ExactMembership` | Exact key lookup without imposing one collection representation |
| `RangeSummary` | Observe keys and conservatively test block bounds |

Associated row, block, address and store-identity types keep layout out of the
compiler. There is no independent random-row-lookup or hashing requirement in
the block access interface. Metadata is optional. Query access uses qualified
trait calls; ingestion constructors remain public and layout helpers private.

The supplied `BlockStore<N>` packs unique `[i64; N]` rows with immutable coordinate
bounds. `BlockMask` holds independent selected bits. A provider's view pairs a
store with a mask; its handles carry enough identity to reject foreign-store
handles, including zero-column rows. `ZoneSummary<K>` distinguishes empty keys
from a nonempty zero-column key and uses comparisons without range subtraction.
Native HashSet and BTreeSet providers can coexist with this block provider in one
query. A test storage module with flat payload, sparse addresses and different
selection/summary types exercises the same public operation boundary.

A pruning scan enumerates current nonempty descriptors, tests metadata, then opens
payload only for admitted blocks. The exact membership guard subsequently decides
which handles enter the new mask. Original bounds remain conservative after
selection, and `block_rows` intersects with the current mask. Reusing the original
payload is safe; enumerating its original unselected rows is not.

Key construction scans the current RHS and projects the shared fields in the
compiler-derived order. Resource identity includes that RHS version, separator,
key-provider path and collection family. Scan policy is deliberately independent.
`scan_rows` does not expand unused metadata tokens; native storage therefore does
not need block operations. Forced pruning requires both a block-capable selected
provider and a key provider implementing the requested summary operation, as
checked by macro expansion and Rust typing.

`run` omits optional observation operations. `run_with_stats` uses the same
lowering with counters for work such as scanned rows, built keys, exact probes
and skipped blocks. These are operational measurements, not complexity proofs.
The generated API still exposes a common block-size argument and statistics
surface; those administrative choices are narrower future cleanup work.

## Reproduction and scope

```sh
cargo test -p cq-storage -p cq-compiler -p mini-linq --lib --tests --locked
cargo test -p mini-linq --test heterogeneous --release --locked
cargo test -p cq-compiler --test public_boundaries --locked
cargo test -p cq-compiler --test public_boundaries --release --locked
make -C doc/formal model
make -C doc/formal example
```

The direct route admits `i64` array rows and Unit/Rename/Project/Semijoin/NaturalJoin
with variable-only output, at most twelve fields. The fact-star example is one
integer query instance, not evidence for arbitrary host expressions, all types,
all RA operators or an automatic optimizer. Small mask, empty-key, permuted-field,
resource-version and heterogeneous-provider tests target semantic boundaries;
positive Rust compilation alone does not prove trait laws. Current gate results
belong in [the snapshot](formal/snapshot.txt), after a consolidated run.

The [bounded model](formal/physical_model.py) independently checks candidate plus
exact-residual preservation over current masks and conservative original bounds.
It is specification evidence, not a runtime library or a model of Rust expansion.
