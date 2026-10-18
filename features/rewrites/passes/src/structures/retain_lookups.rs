use cq_lang_structures::{self as ir, Structure, physical::Operator, symbol_name};
use proc_macro2::TokenStream;

/// Record Copy range witnesses for eager semijoins with a later matching probe.
///
/// Candidate generation only selects SortedRun payload builds; the language's
/// structural contract independently proves key preservation and witness escape.
/// Existing representation choices are preserved, including positions carriers.
pub fn retain_lookups(source: &ir::Program, arguments: TokenStream) -> syn::Result<ir::Program> {
    if !arguments.is_empty() {
        return Err(syn::Error::new_spanned(
            arguments,
            "retain_lookups takes no arguments",
        ));
    }
    let facts = ir::contract::check(source)?;
    let definitions = &source.physical.program.definitions;
    let mut target = source.clone();
    for (index, first) in definitions.iter().enumerate() {
        let Operator::Semi(_, read) = &first.operator else {
            continue;
        };
        if !matches!(read.choice, Some(ir::physical::Choice::Eager(_))) {
            continue;
        }
        let name = symbol_name(&first.name);
        if facts.witnesses.contains_key(&name) {
            continue;
        }
        let build = symbol_name(&read.right);
        let mut keys: Vec<_> = read.columns.iter().map(symbol_name).collect();
        keys.sort();
        let eligible_build = source.structures.entries.iter().any(|entry| {
            symbol_name(&entry.name) == build
                && matches!(&entry.structure, Structure::Built { keyed, .. }
                    if keyed.membership.is_none()
                        && keyed.level_names().as_slice() == std::slice::from_ref(&keys)
                        && matches!(&keyed.ty, syn::Type::Path(path)
                            if path.qself.is_none()
                                && path.path.segments.last()
                                    .is_some_and(|segment| segment.ident == "SortedRun")))
        });
        if !eligible_build {
            continue;
        }
        for force in &definitions[index + 1..] {
            let Operator::Probe(_, probe) = &force.operator else {
                continue;
            };
            if symbol_name(&probe.right) != build
                || probe.columns.len() != keys.len()
                || !probe
                    .columns
                    .iter()
                    .all(|column| keys.contains(&symbol_name(column)))
                || facts
                    .shares
                    .get(&name)
                    .is_some_and(|share| share.force != symbol_name(&force.name))
            {
                continue;
            }
            let first_name = &first.name;
            let force_name = &force.name;
            let build_name = &read.right;
            let columns = &read.columns;
            let witness = syn::parse_quote! {
                witness { #build_name [#columns] at #first_name as ::cq_storage::RunCursor<'_, _> }
            };
            let original_len = target.structures.entries.len();
            if !facts.shares.contains_key(&name) {
                target.structures.entries.push(syn::parse_quote! {
                    #first_name share = lookup #build_name [#columns] first #first_name force #force_name;
                });
            }
            let repr_index = target.structures.entries.iter().position(|entry| {
                symbol_name(&entry.name) == name
                    && matches!(entry.structure, Structure::Repr { .. })
            });
            if let Some(repr_index) = repr_index {
                let Structure::Repr { witness: slot, .. } =
                    &mut target.structures.entries[repr_index].structure
                else {
                    unreachable!("selected repr entry")
                };
                *slot = Some(witness);
            } else {
                target.structures.entries.push(syn::parse_quote! {
                    #first_name repr = tuples #witness;
                });
            }
            // A failed candidate is ineligible, not a malformed input: the source
            // was checked above, and only this pair's two decisions were added.
            if ir::contract::check(&target).is_ok() {
                break;
            }
            target.structures.entries.truncate(original_len);
            if let Some(repr_index) = repr_index {
                let Structure::Repr { witness, .. } =
                    &mut target.structures.entries[repr_index].structure
                else {
                    unreachable!("selected repr entry")
                };
                *witness = None;
            }
        }
    }
    ir::check_rewrite(source, &target)?;
    Ok(target)
}
