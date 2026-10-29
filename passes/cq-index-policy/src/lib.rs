//! RA prime → IX: name exact stream and const demands over result versions.
//!
//! This analysis preserves the relational plan and makes no representation,
//! physical algorithm, or continuation choice. [`check`] verifies
//! the ordered first-use demands and their rule-mandated names; fixtures
//! separately pin the expansion.
//! Opaque, total Rust expressions are preserved exactly and create no demands.
//! Only explicit relational constraints supply point, range or membership keys.

//! Demand derivation preserves union and occurrence paths and derives exact anti membership keys independently of the constructor.
//!
//! Core blocks denote `bottom + Set`: `{ B1 } | { B2 }` unions projected
//! answers; `{ B1 } else { B2 }` selects the first nonempty arm. F is the free
//! heading and I the common new interface; other binders remain branch-local.
//! Join distributes over union; fallback is `B1 union (B2 anti_F B1)`.
//! Union siblings are unordered; fallback retains its relational anti steps.
//! Only differing literals on one interface column prove union arms disjoint.
//!

use cq_lang_ix as ix;
use cq_lang_ra as ra;

/// Visit the fixed RA definitions in order, sharing identical descriptors.
/// Group is copied unchanged; grouping does not request a provider lookup.
pub fn compile(source: &ra::Program) -> syn::Result<ix::Program> {
    let info = ra::contract::check(source)?;
    let mut indexes = cq_lang_ra::syntax::SemicolonList::new();
    for demand in ix::demands(source, &info) {
        let right = demand.relation;
        let (_, columns, supply) = demand.signature;
        indexes.push(ix::IndexDefinition {
            name: syn::Ident::new(&format!("i{}", indexes.len()), right.span()),
            eq_token: Default::default(),
            requirement: ix::IndexRequirement {
                at_token: Default::default(),
                relation: right.clone(),
                bracket_token: Default::default(),
                columns: columns.into_iter().map(syn::Index::from).collect(),
                supply: supply.into(),
            },
        });
        indexes.push_punct(Default::default());
    }
    let target = ix::Program {
        ra: source.clone(),
        indexes_token: Default::default(),
        indexes_brace: Default::default(),
        indexes,
    };
    ix::contract::check(&target)?;
    Ok(target)
}

mod correspondence;
pub use correspondence::check;
