# Terminology and source correspondence

The specification follows Abiteboul, Hull and Vianu, *Foundations of Databases*
(AHV), and the course notes at revision
`3f971516f8784ead40e8635a85dbe319ba5e4729`. Page numbers below are the book's
printed pages.

## Textbook definitions

| Term or notation | Use in this specification | Source |
| --- | --- | --- |
| `dom` | Domain of constants; distinct from the attribute and variable name sets | [AHV §3.1, p. 30](https://webdam.di.ens.fr/Alice/pdfs/Chapter-3.pdf#page=3) |
| **Sort**, `sort(R)` | Finite attribute set of a relation schema; its size is the arity | [AHV §3.1, p. 31](https://webdam.di.ens.fr/Alice/pdfs/Chapter-3.pdf#page=4) |
| **Named tuple**, `t : U → dom`; `t[V]` | Attribute-indexed tuple and its restriction | [AHV §3.2, p. 32](https://webdam.di.ens.fr/Alice/pdfs/Chapter-3.pdf#page=5) |
| **Unnamed tuple** | Ordered tuple of constants | [AHV §3.2, p. 32](https://webdam.di.ens.fr/Alice/pdfs/Chapter-3.pdf#page=5) |
| **Valuation**, `ν : X → dom` | Total assignment on its stated variable set; fixes constants when extended to terms | [AHV §4.2, p. 41](https://webdam.di.ens.fr/Alice/pdfs/Chapter-4.pdf#page=5) |
| **CQ semantics**, `q(I)` | Head tuples obtained by satisfying valuations | [AHV Definition 4.2.1 and following semantics, p. 41](https://webdam.di.ens.fr/Alice/pdfs/Chapter-4.pdf#page=5) |
| **Natural join**, **projection**, **renaming** `δ` | Named relational operators; each has an explicit input/output sort | [AHV §4.4, pp. 57–59](https://webdam.di.ens.fr/Alice/pdfs/Chapter-4.pdf#page=21) |
| **SIP graph** | Relational-atom adjacency through shared variables, with constant marks | [AHV §6.1, p. 113](https://webdam.di.ens.fr/Alice/pdfs/Chapter-6.pdf#page=9) |
| **Semijoin**, **pairwise/global consistency**, **full reducer** | Relational reduction and its universal condition over input instances | [AHV §6.4, pp. 128–129](https://webdam.di.ens.fr/Alice/pdfs/Chapter-6.pdf#page=24) |
| **Join tree**, **hypergraph**, **ear**, **GYO** | Structural objects and reduction algorithm, with their domains stated separately | [AHV §6.4, pp. 129–132](https://webdam.di.ens.fr/Alice/pdfs/Chapter-6.pdf#page=25) |

## Explicit course adaptations

- [Note 1.3][note-1.3] labels
  **valuation relation**, `Val_I(C)`, as course terminology. Its members are
  valuations satisfying a sub-body, viewed as named tuples. The specification
  makes that view precise with a naming map from variables to attributes.
  A local valuation is total on its own variable set; it may fail to extend
  to a valuation satisfying the whole query.
- The same note derives **conjunction as natural join** and defines a
  **logical join plan** as a binary expression tree. These are course
  constructions using AHV's semantics and algebra, not separately named AHV
  definitions. The specification includes the conjunction proof and the
  explicit conversion from a named projection to ordered head tuples.
- [Note 2.2][note-2.2] fixes occurrence
  identities and full shared-variable labels for the **SIP graph**.
- [Note 2.3][note-2.3] explicitly keeps
  occurrence labels in its **query hypergraph**, including equal variable
  sets. AHV's hyperedges are distinct and nonempty. The specification states
  this adaptation and treats empty residual sets separately.
- [Note 2.4][note-2.4] supplies the
  retained-variable function `keep(C)` and distinguishes **logical arity**,
  **cardinality**, and **physical tuple width**. Its pending-use set is the
  union of variables in our pending predicate identities. Our `K(C,P)` is
  that retained-sort function.
- [Note 2.5][note-2.5] uses **α-acyclic**
  for the join-tree characterization and explains **complete support** using
  global consistency. The specification retains those connections explicitly.

## Compiler-specific notation and differences

- `heading` denotes a sort; elaboration's `finish` computes a `retained` sort
  for the output projection.
- `GreedyWidth` measures logical arity. Its name supplies no cardinality,
  physical-byte-width, or hypergraph-decomposition-width guarantee.
- The `G-Private`/`G-Absorb` system is a **variant** of AHV Algorithm 6.4.4.
  The manuscript proves that its absorptions correspond to ear removals on
  remaining original edges; its successful final root is the last isolated
  ear in the textbook presentation. Occurrence duplicates require the stated
  projection to the underlying textbook hypergraph.
- Dependency facts, valuation schedules, immutable RA prime definitions,
  bottom + finite Set denotation, Rust chain/Before constraints, core block
  interfaces, union/anti origins, and staged contexts are compiler specification
  constructs. They are not attributed to AHV as textbook definitions.
- A sort is a set of attribute names. Lexically sorting those names chooses
  a physical tuple layout; it does not sort the rows of a relation or index.
  Ordered-read traits specify a separate enumeration property. The implemented hash representations promise no such row order.
- The computed-attribute subset in RA typing records field origin for Rust
  representation; it is not an AHV definition.

These correspondences establish which definitions and adaptations are being
used. They do not discharge the implementation and host-language proof
obligations listed in [foundations.tex](foundations.tex).

[note-1.3]: https://github.com/StarGazerM/modern-query-processing-notes/blob/3f971516f8784ead40e8635a85dbe319ba5e4729/03-the-plan-that-meant-the-same-thing.md
[note-2.2]: https://github.com/StarGazerM/modern-query-processing-notes/blob/3f971516f8784ead40e8635a85dbe319ba5e4729/04-the-picture-that-did-not-choose.md
[note-2.3]: https://github.com/StarGazerM/modern-query-processing-notes/blob/3f971516f8784ead40e8635a85dbe319ba5e4729/04-the-variables-inside-the-picture.md
[note-2.4]: https://github.com/StarGazerM/modern-query-processing-notes/blob/3f971516f8784ead40e8635a85dbe319ba5e4729/05-the-variables-the-plan-still-needed.md
[note-2.5]: https://github.com/StarGazerM/modern-query-processing-notes/blob/3f971516f8784ead40e8635a85dbe319ba5e4729/06-the-order-the-compiler-chose.md
