//! Exact positional decisions. All lineage is rederived from Physical; entries
//! cannot move a predicate or cross an occurrence-expanding/scoped boundary.
use super::*;

#[derive(Clone, Debug, PartialEq, Eq, syn_derive::Parse, syn_derive::ToTokens)]
pub struct Origin {
    pub base_token: kw::base,
    pub base: Ident,
    pub lineage_token: kw::lineage,
    #[syn(braced)]
    pub brace: token::Brace,
    #[syn(in = brace)]
    #[parse(CommaList::parse_terminated)]
    pub columns: CommaList<ColumnLineage>,
}
#[derive(Clone, Debug, PartialEq, Eq, syn_derive::Parse, syn_derive::ToTokens)]
pub struct ColumnLineage {
    pub logical: Ident,
    pub arrow: Token![->],
    pub column: Ident,
}
#[derive(Clone, Debug, PartialEq, Eq, syn_derive::Parse, syn_derive::ToTokens)]
pub enum Summary {
    #[parse(peek = kw::none)]
    None(kw::none),
    #[parse(peek = kw::per_block)]
    PerBlock(kw::per_block),
}
#[derive(Clone, Debug, PartialEq, Eq, syn_derive::Parse, syn_derive::ToTokens)]
pub enum KeyOwnership {
    #[parse(peek = kw::owned)]
    Owned(kw::owned),
    #[parse(peek = kw::borrowed)]
    Borrowed(kw::borrowed),
}
#[derive(Clone, Debug, PartialEq, Eq, syn_derive::Parse, syn_derive::ToTokens)]
pub enum ReadPredicate {
    #[parse(peek = kw::physical)]
    Physical(kw::physical),
    #[parse(peek = kw::membership)]
    Membership {
        membership_token: kw::membership,
        build: Ident,
        key: Level,
        summary_token: kw::summary,
        summary: MembershipSummary,
    },
}
#[derive(Clone, Debug, PartialEq, Eq, syn_derive::Parse, syn_derive::ToTokens)]
pub enum MembershipSummary {
    #[parse(peek = kw::exact)]
    Exact(kw::exact),
    #[parse(peek = kw::row)]
    Row(kw::row),
}
#[derive(Clone, Debug)]
pub struct Selection {
    pub ty: syn::Type,
    pub origin: Origin,
    pub summary: Summary,
}
#[derive(Clone, Debug)]
pub struct KeySet {
    pub ty: syn::Type,
    pub key: Vec<String>,
    pub ownership: KeyOwnership,
    pub origin: Origin,
}
#[derive(Clone, Debug)]
pub struct Read {
    pub origin: Origin,
    pub column: String,
    pub marker: syn::Type,
    pub predicate: ReadPredicate,
}
/// A single host occurrence with a total, injective mapping over its heading.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Lineage {
    pub base: String,
    pub occurrence: String,
    pub columns: BTreeMap<String, String>,
}

/// Formal rule STR-Lineage: Derive positions only through an occurrence-preserving declarative spine.
pub fn lineages(
    program: &physical::Program,
    facts: &physical::Analysis,
) -> BTreeMap<String, Lineage> {
    let mut result: BTreeMap<String, Lineage> = BTreeMap::new();
    let scoped: BTreeSet<_> = facts
        .fused
        .values()
        .flat_map(|g| g.members.iter().cloned())
        .collect();
    for d in &program.program.definitions {
        let name = symbol_name(&d.name);
        if d.canon.is_some() || scoped.contains(&name) || !facts.rows[&name].computed.is_empty() {
            continue;
        }
        let origin = match &d.operator {
            Operator::Rename(r) if r.constants.is_none() => Some(Lineage {
                base: symbol_name(&r.relation),
                occurrence: name.clone(),
                columns: r
                    .mapping
                    .iter()
                    .map(|m| (symbol_name(&m.target), symbol_name(&m.source)))
                    .collect(),
            }),
            Operator::Select(s) | Operator::Point(_, s) | Operator::Range(_, s) => {
                result.get(&symbol_name(&s.input)).cloned()
            }
            Operator::Project(p) => result.get(&symbol_name(&p.input)).cloned().map(|mut l| {
                l.columns.retain(|c, _| facts.headings[&name].contains(c));
                l
            }),
            Operator::Semi(_, k) if !matches!(k.choice, Some(physical::Choice::Lazy(_))) => {
                result.get(&symbol_name(&k.left)).cloned()
            }
            _ => None,
        };
        if let Some(origin) = origin {
            if origin.columns.keys().cloned().collect::<Vec<_>>() == facts.headings[&name]
                && origin.columns.values().collect::<BTreeSet<_>>().len() == origin.columns.len()
            {
                result.insert(name, origin);
            }
        }
    }
    result
}

