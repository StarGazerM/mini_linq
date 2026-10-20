use cq_lang_datalog_source as source;
use cq_lang_datalog_storage as storage;
use cq_lang_datalog_strata as strata;
use cq_syntax::symbol_name;

pub fn deduplicate_source(source: &source::Program) -> syn::Result<source::Program> {
    source::contract::check(source)?;
    let mut target = source.clone();
    target.rules.clear();
    for rule in &source.rules {
        if !target.rules.contains(rule) {
            target.rules.push(rule.clone());
        }
    }
    Ok(target)
}

pub fn merge_strata(source: &strata::Program) -> syn::Result<strata::Program> {
    let analysis = strata::contract::check(source)?;
    let mut target = source.clone();
    let mut index = 0;
    while index + 1 < target.components.len() {
        let independent = target.components[index].iter().all(|left| {
            target.components[index + 1].iter().all(|right| {
                let l = analysis.positions[&symbol_name(left)];
                let r = analysis.positions[&symbol_name(right)];
                !analysis.graph.reach[l][r] && !analysis.graph.reach[r][l]
            })
        });
        if independent {
            let right = target.components.remove(index + 1);
            target.components[index].extend(right);
        } else {
            index += 1;
        }
    }
    Ok(target)
}

pub fn alternate_storage(source: &storage::Program) -> syn::Result<storage::Program> {
    storage::contract::check(source)?;
    let mut target = source.clone();
    if let Some(read) = target.reads.iter_mut().find(|r| r.kind == "full") {
        let old = &source.layouts[read.index];
        let mut columns: Vec<_> = (0..old.columns.len()).rev().collect();
        if columns == old.columns && columns.len() > 1 {
            columns.rotate_left(1);
        }
        let declaration = source
            .incremental
            .strata
            .source
            .declarations
            .iter()
            .find(|declaration| declaration.name == old.relation)
            .expect("checked layout relation");
        let types = columns.iter().map(|&i| &declaration.columns[i].ty);
        let layout = storage::Layout {
            relation: old.relation.clone(),
            columns: columns.clone(),
            key: 0,
            provider: syn::parse_quote!(::std::collections::HashMap<(), ::std::vec::Vec<(#(#types,)*)>, ::cq_storage::HashState>),
            maintenance: old.maintenance.clone(),
        };
        // The fixed publication policy maintains every declared layout, even
        // one no longer referenced. A key-zero scan is legal at every binding.
        read.index = match target.layouts.iter().position(|l| l == &layout) {
            Some(index) => index,
            None => {
                let index = target.layouts.len();
                target.layouts.push(layout);
                index
            }
        };
        read.columns = columns;
    }
    Ok(target)
}

pub fn ordered_storage(source: &storage::Program) -> syn::Result<storage::Program> {
    let analysis = storage::contract::check(source)?;
    let mut target = source.clone();
    for layout in target.layouts.iter_mut().chain(&mut target.delta_layouts) {
        let declaration = &source.incremental.strata.source.declarations
            [analysis.positions[&symbol_name(&layout.relation)]];
        let types: Vec<_> = layout
            .columns
            .iter()
            .map(|&i| &declaration.columns[i].ty)
            .collect();
        let (keys, values) = types.split_at(layout.key);
        layout.provider = syn::parse_quote!(::std::collections::BTreeMap<(#(#keys,)*), ::std::vec::Vec<(#(#values,)*)>>);
    }
    Ok(target)
}

pub fn direct_later_delta(source: &storage::Program) -> syn::Result<storage::Program> {
    let analysis = storage::contract::check(source)?;
    let mut target = source.clone();
    for read in &mut target.reads {
        if read.kind == "delta" && read.position > 0 {
            let layout = &source.delta_layouts[read.index];
            read.kind = syn::parse_quote!(direct);
            read.index = analysis.positions[&symbol_name(&layout.relation)];
            read.columns =
                (0..source.incremental.strata.source.declarations[read.index].arity()).collect();
        }
    }
    Ok(target)
}

pub fn reverse_publication(
    source: &cq_lang_datalog_round::Program,
) -> syn::Result<cq_lang_datalog_round::Program> {
    cq_lang_datalog_round::contract::check(source)?;
    let mut target = source.clone();
    for component in &mut target.context.strata {
        component.publication.reverse();
    }
    Ok(target)
}

#[cfg(test)]
mod tests;
