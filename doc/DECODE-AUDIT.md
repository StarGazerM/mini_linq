# Decode-site audit

## Contract, scope, and evidence

This is a source audit, not a performance experiment or a behavior change. Runtime/compiler edits in this assignment are comments only. The mechanical checker extension and the TSV/report artifacts are the only executable/documentation additions. Concurrent implementation work by other agents is outside this claim.

Authority: `STORAGE.md` §1 (preserved operations), §§10–12, and `PLANNING-REDESIGN.md` contract 7. R is reconstruction required by an opaque authored Rust read; P is logical answer publication on the host after device download; V is premature reconstruction with relational consumers (the comment names P4/P5/P6); I is immediate decode/re-encode reorganization, with avoidability stated; ? means a generic primitive, mixed-use boundary, or diagnostic invocation lacks enough consumer evidence for an unconditional R/P/V classification. **? does not authorize decoding.** Runtime call attribution and per-column read facts are still required at those sites.

A site is a conversion implementation or a conversion-emitting/calling block. A marker may cover adjacent alternatives of the same boundary (for example borrowed/owned answer formatting, nullable/nonnullable codecs, or a macro's generated scalar family); it is not a count of dynamic decodes. Both a primitive and its concrete caller can be listed. A P marker explicitly mentioning a test/oracle is diagnostic exposure, not a claim that intermediate engine materialization is lawful. Several diagnostic fixtures instead remain ? because they feed independent CPU references.

The inventory covers `experiments/gpu/**`, `runtime/cq-load`, `runtime/cq-storage`, `passes/cq-print`, Sequence/pull printers, and DOOP symbol resolution under `features/datalog`. It includes unnamed FOR base-plus-offset reconstruction, scaled/date wrappers, dictionary resolution, ordinal inversion and logical tuple downloads. It excludes transport-only raw buffer/code downloads, integer endian parsing, unpacking residual bits while retaining their code domain, and code-preserving casts/wrappers. It also excludes SQL/CSV input parsing without a preceding encoded representation.

`DECODE-SITES.tsv` is the authoritative four-column registry: path, exact marker, class, reason. `formal/check.py` requires a one-to-one correspondence between registry rows and flagged comments, validates classes and V plan steps, and prints counts. It checks **marker coverage, not semantic completeness or correctness of classifications**. The file:line entries below identify comment lines in this audit snapshot; exact marker text survives unrelated line movement.

## Sites that are not logical decodes

* Sequence `capture.rs` encodes closed operands and supplies opaque callables; its free-variable dependencies feed the identity-representation gate and R bindings in `rows.rs`. No independent code-to-value operation occurs in the capture implementation.
* Sequence `layout.rs` records allocation/layout; widening policy there is not itself a conversion implementation. Concrete reconstruction sites are in emitters and runtime kernels.
* TPC-H `loader.rs` reads numeric codes and constructs encoded device columns; it does not resolve dictionary codes. `Manifest::dictionary` loads dictionary metadata, while `text_id` translates a closed value to a code. Logical HostValue and publication conversions are flagged separately.
* `DenseOrdinal`/`KeyCodec` in `cq-storage` are forward encoding contracts, not hidden inverse APIs. `encode_predicate`, dictionary lookup of a value's code, and codec catalog registration are likewise encoding. Concrete inverse key methods in GPU domains are registered.
* DOOP `IStr::from_code` wraps an existing code without string resolution. `res()` is the string conversion. Authored descriptor `format!` callers are R: they need string contents to compute a new descriptor, even when the result is subsequently interned.
* Datalog run merges compare packed words; they do not necessarily decode. Gather/repack paths that reconstruct logical fields are I and explicitly identify avoidability.

## Verification exercised

Only `python3 doc/formal/check.py` was run for this assignment. It passed:

```
Documentation inventory: 309 rules.
Conformance records: 365 rows name existing rules, files and statuses (doc/formal/CONFORMANCE.md).
Design anchors: 22 registered comments identify existing tracked sources.
Decode sites: R=9, P=252, V=96, I=16, ?=123 (496 registered comments).
```

No build, Rust test, formatter, benchmark, profiler, or new GPU execution was run.

## Violations ranked by likely cost

**[INFERENCE]** This ranking is an investigation order, not measured site timings. Whole-relation materialization and string allocation rank above scalar reconstruction; actual invocation counts/selectivity can reverse the order. Existing storage evidence (§12) motivates prioritizing encoded execution, but does not establish a speedup for any individual site below. Diagnostic-only paths are ranked last regardless of their small-fixture implementation cost.

1. P5 eager CPU row/String materialization before pruning, grouping and indexing: whole-relation work and allocation; retain column codes and materialize only R/P demands.
2. P6 whole-column/device-output/intermediate host reconstruction: extra wide buffers, transfers and downstream re-encoding; preserve packed/narrow carriers across boundaries.
3. P4/P6 per-row and per-probe reconstruction for joins, prefix/range search, grouping and membership: potentially multiplied by probe counts; compare keys/codes under recorded preserved-op laws.
4. Other V implementations whose runtime frequency is not established.
5. Diagnostic/fixture-only reproductions: correctness/design debt, but no inferred production cost.

Every V marker is listed below, sorted by those tiers and then path; ties are not an asserted cost ordering.

| Priority tier | File:line | Class | Reason / plan step |
|---|---|---|---|
| 1 | `runtime/cq-load/src/csv_source.rs:135` | V | Loader::csv_hash/csv_btree group these decoded rows; P4 keeps index keys encoded and P5 builds scan columns directly. |
| 1 | `runtime/cq-load/src/loader.rs:44` | V | CSV batch decoding precedes hash grouping; P4 builds indexes over retained representations and P5 loads columnar scans directly. |
| 1 | `runtime/cq-load/src/loader.rs:57` | V | CSV batch decoding precedes ordered grouping; P4 retains encoded index keys and P5 avoids eager logical scan tuples. |
| 1 | `runtime/cq-load/src/parquet_source.rs:117` | V | zone_tpch feeds these owned decoded tuples to membership scans; P5 must retain columnar payloads and materialize late. |
| 1 | `runtime/cq-load/src/zone.rs:214` | V | consuming Csv/Parquet batches drives eager tuple decoding before summary scans; P5 must build encoded columns and materialize survivors. |
| 2 | `experiments/gpu/sequence-runtime/src/canon.rs:565` | V | into_columns invokes device expansion before host publication; P6 preserve the packed view. |
| 2 | `experiments/gpu/sequence-runtime/src/canon.rs:584` | V | packed_rows wrapper dispatches device expansion before publication; P6 defer logical inversion. |
| 2 | `experiments/gpu/sequence-runtime/src/canon.rs:661` | V | canonical output adapter expands packed values before possible chained relations; P6 retain encoded fields. |
| 2 | `experiments/gpu/sequence-runtime/src/canon.rs:682` | V | output adapter materializes full i64 columns before their consumers; P6 keep packed values resident. |
| 2 | `experiments/gpu/sequence-runtime/src/canon.rs:840` | V | packed FOR base-plus-offset inversion precedes host publication and may feed chaining; P6 delay reconstruction. |
| 2 | `experiments/gpu/sequence-runtime/src/lib.rs:354` | V | measuring a computed packed publication reconstructs values before provider planning; P6 reuse recorded summaries. |
| 2 | `experiments/gpu/sequence/core/src/emit.rs:1366` | V | P6 intermediate host publication reconstructs packed FOR fields for the whole stored layout. |
| 2 | `experiments/gpu/sequence/core/src/emit.rs:1377` | V | P6 intermediate host fallback resolves symbols and logical values before more engine operators, not final answers. |
| 2 | `experiments/gpu/sequence/core/src/emit.rs:1987` | V | P6 canonical union output expands FOR fields on device before host download; retain packed publication. |
| 2 | `experiments/gpu/sequence/core/src/emit.rs:1992` | V | P6 noncanonical union output expands FOR fields on device before host download. |
| 2 | `experiments/gpu/sequence/core/src/emit.rs:2354` | V | P6 stored intermediates reconstruct FOR fields before downstream relational operators instead of retaining codes. |
| 2 | `experiments/gpu/sequence/core/src/emit.rs:2647` | V | P6 canonical output adapter reconstructs device i64 columns before final host publication. |
| 2 | `experiments/gpu/sequence/core/src/emit.rs:2652` | V | P6 packed output adapter reconstructs device i64 columns before final host publication. |
| 2 | `experiments/gpu/sequence/core/src/emit.rs:2816` | V | P6 host-origin canonical output is reuploaded then expanded on device, rather than decoded at host publication. |
| 2 | `experiments/gpu/sequence/core/src/emit.rs:2822` | V | P6 host-origin packed output expands device columns before the user's host download. |
| 2 | `experiments/gpu/sequence/core/src/rows.rs:1431` | V | P6 host fallback reconstructs the complete run layout, not only opaque-Rust rust_reads columns. |
| 2 | `experiments/gpu/sequence/core/src/rows.rs:1437` | V | P6 host fallback materializes complete logical tuples before further relational processing, not final publication. |
| 2 | `experiments/gpu/tpch/build.rs:717` | V | P6 staged host execution downloads entire logical input tuples before subsequent relational operators; retain codes outside rust_reads. |
| 3 | `experiments/gpu/datalog-runtime/src/layout.rs:369` | V | packed inverse feeds compare_prefix and generated relational reads; P6 compare encoded fields. |
| 3 | `experiments/gpu/datalog-runtime/src/layout.rs:435` | V | prefix lookup reconstructs packed fields during binary search; P6 encode probes and compare packed prefixes. |
| 3 | `experiments/gpu/device-columns/src/device.rs:114` | V | narrowed source reconstruction feeds Sequence sorting/indexing; P6 retain encoded keys. |
| 3 | `experiments/gpu/device-columns/src/device.rs:117` | V | ordinary source decode precedes key sorting; P6 avoid pre-sort expansion. |
| 3 | `experiments/gpu/device-columns/src/device.rs:120` | V | packed source decode precedes key sorting; P6 compare recorded packed fields. |
| 3 | `experiments/gpu/sequence-runtime/src/canon.rs:807` | V | unpack_field constructs device logical columns before possible relational consumers; P6 preserve packed fields. |
| 3 | `experiments/gpu/sequence-runtime/src/device.rs:59` | V | lower directory offset FOR inversion precedes range expansion; P6 use plain index directories. |
| 3 | `experiments/gpu/sequence-runtime/src/device.rs:61` | V | upper directory offset FOR inversion precedes range expansion; P6 avoid compressed random-access directories. |
| 3 | `experiments/gpu/sequence-runtime/src/device.rs:79` | V | binary membership probe reconstructs each midpoint; P6 compare code-domain keys or use plain indexes. |
| 3 | `experiments/gpu/sequence-runtime/src/device.rs:86` | V | final membership comparison reconstructs the candidate; P6 retain encoded comparison operands. |
| 3 | `experiments/gpu/sequence-runtime/src/kernels.rs:61` | V | index key extraction reconstructs values before sorting; P6 sort recorded codes or keep index runs plain. |
| 3 | `experiments/gpu/sequence-runtime/src/kernels.rs:75` | V | multi-key index build expands each column before lexicographic sorting; P6 retain encoded keys. |
| 3 | `experiments/gpu/sequence-runtime/src/kernels.rs:86` | V | permutation gather reconstructs keys for a later sort; P6 compare encoded fields. |
| 3 | `experiments/gpu/sequence-runtime/src/kernels.rs:105` | V | range selection decodes before relational comparison; P6 translate bounds into the encoded domain. |
| 3 | `experiments/gpu/sequence-runtime/src/lib.rs:813` | V | index selection launch reconstructs values before range comparison; P6 translate predicates to codes. |
| 3 | `experiments/gpu/sequence-runtime/src/lib.rs:928` | V | multi-key index build launches full column reconstruction before sorting; P6 sort retained encoded fields. |
| 3 | `experiments/gpu/sequence-runtime/src/lib.rs:967` | V | leading-key reconstruction follows tuple sort for directory building; P6 retain encoded key order. |
| 3 | `experiments/gpu/sequence-runtime/src/lib.rs:981` | V | single-key reconstruction precedes radix sort; P6 sort recorded codes or use plain indexes. |
| 3 | `experiments/gpu/sequence-runtime/src/selected.rs:68` | V | selection launches value reconstruction before range comparison; P6 evaluate translated encoded predicates. |
| 3 | `experiments/gpu/sequence/core/src/emit.rs:556` | V | P4 run membership prefix_range reconstructs packed signed fields through compare_prefix solely for key comparison. |
| 3 | `experiments/gpu/sequence/core/src/emit.rs:566` | V | P4 run lookup prefix_range reconstructs packed signed fields through compare_prefix instead of comparing encoded keys. |
| 3 | `experiments/gpu/sequence/core/src/emit.rs:609` | V | P4 lower-bound key search reconstructs FOR values solely for ordered relational comparison. |
| 3 | `experiments/gpu/sequence/core/src/emit.rs:618` | V | P4 upper-bound key search reconstructs FOR values solely for ordered relational comparison. |
| 3 | `experiments/gpu/sequence/core/src/emit.rs:642` | V | P4 run-provider payload unpack widens keys and code-only fields before relational consumers. |
| 3 | `experiments/gpu/sequence/core/src/reduce.rs:186` | V | P6 grouped input reconstructs packed FOR fields before relational grouping and aggregation. |
| 3 | `experiments/gpu/sequence/core/src/rows.rs:1034` | V | P6 run scans unpack every demanded field before relational consumers; retain narrow codes unless rust_reads requires identity. |
| 3 | `experiments/gpu/sequence/core/src/rows.rs:1055` | V | P6 source scans reconstruct FOR values into i64 even for code-only joins and predicates. |
| 3 | `experiments/gpu/sequence/core/src/rows.rs:1286` | V | P6 relational Select/Point/Range reconstructs Scalar values; preserved-op predicates must compare codes. |
| 4 | `experiments/gpu/datalog-runtime/src/layout.rs:381` | V | signed ordinal inversion precedes prefix comparisons; P6 translate probes to packed order. |
| 4 | `experiments/gpu/datalog-runtime/src/layout.rs:444` | V | lower-bound comparison invokes field inversion before ordering; P6 compare encoded prefixes. |
| 4 | `experiments/gpu/datalog-runtime/src/layout.rs:453` | V | upper-bound comparison invokes field inversion before ordering; P6 retain packed prefix comparisons. |
| 4 | `experiments/gpu/datalog-runtime/src/layout.rs:462` | V | tuple prefix comparison reconstructs each numeric field; P6 compare encoded field order directly. |
| 4 | `experiments/gpu/datalog-runtime/src/layout.rs:468` | V | prefix search decodes each compared field; P6 compare encoded prefixes directly. |
| 4 | `experiments/gpu/sequence-runtime/src/canon.rs:557` | V | packed answers expand into resident columns usable by chained queries; P6 retain packed fields until an allowed read. |
| 4 | `experiments/gpu/sequence-runtime/src/canon.rs:575` | V | packed_rows wrapper eagerly expands into device columns before consumers; P6 retain packed resident fields. |
| 4 | `experiments/gpu/sequence-runtime/src/canon.rs:623` | V | packed adapter expands the live prefix before possible chained relations; P6 keep its encoded view. |
| 4 | `experiments/gpu/sequence-runtime/src/canon.rs:647` | V | packed_columns expands every field into device buffers; P6 defer inversion until logical reads. |
| 4 | `experiments/gpu/sequence-runtime/src/canon.rs:670` | V | canonical unpack delegates eager device expansion; P6 publish a packed resident view. |
| 4 | `experiments/gpu/sequence-runtime/src/canon.rs:724` | V | per-field unpack launch expands all live rows before chaining; P6 decode only at the allowed consumer. |
| 4 | `experiments/gpu/sequence-runtime/src/kernels.rs:21` | V | bound reduction reconstructs every source value before index planning; P6 reuse recorded encoded-domain bounds. |
| 4 | `experiments/gpu/sequence-runtime/src/lib.rs:381` | V | bounds discovery launches source reconstruction before index building; P6 consume recorded summaries instead. |
| 4 | `experiments/gpu/sequence/core/src/emit.rs:652` | V | P6 provider payload reconstructs FOR values without restricting decode to rust_reads columns. |
| 5 | `experiments/gpu/datalog-runtime/tests/layout_arena.rs:173` | V | diagnostic prefix inversion precedes relational lookup; P6 compare the encoded prefix instead. |
| 5 | `experiments/gpu/h05-reference/src/kernels.rs:101` | V | both FOR directory offsets are reconstructed before join expansion; P6 use plain index directories. |
| 5 | `experiments/gpu/h05-reference/src/kernels.rs:112` | V | binary membership reconstructs each midpoint before comparison; P6 compare encoded keys. |
| 5 | `experiments/gpu/h05-reference/src/kernels.rs:119` | V | final membership reconstructs the candidate before equality; P6 retain code-domain operands. |
| 5 | `experiments/gpu/h05-reference/src/kernels.rs:171` | V | full key column expands before radix sorting; P6 sort encoded keys or use plain index runs. |
| 5 | `experiments/gpu/h05-reference/src/kernels.rs:193` | V | selected keys expand before the next sorting pass; P6 preserve recorded code representations. |
| 5 | `experiments/gpu/h05-reference/src/kernels.rs:209` | V | column decoder materializes all index keys before sort; P6 keep keys encoded. |
| 5 | `experiments/gpu/h05-reference/src/kernels.rs:216` | V | gathered key reconstruction feeds lexicographic sort; P6 compare retained codes. |
| 5 | `experiments/gpu/h05-reference/src/kernels.rs:237` | V | date/region selection decodes before relational comparison; P6 translate range constants. |
| 5 | `experiments/gpu/h05-reference/src/kernels.rs:262` | V | compressed region directory endpoint controls further expansion; P6 use plain index directories. |
| 5 | `experiments/gpu/h05-reference/src/kernels.rs:266` | V | region ID FOR inverse precedes the nation join; P6 retain plain/code-domain index keys. |
| 5 | `experiments/gpu/h05-reference/src/kernels.rs:271` | V | nation ID FOR inverse precedes the customer join; P6 avoid compressed index payload probes. |
| 5 | `experiments/gpu/h05-reference/src/kernels.rs:352` | V | customer ID FOR inverse precedes the orders join; P6 retain index keys without expansion. |
| 5 | `experiments/gpu/h05-reference/src/kernels.rs:381` | V | order ID FOR inverse precedes the lineitem join; P6 avoid compressed random-access index payloads. |
| 5 | `experiments/gpu/h05-reference/src/kernels.rs:420` | V | supplier ID FOR inverse precedes membership probing; P6 compare retained encoded keys. |
| 5 | `experiments/gpu/h05-reference/src/main.rs:156` | V | reference selection launches decode-before-range filtering; P6 translate predicates to the encoded domain. |
| 5 | `experiments/gpu/h05-reference/src/main.rs:190` | V | full index key reconstruction precedes sorting; P6 retain encoded keys. |
| 5 | `experiments/gpu/h05-reference/src/main.rs:192` | V | selected index key reconstruction precedes sorting; P6 compare recorded codes. |
| 5 | `experiments/gpu/h05-reference/src/main.rs:217` | V | index builder launches whole-key decode before lexicographic sort; P6 retain encoded keys. |
| 5 | `runtime/cq-load/examples/zone_tpch.rs:13` | V | this benchmark decoder materializes lineitem before membership selection; P5 requires encoded scan columns and late materialization. |
| 5 | `runtime/cq-load/examples/zone_tpch.rs:16` | V | order keys are decoded before membership tests; P5 evaluates preserved predicates in the scan representation. |
| 5 | `runtime/cq-load/examples/zone_tpch.rs:19` | V | part keys are materialized before row selection despite no query read; P5 must retain them encoded. |
| 5 | `runtime/cq-load/examples/zone_tpch.rs:21` | V | supplier keys are materialized before membership selection; P5 materializes only demanded survivor columns. |
| 5 | `runtime/cq-load/examples/zone_tpch.rs:44` | V | the benchmark loads all logical tuples before zone pruning; P5 must replace eager row loading with columnar scan storage. |
| 5 | `runtime/cq-load/tests/zone_parquet.rs:15` | V | the zone diagnostic builds logical tuples before relational selections; P5 must build summaries with encoded scan columns. |
| 5 | `runtime/cq-load/tests/zone_parquet.rs:18` | V | the zone diagnostic reconstructs id before Point selection; P5 keeps preserved predicates in the encoded scan domain. |
| 5 | `runtime/cq-load/tests/zone_parquet.rs:21` | V | the zone diagnostic reconstructs dates before building bounds; P5 builds payload and summaries together. |
| 5 | `runtime/cq-load/tests/zone_parquet.rs:23` | V | the zone diagnostic reconstructs Decimal before building bounds; P5 requires columnar payload plus summaries. |
| 5 | `runtime/cq-load/tests/zone_parquet.rs:25` | V | the zone diagnostic owns strings before subsequent selection; P5 delays reconstruction to demanded survivors. |
| 5 | `runtime/cq-load/tests/zone_parquet.rs:89` | V | this test consumes decoding batches before zone predicates; P5 retains encoded columns through selection. |
| 5 | `runtime/cq-load/tests/zone_parquet.rs:91` | V | the expected zone host also eagerly loads tuples before relational reads; P5 builds scan payload and bounds together. |

## Nineteen i64 input columns

The table distinguishes current authored Rust reads from relational-only uses. Method and query-specific source/fact limitations are in `DECODE-COLUMNS.md`, which covers all 27 catalog query variants. `DECODE-COLUMNS.tsv` is the machine-readable companion. Bits are the supplied SF1/SF10 measurements, not measurements rerun here. All supplied input ranges fit i32; that does not permit rewriting authored Decimal Rust or narrowing aggregate/intermediate arithmetic. C means code-only; R means opaque Rust read; final P is stated per query in the companion. Output-purpose `.scaled()` in authored Rust is R, not output-only P. h11 key-copy heads are syntactic R but do not justify logical key inversion: preserve the encoded identity or change the authored representation contract explicitly.

| Column | Queries / use class | Bits SF1/SF10 | Recommended device carrier | Accumulator / expression width | Evidence |
|---|---|---|---|---|---|
| c_acctbal | R:h10; C:- | 21 signed/21 signed | recommend i32 storage, i32 value lane SF1/SF10; saved Sequence i64 where present; preserve authored identity type at Rust boundary | no sum; scaled() authored i64 output; i32 projection safe at supplied bounds | method=authored rule/free-variable inspection + saved Sequence carrier extraction; retail_h10.rs; DECODE-COLUMNS.md |
| c_custkey | R:-; C:d19,d72_inner,d85,h03,h05,h07,h07_block,h08,h10,h13,h18,h22 | 18/21 | recommend u32 shared-domain/dense codes SF1/SF10; saved i64 where present; h05 u32 | no value accumulator; no key arithmetic; i32 comparisons fit supplied SF1/SF10 values; h13 count is usize (not key width) | method=authored rule/free-variable inspection + saved Sequence carrier extraction; retail_d19.rs,retail_d72_inner.rs,retail_d85.rs,retail_h03.rs,retail_h05.rs,retail_h07.rs,retail_h07_block.rs,retail_h08.rs,retail_h10.rs,retail_h13.rs,retail_h18.rs,retail_h22.rs; DECODE-COLUMNS.md |
| l_discount | R:h06,h10,h14,h15,h19; C:- | 4/4 | recommend u8 storage, i32 value lane SF1/SF10; saved Sequence i64 where present; preserve authored identity type at Rust boundary | i64 revenue sums h06/h10/h14/h15/h19; i32 row products safe: price*15<=251658225, price*100<=1677721500; scaled() authored i64 | method=authored rule/free-variable inspection + saved Sequence carrier extraction; retail_h06.rs,retail_h10.rs,retail_h14.rs,retail_h15.rs,retail_h19.rs; DECODE-COLUMNS.md |
| l_extendedprice | R:h01,h06,h10,h14,h15,h19; C:- | 24/24 | recommend u32 storage, i32 value lane SF1/SF10; saved Sequence i64 where present; preserve authored identity type at Rust boundary | h01 i64 price sum; h06/h10/h14/h15/h19 i64 revenue sums; i32 row products safe <=1677721500; h15 max Option<i64>; scaled() authored i64 | method=authored rule/free-variable inspection + saved Sequence carrier extraction; retail_h01.rs,retail_h06.rs,retail_h10.rs,retail_h14.rs,retail_h15.rs,retail_h19.rs; DECODE-COLUMNS.md |
| l_linenumber | R:-; C:h01,h06,h10,h12,h14,h15,h18,h19,h20 | 3/3 | recommend u8 identity/distinct code SF1/SF10; saved i64 | no value accumulator; i32 unnecessary for identity; h01 count usize, h12 high/low sums i64 | method=authored rule/free-variable inspection + saved Sequence carrier extraction; retail_h01.rs,retail_h06.rs,retail_h10.rs,retail_h12.rs,retail_h14.rs,retail_h15.rs,retail_h18.rs,retail_h19.rs,retail_h20.rs; DECODE-COLUMNS.md |
| l_orderkey | R:-; C:d19,d27,d72_inner,d85,h01,h03,h04,h04_not_exists,h05,h06,h07,h07_block,h08,h09,h10,h12,h14,h15,h18,h19,h20,h21_positive | 23/26 | recommend u32 shared-domain/dense codes SF1/SF10; saved i64 where present; h05 u32 | no value accumulator; no key arithmetic; i32 comparisons fit supplied SF1/SF10 values | method=authored rule/free-variable inspection + saved Sequence carrier extraction; retail_d19.rs,retail_d27.rs,retail_d72_inner.rs,retail_d85.rs,retail_h01.rs,retail_h03.rs,retail_h04.rs,retail_h04_not_exists.rs,retail_h05.rs,retail_h06.rs,retail_h07.rs,retail_h07_block.rs,retail_h08.rs,retail_h09.rs,retail_h10.rs,retail_h12.rs,retail_h14.rs,retail_h15.rs,retail_h18.rs,retail_h19.rs,retail_h20.rs,retail_h21_positive.rs; DECODE-COLUMNS.md |
| l_partkey | R:-; C:d19,d27,d72_inner,d85,h08,h09,h14,h19,h20 | 18/21 | recommend u32 shared-domain/dense codes SF1/SF10; saved i64 where present | no value accumulator; no key arithmetic; i32 comparisons fit supplied SF1/SF10 values | method=authored rule/free-variable inspection + saved Sequence carrier extraction; retail_d19.rs,retail_d27.rs,retail_d72_inner.rs,retail_d85.rs,retail_h08.rs,retail_h09.rs,retail_h14.rs,retail_h19.rs,retail_h20.rs; DECODE-COLUMNS.md |
| l_quantity | R:h01,h06,h18,h19,h20; C:d72_inner | 13/13 | recommend u16 storage, i32 value lane SF1/SF10; saved Sequence i64 where present; preserve authored identity type at Rust boundary | h01/h18/h20 sums i64; h06/h19 row predicates fit i32; h18 bounded dbgen sum<=35000; h20 compares derived sum as i128; scaled() authored i64 | method=authored rule/free-variable inspection + saved Sequence carrier extraction; retail_d72_inner.rs,retail_h01.rs,retail_h06.rs,retail_h18.rs,retail_h19.rs,retail_h20.rs; DECODE-COLUMNS.md |
| l_suppkey | R:-; C:d19,d27,d72_inner,d85,h05,h07,h07_block,h08,h09,h15,h20,h21_positive | 14/17 | recommend u16 SF1 / u32 SF10 shared-domain codes; saved i64 where present; h05 u32 | no value accumulator; no key arithmetic; i32 comparisons fit supplied SF1/SF10 values | method=authored rule/free-variable inspection + saved Sequence carrier extraction; retail_d19.rs,retail_d27.rs,retail_d72_inner.rs,retail_d85.rs,retail_h05.rs,retail_h07.rs,retail_h07_block.rs,retail_h08.rs,retail_h09.rs,retail_h15.rs,retail_h20.rs,retail_h21_positive.rs; DECODE-COLUMNS.md |
| o_custkey | R:-; C:d19,d72_inner,d85,h03,h05,h07,h07_block,h08,h10,h13,h18,h22 | 18/21 | recommend u32 shared-domain/dense codes SF1/SF10; saved i64 where present; h05 u32 | no value accumulator; no key arithmetic; i32 comparisons fit supplied SF1/SF10 values; h13 count is usize (not key width) | method=authored rule/free-variable inspection + saved Sequence carrier extraction; retail_d19.rs,retail_d72_inner.rs,retail_d85.rs,retail_h03.rs,retail_h05.rs,retail_h07.rs,retail_h07_block.rs,retail_h08.rs,retail_h10.rs,retail_h13.rs,retail_h18.rs,retail_h22.rs; DECODE-COLUMNS.md |
| o_orderkey | R:-; C:d19,d27,d72_inner,d85,h03,h04,h04_not_exists,h05,h07,h07_block,h08,h09,h10,h12,h13,h18,h21_positive; unused:h22 | 23/26 | recommend u32 shared-domain/dense codes SF1/SF10; saved i64 where present; h05 u32 | no value accumulator; no key arithmetic; i32 comparisons fit supplied SF1/SF10 values; h13 count is usize (not key width) | method=authored rule/free-variable inspection + saved Sequence carrier extraction; retail_d19.rs,retail_d27.rs,retail_d72_inner.rs,retail_d85.rs,retail_h03.rs,retail_h04.rs,retail_h04_not_exists.rs,retail_h05.rs,retail_h07.rs,retail_h07_block.rs,retail_h08.rs,retail_h09.rs,retail_h10.rs,retail_h12.rs,retail_h13.rs,retail_h18.rs,retail_h21_positive.rs,retail_h22.rs; DECODE-COLUMNS.md |
| o_totalprice | R:h18; C:- | 26/26 | recommend u32 storage, i32 value lane SF1/SF10; no saved Sequence carrier for owning staged queries; preserve authored identity type at Rust boundary | no sum; scaled() authored i64 output; i32 projection safe at supplied bounds | method=authored rule/free-variable inspection + saved Sequence carrier extraction; retail_h18.rs; DECODE-COLUMNS.md |
| p_partkey | R:-; C:d19,d27,d72_inner,d85,h02,h08,h09,h14,h16,h19,h20 | 18/21 | recommend u32 shared-domain/dense codes SF1/SF10; saved i64 where present | no value accumulator; no key arithmetic; i32 comparisons fit supplied SF1/SF10 values | method=authored rule/free-variable inspection + saved Sequence carrier extraction; retail_d19.rs,retail_d27.rs,retail_d72_inner.rs,retail_d85.rs,retail_h02.rs,retail_h08.rs,retail_h09.rs,retail_h14.rs,retail_h16.rs,retail_h19.rs,retail_h20.rs; DECODE-COLUMNS.md |
| ps_availqty | R:h11,h20; C:d72_inner,d85 | 14/14 | recommend u16 storage, i32 value lane SF1/SF10; saved Sequence i64 where present; preserve authored identity type at Rust boundary | h11 product/sums authored i64, product fits i32 <=2147336193 from bit bounds; h20 available*200<=3276600 fits i32 but authored i128 comparison to shipment sum; d72_inner/d85 no aggregate | method=authored rule/free-variable inspection + saved Sequence carrier extraction; retail_d72_inner.rs,retail_d85.rs,retail_h11.rs,retail_h20.rs; DECODE-COLUMNS.md |
| ps_partkey | R:h11; C:d72_inner,d85,h02,h09,h16,h20 | 18/21 | recommend u32 shared-domain/dense codes SF1/SF10; saved i64 where present; h11 syntactic Rust copy requires identity today; code-preserving relational projection would avoid decode | no value accumulator; no key arithmetic; i32 comparisons fit supplied SF1/SF10 values | method=authored rule/free-variable inspection + saved Sequence carrier extraction; retail_d72_inner.rs,retail_d85.rs,retail_h02.rs,retail_h09.rs,retail_h11.rs,retail_h16.rs,retail_h20.rs; DECODE-COLUMNS.md |
| ps_suppkey | R:h11; C:d72_inner,d85,h02,h09,h16,h20 | 14/17 | recommend u16 SF1 / u32 SF10 shared-domain codes; saved i64 where present; h11 syntactic Rust copy requires identity today; code-preserving relational projection would avoid decode | no value accumulator; no key arithmetic; i32 comparisons fit supplied SF1/SF10 values | method=authored rule/free-variable inspection + saved Sequence carrier extraction; retail_d72_inner.rs,retail_d85.rs,retail_h02.rs,retail_h09.rs,retail_h11.rs,retail_h16.rs,retail_h20.rs; DECODE-COLUMNS.md |
| ps_supplycost | R:h02,h11; C:- | 17/17 | recommend u32 storage, i32 value lane SF1/SF10; no saved Sequence carrier for owning staged queries; preserve authored identity type at Rust boundary | h02 min Option<i64>; h11 product/sums i64, row product fits i32 <=2147336193 from bit bounds; derived aggregate*10000 authored i128; scaled() authored i64 | method=authored rule/free-variable inspection + saved Sequence carrier extraction; retail_h02.rs,retail_h11.rs; DECODE-COLUMNS.md |
| s_acctbal | R:h02; C:- | 21 signed/21 signed | recommend i32 storage, i32 value lane SF1/SF10; no saved Sequence carrier for owning staged queries; preserve authored identity type at Rust boundary | no sum; scaled() authored i64 output; i32 projection safe at supplied bounds | method=authored rule/free-variable inspection + saved Sequence carrier extraction; retail_h02.rs; DECODE-COLUMNS.md |
| s_suppkey | R:-; C:d19,d27,d72_inner,d85,h02,h05,h07,h07_block,h08,h09,h11,h15,h16,h20,h21_positive | 14/17 | recommend u16 SF1 / u32 SF10 shared-domain codes; saved i64 where present; h05 u32 | no value accumulator; no key arithmetic; i32 comparisons fit supplied SF1/SF10 values | method=authored rule/free-variable inspection + saved Sequence carrier extraction; retail_d19.rs,retail_d27.rs,retail_d72_inner.rs,retail_d85.rs,retail_h02.rs,retail_h05.rs,retail_h07.rs,retail_h07_block.rs,retail_h08.rs,retail_h09.rs,retail_h11.rs,retail_h15.rs,retail_h16.rs,retail_h20.rs,retail_h21_positive.rs; DECODE-COLUMNS.md |

## Complete decode-site table

| File:line | Class | Reason |
|---|---|---|
| `experiments/gpu/compress/src/lib.rs:137` | ? | source-key ordinal inversion serves answer decoding and generic DomainMap callers; require caller read evidence. |
| `experiments/gpu/datalog-runtime/src/layout.rs:52` | ? | Scalar inverse lacks a caller-level opaque-read/publication requirement. |
| `experiments/gpu/datalog-runtime/src/layout.rs:70` | ? | numeric Scalar inverse has no local logical-consumer evidence. |
| `experiments/gpu/datalog-runtime/src/layout.rs:85` | ? | boolean Scalar inverse needs caller evidence rather than type spelling. |
| `experiments/gpu/datalog-runtime/src/layout.rs:94` | ? | Symbol Scalar delegates dictionary inversion without a caller publication fact. |
| `experiments/gpu/datalog-runtime/src/layout.rs:109` | ? | Decimal construction needs caller read evidence despite identity storage. |
| `experiments/gpu/datalog-runtime/src/layout.rs:122` | ? | epoch-day Date construction carries no caller read or publication fact. |
| `experiments/gpu/datalog-runtime/src/layout.rs:201` | P | to_host_codes diagnostic download unpacks numeric fields while retaining symbol IDs. |
| `experiments/gpu/datalog-runtime/src/layout.rs:312` | P | explicit diagnostic host publication unpacks numeric fields, not symbol dictionaries. |
| `experiments/gpu/datalog-runtime/src/layout.rs:369` | V | packed inverse feeds compare_prefix and generated relational reads; P6 compare encoded fields. |
| `experiments/gpu/datalog-runtime/src/layout.rs:381` | V | signed ordinal inversion precedes prefix comparisons; P6 translate probes to packed order. |
| `experiments/gpu/datalog-runtime/src/layout.rs:435` | V | prefix lookup reconstructs packed fields during binary search; P6 encode probes and compare packed prefixes. |
| `experiments/gpu/datalog-runtime/src/layout.rs:444` | V | lower-bound comparison invokes field inversion before ordering; P6 compare encoded prefixes. |
| `experiments/gpu/datalog-runtime/src/layout.rs:453` | V | upper-bound comparison invokes field inversion before ordering; P6 retain packed prefix comparisons. |
| `experiments/gpu/datalog-runtime/src/layout.rs:462` | V | tuple prefix comparison reconstructs each numeric field; P6 compare encoded field order directly. |
| `experiments/gpu/datalog-runtime/src/layout.rs:468` | V | prefix search decodes each compared field; P6 compare encoded prefixes directly. |
| `experiments/gpu/datalog-runtime/src/layout.rs:490` | P | diagnostic unsigned identity assertion exposes the reconstructed maximum. |
| `experiments/gpu/datalog-runtime/src/layout.rs:508` | P | diagnostic Decimal fixture constructs the expected coefficient answer. |
| `experiments/gpu/datalog-runtime/src/layout.rs:511` | P | diagnostic Decimal round trip exposes each reconstructed coefficient. |
| `experiments/gpu/datalog-runtime/src/layout.rs:516` | P | diagnostic scale-18 fixture constructs the expected fractional answer. |
| `experiments/gpu/datalog-runtime/src/layout.rs:518` | P | diagnostic fine-scale round trip exposes a reconstructed Decimal. |
| `experiments/gpu/datalog-runtime/src/layout.rs:521` | P | diagnostic Date fixture constructs the expected epoch-day answer. |
| `experiments/gpu/datalog-runtime/src/layout.rs:524` | P | diagnostic Date round trip exposes each reconstructed date. |
| `experiments/gpu/datalog-runtime/src/layout.rs:527` | P | diagnostic Decimal ordering assertion exposes both inverse values. |
| `experiments/gpu/datalog-runtime/src/layout.rs:529` | P | diagnostic Date ordering assertion exposes both inverse dates. |
| `experiments/gpu/datalog-runtime/src/layout.rs:562` | P | diagnostic cross-lane i8 minimum inverse exposes the expected host value. |
| `experiments/gpu/datalog-runtime/src/layout.rs:564` | P | diagnostic cross-lane u64 maximum inverse exposes its signed lane bits. |
| `experiments/gpu/datalog-runtime/src/layout.rs:566` | P | diagnostic cross-lane i32 minimum inverse exposes the expected host value. |
| `experiments/gpu/datalog-runtime/src/layout.rs:568` | P | diagnostic cross-lane i8 maximum inverse exposes the expected host value. |
| `experiments/gpu/datalog-runtime/src/layout.rs:570` | P | diagnostic cross-lane unsigned zero inverse exposes the expected host value. |
| `experiments/gpu/datalog-runtime/src/layout.rs:572` | P | diagnostic cross-lane i32 maximum inverse exposes the expected host value. |
| `experiments/gpu/datalog-runtime/src/layout.rs:584` | P | diagnostic full-width signed minimum inverse exposes the expected host value. |
| `experiments/gpu/datalog-runtime/src/layout.rs:586` | P | diagnostic full-width signed maximum inverse exposes the expected host value. |
| `experiments/gpu/datalog-runtime/src/layout.rs:611` | P | diagnostic wide-layout inverse helper reconstructs fields for exact host assertions. |
| `experiments/gpu/datalog-runtime/src/layout.rs:623` | P | diagnostic wide-word field inverse exposes each expected logical scalar. |
| `experiments/gpu/datalog-runtime/src/layout.rs:651` | P | diagnostic three-lane inverse helper exposes reconstructed tuples for exact assertions. |
| `experiments/gpu/datalog-runtime/src/layout.rs:657` | P | diagnostic four-lane inverse helper exposes reconstructed tuples for exact assertions. |
| `experiments/gpu/datalog-runtime/src/lib.rs:28` | P | datalog print output reconstruction invokes this inverse for final host answer rows. |
| `experiments/gpu/datalog-runtime/src/lib.rs:70` | P | diagnostic Text codec exposes logical strings for round-trip assertions. |
| `experiments/gpu/datalog-runtime/src/lib.rs:85` | P | diagnostic direct symbol round trip publishes its reconstructed String. |
| `experiments/gpu/datalog-runtime/src/lib.rs:94` | P | diagnostic Scalar symbol round trip publishes its reconstructed logical value. |
| `experiments/gpu/datalog-runtime/src/pack.rs:146` | ? | public sign-ordinal inverse has no enforced caller read boundary. |
| `experiments/gpu/datalog-runtime/src/pack.rs:151` | ? | Field inverse delegation inherits the public Column caller boundary. |
| `experiments/gpu/datalog-runtime/src/pack.rs:175` | ? | field sign-ordinal inverse serves publication and diagnostic data generators. |
| `experiments/gpu/datalog-runtime/src/pack.rs:276` | P | datalog output printer and layout diagnostic download unpack host answer rows. |
| `experiments/gpu/datalog-runtime/src/pack.rs:286` | P | per-field ordinal inversion implements host answer unpacking. |
| `experiments/gpu/datalog-runtime/src/pack.rs:448` | I | diagnostic random codes become values then are repacked; this reorganization is avoidable. |
| `experiments/gpu/datalog-runtime/src/pack.rs:463` | P | diagnostic left tuple unpack exposes the round-trip answer. |
| `experiments/gpu/datalog-runtime/src/pack.rs:465` | P | diagnostic right tuple unpack exposes the round-trip answer. |
| `experiments/gpu/datalog-runtime/src/pack.rs:508` | P | diagnostic signed boundary inverse exposes the numeric answer. |
| `experiments/gpu/datalog-runtime/src/pack.rs:518` | P | diagnostic unsigned maximum inverse exposes the round-trip answer. |
| `experiments/gpu/datalog-runtime/src/pack.rs:523` | P | diagnostic over-width decode checks answer rejection. |
| `experiments/gpu/datalog-runtime/src/pack.rs:539` | P | diagnostic empty tuple inverse exposes the zero-column answer. |
| `experiments/gpu/datalog-runtime/src/pack.rs:557` | P | diagnostic permutation inverse exposes reconstructed host fields. |
| `experiments/gpu/datalog-runtime/src/pack.rs:572` | P | diagnostic undersized word decode checks schema rejection. |
| `experiments/gpu/datalog-runtime/src/pack.rs:579` | P | diagnostic stray packed bits check answer rejection. |
| `experiments/gpu/datalog-runtime/src/pack.rs:623` | P | diagnostic excess high lane checks narrow-layout decode rejection. |
| `experiments/gpu/datalog-runtime/src/pack.rs:625` | P | diagnostic excess low lane checks narrow-layout decode rejection. |
| `experiments/gpu/datalog-runtime/tests/layout_arena.rs:162` | P | diagnostic layout decoder exposes fields and lookup ranges for host exactness assertions. |
| `experiments/gpu/datalog-runtime/tests/layout_arena.rs:169` | P | diagnostic device field inverse publishes reconstructed values to the oracle output. |
| `experiments/gpu/datalog-runtime/tests/layout_arena.rs:173` | V | diagnostic prefix inversion precedes relational lookup; P6 compare the encoded prefix instead. |
| `experiments/gpu/datalog-runtime/tests/layout_arena.rs:197` | P | diagnostic wide-layout launch reconstructs values for host exactness checking. |
| `experiments/gpu/datalog-runtime/tests/layout_arena.rs:219` | P | diagnostic original layout unpack exposes the pre-permutation host answer. |
| `experiments/gpu/datalog-runtime/tests/layout_arena.rs:222` | P | diagnostic retained layout unpack exposes the unchanged post-permutation answer. |
| `experiments/gpu/datalog-runtime/tests/layout_arena.rs:233` | P | diagnostic permuted-layout launch reconstructs values for host equality checking. |
| `experiments/gpu/datalog/facade/tests/symbols.rs:9` | P | diagnostic final datalog answers reconstruct TestSymbol handles for host assertions. |
| `experiments/gpu/datalog/src/print.rs:399` | P | generated SymbolCodec inverse resolves final datalog host output fields. |
| `experiments/gpu/datalog/src/print.rs:401` | P | generated numeric carrier inversion constructs final host output fields. |
| `experiments/gpu/datalog/src/print.rs:409` | P | generated final host output unpack inverts packed tuple ordinals after raw word download. |
| `experiments/gpu/device-columns-oracle/src/main.rs:118` | P | diagnostic codec oracle publishes reconstructed column values for exact comparison. |
| `experiments/gpu/device-columns-oracle/src/main.rs:157` | I | diagnostic device chain decodes then immediately re-encodes the column; this reorganization is avoidable. |
| `experiments/gpu/device-columns-oracle/src/main.rs:240` | P | diagnostic adopted Plain column publishes its logical answer prefix. |
| `experiments/gpu/device-columns-oracle/src/main.rs:244` | P | diagnostic scratch inverse exposes live logical values for the following host assertion. |
| `experiments/gpu/device-columns-oracle/src/main.rs:375` | P | diagnostic left dictionary-code column exposes FOR-reconstructed IDs for equality checks. |
| `experiments/gpu/device-columns-oracle/src/main.rs:377` | P | diagnostic right dictionary-code column exposes FOR-reconstructed IDs for equality checks. |
| `experiments/gpu/device-columns-oracle/src/main.rs:383` | P | diagnostic dictionary resolution exposes both expected strings for shared-ID equality checking. |
| `experiments/gpu/device-columns/src/device.rs:90` | ? | source inverse serves gather re-encoding and relational index builds; classify the consuming call. |
| `experiments/gpu/device-columns/src/device.rs:104` | ? | packed base-plus-offset reconstruction inherits the source caller boundary. |
| `experiments/gpu/device-columns/src/device.rs:107` | ? | column delegation serves mixed source consumers without a local read fact. |
| `experiments/gpu/device-columns/src/device.rs:114` | V | narrowed source reconstruction feeds Sequence sorting/indexing; P6 retain encoded keys. |
| `experiments/gpu/device-columns/src/device.rs:117` | V | ordinary source decode precedes key sorting; P6 avoid pre-sort expansion. |
| `experiments/gpu/device-columns/src/device.rs:120` | V | packed source decode precedes key sorting; P6 compare recorded packed fields. |
| `experiments/gpu/device-columns/src/device.rs:185` | ? | random-access FOR/delta inverse has mixed callers; no local opaque-read/publication fact. |
| `experiments/gpu/device-columns/src/device.rs:198` | ? | regular delta base reconstruction needs the consuming row-read fact. |
| `experiments/gpu/device-columns/src/device.rs:204` | ? | delta prefix reconstruction adds base and residuals for an unspecified caller. |
| `experiments/gpu/device-columns/src/device.rs:212` | ? | FOR base restoration is logical decoding even when the result is another domain code. |
| `experiments/gpu/device-columns/src/device.rs:222` | ? | u32 FOR inverse is shared by selection, lookup and diagnostic consumers. |
| `experiments/gpu/device-columns/src/device.rs:225` | ? | native u32 fallback inherits its caller consumption boundary. |
| `experiments/gpu/device-columns/src/device.rs:230` | ? | delta u32 fallback inherits its caller consumption boundary. |
| `experiments/gpu/device-columns/src/device.rs:234` | ? | narrowed FOR base addition reconstructs a value for an unspecified consumer. |
| `experiments/gpu/device-columns/src/device.rs:254` | ? | stripe reconstruction serves generated consumers without a local logical-read fact. |
| `experiments/gpu/device-columns/src/device.rs:271` | ? | packed stripe delegation inherits the generated caller boundary. |
| `experiments/gpu/device-columns/src/device.rs:321` | ? | packed stripe reconstruction requires caller opaque-read or publication evidence. |
| `experiments/gpu/device-columns/src/device.rs:337` | ? | regular delta stripe restores restart/base before an unspecified consumer. |
| `experiments/gpu/device-columns/src/device.rs:340` | ? | narrow delta stripe restores frame plus prefix without caller evidence. |
| `experiments/gpu/device-columns/src/device.rs:375` | ? | wide delta stripe restores base plus prefix without caller evidence. |
| `experiments/gpu/device-columns/src/device.rs:385` | ? | FOR stripe base addition reconstructs a logical value before an unspecified consumer. |
| `experiments/gpu/device-columns/src/device.rs:397` | ? | pair reconstruction serves both publication and generated relational kernels. |
| `experiments/gpu/device-columns/src/device.rs:409` | ? | first FOR pair restores its base without a consumer fact. |
| `experiments/gpu/device-columns/src/device.rs:415` | ? | second FOR pair restores its base without a consumer fact. |
| `experiments/gpu/device-columns/src/device.rs:425` | ? | paired delta restart/base reconstruction has no local read boundary. |
| `experiments/gpu/device-columns/src/device.rs:464` | ? | paired delta residual inversion completes values for an unspecified tile caller. |
| `experiments/gpu/device-columns/src/device.rs:479` | ? | tile decoder serves whole-column publication and generated kernels with different boundaries. |
| `experiments/gpu/device-columns/src/device.rs:499` | ? | first tile-pair delegation cannot establish the eventual logical consumer. |
| `experiments/gpu/device-columns/src/device.rs:501` | ? | second tile-pair delegation cannot establish final-answer publication. |
| `experiments/gpu/device-columns/src/device.rs:511` | ? | tile-pair decoder permits consumers with further joins; require caller evidence. |
| `experiments/gpu/device-columns/src/device.rs:529` | ? | requested tile-pair reconstruction inherits the generated consumer boundary. |
| `experiments/gpu/device-columns/src/encoding.rs:52` | ? | borrowed dictionary inverse requires caller publication or opaque Rust read evidence. |
| `experiments/gpu/device-columns/src/encoding.rs:70` | ? | owned/borrowed dictionary lookup alone does not establish its logical read boundary. |
| `experiments/gpu/device-columns/src/encoding.rs:152` | ? | logical scalar inverse contract is shared by host reads and answer publication. |
| `experiments/gpu/device-columns/src/encoding.rs:163` | ? | generic typed reconstruction needs the caller read fact; a Rust type is not evidence. |
| `experiments/gpu/device-columns/src/encoding.rs:168` | ? | trait dispatch inherits the generic caller boundary. |
| `experiments/gpu/device-columns/src/encoding.rs:186` | ? | integer carrier inverse serves generic typed reads and answers. |
| `experiments/gpu/device-columns/src/encoding.rs:211` | ? | Decimal inverse is host-only but its caller must establish the allowed read. |
| `experiments/gpu/device-columns/src/encoding.rs:213` | ? | coefficient-to-Decimal construction inherits the host caller boundary. |
| `experiments/gpu/device-columns/src/encoding.rs:228` | ? | Date inverse is host-only but its caller must establish publication or opaque read. |
| `experiments/gpu/device-columns/src/encoding.rs:230` | ? | checked day-carrier inverse and Date construction inherit the host caller boundary. |
| `experiments/gpu/device-columns/src/encoding.rs:246` | ? | boolean carrier inverse requires caller logical-consumption evidence. |
| `experiments/gpu/device-columns/src/encoding.rs:274` | ? | borrowed string decoder delegates dictionary inversion without caller evidence. |
| `experiments/gpu/device-columns/src/encoding.rs:276` | ? | borrowed dictionary resolution inherits its generic caller boundary. |
| `experiments/gpu/device-columns/src/encoding.rs:281` | ? | owned string decoder lacks a caller-level publication fact. |
| `experiments/gpu/device-columns/src/encoding.rs:283` | ? | dictionary resolution and String allocation require the eventual logical-read fact. |
| `experiments/gpu/device-columns/src/encoding.rs:297` | P | diagnostic Decimal fixture constructs the expected coefficient answer. |
| `experiments/gpu/device-columns/src/encoding.rs:302` | P | diagnostic Decimal round trip exposes the reconstructed scalar. |
| `experiments/gpu/device-columns/src/encoding.rs:311` | P | diagnostic unsupported scale checks logical decode rejection. |
| `experiments/gpu/device-columns/src/encoding.rs:319` | P | diagnostic Date fixture constructs the expected epoch-day answer. |
| `experiments/gpu/device-columns/src/encoding.rs:323` | P | diagnostic Date round trip exposes the reconstructed date. |
| `experiments/gpu/device-columns/src/encoding.rs:326` | P | diagnostic lower day overflow checks logical decode rejection. |
| `experiments/gpu/device-columns/src/encoding.rs:328` | P | diagnostic upper day overflow checks logical decode rejection. |
| `experiments/gpu/device-columns/src/encoding.rs:338` | P | diagnostic row fixture constructs its Decimal field. |
| `experiments/gpu/device-columns/src/encoding.rs:340` | P | diagnostic row fixture constructs its Date field. |
| `experiments/gpu/device-columns/src/encoding.rs:350` | P | diagnostic row inverse exposes the reconstructed scalar-and-symbol answer. |
| `experiments/gpu/device-columns/src/encoding.rs:353` | P | diagnostic dictionary lookup exposes the preserved earlier symbol. |
| `experiments/gpu/device-columns/src/encoding.rs:362` | P | diagnostic integer round trip exposes the reconstructed maximum. |
| `experiments/gpu/device-columns/src/encoding.rs:372` | P | diagnostic integer overflow checks typed decode rejection. |
| `experiments/gpu/device-columns/src/encoding.rs:374` | P | diagnostic invalid boolean checks typed decode rejection. |
| `experiments/gpu/device-columns/src/encoding.rs:386` | P | diagnostic negative dictionary ID checks answer rejection. |
| `experiments/gpu/device-columns/src/encoding.rs:388` | P | diagnostic borrowed answer is exposed for lifetime and equality assertions. |
| `experiments/gpu/device-columns/src/encoding.rs:406` | P | diagnostic first owned symbol exposes the merged-dictionary answer. |
| `experiments/gpu/device-columns/src/encoding.rs:411` | P | diagnostic second owned symbol exposes the appended-dictionary answer. |
| `experiments/gpu/device-columns/src/encoding.rs:415` | P | diagnostic owned-only dictionary checks borrowed answer rejection. |
| `experiments/gpu/device-columns/src/encoding.rs:429` | P | diagnostic common dictionary exposes a promoted borrowed answer. |
| `experiments/gpu/device-columns/src/encoding.rs:436` | P | diagnostic merged first symbol exposes its retained borrowed answer. |
| `experiments/gpu/device-columns/src/encoding.rs:438` | P | diagnostic merged second symbol exposes its extended borrowed answer. |
| `experiments/gpu/device-columns/src/host.rs:34` | ? | row inverse contract has no enforced caller read boundary. |
| `experiments/gpu/device-columns/src/host.rs:42` | ? | zero-column row inverse is vacuous but inherits the generic caller boundary. |
| `experiments/gpu/device-columns/src/host.rs:55` | ? | tuple reconstruction is shared by publication and diagnostic callers. |
| `experiments/gpu/device-columns/src/host.rs:57` | ? | per-field typed inversion inherits the generic row consumer boundary. |
| `experiments/gpu/device-columns/src/host.rs:315` | P | explicit DeviceColumns host answer publication is not called by query binding or chaining. |
| `experiments/gpu/device-columns/src/host.rs:325` | P | final host row publication expands only the live answer columns before transfer. |
| `experiments/gpu/device-columns/src/host.rs:334` | P | final host answer rows resolve scalar and dictionary representations. |
| `experiments/gpu/device-columns/src/host.rs:401` | I | diagnostic inverse is immediately re-encoded for round-trip checking; the reorganization is avoidable. |
| `experiments/gpu/device-columns/src/kernels.rs:357` | I | gather reconstructs source values then recomputes FOR frames and re-encodes; avoidable reorganization, P4 keep index runs plain. |
| `experiments/gpu/device-columns/src/kernels.rs:432` | ? | whole-column kernel serves publication and build scratch; caller evidence determines the boundary. |
| `experiments/gpu/device-columns/src/kernels.rs:438` | ? | tile reconstruction feeds the generic whole-column destination without a consumer fact. |
| `experiments/gpu/device-columns/src/lib.rs:296` | I | gather wrapper reconstructs then FOR-reencodes a permutation; avoidable reorganization, P4 keep index runs plain. |
| `experiments/gpu/device-columns/src/lib.rs:307` | I | gather delegates source inversion and immediate FOR re-encoding; this reorganization is avoidable. |
| `experiments/gpu/device-columns/src/lib.rs:328` | I | source gather recomputes frames after value reconstruction; avoidable reorganization, P4 prefer plain indexes. |
| `experiments/gpu/device-columns/src/lib.rs:364` | I | fused gather launch decodes and immediately re-encodes values; this reorganization is avoidable. |
| `experiments/gpu/device-columns/src/lib.rs:536` | ? | whole-column scratch inverse serves both final answers and device-side build consumers. |
| `experiments/gpu/device-columns/src/lib.rs:540` | ? | scratch reconstruction inherits decoded caller evidence, not the allocation location. |
| `experiments/gpu/device-columns/src/lib.rs:548` | ? | decode_into accepts arbitrary device consumers; no opaque-read or publication fact is recorded. |
| `experiments/gpu/device-columns/src/lib.rs:557` | ? | whole-column kernel launch inherits the caller boundary and may precede relational work. |
| `experiments/gpu/device-columns/src/lib.rs:579` | P | explicit logical column host publication reconstructs stored integer values. |
| `experiments/gpu/device-columns/src/lib.rs:585` | P | compressed host answer download invokes the column inverse before transfer. |
| `experiments/gpu/device-columns/src/lib.rs:607` | P | diagnostic Plain inspection uses logical publication; its identity branch performs no packed inversion. |
| `experiments/gpu/doop/src/lib.rs:12` | P | datalog output printer invokes DoopSymbols only for final host answer symbols. |
| `experiments/gpu/doop/src/lib.rs:14` | P | process-global symbol code inversion restores the final DOOP host answer handle. |
| `experiments/gpu/h05-compiled/src/bin/printer-oracle.rs:66` | P | diagnostic projection answer publication feeds the exact host oracle assertion. |
| `experiments/gpu/h05-compiled/src/bin/printer-oracle.rs:110` | P | diagnostic repeated-head answer publication checks lossy projection deduplication. |
| `experiments/gpu/h05-compiled/src/bin/printer-smoke.rs:42` | P | diagnostic smoke answer publication exposes host tuples for reference comparison. |
| `experiments/gpu/h05-compiled/src/bin/triangle-compiled.rs:129` | P | diagnostic triangle answer publication exposes logical tuples for CPU comparison. |
| `experiments/gpu/h05-compiled/src/bin/triangle-compiled.rs:190` | P | diagnostic signed-boundary triangle publication exposes reconstructed result tuples. |
| `experiments/gpu/h05-compiled/src/main.rs:200` | P | diagnostic final chained H05 answer publishes logical host tuples for exact checking. |
| `experiments/gpu/h05-compiled/src/main.rs:203` | P | diagnostic final H05 answer inverts order, supplier and nation ordinal domains. |
| `experiments/gpu/h05-reference/src/kernels.rs:101` | V | both FOR directory offsets are reconstructed before join expansion; P6 use plain index directories. |
| `experiments/gpu/h05-reference/src/kernels.rs:112` | V | binary membership reconstructs each midpoint before comparison; P6 compare encoded keys. |
| `experiments/gpu/h05-reference/src/kernels.rs:119` | V | final membership reconstructs the candidate before equality; P6 retain code-domain operands. |
| `experiments/gpu/h05-reference/src/kernels.rs:171` | V | full key column expands before radix sorting; P6 sort encoded keys or use plain index runs. |
| `experiments/gpu/h05-reference/src/kernels.rs:193` | V | selected keys expand before the next sorting pass; P6 preserve recorded code representations. |
| `experiments/gpu/h05-reference/src/kernels.rs:209` | V | column decoder materializes all index keys before sort; P6 keep keys encoded. |
| `experiments/gpu/h05-reference/src/kernels.rs:216` | V | gathered key reconstruction feeds lexicographic sort; P6 compare retained codes. |
| `experiments/gpu/h05-reference/src/kernels.rs:237` | V | date/region selection decodes before relational comparison; P6 translate range constants. |
| `experiments/gpu/h05-reference/src/kernels.rs:262` | V | compressed region directory endpoint controls further expansion; P6 use plain index directories. |
| `experiments/gpu/h05-reference/src/kernels.rs:266` | V | region ID FOR inverse precedes the nation join; P6 retain plain/code-domain index keys. |
| `experiments/gpu/h05-reference/src/kernels.rs:271` | V | nation ID FOR inverse precedes the customer join; P6 avoid compressed index payload probes. |
| `experiments/gpu/h05-reference/src/kernels.rs:352` | V | customer ID FOR inverse precedes the orders join; P6 retain index keys without expansion. |
| `experiments/gpu/h05-reference/src/kernels.rs:381` | V | order ID FOR inverse precedes the lineitem join; P6 avoid compressed random-access index payloads. |
| `experiments/gpu/h05-reference/src/kernels.rs:420` | V | supplier ID FOR inverse precedes membership probing; P6 compare retained encoded keys. |
| `experiments/gpu/h05-reference/src/main.rs:156` | V | reference selection launches decode-before-range filtering; P6 translate predicates to the encoded domain. |
| `experiments/gpu/h05-reference/src/main.rs:190` | V | full index key reconstruction precedes sorting; P6 retain encoded keys. |
| `experiments/gpu/h05-reference/src/main.rs:192` | V | selected index key reconstruction precedes sorting; P6 compare recorded codes. |
| `experiments/gpu/h05-reference/src/main.rs:217` | V | index builder launches whole-key decode before lexicographic sort; P6 retain encoded keys. |
| `experiments/gpu/h05-reference/src/main.rs:264` | I | sorted payload gather decodes and immediately FOR-reencodes; avoidable reorganization, P4 keep index runs plain. |
| `experiments/gpu/h05-reference/src/main.rs:596` | P | diagnostic final H05 answer unpacks order bits and inverts the order domain for exact comparison. |
| `experiments/gpu/h05-reference/src/main.rs:598` | P | diagnostic final H05 answer unpacks supplier bits and inverts the supplier domain. |
| `experiments/gpu/h05-reference/src/main.rs:600` | P | diagnostic final H05 answer unpacks nation bits and inverts the nation domain. |
| `experiments/gpu/sequence-runtime/src/canon.rs:557` | V | packed answers expand into resident columns usable by chained queries; P6 retain packed fields until an allowed read. |
| `experiments/gpu/sequence-runtime/src/canon.rs:565` | V | into_columns invokes device expansion before host publication; P6 preserve the packed view. |
| `experiments/gpu/sequence-runtime/src/canon.rs:575` | V | packed_rows wrapper eagerly expands into device columns before consumers; P6 retain packed resident fields. |
| `experiments/gpu/sequence-runtime/src/canon.rs:584` | V | packed_rows wrapper dispatches device expansion before publication; P6 defer logical inversion. |
| `experiments/gpu/sequence-runtime/src/canon.rs:623` | V | packed adapter expands the live prefix before possible chained relations; P6 keep its encoded view. |
| `experiments/gpu/sequence-runtime/src/canon.rs:647` | V | packed_columns expands every field into device buffers; P6 defer inversion until logical reads. |
| `experiments/gpu/sequence-runtime/src/canon.rs:661` | V | canonical output adapter expands packed values before possible chained relations; P6 retain encoded fields. |
| `experiments/gpu/sequence-runtime/src/canon.rs:670` | V | canonical unpack delegates eager device expansion; P6 publish a packed resident view. |
| `experiments/gpu/sequence-runtime/src/canon.rs:682` | V | output adapter materializes full i64 columns before their consumers; P6 keep packed values resident. |
| `experiments/gpu/sequence-runtime/src/canon.rs:724` | V | per-field unpack launch expands all live rows before chaining; P6 decode only at the allowed consumer. |
| `experiments/gpu/sequence-runtime/src/canon.rs:807` | V | unpack_field constructs device logical columns before possible relational consumers; P6 preserve packed fields. |
| `experiments/gpu/sequence-runtime/src/canon.rs:840` | V | packed FOR base-plus-offset inversion precedes host publication and may feed chaining; P6 delay reconstruction. |
| `experiments/gpu/sequence-runtime/src/device.rs:59` | V | lower directory offset FOR inversion precedes range expansion; P6 use plain index directories. |
| `experiments/gpu/sequence-runtime/src/device.rs:61` | V | upper directory offset FOR inversion precedes range expansion; P6 avoid compressed random-access directories. |
| `experiments/gpu/sequence-runtime/src/device.rs:79` | V | binary membership probe reconstructs each midpoint; P6 compare code-domain keys or use plain indexes. |
| `experiments/gpu/sequence-runtime/src/device.rs:86` | V | final membership comparison reconstructs the candidate; P6 retain encoded comparison operands. |
| `experiments/gpu/sequence-runtime/src/group.rs:171` | P | GroupedRows explicitly publishes final logical host aggregate answers. |
| `experiments/gpu/sequence-runtime/src/group.rs:182` | P | final grouped answer decoder reconstructs logical keys and nullable aggregates. |
| `experiments/gpu/sequence-runtime/src/group.rs:600` | P | diagnostic resident grouped projection exposes reconstructed host rows. |
| `experiments/gpu/sequence-runtime/src/group.rs:603` | P | diagnostic grouped publication checks retained device-error rejection. |
| `experiments/gpu/sequence-runtime/src/kernels.rs:21` | V | bound reduction reconstructs every source value before index planning; P6 reuse recorded encoded-domain bounds. |
| `experiments/gpu/sequence-runtime/src/kernels.rs:61` | V | index key extraction reconstructs values before sorting; P6 sort recorded codes or keep index runs plain. |
| `experiments/gpu/sequence-runtime/src/kernels.rs:75` | V | multi-key index build expands each column before lexicographic sorting; P6 retain encoded keys. |
| `experiments/gpu/sequence-runtime/src/kernels.rs:86` | V | permutation gather reconstructs keys for a later sort; P6 compare encoded fields. |
| `experiments/gpu/sequence-runtime/src/kernels.rs:105` | V | range selection decodes before relational comparison; P6 translate bounds into the encoded domain. |
| `experiments/gpu/sequence-runtime/src/lib.rs:208` | ? | domain ordinal inverse has no local caller read/publication evidence. |
| `experiments/gpu/sequence-runtime/src/lib.rs:210` | ? | checked domain lookup reconstructs a source key for an unspecified consumer. |
| `experiments/gpu/sequence-runtime/src/lib.rs:354` | V | measuring a computed packed publication reconstructs values before provider planning; P6 reuse recorded summaries. |
| `experiments/gpu/sequence-runtime/src/lib.rs:366` | ? | source-bound reduction accepts identity and packed sources; caller representation facts decide whether it decodes. |
| `experiments/gpu/sequence-runtime/src/lib.rs:381` | V | bounds discovery launches source reconstruction before index building; P6 consume recorded summaries instead. |
| `experiments/gpu/sequence-runtime/src/lib.rs:813` | V | index selection launch reconstructs values before range comparison; P6 translate predicates to codes. |
| `experiments/gpu/sequence-runtime/src/lib.rs:861` | I | keyless payload publication decodes then FOR-reencodes selected rows; avoidable reorganization, P4 keep index payloads plain. |
| `experiments/gpu/sequence-runtime/src/lib.rs:928` | V | multi-key index build launches full column reconstruction before sorting; P6 sort retained encoded fields. |
| `experiments/gpu/sequence-runtime/src/lib.rs:967` | V | leading-key reconstruction follows tuple sort for directory building; P6 retain encoded key order. |
| `experiments/gpu/sequence-runtime/src/lib.rs:981` | V | single-key reconstruction precedes radix sort; P6 sort recorded codes or use plain indexes. |
| `experiments/gpu/sequence-runtime/src/lib.rs:1030` | I | sorted index payload publication decodes then FOR-reencodes rows; avoidable reorganization, P4 keep index runs plain. |
| `experiments/gpu/sequence-runtime/src/lib.rs:1082` | I | payload packing reconstructs source values then emits new FOR frames; avoidable reorganization, P4 keep index runs plain. |
| `experiments/gpu/sequence-runtime/src/lib.rs:1093` | I | u32 payload gather decodes then re-encodes a permutation; avoidable reorganization, P4 prefer plain indexes. |
| `experiments/gpu/sequence-runtime/src/lib.rs:1105` | I | i32 payload gather decodes then re-encodes a permutation; avoidable reorganization, P4 prefer plain indexes. |
| `experiments/gpu/sequence-runtime/src/lib.rs:1117` | I | i64 payload gather decodes then re-encodes a permutation; avoidable reorganization, P4 prefer plain indexes. |
| `experiments/gpu/sequence-runtime/src/selected.rs:68` | V | selection launches value reconstruction before range comparison; P6 evaluate translated encoded predicates. |
| `experiments/gpu/sequence-runtime/tests/grouped_runtime.rs:26` | P | diagnostic grouped monoids publish host values and validity for exact assertions. |
| `experiments/gpu/sequence-runtime/tests/grouped_runtime.rs:37` | P | diagnostic selected aggregate publishes its final host answer. |
| `experiments/gpu/sequence-runtime/tests/grouped_runtime.rs:43` | P | diagnostic empty global aggregate publishes identity values and validity. |
| `experiments/gpu/sequence-runtime/tests/grouped_runtime.rs:49` | P | diagnostic empty keyed aggregate exposes its empty host answer. |
| `experiments/gpu/sequence-runtime/tests/grouped_runtime.rs:68` | P | diagnostic u32 aggregate overflow checks logical answer publication rejection. |
| `experiments/gpu/sequence-runtime/tests/grouped_runtime.rs:73` | P | diagnostic i64 aggregate overflow checks logical answer publication rejection. |
| `experiments/gpu/sequence-runtime/tests/providers.rs:47` | P | diagnostic typed numeric upload round trip publishes logical host values for exact assertions. |
| `experiments/gpu/sequence-runtime/tests/providers.rs:50` | P | diagnostic empty reused upload publishes logical host values for exact assertions. |
| `experiments/gpu/sequence-runtime/tests/providers.rs:56` | P | diagnostic changed reused upload publishes logical host values for exact assertions. |
| `experiments/gpu/sequence-runtime/tests/providers.rs:65` | P | diagnostic large signed maximum publishes logical host values for exact assertions. |
| `experiments/gpu/sequence-runtime/tests/providers.rs:125` | P | diagnostic gathered wide payload publishes logical host values for exact assertions. |
| `experiments/gpu/sequence-runtime/tests/providers.rs:128` | P | diagnostic canonical tuple adapter reconstructs packed fields solely for the following host oracle. |
| `experiments/gpu/sequence-runtime/tests/providers.rs:130` | P | diagnostic canonical packed tuples publishes logical host values for exact assertions. |
| `experiments/gpu/sequence-runtime/tests/providers.rs:132` | P | diagnostic noncanonical tuple adapter reconstructs packed fields solely for the following host oracle. |
| `experiments/gpu/sequence-runtime/tests/providers.rs:134` | P | diagnostic noncanonical packed tuples publishes logical host values for exact assertions. |
| `experiments/gpu/sequence-runtime/tests/providers.rs:144` | P | diagnostic signed-boundary adapter reconstructs packed fields solely for the following host oracle. |
| `experiments/gpu/sequence-runtime/tests/providers.rs:146` | P | diagnostic full-width signed tuple boundaries publishes logical host values for exact assertions. |
| `experiments/gpu/sequence-runtime/tests/providers.rs:186` | P | diagnostic three-lane adapter reconstructs packed fields solely for the following host oracle. |
| `experiments/gpu/sequence-runtime/tests/providers.rs:193` | P | diagnostic expected three-lane answer restores each recorded signed frame base. |
| `experiments/gpu/sequence-runtime/tests/providers.rs:195` | P | diagnostic three-lane packed tuples publishes logical host values for exact assertions. |
| `experiments/gpu/sequence-runtime/tests/providers.rs:238` | P | diagnostic selected directory offsets publishes logical host values for exact assertions. |
| `experiments/gpu/sequence-runtime/tests/providers.rs:240` | P | diagnostic selected payload values publishes logical host values for exact assertions. |
| `experiments/gpu/sequence-runtime/tests/providers.rs:251` | P | diagnostic empty selected directory publishes logical host values for exact assertions. |
| `experiments/gpu/sequence-runtime/tests/providers.rs:253` | P | diagnostic empty selected payload publishes logical host values for exact assertions. |
| `experiments/gpu/sequence-runtime/tests/providers.rs:268` | P | diagnostic canonical scalar adapter reconstructs packed fields solely for the following host oracle. |
| `experiments/gpu/sequence-runtime/tests/providers.rs:270` | P | diagnostic canonical scalar keys publishes logical host values for exact assertions. |
| `experiments/gpu/sequence-runtime/tests/providers.rs:272` | P | diagnostic noncanonical scalar adapter reconstructs packed fields solely for the following host oracle. |
| `experiments/gpu/sequence-runtime/tests/providers.rs:274` | P | diagnostic noncanonical scalar keys publishes logical host values for exact assertions. |
| `experiments/gpu/sequence-runtime/tests/providers.rs:283` | P | diagnostic retained workspace payload publishes logical host values for exact assertions. |
| `experiments/gpu/sequence-runtime/tests/providers.rs:285` | P | diagnostic new workspace payload publishes logical host values for exact assertions. |
| `experiments/gpu/sequence-runtime/tests/providers.rs:287` | P | diagnostic retained workspace directory publishes logical host values for exact assertions. |
| `experiments/gpu/sequence-runtime/tests/providers.rs:289` | P | diagnostic new workspace directory publishes logical host values for exact assertions. |
| `experiments/gpu/sequence-runtime/tests/providers.rs:293` | P | diagnostic reused workspace payload publishes logical host values for exact assertions. |
| `experiments/gpu/sequence-runtime/tests/providers.rs:298` | P | diagnostic first retained adapter reconstructs packed fields solely for the following host oracle. |
| `experiments/gpu/sequence-runtime/tests/providers.rs:306` | P | diagnostic second retained adapter reconstructs packed fields solely for the following host oracle. |
| `experiments/gpu/sequence-runtime/tests/providers.rs:314` | P | diagnostic first retained adapter output publishes logical host values for exact assertions. |
| `experiments/gpu/sequence-runtime/tests/providers.rs:316` | P | diagnostic second retained adapter output publishes logical host values for exact assertions. |
| `experiments/gpu/sequence-runtime/tests/providers.rs:319` | P | diagnostic empty reused adapter reconstructs packed fields solely for the following host oracle. |
| `experiments/gpu/sequence-runtime/tests/providers.rs:327` | P | diagnostic empty reused adapter output publishes logical host values for exact assertions. |
| `experiments/gpu/sequence-runtime/tests/providers.rs:374` | P | diagnostic codec-sweep gathered payload publishes logical host values for exact assertions. |
| `experiments/gpu/sequence-runtime/tests/providers.rs:376` | P | diagnostic codec-sweep gathered directory publishes logical host values for exact assertions. |
| `experiments/gpu/sequence-runtime/tests/providers.rs:384` | P | diagnostic codec-sweep empty payload publishes logical host values for exact assertions. |
| `experiments/gpu/sequence-runtime/tests/providers.rs:386` | P | diagnostic codec-sweep empty directory publishes logical host values for exact assertions. |
| `experiments/gpu/sequence-runtime/tests/providers.rs:439` | P | diagnostic signed extrema retained payload publishes logical host values for exact assertions. |
| `experiments/gpu/sequence-runtime/tests/providers.rs:449` | P | diagnostic negative-only directory publishes logical host values for exact assertions. |
| `experiments/gpu/sequence-runtime/tests/providers.rs:451` | P | diagnostic negative-only payload publishes logical host values for exact assertions. |
| `experiments/gpu/sequence-runtime/tests/providers.rs:505` | P | diagnostic packed-source directory publishes logical host values for exact assertions. |
| `experiments/gpu/sequence-runtime/tests/providers.rs:508` | P | diagnostic packed-source signed payload publishes logical host values for exact assertions. |
| `experiments/gpu/sequence-runtime/tests/providers.rs:565` | P | diagnostic shifted packed-source directory publishes logical host values for exact assertions. |
| `experiments/gpu/sequence-runtime/tests/providers.rs:568` | P | diagnostic shifted packed-source payload publishes logical host values for exact assertions. |
| `experiments/gpu/sequence-runtime/tests/providers.rs:593` | P | diagnostic keyless complete empty directory publishes logical host values for exact assertions. |
| `experiments/gpu/sequence-runtime/tests/providers.rs:595` | P | diagnostic keyless complete payload publishes logical host values for exact assertions. |
| `experiments/gpu/sequence-runtime/tests/providers.rs:599` | P | diagnostic keyless selected payload publishes logical host values for exact assertions. |
| `experiments/gpu/sequence-runtime/tests/providers.rs:605` | P | diagnostic keyless empty directory publishes logical host values for exact assertions. |
| `experiments/gpu/sequence-runtime/tests/providers.rs:607` | P | diagnostic keyless empty payload publishes logical host values for exact assertions. |
| `experiments/gpu/sequence-runtime/tests/providers.rs:609` | P | diagnostic keyless retained complete payload publishes logical host values for exact assertions. |
| `experiments/gpu/sequence/core/src/emit.rs:556` | V | P4 run membership prefix_range reconstructs packed signed fields through compare_prefix solely for key comparison. |
| `experiments/gpu/sequence/core/src/emit.rs:566` | V | P4 run lookup prefix_range reconstructs packed signed fields through compare_prefix instead of comparing encoded keys. |
| `experiments/gpu/sequence/core/src/emit.rs:609` | V | P4 lower-bound key search reconstructs FOR values solely for ordered relational comparison. |
| `experiments/gpu/sequence/core/src/emit.rs:618` | V | P4 upper-bound key search reconstructs FOR values solely for ordered relational comparison. |
| `experiments/gpu/sequence/core/src/emit.rs:642` | V | P4 run-provider payload unpack widens keys and code-only fields before relational consumers. |
| `experiments/gpu/sequence/core/src/emit.rs:652` | V | P6 provider payload reconstructs FOR values without restricting decode to rust_reads columns. |
| `experiments/gpu/sequence/core/src/emit.rs:1366` | V | P6 intermediate host publication reconstructs packed FOR fields for the whole stored layout. |
| `experiments/gpu/sequence/core/src/emit.rs:1377` | V | P6 intermediate host fallback resolves symbols and logical values before more engine operators, not final answers. |
| `experiments/gpu/sequence/core/src/emit.rs:1853` | I | union arms expand FOR fields into a shared carrier then repack; avoidable with encoded-domain rebasing. |
| `experiments/gpu/sequence/core/src/emit.rs:1987` | V | P6 canonical union output expands FOR fields on device before host download; retain packed publication. |
| `experiments/gpu/sequence/core/src/emit.rs:1992` | V | P6 noncanonical union output expands FOR fields on device before host download. |
| `experiments/gpu/sequence/core/src/emit.rs:2354` | V | P6 stored intermediates reconstruct FOR fields before downstream relational operators instead of retaining codes. |
| `experiments/gpu/sequence/core/src/emit.rs:2647` | V | P6 canonical output adapter reconstructs device i64 columns before final host publication. |
| `experiments/gpu/sequence/core/src/emit.rs:2652` | V | P6 packed output adapter reconstructs device i64 columns before final host publication. |
| `experiments/gpu/sequence/core/src/emit.rs:2816` | V | P6 host-origin canonical output is reuploaded then expanded on device, rather than decoded at host publication. |
| `experiments/gpu/sequence/core/src/emit.rs:2822` | V | P6 host-origin packed output expands device columns before the user's host download. |
| `experiments/gpu/sequence/core/src/reduce.rs:186` | V | P6 grouped input reconstructs packed FOR fields before relational grouping and aggregation. |
| `experiments/gpu/sequence/core/src/reduce.rs:206` | P | grouped answer decoder resolves symbols, nullable extrema and scalars only when the host requests logical rows. |
| `experiments/gpu/sequence/core/src/rows.rs:1034` | V | P6 run scans unpack every demanded field before relational consumers; retain narrow codes unless rust_reads requires identity. |
| `experiments/gpu/sequence/core/src/rows.rs:1055` | V | P6 source scans reconstruct FOR values into i64 even for code-only joins and predicates. |
| `experiments/gpu/sequence/core/src/rows.rs:1286` | V | P6 relational Select/Point/Range reconstructs Scalar values; preserved-op predicates must compare codes. |
| `experiments/gpu/sequence/core/src/rows.rs:1310` | R | Bind free-variable dependencies select these columns; checked identity representation permits the verbatim opaque Rust expression. |
| `experiments/gpu/sequence/core/src/rows.rs:1331` | R | Guard free-variable dependencies select these columns; checked identity representation permits the verbatim opaque Rust condition. |
| `experiments/gpu/sequence/core/src/rows.rs:1431` | V | P6 host fallback reconstructs the complete run layout, not only opaque-Rust rust_reads columns. |
| `experiments/gpu/sequence/core/src/rows.rs:1437` | V | P6 host fallback materializes complete logical tuples before further relational processing, not final publication. |
| `experiments/gpu/sequence/examples/h05.rs:141` | P | download the final H05 answer before host-side source-key publication. |
| `experiments/gpu/sequence/examples/h05.rs:147` | P | invert the output order domain only after the GPU answer download. |
| `experiments/gpu/sequence/examples/h05.rs:151` | P | invert the output supplier domain only after the GPU answer download. |
| `experiments/gpu/sequence/examples/h05.rs:155` | P | invert the output nation domain only after the GPU answer download. |
| `experiments/gpu/sequence/tests/group.rs:76` | P | publish grouped nullable answers to the host correctness oracle. |
| `experiments/gpu/sequence/tests/group.rs:130` | P | publish filtered grouped answers to the host correctness oracle. |
| `experiments/gpu/sequence/tests/group.rs:170` | P | publish the final scalar fold answer to the host correctness oracle. |
| `experiments/gpu/sequence/tests/grouped_h01.rs:132` | P | publish final H01 groups for exact host comparison with DuckDB. |
| `experiments/gpu/sequence/tests/h05_conformance.rs:36` | ? | diagnostic fixture download reconstructs six input relations for a separate CPU oracle, not production answer publication. |
| `experiments/gpu/sequence/tests/h05_conformance.rs:121` | P | publish the final CUDA H05 answer for host conformance comparison. |
| `experiments/gpu/sequence/tests/h05_conformance.rs:133` | P | publish A/B/R plan answers for host comparison; all three downloads terminate their GPU queries. |
| `experiments/gpu/tpch/build.rs:561` | ? | typed host-input loading may reconstruct Date/Decimal/String; actual opaque-Rust demand is required per column. |
| `experiments/gpu/tpch/build.rs:582` | ? | independent Sequential oracle downloads logical inputs; this diagnostic path is not final GPU answer publication. |
| `experiments/gpu/tpch/build.rs:603` | ? | authored derived-input Rust consumes this download, but whole-row reconstruction needs column-level read evidence. |
| `experiments/gpu/tpch/build.rs:633` | P | final answer formatting inverts dense source-key domains on the host. |
| `experiments/gpu/tpch/build.rs:646` | P | final answer formatting resolves source string dictionaries on the host. |
| `experiments/gpu/tpch/build.rs:653` | P | final answer formatting reconstructs Date values from epoch-day codes on the host. |
| `experiments/gpu/tpch/build.rs:660` | P | final answer formatting reconstructs Decimal values from scaled coefficients on the host. |
| `experiments/gpu/tpch/build.rs:717` | V | P6 staged host execution downloads entire logical input tuples before subsequent relational operators; retain codes outside rust_reads. |
| `experiments/gpu/tpch/build.rs:745` | ? | nonresident stage output may feed later stages or final publication; terminal-use evidence determines R/P/V. |
| `experiments/gpu/tpch/build.rs:885` | P | standalone query execution downloads its final answer for host formatting and comparison. |
| `experiments/gpu/tpch/build.rs:985` | P | diagnostic fixture publishes typed input rows to the host test oracle while preserving string codes. |
| `experiments/gpu/tpch/build_stages.rs:121` | P | only terminal stage results download logical tuples for final user-answer composition. |
| `experiments/gpu/tpch/build_stages.rs:129` | P | terminal GPU answer resolves source dictionaries and formats values after download. |
| `experiments/gpu/tpch/build_stages.rs:134` | P | terminal CPU reference answer resolves the same source dictionaries for final comparison. |
| `experiments/gpu/tpch/src/manifest.rs:75` | ? | Date HostValue reconstruction serves typed host inputs; caller rust_reads versus reference-only use determines necessity. |
| `experiments/gpu/tpch/src/manifest.rs:87` | ? | Decimal HostValue reconstructs a whole typed input column; requires caller rust_reads evidence, not its type alone. |
| `experiments/gpu/tpch/src/manifest.rs:100` | ? | String HostValue resolves dictionary entries for typed host inputs; classify each caller as opaque Rust or premature relational decode. |
| `experiments/gpu/tpch/src/source.rs:297` | R | explicit authored dictionary_where closure reads each dictionary value once; its derived code set feeds relational joins. |
| `experiments/gpu/tpch/src/source.rs:1000` | R | closed authored date literal is reconstructed for its opaque host expression, not a row-dependent relational decode. |
| `features/datalog/doop/shared/src/ascent_engine.rs:208` | R | Method_Descriptor's authored format! reads returntype text to compute a new string, not a relational key comparison. |
| `features/datalog/doop/shared/src/ascent_engine.rs:209` | R | Method_Descriptor's authored format! reads params text to compute a new string before sym encodes that result. |
| `features/datalog/doop/shared/src/interner.rs:62` | ? | res resolves dictionary strings for opaque descriptor computations and diagnostics; generic callers have no single boundary. |
| `features/datalog/doop/shared/src/interner.rs:76` | P | global_identity_and_cat exposes the resolved symbol only to its diagnostic string assertion. |
| `features/datalog/doop/shared/src/interner.rs:78` | R | this diagnostic format! reads resolved text to compute a new descriptor, not to reorganize unchanged codes. |
| `features/datalog/doop/shared/src/interner.rs:86` | P | unknown_code_does_not_poison_interner publishes resolved text to its recovery assertion. |
| `features/datalog/doop/shared/src/loader.rs:95` | R | method_descriptors' authored format! below needs return-type text to compute a new descriptor. |
| `features/datalog/doop/shared/src/loader.rs:98` | R | method_descriptors reads parameter text in opaque format! before interning the computed descriptor. |
| `features/datalog/doop/shared/src/loader.rs:131` | P | exact_columns_and_empty_strings exposes the parsed name for a diagnostic answer assertion. |
| `features/datalog/doop/shared/src/loader.rs:133` | P | exact_columns_and_empty_strings exposes the empty symbol for a diagnostic answer assertion. |
| `runtime/cq-load/examples/zone_tpch.rs:13` | V | this benchmark decoder materializes lineitem before membership selection; P5 requires encoded scan columns and late materialization. |
| `runtime/cq-load/examples/zone_tpch.rs:16` | V | order keys are decoded before membership tests; P5 evaluates preserved predicates in the scan representation. |
| `runtime/cq-load/examples/zone_tpch.rs:19` | V | part keys are materialized before row selection despite no query read; P5 must retain them encoded. |
| `runtime/cq-load/examples/zone_tpch.rs:21` | V | supplier keys are materialized before membership selection; P5 materializes only demanded survivor columns. |
| `runtime/cq-load/examples/zone_tpch.rs:44` | V | the benchmark loads all logical tuples before zone pruning; P5 must replace eager row loading with columnar scan storage. |
| `runtime/cq-load/src/csv_source.rs:135` | V | Loader::csv_hash/csv_btree group these decoded rows; P4 keeps index keys encoded and P5 builds scan columns directly. |
| `runtime/cq-load/src/csv_source.rs:146` | ? | read_csv collects the logical batch decoder for arbitrary callers without a recorded final-use boundary. |
| `runtime/cq-load/src/csv_source.rs:148` | ? | try_fold drives Csv::next reconstruction; caller-specific relational and diagnostic uses are classified at their sites. |
| `runtime/cq-load/src/lib.rs:125` | ? | Load is a generic source-to-logical conversion contract with no consumer boundary evidence. |
| `runtime/cq-load/src/lib.rs:129` | ? | default Arrow loading delegates logical reconstruction to a caller-defined Load implementation. |
| `runtime/cq-load/src/lib.rs:139` | ? | Relation requires owned logical rows but does not record whether callers publish or continue querying. |
| `runtime/cq-load/src/lib.rs:171` | ? | Batch::column materializes logical values for arbitrary Relation decoders; consumer facts are absent. |
| `runtime/cq-load/src/lib.rs:187` | ? | CSV field parsing invokes arbitrary T::load without a recorded downstream read boundary. |
| `runtime/cq-load/src/lib.rs:201` | ? | Arrow column reconstruction invokes arbitrary T::load_arrow before its consumer is known. |
| `runtime/cq-load/src/loader.rs:31` | ? | read_csv reconstructs the entire host relation; this generic convenience API records no final consumer. |
| `runtime/cq-load/src/loader.rs:44` | V | CSV batch decoding precedes hash grouping; P4 builds indexes over retained representations and P5 loads columnar scans directly. |
| `runtime/cq-load/src/loader.rs:57` | V | CSV batch decoding precedes ordered grouping; P4 retains encoded index keys and P5 avoids eager logical scan tuples. |
| `runtime/cq-load/src/loader.rs:73` | ? | read_parquet reconstructs a tuple-vector host but this generic loader records no publication or Rust-read boundary. |
| `runtime/cq-load/src/parquet_source.rs:117` | V | zone_tpch feeds these owned decoded tuples to membership scans; P5 must retain columnar payloads and materialize late. |
| `runtime/cq-load/src/parquet_source.rs:129` | ? | read_parquet collects logical rows for arbitrary hosts without a caller publication or opaque-read fact. |
| `runtime/cq-load/src/parquet_source.rs:134` | ? | this fold drives Parquet::next reconstruction before the generic caller's downstream use is known. |
| `runtime/cq-load/src/parquet_source.rs:150` | ? | string_values invokes caller parsing over Arrow strings without a recorded logical consumer. |
| `runtime/cq-load/src/parquet_source.rs:163` | ? | this Arrow string entry is converted by an arbitrary parser; publication or Rust-read evidence is absent. |
| `runtime/cq-load/src/parquet_source.rs:177` | ? | nonstring Arrow fallback delegates reconstruction with the same unknown consumer as string_values. |
| `runtime/cq-load/src/parquet_source.rs:181` | ? | parse_arrow is a generic logical reconstruction adapter, not an established publication boundary. |
| `runtime/cq-load/src/parquet_source.rs:186` | ? | parse_arrow forwards to string_values without adding caller-use evidence. |
| `runtime/cq-load/src/parquet_source.rs:190` | ? | parse_arrow_nonstring reconstructs logical values for arbitrary Load implementations. |
| `runtime/cq-load/src/parquet_source.rs:200` | ? | primitive Arrow text is parsed into caller T before its runtime use is known. |
| `runtime/cq-load/src/parquet_source.rs:224` | ? | Arrow epoch days become Date for fallback rendering, with no opaque-read or answer fact. |
| `runtime/cq-load/src/parquet_source.rs:227` | ? | the rendered Date is parsed into arbitrary T whose subsequent consumer is unrecorded. |
| `runtime/cq-load/src/parquet_source.rs:246` | ? | Decimal128 coefficient and scale become logical decimal text for generic fallback loading. |
| `runtime/cq-load/src/parquet_source.rs:248` | ? | decimal text is reconstructed as caller T with no recorded final consumer. |
| `runtime/cq-load/src/parquet_source.rs:265` | ? | decimal_text reconstructs scaled decimal text for both fallback loading and error diagnostics. |
| `runtime/cq-load/src/scalar.rs:8` | ? | primitive text parsing constructs host values for generic Load callers with unknown consumers. |
| `runtime/cq-load/src/scalar.rs:14` | ? | primitive Arrow loading is a generic boundary; identity arrays already contain logical values. |
| `runtime/cq-load/src/scalar.rs:19` | ? | primitive Arrow fallback parses logical values without downstream caller evidence. |
| `runtime/cq-load/src/scalar.rs:37` | ? | owned-string loading materializes source text for arbitrary Load consumers. |
| `runtime/cq-load/src/scalar.rs:43` | ? | owned-string Arrow loading reconstructs values through string_values for unrecorded consumers. |
| `runtime/cq-load/src/scalar.rs:61` | ? | Decimal::from_scaled constructs a logical fixed-point value; this generic constructor has mixed callers. |
| `runtime/cq-load/src/scalar.rs:74` | ? | Decimal::rescale reconstructs a host Decimal from Arrow coefficient/scale without consumer facts. |
| `runtime/cq-load/src/scalar.rs:94` | ? | rescale wraps the adjusted coefficient as Decimal for its generic Arrow loader caller. |
| `runtime/cq-load/src/scalar.rs:100` | ? | Decimal::load parses a logical decimal for generic source, test, and user callers. |
| `runtime/cq-load/src/scalar.rs:142` | ? | decimal text parsing constructs Decimal from its coefficient before downstream use is known. |
| `runtime/cq-load/src/scalar.rs:147` | ? | Decimal Arrow loading reconstructs a whole logical column without a recorded read boundary. |
| `runtime/cq-load/src/scalar.rs:161` | ? | each Arrow decimal coefficient is rescaled into a host value for an unknown consumer. |
| `runtime/cq-load/src/scalar.rs:164` | P | failed decimal rescaling exposes the source value as diagnostic error text only. |
| `runtime/cq-load/src/scalar.rs:171` | ? | non-Decimal128 Arrow fallback reconstructs Decimal through generic text parsing. |
| `runtime/cq-load/src/scalar.rs:180` | ? | Decimal FromStr forwards logical parsing for callers whose runtime use is not recorded. |
| `runtime/cq-load/src/scalar.rs:215` | ? | Date::from_days constructs a logical date from epoch days with no caller-use contract. |
| `runtime/cq-load/src/scalar.rs:226` | ? | Date::load reconstructs a logical Gregorian date for generic Load consumers. |
| `runtime/cq-load/src/scalar.rs:260` | ? | parsed Gregorian components become Date before its downstream purpose is known. |
| `runtime/cq-load/src/scalar.rs:265` | ? | Date Arrow loading reconstructs dates for arbitrary batch consumers. |
| `runtime/cq-load/src/scalar.rs:268` | ? | Date32 epoch-day values become Date wrappers without opaque-read or publication facts. |
| `runtime/cq-load/src/scalar.rs:271` | ? | Date Arrow fallback reconstructs logical dates via the generic parser. |
| `runtime/cq-load/src/scalar.rs:280` | ? | Date FromStr delegates reconstruction with no evidence about subsequent operations. |
| `runtime/cq-load/src/zone.rs:214` | V | consuming Csv/Parquet batches drives eager tuple decoding before summary scans; P5 must build encoded columns and materialize survivors. |
| `runtime/cq-load/tests/loading.rs:35` | P | Typed reconstructs rows solely for CSV/Parquet loading diagnostics and answer assertions. |
| `runtime/cq-load/tests/loading.rs:37` | P | the Typed diagnostic exposes decoded ids to its expected-row oracle. |
| `runtime/cq-load/tests/loading.rs:39` | P | the Typed diagnostic exposes decoded names to its expected-row oracle. |
| `runtime/cq-load/tests/loading.rs:41` | P | the Typed diagnostic exposes decoded dates to its expected-row oracle. |
| `runtime/cq-load/tests/loading.rs:43` | P | the Typed diagnostic exposes decoded prices to its expected-row oracle. |
| `runtime/cq-load/tests/loading.rs:45` | P | the Typed diagnostic exposes decoded booleans to its expected-row oracle. |
| `runtime/cq-load/tests/loading.rs:62` | P | Ids reconstructs singleton rows for batch-size and loader-error diagnostics only. |
| `runtime/cq-load/tests/loading.rs:65` | P | the Ids fixture exposes each decoded integer to its diagnostic row assertions. |
| `runtime/cq-load/tests/loading.rs:77` | P | the integer-range diagnostic publishes the reconstructed minimum to its assertion. |
| `runtime/cq-load/tests/loading.rs:79` | P | the integer-range diagnostic publishes the reconstructed maximum to its assertion. |
| `runtime/cq-load/tests/loading.rs:81` | P | the integer-overflow diagnostic exposes only the failed reconstruction result. |
| `runtime/cq-load/tests/loading.rs:83` | P | the integer-whitespace diagnostic exposes only the parsing rejection. |
| `runtime/cq-load/tests/loading.rs:88` | P | the true-token diagnostic publishes the reconstructed boolean to assert!. |
| `runtime/cq-load/tests/loading.rs:90` | P | the false-token diagnostic publishes the reconstructed boolean to assert!. |
| `runtime/cq-load/tests/loading.rs:92` | P | the uppercase-boolean diagnostic exposes the rejected reconstruction result. |
| `runtime/cq-load/tests/loading.rs:94` | P | the NaN diagnostic publishes the reconstructed float to its classification assertion. |
| `runtime/cq-load/tests/loading.rs:96` | P | the negative-infinity diagnostic publishes the reconstructed float to assert_eq!. |
| `runtime/cq-load/tests/loading.rs:98` | P | the fractional-float diagnostic publishes the reconstructed value to assert_eq!. |
| `runtime/cq-load/tests/loading.rs:100` | P | the empty-string diagnostic publishes the reconstructed String to assert_eq!. |
| `runtime/cq-load/tests/loading.rs:102` | P | the boxed-string diagnostic exposes reconstructed text to assert_eq!. |
| `runtime/cq-load/tests/loading.rs:104` | P | the Arc-string diagnostic exposes reconstructed text to assert_eq!. |
| `runtime/cq-load/tests/loading.rs:106` | P | the Rc-string diagnostic exposes reconstructed text to assert_eq!. |
| `runtime/cq-load/tests/loading.rs:111` | P | this test-only Id loader reconstructs values for extension diagnostics. |
| `runtime/cq-load/tests/loading.rs:116` | P | the Id diagnostic reconstructs its stripped numeric payload for the expected-value oracle. |
| `runtime/cq-load/tests/loading.rs:121` | P | the prefixed-Id diagnostic publishes the reconstructed extension value. |
| `runtime/cq-load/tests/loading.rs:123` | P | the missing-prefix diagnostic exposes the rejected Id reconstruction. |
| `runtime/cq-load/tests/loading.rs:129` | P | the cents diagnostic exposes the decoded decimal coefficient for an exactness assertion. |
| `runtime/cq-load/tests/loading.rs:132` | P | the trailing-zero diagnostic exposes a reconstructed decimal through display comparison. |
| `runtime/cq-load/tests/loading.rs:136` | P | the negative-cent diagnostic exposes the reconstructed decimal coefficient. |
| `runtime/cq-load/tests/loading.rs:138` | P | the scale-zero diagnostic exposes the reconstructed decimal's display text. |
| `runtime/cq-load/tests/loading.rs:141` | P | the scale-eighteen diagnostic exposes its reconstructed least-significant coefficient. |
| `runtime/cq-load/tests/loading.rs:151` | P | decimal-boundary diagnostics reconstruct extrema only for exact coefficient and display assertions. |
| `runtime/cq-load/tests/loading.rs:168` | P | malformed-decimal diagnostics expose reconstruction failures without relational processing. |
| `runtime/cq-load/tests/loading.rs:171` | P | the unsupported-text-scale diagnostic exposes the rejected decimal parse. |
| `runtime/cq-load/tests/loading.rs:173` | P | the unsupported-coefficient-scale diagnostic exposes the rejected Decimal construction. |
| `runtime/cq-load/tests/loading.rs:179` | P | the epoch-date diagnostic publishes the reconstructed date's zero day count. |
| `runtime/cq-load/tests/loading.rs:181` | P | the pre-epoch diagnostic publishes the reconstructed negative day count. |
| `runtime/cq-load/tests/loading.rs:183` | P | the leap-day diagnostic publishes the reconstructed epoch offset. |
| `runtime/cq-load/tests/loading.rs:194` | P | Gregorian-date diagnostics expose reconstructed dates only through round-trip display assertions. |
| `runtime/cq-load/tests/loading.rs:209` | P | invalid-calendar diagnostics expose rejected date reconstruction results. |
| `runtime/cq-load/tests/loading.rs:214` | P | the epoch sweep constructs Date from days solely for its diagnostic text round trip. |
| `runtime/cq-load/tests/loading.rs:215` | P | the epoch sweep parses the rendered date solely for its diagnostic day-count assertion. |
| `runtime/cq-load/tests/loading.rs:229` | P | the reordered-CSV diagnostic publishes reconstructed rows solely to its expected-answer assertions. |
| `runtime/cq-load/tests/loading.rs:246` | P | the CSV answer oracle reconstructs its expected date literal for comparison. |
| `runtime/cq-load/tests/loading.rs:248` | P | the CSV answer oracle reconstructs its expected Decimal literal for comparison. |
| `runtime/cq-load/tests/loading.rs:262` | P | consuming the CSV iterator reconstructs rows solely for batch-boundary and sequence diagnostics. |
| `runtime/cq-load/tests/loading.rs:289` | P | the empty-source diagnostic invokes reconstruction only to expose an empty answer. |
| `runtime/cq-load/tests/loading.rs:310` | P | the quoted-newline diagnostic publishes the first reconstructed batch's string to assert_eq!. |
| `runtime/cq-load/tests/loading.rs:312` | P | the invalid-date batch diagnostic exposes the second reconstruction's contextual error. |
| `runtime/cq-load/tests/loading.rs:345` | P | the row-width diagnostic invokes CSV loading only to expose its rejection. |
| `runtime/cq-load/tests/loading.rs:357` | P | the missing-field diagnostic invokes CSV loading only to expose its source-column error. |
| `runtime/cq-load/tests/loading.rs:374` | P | the reordered-missing-field diagnostic exposes only the failed row-loading result. |
| `runtime/cq-load/tests/loading.rs:456` | P | consuming native Parquet batches exposes reconstructed rows solely to the CSV-equivalence diagnostic. |
| `runtime/cq-load/tests/loading.rs:462` | P | the CSV side of the Parquet-equivalence diagnostic publishes expected logical rows. |
| `runtime/cq-load/tests/loading.rs:475` | P | the nullable-Parquet diagnostic publishes the valid first batch to its row assertion. |
| `runtime/cq-load/tests/loading.rs:477` | P | the nullable-Parquet diagnostic exposes the failing second batch's null error. |
| `runtime/cq-load/tests/loading.rs:492` | P | Prices reconstructs diagnostic rows solely to test contextual decimal errors. |
| `runtime/cq-load/tests/loading.rs:495` | P | the Prices diagnostic exposes Decimal reconstruction failures to its error oracle. |
| `runtime/cq-load/tests/loading.rs:514` | P | the precision/overflow diagnostic invokes Parquet reconstruction only to publish contextual errors. |
| `runtime/cq-load/tests/loading.rs:527` | P | the test-only Label loader constructs host values for extension-adapter diagnostics. |
| `runtime/cq-load/tests/loading.rs:533` | P | the integer-to-Label diagnostic exposes reconstructed labels to its expected answer array. |
| `runtime/cq-load/tests/loading.rs:538` | P | the string-to-Label diagnostic exposes reconstructed labels to its expected answer array. |
| `runtime/cq-load/tests/loading.rs:542` | P | the narrow-integer Arrow diagnostic exposes the failed reconstruction's source value. |
| `runtime/cq-load/tests/loading.rs:557` | P | this deliberately invalid test loader constructs a value only for loader-contract diagnostics. |
| `runtime/cq-load/tests/loading.rs:561` | P | this deliberately empty test decoder exposes a length-contract violation, not query data. |
| `runtime/cq-load/tests/loading.rs:572` | P | InvalidSchema invokes reconstruction only to expose a diagnostic column-length error. |
| `runtime/cq-load/tests/loading.rs:575` | P | the InvalidSchema column conversion exposes the malformed decoder result to its error oracle. |
| `runtime/cq-load/tests/loading.rs:586` | P | the invalid-length diagnostic invokes Parquet reconstruction solely to expose its contract error. |
| `runtime/cq-load/tests/zone_parquet.rs:15` | V | the zone diagnostic builds logical tuples before relational selections; P5 must build summaries with encoded scan columns. |
| `runtime/cq-load/tests/zone_parquet.rs:18` | V | the zone diagnostic reconstructs id before Point selection; P5 keeps preserved predicates in the encoded scan domain. |
| `runtime/cq-load/tests/zone_parquet.rs:21` | V | the zone diagnostic reconstructs dates before building bounds; P5 builds payload and summaries together. |
| `runtime/cq-load/tests/zone_parquet.rs:23` | V | the zone diagnostic reconstructs Decimal before building bounds; P5 requires columnar payload plus summaries. |
| `runtime/cq-load/tests/zone_parquet.rs:25` | V | the zone diagnostic owns strings before subsequent selection; P5 delays reconstruction to demanded survivors. |
| `runtime/cq-load/tests/zone_parquet.rs:89` | V | this test consumes decoding batches before zone predicates; P5 retains encoded columns through selection. |
| `runtime/cq-load/tests/zone_parquet.rs:91` | V | the expected zone host also eagerly loads tuples before relational reads; P5 builds scan payload and bounds together. |
| `runtime/cq-storage/src/sequence.rs:46` | ? | SequenceCodec exposes a generic logical boundary without caller Rust-read or publication facts. |
| `runtime/cq-storage/src/sequence.rs:58` | ? | Identity implements the generic decode boundary as a no-op; caller purpose is not recorded. |
| `runtime/cq-storage/src/sequence.rs:90` | ? | Dense resolves a dictionary ordinal to T; mixed generic callers supply no boundary evidence. |
| `runtime/cq-storage/src/sequence.rs:111` | P | check_codec exposes the reconstructed value only to its round-trip diagnostic equality oracle. |
| `runtime/cq-storage/src/sequence.rs:177` | P | this diagnostic invokes check_codec to expose every dense sample for round-trip assertions. |
| `runtime/cq-storage/src/sequence.rs:181` | P | this identity-codec diagnostic exposes sample values through check_codec's equality oracle. |