fn check_origin(
    origin: &Origin,
    name: &str,
    _program: &Program,
    facts: &Analysis,
) -> syn::Result<()> {
    let Some(expected) = facts.lineage.get(name) else {
        return error(
            origin,
            "no single-base occurrence-preserving lineage (canon, computed, bind, guard, join, union, anti or lazy boundary)",
        );
    };
    let columns: BTreeMap<_, _> = origin
        .columns
        .iter()
        .map(|c| (symbol_name(&c.logical), symbol_name(&c.column)))
        .collect();
    if symbol_name(&origin.base) != expected.base
        || columns != expected.columns
        || columns.len() != origin.columns.len()
    {
        return error(
            origin,
            "lineage must be total and injective over the reader heading and match its declared base occurrence",
        );
    }
    Ok(())
}

pub(crate) fn entry(program: &Program, entry: &Entry, facts: &mut Analysis) -> syn::Result<bool> {
    let name = symbol_name(&entry.name);
    match &entry.structure {
        Structure::JoinFilter { filter, .. } => {
            let filters = facts.join_filters.entry(name).or_default();
            if filters.iter().any(|f| {
                symbol_name(&f.covering) == symbol_name(&filter.covering)
                    && names(&f.key.columns) == names(&filter.key.columns)
            }) {
                return error(entry, "duplicate join filter decision");
            }
            filters.push(filter.clone());
        }
        Structure::Selection {
            ty,
            origin,
            summary,
            ..
        } => {
            concrete_type(ty)?;
            check_origin(origin, &name, program, facts)?;
            if facts.physical.modes[&name] != ModeKind::Let {
                return error(entry, "selection requires a let occurrence bag");
            }
            if facts
                .selections
                .insert(
                    name,
                    Selection {
                        ty: ty.clone(),
                        origin: origin.clone(),
                        summary: summary.clone(),
                    },
                )
                .is_some()
            {
                return error(entry, "duplicate selection decision");
            }
        }
        Structure::KeySet {
            ty,
            key,
            ownership,
            origin,
            ..
        } => {
            concrete_type(ty)?;
            admissible(ty, "membership")?;
            check_origin(origin, &name, program, facts)?;
            let columns = names(&key.columns);
            if facts.physical.modes[&name] != ModeKind::Let
                || columns.is_empty()
                || columns.windows(2).any(|p| p[0] == p[1])
                || columns
                    .iter()
                    .any(|c| !facts.physical.headings[&name].contains(c))
            {
                return error(entry, "keyset requires distinct columns of an earlier let");
            }
            for column in &key.columns {
                cq_syntax::algebra::check_leibniz_type(
                    &facts.physical.rows[&name].types[&symbol_name(column)],
                    column,
                )?;
            }
            if matches!(ownership, KeyOwnership::Owned(_))
                && (columns.len() != 1
                    || ungroup_type(&facts.physical.rows[&name].types[&columns[0]])
                        != &syn::parse_quote!(i64))
            {
                return error(
                    entry,
                    "owned keyset layout is one i64 scalar; other keys use borrowed tuples",
                );
            }
            if facts
                .keysets
                .insert(
                    (name, columns.clone()),
                    KeySet {
                        ty: ty.clone(),
                        key: columns,
                        ownership: ownership.clone(),
                        origin: origin.clone(),
                    },
                )
                .is_some()
            {
                return error(entry, "duplicate keyset decision");
            }
        }
        Structure::Read {
            origin,
            column,
            marker,
            predicate,
            ..
        } => {
            check_origin(origin, &name, program, facts)?;
            concrete_type(marker)?;
            if !origin.columns.iter().any(|c| c.column == *column) {
                return error(column, "read column is absent from lineage");
            }
            if let syn::Type::Path(path) = marker {
                if path
                    .path
                    .segments
                    .last()
                    .is_some_and(|s| s.ident == "ColumnAt")
                {
                    let base = symbol_name(&origin.base);
                    let n = syn::Index::from(
                        facts.host_layouts[&base]
                            .iter()
                            .position(|c| *c == symbol_name(column))
                            .expect("checked base column"),
                    );
                    let expected: syn::Type = syn::parse_quote!(::cq_load::ColumnAt<#n>);
                    if marker != &expected {
                        return error(
                            marker,
                            "marker must match the recorded declaration-order column",
                        );
                    }
                }
            }
            if facts
                .reads
                .insert(
                    name,
                    Read {
                        origin: origin.clone(),
                        column: symbol_name(column),
                        marker: marker.clone(),
                        predicate: predicate.clone(),
                    },
                )
                .is_some()
            {
                return error(entry, "duplicate positional read decision");
            }
        }
        _ => return Ok(false),
    }
    Ok(true)
}

/// First physical point/range column. Residual conjuncts remain authoritative.
pub fn condition_column(op: &Operator) -> Option<String> {
    match op {
        Operator::Point(_, s) => s
            .condition
            .point_columns()
            .and_then(|terms| terms.into_iter().next()),
        Operator::Range(_, s) => s.condition.range_bounds().map(|(c, _)| c),
        _ => None,
    }
}

pub(crate) fn check(program: &Program, facts: &Analysis) -> syn::Result<()> {
    let defs: BTreeMap<_, _> = program
        .physical
        .program
        .definitions
        .iter()
        .enumerate()
        .map(|(i, d)| (symbol_name(&d.name), (i, d)))
        .collect();
    for (name, s) in &facts.selections {
        if !matches!(
            defs[name].1.operator,
            Operator::Point(..) | Operator::Range(..) | Operator::Semi(..)
        ) {
            return error(
                &s.ty,
                "selection constructor requires a physical condition or semi reduction",
            );
        }
        if facts.retained.contains_key(name) || facts.filtered.contains_key(name) {
            return error(
                &s.ty,
                "selection cannot coexist with a tuple buffer or filtered projection",
            );
        }
        // Every filtering step down to a prior immutable selection must have an
        // explicit read. This also prevents re-evaluating an unrecorded let.
        let mut current = name.clone();
        loop {
            if current != *name && facts.selections.contains_key(&current) {
                break;
            }
            let d = defs[&current].1;
            if current != *name && facts.physical.modes[&current] != ModeKind::Pipe {
                return error(
                    &d.name,
                    "selection spine crosses a retained nonselection result",
                );
            }
            current = match &d.operator {
                Operator::Rename(_) => break,
                Operator::Project(p) => symbol_name(&p.input),
                Operator::Point(_, s) | Operator::Range(_, s)
                    if facts.reads.contains_key(&current)
                        && s.condition.columns().len() == 1
                        && (!matches!(d.operator, Operator::Point(..))
                            || s.condition.equalities.len() + s.condition.constraints.len()
                                == 1) =>
                {
                    symbol_name(&s.input)
                }
                Operator::Semi(_, k) if facts.reads.contains_key(&current) => symbol_name(&k.left),
                _ => {
                    return error(
                        &d.name,
                        "selection spine requires a recorded exact read at every filtering step",
                    );
                }
            };
        }
    }
    // Formal rule STR-Keyset: Key-only providers serve membership independently of payload indices.
    for ((name, key), set) in &facts.keysets {
        let demanded = facts
            .physical
            .key_reads
            .get(name)
            .into_iter()
            .flatten()
            .any(|(reader, columns)| {
                columns == key
                    && matches!(
                        defs[reader].1.operator,
                        Operator::Semi(..) | Operator::Anti(..)
                    )
            });
        if !demanded || !facts.selections.contains_key(name) && !facts.retained.contains_key(name) {
            return error(
                &set.ty,
                "keyset requires a membership demand and recorded reusable source rows",
            );
        }
        // Known providers cannot silently lose interval or key-layout contracts.
        if let syn::Type::Path(p) = &set.ty {
            if let Some(last) = p.path.segments.last() {
                if last.ident == "AdaptiveKeySet" {
                    let syn::PathArguments::AngleBracketed(args) = &last.arguments else {
                        return error(&set.ty, "keyset must record its codec and bit budget");
                    };
                    if args.args.len() != 3
                        || !matches!(args.args.last(), Some(syn::GenericArgument::Const(_)))
                    {
                        return error(&set.ty, "keyset must record its codec and const bit budget");
                    }
                    let expected: syn::Type = if matches!(set.ownership, KeyOwnership::Owned(_)) {
                        syn::parse_quote!(::cq_storage::I64Codec)
                    } else if key.len() == 1
                        && ungroup_type(&facts.physical.rows[name].types[&key[0]])
                            == &syn::parse_quote!(i64)
                    {
                        syn::parse_quote!(::cq_storage::BorrowedI64TupleCodec)
                    } else {
                        syn::parse_quote!(::cq_storage::NoDense)
                    };
                    if !matches!(&args.args[1], syn::GenericArgument::Type(t) if *t == expected || *t == syn::parse_quote!(::cq_storage::NoDense))
                    {
                        return error(&set.ty, "keyset codec disagrees with key ownership/layout");
                    }
                }
            }
        }
    }
    // Formal rule STR-Read: A summary names only the existing right build of this reader, earlier in Physical.
    for (name, read) in &facts.reads {
        let (position, d) = defs[name];
        if facts
            .fallbacks
            .keys()
            .any(|(_, reader, _)| reader.result() == Some(name))
        {
            return error(
                &d.name,
                "positional read replaces its native-reader fallback; record only one adapter",
            );
        }
        let mut input = match &d.operator {
            Operator::Point(_, s) | Operator::Range(_, s) => Some(symbol_name(&s.input)),
            Operator::Semi(_, k) => Some(symbol_name(&k.left)),
            _ => None,
        };
        while let Some(name) = input {
            let source = defs[&name].1;
            if facts.selections.contains_key(&name) {
                break;
            }
            if facts.physical.modes[&name] != ModeKind::Pipe {
                return error(
                    &d.name,
                    "positional read cannot bypass a retained nonselection source",
                );
            }
            input = match &source.operator {
                Operator::Rename(_) => None,
                Operator::Project(p) => Some(symbol_name(&p.input)),
                _ if facts.reads.contains_key(&name) => None,
                _ => {
                    return error(
                        &d.name,
                        "positional read requires a recorded positional source spine",
                    );
                }
            };
        }
        let logical = read
            .origin
            .columns
            .iter()
            .find(|c| symbol_name(&c.column) == read.column)
            .expect("checked column");
        match (&read.predicate, &d.operator) {
            (ReadPredicate::Physical(_), op @ (Operator::Point(..) | Operator::Range(..)))
                if condition_column(op) == Some(symbol_name(&logical.logical))
                    && match op {
                        Operator::Point(_, s) => {
                            selection_constants(s).contains_key(&symbol_name(&logical.logical))
                        }
                        _ => true,
                    } => {}
            (
                ReadPredicate::Membership {
                    build,
                    key,
                    summary,
                    ..
                },
                Operator::Semi(_, k),
            ) => {
                let right = symbol_name(build);
                let columns = names(&key.columns);
                if build != &k.right
                    || !defs.get(&right).is_some_and(|(p, _)| *p < position)
                    || columns != names(&k.columns)
                    || columns != [symbol_name(&logical.logical)]
                    || matches!(k.choice, Some(physical::Choice::Lazy(_)))
                {
                    return error(
                        &d.name,
                        "read summary must name this eager semi's earlier right build and exact key; nonlocal probe motion is forbidden",
                    );
                }
                if matches!(summary, MembershipSummary::Exact(_)) {
                    let Some(set) = facts.keysets.get(&(right, columns)) else {
                        return error(&d.name, "exact summary requires a recorded earlier keyset");
                    };
                    if !matches!(set.ownership, KeyOwnership::Owned(_)) {
                        return error(&set.ty, "scalar marker summary requires owned scalar keys");
                    }
                    admissible(&set.ty, "interval")?;
                } else if !facts
                    .keysets
                    .get(&(right, columns))
                    .is_some_and(|s| matches!(s.ownership, KeyOwnership::Owned(_)))
                {
                    return error(
                        &d.name,
                        "scalar row membership requires a recorded owned keyset",
                    );
                }
            }
            _ => {
                return error(
                    &d.name,
                    "positional read does not match the existing Physical predicate",
                );
            }
        }
    }
    check_join_filters(program, facts)?;
    Ok(())
}

/// Remove Rust macro type-fragment grouping without changing the named type.
/// A forwarded `$ty:ty` and the same directly written type have one key layout.
pub fn ungroup_type(ty: &syn::Type) -> &syn::Type {
    match ty {
        syn::Type::Group(group) => ungroup_type(&group.elem),
        syn::Type::Paren(paren) => ungroup_type(&paren.elem),
        _ => ty,
    }
}
