//! Result representation is independent of the single-base positional read contract.
use super::*;

#[derive(Clone, Debug, PartialEq, Eq, syn_derive::Parse, syn_derive::ToTokens)]
pub enum Repr {
    #[parse(peek = kw::tuples)]
    Tuples(kw::tuples),
    #[parse(peek = kw::positions)]
    Positions {
        positions_token: kw::positions,
        #[syn(braced)]
        brace: token::Brace,
        #[syn(in = brace)]
        #[parse(CommaList::parse_terminated)]
        bases: CommaList<ReprBase>,
        owned_token: kw::owned,
        owned: Level,
    },
}

/// One carried host occurrence, using the same column mapping as `Origin`.
#[derive(Clone, Debug, PartialEq, Eq, syn_derive::Parse, syn_derive::ToTokens)]
pub struct ReprBase {
    pub base: Ident,
    pub lineage_token: kw::lineage,
    #[syn(braced)]
    pub brace: token::Brace,
    #[syn(in = brace)]
    #[parse(CommaList::parse_terminated)]
    pub columns: CommaList<ColumnLineage>,
}

/// Unlike a positional read, a representation can retain several occurrences
/// and carry computed values alongside their positions. Do not relax `lineages`:
/// selection/keyset/read still require its original single-base proof.
pub fn representation_bases(program: &Program, facts: &Analysis) -> BTreeMap<String, Vec<Lineage>> {
    let mut result: BTreeMap<String, Vec<Lineage>> = BTreeMap::new();
    let scoped: BTreeSet<_> = facts
        .physical
        .fused
        .values()
        .flat_map(|group| group.members.iter())
        .collect();
    for definition in &program.physical.program.definitions {
        let name = symbol_name(&definition.name);
        if scoped.contains(&name) {
            continue;
        }
        let of = |input: &Ident| result.get(&symbol_name(input)).cloned().unwrap_or_default();
        let mut origins = match &definition.operator {
            Operator::Rename(rename) => vec![Lineage {
                base: symbol_name(&rename.relation),
                occurrence: name.clone(),
                columns: rename
                    .mapping
                    .iter()
                    .map(|column| (symbol_name(&column.target), symbol_name(&column.source)))
                    .collect(),
            }],
            Operator::Probe(_, keyed) | Operator::Semi(_, keyed) | Operator::Merge(_, keyed) => {
                let mut origins = of(&keyed.left);
                for origin in of(&keyed.right) {
                    if !origins
                        .iter()
                        .any(|left| left.occurrence == origin.occurrence)
                    {
                        origins.push(origin);
                    }
                }
                origins
            }
            Operator::Select(select) | Operator::Point(_, select) | Operator::Range(_, select) => {
                of(&select.input)
            }
            Operator::Project(project) => of(&project.input),
            Operator::Bind(bind) => of(&bind.input),
            Operator::Guard(guard) => of(&guard.input),
            Operator::Anti(_, keyed) => of(&keyed.left),
            Operator::Unit(_) | Operator::Union(_) => Vec::new(),
        };
        for origin in &mut origins {
            origin.columns.retain(|column, _| {
                facts.physical.headings[&name].contains(column)
                    && !facts.physical.rows[&name].computed.contains(column)
            });
        }
        result.insert(name, origins);
    }
    result
}

pub(crate) fn check(program: &Program, facts: &Analysis) -> syn::Result<BTreeMap<String, Repr>> {
    let origins = representation_bases(program, facts);
    let mut result = BTreeMap::new();
    // STR-Repr: Each carried occurrence has truthful injective lineage; lineage
    // plus owned computed values covers the entire logical result heading.
    for entry in &program.structures.entries {
        let Structure::Repr { repr, .. } = &entry.structure else {
            continue;
        };
        let name = symbol_name(&entry.name);
        if result.insert(name.clone(), repr.clone()).is_some() {
            return error(entry, "duplicate repr decision");
        }
        let Repr::Positions { bases, owned, .. } = repr else {
            continue;
        };
        let heading = &facts.physical.headings[&name];
        let mut covered = BTreeSet::new();
        let mut occurrences = BTreeSet::new();
        for base in bases {
            let columns: BTreeMap<_, _> = base
                .columns
                .iter()
                .map(|column| (symbol_name(&column.logical), symbol_name(&column.column)))
                .collect();
            if columns.len() != base.columns.len()
                || columns.values().collect::<BTreeSet<_>>().len() != columns.len()
            {
                return error(
                    base,
                    "repr lineage must be a total injective map on its covered columns",
                );
            }
            let expected = origins.get(&name).into_iter().flatten().find(|origin| {
                origin.base == symbol_name(&base.base)
                    && !occurrences.contains(&origin.occurrence)
                    && columns
                        .iter()
                        .all(|(logical, host)| origin.columns.get(logical) == Some(host))
            });
            let Some(expected) = expected else {
                return error(
                    base,
                    "repr base and lineage must match a declared host occurrence on the result spine",
                );
            };
            occurrences.insert(expected.occurrence.clone());
            covered.extend(columns.into_keys());
        }
        let mut owned_columns = BTreeSet::new();
        for column in &owned.columns {
            let column_name = symbol_name(column);
            if !heading.contains(&column_name)
                || !facts.physical.rows[&name].computed.contains(&column_name)
            {
                return error(
                    column,
                    "repr owned columns must be computed columns of the result",
                );
            }
            if !owned_columns.insert(column_name.clone()) {
                return error(column, "duplicate repr owned column");
            }
            covered.insert(column_name);
        }
        if heading.iter().any(|column| !covered.contains(column)) {
            return error(entry, "repr leaves an uncovered heading column");
        }
    }
    Ok(result)
}
