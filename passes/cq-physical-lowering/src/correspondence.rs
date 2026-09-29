//! Logical erasure and policy obligations, independent of lowering decisions.

use cq_lang_ix as ix;
use cq_lang_physical::{self as physical, Operator, policy::Policy, syntax::symbol_name};
use std::collections::{BTreeMap, BTreeSet};

/// Check source/target contracts, logical erasure, IX demands, and policy.
/// Materialization is free whenever the language and policy premises hold.
/// Formal rule CK-Physical: Check local contracts, RA erasure, IX demands and independent policy premises.
pub fn check(source: &ix::Program, target: &physical::Program, policy: &Policy) -> syn::Result<()> {
    ix::contract::check(source)?;
    let facts = physical::contract::check(target)?;
    let erased = super::erase(target);
    if erased != source.ra {
        let construct = source
            .ra
            .plan
            .definitions
            .iter()
            .zip(&erased.plan.definitions)
            .find(|(a, b)| a != b)
            .map(|(a, _)| &a.result)
            .unwrap_or(&target.program.returned);
        return Err(syn::Error::new_spanned(
            construct,
            format!("result `{construct}`: physical erasure differs from the source RA program"),
        ));
    }
    super::check_demands(source, target)?;
    let hosts: BTreeMap<_, _> = target
        .inputs()
        .map(|d| Ok((symbol_name(&d.name), d.host_facts()?)))
        .collect::<syn::Result<_>>()?;
    let mut keys: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for definition in &target.program.definitions {
        let name = symbol_name(&definition.name);
        let fail = |message: &str| {
            syn::Error::new_spanned(&definition.name, format!("definition `{name}`: {message}"))
        };
        let heading: BTreeSet<String> = facts.headings[&name].iter().cloned().collect();
        let inherited = match &definition.operator {
            Operator::Unit(_) => Some(BTreeSet::new()),
            Operator::Rename(r) => hosts[&symbol_name(&r.relation)]
                .canon
                .is_some()
                .then(|| heading.clone()),
            Operator::Probe(_, k) | Operator::Merge(_, k) => {
                let merge = policy.merge_joins() && !k.columns.is_empty();
                if merge != matches!(definition.operator, Operator::Merge(..)) {
                    return Err(fail(if merge {
                        "join policy requires merge"
                    } else {
                        "join policy requires probe (including Cartesian joins)"
                    }));
                }
                keys.get(&symbol_name(&k.left))
                    .zip(keys.get(&symbol_name(&k.right)))
                    .map(|(l, r)| l.union(r).cloned().collect())
            }
            Operator::Semi(_, k) => keys.get(&symbol_name(&k.left)).cloned(),
            Operator::Select(s) => keys.get(&symbol_name(&s.input)).cloned(),
            Operator::Range(_, r) => {
                let input = symbol_name(&r.input);
                let ordered = facts
                    .hosts
                    .get(&input)
                    .and_then(|h| hosts[h].key.as_ref())
                    .is_some_and(|k| {
                        k.ordered.is_some()
                            && k.columns.len() == 1
                            && facts.renames[&input].get(&symbol_name(&r.column))
                                == k.columns.first().map(symbol_name).as_ref()
                    });
                if !ordered {
                    return Err(fail(&format!(
                        "range of `{input}` requires the declaration's single ordered host key `{}`",
                        r.column
                    )));
                }
                if !has_bound(&r.condition, &symbol_name(&r.column)) {
                    return Err(fail(&format!(
                        "range condition does not bound key `{}`",
                        r.column
                    )));
                }
                keys.get(&input).cloned()
            }
            Operator::Extend(e) => keys.get(&symbol_name(&e.input)).cloned(),
            Operator::Project(p) => keys
                .get(&symbol_name(&p.input))
                .filter(|k| k.is_subset(&heading))
                .cloned(),
        };
        if matches!(definition.mode, physical::Mode::Alias(_)) {
            let Some(host) = facts.hosts.get(&name).and_then(|h| hosts.get(h)) else {
                return Err(fail("alias must rename a declared host"));
            };
            // A host alias must answer its reads; a let can instead carry builds.
            let answers = |columns: &[String], ordered: bool| {
                host.key.as_ref().is_some_and(|k| {
                    let wanted: BTreeSet<_> = columns
                        .iter()
                        .map(|c| facts.renames[&name][c].clone())
                        .collect();
                    let actual: BTreeSet<_> = k.columns.iter().map(symbol_name).collect();
                    wanted == actual && (!ordered || k.ordered.is_some())
                })
            };
            if facts
                .key_reads
                .get(&name)
                .into_iter()
                .flatten()
                .any(|(_, c)| !answers(c, false))
                || facts
                    .merge_reads
                    .get(&name)
                    .into_iter()
                    .flatten()
                    .any(|(_, c)| !answers(c, true))
                || facts
                    .range_reads
                    .get(&name)
                    .into_iter()
                    .flatten()
                    .any(|(_, c)| !answers(std::slice::from_ref(c), true))
            {
                return Err(fail(&format!(
                    "alias reads are not answered by declaration for `{}`",
                    facts.hosts[&name]
                )));
            }
        }
        if definition.canon.is_some()
            && definition.name != target.program.returned
            && (!policy.canon_projections() || !matches!(definition.operator, Operator::Project(_)))
        {
            return Err(fail(
                "canon is allowed only at return or a project=canon projection",
            ));
        }
        if policy.canon_projections()
            && matches!(definition.operator, Operator::Project(_))
            && inherited.is_none()
            && definition.canon.is_none()
        {
            return Err(fail(
                "project=canon requires canon on a key-dropping projection",
            ));
        }
        if let Some(key) = inherited.or_else(|| definition.canon.is_some().then_some(heading)) {
            keys.insert(name, key);
        }
    }
    let returned = symbol_name(&target.program.returned);
    if !keys.contains_key(&returned) {
        return Err(syn::Error::new_spanned(
            &target.program.returned,
            format!("return `{returned}`: not canonical by flag or declaration key inference"),
        ));
    }
    Ok(())
}

/// Independently recognize an integer bound among conjunction leaves.
fn has_bound(expr: &syn::Expr, column: &str) -> bool {
    use physical::syntax::predicate::{Comparison, Predicate, Term};
    let expr = physical::syntax::rust::unparen(expr);
    if let syn::Expr::Binary(b) = expr {
        if matches!(b.op, syn::BinOp::And(_)) {
            return has_bound(&b.left, column) || has_bound(&b.right, column);
        }
    }
    let Some(p) = Predicate::from_expr(expr) else {
        return false;
    };
    let (n, forward) = match (&p.left, &p.right) {
        (Term::Variable(v), Term::Integer(n)) if symbol_name(v) == column => (*n, true),
        (Term::Integer(n), Term::Variable(v)) if symbol_name(v) == column => (*n, false),
        _ => return false,
    };
    match p.comparison {
        Comparison::Ne(_) => false,
        Comparison::Eq(_) | Comparison::Ge(_) | Comparison::Le(_) => true,
        Comparison::Gt(_) if forward => n < i32::MAX,
        Comparison::Lt(_) if !forward => n < i32::MAX,
        Comparison::Gt(_) | Comparison::Lt(_) => n > i32::MIN,
    }
}
