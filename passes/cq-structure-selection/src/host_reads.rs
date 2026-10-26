use std::collections::BTreeSet;

use cq_lang_physical::{self as ir, Choice, Operator, symbol_name};
use proc_macro2::TokenStream;

use crate::arguments::Results;

/// Retain reusable host renames that serve integral keyed stream reads.
///
/// This records only a Physical mode decision. Key columns, predicates and
/// reader order are unchanged; selection subsequently chooses the providers.
/// Const-only point/range scans and lazy membership groups remain host reads.
pub fn build_host_reads(source: &ir::Program, arguments: TokenStream) -> syn::Result<ir::Program> {
    let facts = ir::contract::check(source)?;
    let selected = Results::parse(arguments)?;
    let mut target = source.clone();
    let mut matched = BTreeSet::new();
    for definition in &mut target.program.definitions {
        if !selected.includes(&definition.name)
            || !matches!(definition.operator, Operator::Rename(_))
        {
            continue;
        }
        let name = symbol_name(&definition.name);
        let reads = facts
            .keyed_reads
            .get(&name)
            .into_iter()
            .flatten()
            .filter(|(_, _, supply)| *supply == ir::syntax::SupplyKind::Stream)
            .collect::<Vec<_>>();
        if reads.is_empty()
            || !reads.iter().all(|(_, keys, _)| {
                keys.iter()
                    .all(|key| crate::arguments::sorted_key(&facts.rows[&name].types[key]))
            })
        {
            continue;
        }
        // A lazy keyed read has a distinct demand contract; retaining its host
        // eagerly is not a mode-only refinement.
        if source.program.definitions.iter().any(|reader| {
            matches!(&reader.operator, Operator::Semi(_, key) | Operator::Anti(_, key)
                if symbol_name(&key.right) == name && matches!(key.choice, Some(Choice::Lazy(_))))
        }) {
            continue;
        }
        definition.mode = ir::Mode::Let(Default::default());
        matched.insert(name);
    }
    selected.require_matches(&matched)?;
    ir::check_rewrite(source, &target)?;
    Ok(target)
}
