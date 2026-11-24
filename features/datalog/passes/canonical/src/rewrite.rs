use cq_datalog_support::ensure;
use cq_lang_datalog_storage as storage;
use cq_syntax::symbol_name;
use storage::{incremental, provider};
use syn::ext::IdentExt;

/// The index kind whose full layouts are complete rows in column order.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Ordered;

impl syn::parse::Parse for Ordered {
    fn parse(input: syn::parse::ParseStream<'_>) -> syn::Result<Self> {
        let name = input.call(syn::Ident::parse_any)?;
        if name != "ordered" || !input.is_empty() {
            return Err(syn::Error::new_spanned(
                name,
                "expected exactly the index kind `ordered`",
            ));
        }
        Ok(Self)
    }
}

/// Whether an occurrence reads full layout `index`.
fn read(program: &storage::Program, index: usize) -> bool {
    program
        .reads
        .iter()
        .any(|read| read.kind == "full" && read.index == index)
}

/// The canonical layouts this rewrite may re-order: keyed by every column,
/// read by no occurrence, with a token that respells from its schema, and no
/// read layout of the relation already in the canonical order.
fn selectable(program: &storage::Program, relation: usize) -> syn::Result<bool> {
    let declaration = &program.incremental.strata.source.declarations[relation];
    let canonical = program.providers[relation].canonical;
    let layout = &program.layouts[canonical];
    let token = provider::lookup(&program.tokens, &layout.provider.token)?;
    Ok(layout.key == declaration.arity()
        && declaration.arity() > 1
        && token.spelling != provider::Spelling::Verbatim
        && !read(program, canonical)
        && !program.layouts.iter().enumerate().any(|(index, other)| {
            index != canonical
                && other.relation == layout.relation
                && other.columns == layout.columns
                && read(program, index)
        }))
}

/// Formal rule DL-CanonicalOrder: the read layout whose order the canonical
/// layout takes. A layout read as a full version by a rule of the relation's
/// own component ranks first; ties take the earliest layout.
fn choose(
    program: &storage::Program,
    analysis: &incremental::Analysis,
    relation: usize,
) -> Option<usize> {
    let name = &program.incremental.strata.source.declarations[relation].name;
    let own = analysis.component[relation];
    program
        .layouts
        .iter()
        .enumerate()
        .filter(|(_, layout)| layout.relation == *name)
        .filter_map(|(index, _)| {
            let rank = program
                .reads
                .iter()
                .filter(|read| read.kind == "full" && read.index == index)
                .map(|read| {
                    let rule = program.incremental.variants[read.variant].rule;
                    let head = &program.incremental.strata.source.rules[rule].head.relation;
                    let component = analysis
                        .positions
                        .get(&symbol_name(head))
                        .and_then(|&head| analysis.component[head]);
                    usize::from(own.is_none() || component != own)
                })
                .min()?;
            Some((rank, index))
        })
        .min()
        .map(|(_, index)| index)
}

fn reorder(
    program: &storage::Program,
    relation: usize,
    columns: &[usize],
) -> syn::Result<storage::Layout> {
    let declaration = &program.incremental.strata.source.declarations[relation];
    let mut layout = program.layouts[program.providers[relation].canonical].clone();
    let (key, payload) = storage::layout_schema(declaration, columns, layout.key);
    let token = provider::lookup(&program.tokens, &layout.provider.token)?;
    layout.provider.ty = provider::spell(token, &key, &payload, layout.hasher)?;
    layout.columns = columns.to_vec();
    Ok(layout)
}

pub(crate) fn storage(source: &storage::Program, _: Ordered) -> syn::Result<storage::Program> {
    let analysis = incremental::contract::check(&source.incremental)?;
    let mut target = source.clone();
    for relation in 0..source.providers.len() {
        if !selectable(source, relation)? {
            continue;
        }
        if let Some(index) = choose(source, &analysis, relation) {
            let columns = source.layouts[index].columns.clone();
            target.layouts[source.providers[relation].canonical] =
                reorder(source, relation, &columns)?;
        }
    }
    check_storage(source, &target)?;
    Ok(target)
}

/// Independent relation between the source and the selected program: only
/// canonical layouts differ, each changed one was selectable, now has the
/// column order of a read layout of its relation, and is otherwise the source
/// layout respelled over that order.
pub(crate) fn check_storage(
    source: &storage::Program,
    target: &storage::Program,
) -> syn::Result<()> {
    storage::check_rewrite(source, target)?;
    ensure(
        source.layouts.len() == target.layouts.len(),
        target,
        "canonical order selection changed the layouts",
    )?;
    let mut unchanged = target.clone();
    for (relation, providers) in source.providers.iter().enumerate() {
        let (before, after) = (
            &source.layouts[providers.canonical],
            &target.layouts[providers.canonical],
        );
        if before == after {
            continue;
        }
        ensure(
            selectable(source, relation)?
                && source.layouts.iter().enumerate().any(|(index, layout)| {
                    layout.relation == before.relation
                        && layout.columns == after.columns
                        && read(source, index)
                })
                && *after == reorder(source, relation, &after.columns)?,
            after,
            "a canonical layout may only take the column order of one of its relation's read layouts",
        )?;
        unchanged.layouts[providers.canonical] = before.clone();
    }
    ensure(
        source == &unchanged,
        target,
        "canonical order selection changed other storage decisions",
    )
}

#[cfg(test)]
mod tests;
