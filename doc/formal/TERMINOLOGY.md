# Terminology and source correspondence

The specification follows Abiteboul, Hull and Vianu, *Foundations of Databases*
(AHV), and the course notes at revision
`3f971516f8784ead40e8635a85dbe319ba5e4729`. Page numbers below are the book's
printed pages. The notes checkout was refreshed with `git pull --ff-only` and
was already current; its source was not edited.

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

- [Note 1.3](../notes/03-the-plan-that-meant-the-same-thing.md) labels
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
- [Note 2.2](../notes/04-the-picture-that-did-not-choose.md) fixes occurrence
  identities and full shared-variable labels for the **SIP graph**.
- [Note 2.3](../notes/04-the-variables-inside-the-picture.md) explicitly keeps
  occurrence labels in its **query hypergraph**, including equal variable
  sets. AHV's hyperedges are distinct and nonempty. The specification states
  this adaptation and treats empty residual sets separately.
- [Note 2.4](../notes/05-the-variables-the-plan-still-needed.md) supplies the
  retained-variable function `keep(C)` and distinguishes **logical arity**,
  **cardinality**, and **physical tuple width**. Its pending-use set is the
  union of variables in our pending predicate identities. Our `K(C,P)` is
  that retained-sort function.
- [Note 2.5](../notes/06-the-order-the-compiler-chose.md) uses **α-acyclic**
  for the join-tree characterization and explains **complete support** using
  global consistency. The specification retains those connections explicitly.

## Compiler-specific notation and differences

- Source names such as `heading` and `retained_heading` remain implementation
  identifiers. Their mathematical objects are a sort and a retained sort.
  No implementation API was renamed during this terminology revision.
- `GreedyWidth` measures logical arity. Its name supplies no cardinality,
  physical-byte-width, or hypergraph-decomposition-width guarantee.
- The `G-Private`/`G-Absorb` system is a **variant** of AHV Algorithm 6.4.4.
  The manuscript proves that its absorptions correspond to ear removals on
  remaining original edges; its successful final root is the last isolated
  ear in the textbook presentation. Occurrence duplicates require the stated
  projection to the underlying textbook hypergraph.
- Administrative frontiers, dependency facts, immutable RA definitions,
  host-expression extensions and staged contexts are compiler specification
  constructs. They are not attributed to AHV as textbook definitions.
- A sort is a set of attribute names. Lexically sorting those names chooses
  a physical tuple layout; it does not sort the rows of a relation or index.
  The manuscript's sorted-storage comparison specifies a separate enumeration
  property. The implemented hash representations promise no such row order.
- The computed-attribute subset in RA typing records field origin for Rust
  representation. The dynamic provenance record in the extended manuscript
  is a separate semantic investigation, grounded in the cited provenance
  literature; it is not an existing analysis result or an AHV definition.

These correspondences establish which definitions and adaptations are being
used. They do not discharge the implementation and host-language proof
obligations listed in [foundations.tex](foundations.tex).
