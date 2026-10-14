//! Opt-in host capabilities are selection inputs, never type-dispatch guesses.
//! Unknown hosts keep the exact pre-existing entries.
use super::*;
use structures::{Origin, Structure};

/// Formal rule SEL-Positional: Record only statically eligible reads of explicitly positional hosts.
pub(super) fn choose(
    target: &mut structures::Program,
    bases: &BTreeSet<String>,
) -> syn::Result<()> {
    let facts = structures::contract::check(target)?;
    let lineage = &facts.lineage;
    let defs = &target.physical.program.definitions;
    let origin = |name: &str| -> syn::Result<Origin> {
        let l = &lineage[name];
        let input = target
            .physical
            .inputs()
            .find(|r| symbol_name(&r.name) == l.base)
            .expect("checked base");
        let base = &input.name;
        let columns = l.columns.iter().map(|(a, b)| {
            let a = &facts.physical.rows[name].bindings[a];
            let b = &input
                .columns
                .iter()
                .find(|column| symbol_name(&column.name) == *b)
                .expect("checked base column")
                .name;
            quote!(#a -> #b)
        });
        syn::parse2(quote!(base #base lineage { #(#columns),* }))
    };
    for base in bases {
        if !facts.host_layouts.contains_key(base) {
            return Err(syn::Error::new_spanned(
                &target.physical,
                format!("unknown positional host `{base}`"),
            ));
        }
    }
    let eligible = |name: &str| lineage.get(name).is_some_and(|l| bases.contains(&l.base));
    let mut extra = Vec::<structures::Entry>::new();
    let mut sets = BTreeSet::new();
    // Record a separate scalar set even when a payload index serves the same
    // key. Both consume the one recorded rows/selection source, never a second
    // relational computation. No set is finalized for a lazy semi.
    for (name, reads) in &facts.physical.key_reads {
        if !eligible(name)
            || facts.physical.modes[name] != ModeKind::Let
            || facts.filtered.contains_key(name)
        {
            continue;
        }
        for (reader, key) in reads {
            let d = defs
                .iter()
                .find(|d| symbol_name(&d.name) == *reader)
                .unwrap();
            if !matches!(&d.operator, Operator::Semi(_,k) if !matches!(k.choice, Some(physical::Choice::Lazy(_))))
                || key.len() != 1
                || structures::ungroup_type(&facts.physical.rows[name].types[&key[0]])
                    != &syn::parse_quote!(i64)
            {
                continue;
            }
            if !sets.insert((name.clone(), key.clone())) {
                continue;
            }
            let ident = defs
                .iter()
                .find(|d| symbol_name(&d.name) == *name)
                .unwrap()
                .name
                .clone();
            let column = &facts.physical.rows[name].bindings[&key[0]];
            let origin = origin(name)?;
            extra.push(syn::parse2(quote!(#ident keyset = ::cq_storage::AdaptiveKeySet<_, ::cq_storage::I64Codec, 67108864> [#column] owned #origin;))?);
        }
    }
    let mut reads = BTreeSet::new();
    for d in defs {
        let name = symbol_name(&d.name);
        if !eligible(&name) {
            continue;
        }
        let predicate;
        let column = if let Some(column) = structures::condition_column(&d.operator) {
            // Multi-point predicates remain unchanged until the predicate
            // provider can represent every literal without silently choosing one.
            if let Operator::Point(_, s) = &d.operator {
                if !structures::selection_constants(s).contains_key(&column) {
                    continue;
                }
            }
            predicate = quote!(physical);
            column
        } else if let Operator::Semi(_, k) = &d.operator {
            let right = symbol_name(&k.right);
            let key = k.columns.iter().map(symbol_name).collect::<Vec<_>>();
            if !sets.contains(&(right, key.clone())) {
                continue;
            }
            let right = &k.right;
            let key = &k.columns;
            predicate = quote!(membership #right [#key] summary exact);
            symbol_name(k.columns.first().unwrap())
        } else {
            continue;
        };
        // cq-load's default markers support these exact scalar/text schemas.
        let ty = structures::ungroup_type(&facts.physical.rows[&name].types[&column]);
        let type_name = quote!(#ty).to_string();
        if !matches!(
            type_name.as_str(),
            "i8" | "i16"
                | "i32"
                | "i64"
                | "i128"
                | "isize"
                | "u8"
                | "u16"
                | "u32"
                | "u64"
                | "u128"
                | "usize"
                | "bool"
                | "String"
                | "Date"
        ) {
            continue;
        }
        let l = &lineage[&name];
        let host_column = &l.columns[&column];
        let n = syn::Index::from(
            facts.host_layouts[&l.base]
                .iter()
                .position(|c| c == host_column)
                .unwrap(),
        );
        let input = target
            .physical
            .inputs()
            .find(|r| symbol_name(&r.name) == l.base)
            .expect("checked base");
        let host_column = &input
            .columns
            .iter()
            .find(|c| symbol_name(&c.name) == *host_column)
            .expect("checked base column")
            .name;
        let origin = origin(&name)?;
        let ident = &d.name;
        extra.push(syn::parse2(quote!(#ident read = positional #origin column #host_column marker ::cq_load::ColumnAt<#n> predicate #predicate;))?);
        reads.insert(name);
    }
    let mut positional_sources = BTreeSet::new();
    let mut selections = BTreeSet::new();
    let mut spines = BTreeSet::new();
    for d in defs {
        let name = symbol_name(&d.name);
        if !eligible(&name) {
            continue;
        }
        let input = match &d.operator {
            Operator::Point(_, s) | Operator::Range(_, s) => Some(symbol_name(&s.input)),
            Operator::Semi(_, k) => Some(symbol_name(&k.left)),
            _ => None,
        };
        if input
            .as_ref()
            .is_some_and(|i| !positional_sources.contains(i))
        {
            reads.remove(&name);
        }
        let spine = match &d.operator {
            Operator::Rename(_) => true,
            Operator::Project(p) => spines.contains(&symbol_name(&p.input)),
            Operator::Point(_, s) | Operator::Range(_, s) => {
                reads.contains(&name)
                    && s.condition.columns().len() == 1
                    && (!matches!(d.operator, Operator::Point(..))
                        || s.condition.equalities.len() + s.condition.constraints.len() == 1)
                    && spines.contains(&symbol_name(&s.input))
            }
            Operator::Semi(_, k) => reads.contains(&name) && spines.contains(&symbol_name(&k.left)),
            _ => false,
        };
        if !spine {
            continue;
        }
        if facts.physical.modes[&name] == ModeKind::Pipe {
            positional_sources.insert(name.clone());
            spines.insert(name);
            continue;
        }
        let several = facts
            .physical
            .keyed_reads
            .get(&name)
            .is_some_and(|r| r.len() > 1);
        if facts.physical.modes[&name] == ModeKind::Let
            && !facts.filtered.contains_key(&name)
            && (facts.retained.contains_key(&name) || several)
            && matches!(
                d.operator,
                Operator::Point(..) | Operator::Range(..) | Operator::Semi(..)
            )
        {
            let ident = &d.name;
            let origin = origin(&name)?;
            extra.push(syn::parse2(
                quote!(#ident selection = ::cq_storage::Selection<_> #origin summary per_block;),
            )?);
            positional_sources.insert(name.clone());
            selections.insert(name.clone());
            spines.insert(name);
        }
    }
    // A supplementary set needs reusable rows even when the old representation
    // was index-only. Selection can supply them; otherwise do not add that set.
    // A provisional mask can depend on a supplementary set whose source turns
    // out to be index-only. Prune the whole dependent chain to a fixed point
    // before removing any original representation; never repair this in print.
    loop {
        let previous = (sets.clone(), reads.clone(), selections.clone());
        sets.retain(|(name, _)| selections.contains(name) || facts.retained.contains_key(name));
        reads.retain(|name| {
            extra
                .iter()
                .find(|e| {
                    symbol_name(&e.name) == *name && matches!(e.structure, Structure::Read { .. })
                })
                .is_some_and(|e| match &e.structure {
                    Structure::Read {
                        predicate: structures::ReadPredicate::Membership { build, key, .. },
                        ..
                    } => sets.contains(&(symbol_name(build), key.level_names_for_key())),
                    _ => true,
                })
        });
        let available_reads = reads.clone();
        let available_selections = selections.clone();
        reads.retain(|name| {
            let d = defs.iter().find(|d| symbol_name(&d.name) == *name).unwrap();
            let input = match &d.operator {
                Operator::Point(_, s) | Operator::Range(_, s) => &s.input,
                Operator::Semi(_, k) => &k.left,
                _ => unreachable!(),
            };
            positional_source(
                &symbol_name(input),
                defs,
                &facts,
                &available_reads,
                &available_selections,
            )
        });
        selections.retain(|name| {
            let mut current = name.clone();
            loop {
                if current != *name && available_selections.contains(&current) {
                    return true;
                }
                if current != *name && facts.physical.modes[&current] != ModeKind::Pipe {
                    return false;
                }
                let d = defs
                    .iter()
                    .find(|d| symbol_name(&d.name) == current)
                    .unwrap();
                current = match &d.operator {
                    Operator::Rename(_) => return true,
                    Operator::Project(p) => symbol_name(&p.input),
                    Operator::Point(_, s) | Operator::Range(_, s) if reads.contains(&current) => {
                        symbol_name(&s.input)
                    }
                    Operator::Semi(_, k) if reads.contains(&current) => symbol_name(&k.left),
                    _ => return false,
                };
            }
        });
        if previous == (sets.clone(), reads.clone(), selections.clone()) {
            break;
        }
    }
    extra.retain(|entry| match &entry.structure {
        Structure::Selection { .. } => selections.contains(&symbol_name(&entry.name)),
        Structure::Read { .. } if !reads.contains(&symbol_name(&entry.name)) => false,
        Structure::KeySet { key, .. } => {
            sets.contains(&(symbol_name(&entry.name), key.level_names_for_key()))
        }
        Structure::Read {
            predicate: structures::ReadPredicate::Membership { build, key, .. },
            ..
        } => sets.contains(&(symbol_name(build), key.level_names_for_key())),
        _ => true,
    });
    target
        .structures
        .entries
        .retain(|entry| match &entry.structure {
            Structure::Host { reader, .. } if reads.contains(&symbol_name(reader)) => false,
            Structure::Rows { .. } => !selections.contains(&symbol_name(&entry.name)),
            Structure::Built { keyed, .. }
                if keyed.ty == syn::parse_quote!(::cq_storage::HashSet<_>)
                    && keyed.levels.len() == 1 =>
            {
                !sets.contains(&(symbol_name(&entry.name), keyed.level_names()[0].clone()))
            }
            _ => true,
        });
    target.structures.entries.extend(extra);
    choose_join_filters(target, bases)?;
    structures::contract::check(target)?;
    Ok(())
}
trait KeyNames {
    fn level_names_for_key(&self) -> Vec<String>;
}
impl KeyNames for structures::Level {
    fn level_names_for_key(&self) -> Vec<String> {
        let mut key: Vec<_> = self.columns.iter().map(symbol_name).collect();
        key.sort();
        key
    }
}

fn positional_source(
    name: &str,
    defs: &[physical::Definition],
    facts: &structures::Analysis,
    reads: &BTreeSet<String>,
    selections: &BTreeSet<String>,
) -> bool {
    let mut name = name.to_owned();
    loop {
        if selections.contains(&name) {
            return true;
        }
        if facts.physical.modes[&name] != ModeKind::Pipe {
            return false;
        }
        let d = defs.iter().find(|d| symbol_name(&d.name) == name).unwrap();
        match &d.operator {
            Operator::Rename(_) => return true,
            Operator::Project(p) => name = symbol_name(&p.input),
            _ => return reads.contains(&name),
        }
    }
}

/// Formal rule SEL-JoinFilter: Choose exact sets for selection-bounded builds and fixed Bloom filters for total filtered branches.
fn choose_join_filters(
    target: &mut structures::Program,
    bases: &BTreeSet<String>,
) -> syn::Result<()> {
    let facts = structures::contract::check(target)?;
    let mut entries = Vec::new();
    for d in &target.physical.program.definitions {
        let Some((scan, build, key, exact)) =
            structures::join_filter_candidate(target, &facts, &symbol_name(&d.name))
        else {
            continue;
        };
        let origin = &facts.lineage[&scan];
        if !bases.contains(&origin.base) {
            continue;
        }
        let scan = &target
            .physical
            .program
            .definitions
            .iter()
            .find(|d| symbol_name(&d.name) == scan)
            .unwrap()
            .name;
        let build = &target
            .physical
            .program
            .definitions
            .iter()
            .find(|d| symbol_name(&d.name) == build)
            .unwrap()
            .name;
        let index = syn::Index::from(
            facts.host_layouts[&origin.base]
                .iter()
                .position(|c| c == &origin.columns[&key])
                .unwrap(),
        );
        let key = &facts.physical.rows[&symbol_name(build)].bindings[&key];
        let covering = &d.name;
        let kind = if exact { quote!(exact) } else { quote!(bloom) };
        let ty = if exact {
            quote!(::cq_storage::AdaptiveKeySet<i64, ::cq_storage::I64Codec, 67108864>)
        } else {
            quote!(::cq_storage::BloomFilter<16384>)
        };
        entries.push(syn::parse2(quote!(#scan join_filter = #kind #ty build #build [#key] covering #covering marker ::cq_load::ColumnAt<#index>;))?);
    }
    target.structures.entries.extend(entries);
    Ok(())
}
