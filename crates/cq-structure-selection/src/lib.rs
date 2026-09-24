//! IX → Structures: bind every attribute-keyed demand to a catalogue entry.
//!
//! Rules, from mandatory to advisory:
//!
//! 1. Admissibility. Every IX demand `@r[C]` must end up served by a structure
//!    on `r` that provides `seek[C]` (join) or `contains[C]` (semijoin). Host
//!    capabilities, declared by the policy, serve demands on renames through
//!    the rename's column translation; then no structure is built.
//! 2. Guarantee. Defaults preserve the plan's cost premises: O(1) expected
//!    `seek`/`contains`, so joins get `hash` and semijoins `key_set`. An
//!    `ordered` structure (O(log n)) is chosen only where a rule needs key
//!    order — the `merge` strategy, which also orders the left operand on the
//!    shared columns so that co-iteration is admissible.
//! 3. Advice. The policy supplies host capabilities and the join strategy.
//!    Query-specific choices are authored directly in the Structures module.
//!
//! One structure per (name, columns): the strongest requested kind wins,
//! since `ordered ⊒ seek ⊒ contains`.

use std::collections::BTreeMap;

use cq_ir::capabilities::{Operation, operations};
use cq_ir::policy::Policy;
use cq_ir::{index_requirements as ix, relational_plan as ra, structures, symbol_name};
use proc_macro2::TokenStream;
use quote::quote;
use syn::Ident;

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Kind {
    KeySet,
    Hash,
    Ordered,
}

pub fn compile(source: &ix::Module, policy: &Policy) -> syn::Result<structures::Module> {
    ix::contract::check(source)?;
    let ra_module = &source.relational_plan;
    let plan = &ra_module.plan;
    let infos = ra::check_in(ra_module.program.inputs(), plan)?;
    let mut headings: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for input in ra_module.program.inputs() {
        let mut heading: Vec<_> = input.columns.iter().map(|c| symbol_name(&c.name)).collect();
        heading.sort();
        headings.insert(symbol_name(&input.name), heading);
    }
    for definition in &plan.definitions {
        let name = symbol_name(&definition.result);
        headings.insert(name.clone(), infos[&name].heading.iter().cloned().collect());
    }

    // Hosts, exactly as the policy declares them.
    let mut host_operations: BTreeMap<String, Vec<Operation>> = BTreeMap::new();
    let mut hosts = TokenStream::new();
    for host in &policy.hosts {
        let name = symbol_name(&host.name);
        if !headings.contains_key(&name)
            || plan
                .definitions
                .iter()
                .any(|d| symbol_name(&d.result) == name)
        {
            return Err(syn::Error::new_spanned(
                &host.name,
                "policy names an undeclared host relation",
            ));
        }
        if host_operations
            .insert(name, operations(&host.provides))
            .is_some()
        {
            return Err(syn::Error::new_spanned(
                &host.name,
                "host relation configured twice",
            ));
        }
        let (ident, provides) = (&host.name, &host.provides);
        hosts.extend(quote! { #ident = host #provides; });
    }
    for input in ra_module.program.inputs() {
        if !host_operations.contains_key(&symbol_name(&input.name)) {
            return Err(syn::Error::new_spanned(
                policy,
                format!(
                    "policy must declare capabilities for host relation `{}`",
                    input.name
                ),
            ));
        }
    }
    let renames: BTreeMap<String, &ra::Rename> = plan
        .definitions
        .iter()
        .filter_map(|d| match &d.operator {
            ra::Operator::Rename(r) => Some((symbol_name(&d.result), &**r)),
            _ => None,
        })
        .collect();
    // What a rename inherits from its host, in its own heading ordinals.
    let inherited = |name: &str| -> Vec<Operation> {
        let Some(rename) = renames.get(name) else {
            return Vec::new();
        };
        let host = symbol_name(&rename.relation);
        let (host_heading, heading) = (&headings[&host], &headings[name]);
        let translate = |ordinals: &[usize]| -> Vec<usize> {
            let mut t: Vec<usize> = ordinals
                .iter()
                .map(|&c| {
                    let target = rename
                        .mapping
                        .iter()
                        .find(|m| symbol_name(&m.source) == host_heading[c])
                        .map(|m| symbol_name(&m.target))
                        .expect("complete rename");
                    heading
                        .iter()
                        .position(|h| *h == target)
                        .expect("target in heading")
                })
                .collect();
            t.sort_unstable();
            t
        };
        host_operations[&host]
            .iter()
            .map(|op| match op {
                Operation::Iter => Operation::Iter,
                Operation::Contains(c) => Operation::Contains(translate(c)),
                Operation::Seek(c) => Operation::Seek(translate(c)),
                Operation::Ordered(c) => Operation::Ordered(translate(c)),
            })
            .collect()
    };

    let mut chosen: BTreeMap<(String, Vec<usize>), Kind> = BTreeMap::new();
    let mut choose = |name: &str, columns: Vec<usize>, kind: Kind| {
        let required = match kind {
            Kind::KeySet => Operation::Contains(columns.clone()),
            Kind::Hash => Operation::Seek(columns.clone()),
            Kind::Ordered => Operation::Ordered(columns.clone()),
        };
        if inherited(name).iter().any(|p| p.satisfies(&required)) {
            return;
        }
        let slot = chosen.entry((name.to_string(), columns)).or_insert(kind);
        *slot = (*slot).max(kind);
    };
    for definition in &plan.definitions {
        let (left, right) = match &definition.operator {
            ra::Operator::NaturalJoin(j) => (&j.left, &j.right),
            ra::Operator::Semijoin(j) => (&j.left, &j.right),
            _ => continue,
        };
        let (left_name, right_name) = (symbol_name(left), symbol_name(right));
        let shared: Vec<String> = headings[&right_name]
            .iter()
            .filter(|a| headings[&left_name].contains(a))
            .cloned()
            .collect();
        let ordinals = |name: &str| -> Vec<usize> {
            shared
                .iter()
                .map(|a| headings[name].iter().position(|h| h == a).unwrap())
                .collect()
        };
        match &definition.operator {
            ra::Operator::NaturalJoin(_) if policy.merge_joins() => {
                choose(&left_name, ordinals(&left_name), Kind::Ordered);
                choose(&right_name, ordinals(&right_name), Kind::Ordered);
            }
            ra::Operator::NaturalJoin(_) => choose(&right_name, ordinals(&right_name), Kind::Hash),
            ra::Operator::Semijoin(_) => choose(&right_name, ordinals(&right_name), Kind::KeySet),
            _ => unreachable!(),
        }
    }

    let mut entries = TokenStream::new();
    for definition in &plan.definitions {
        let name = symbol_name(&definition.result);
        let ident = &definition.result;
        for ((_, columns), kind) in chosen.iter().filter(|((n, _), _)| *n == name) {
            let kind: Ident = syn::parse_str(match kind {
                Kind::KeySet => "key_set",
                Kind::Hash => "hash",
                Kind::Ordered => "ordered",
            })
            .unwrap();
            let columns = columns.iter().map(|&c| syn::Index::from(c));
            entries.extend(quote! { #ident = #kind [#(#columns),*]; });
        }
    }
    let target: structures::Module =
        syn::parse2(quote! { #source structures { #hosts #entries } })?;
    structures::contract::check(&target)?;
    Ok(target)
}

/// IX → Structures correspondence: IX preserved, the contract holds, and the
/// module is exactly the selection under `policy`.
pub fn check(source: &ix::Module, target: &structures::Module, policy: &Policy) -> bool {
    target.index_requirements == *source
        && structures::contract::well_formed(target)
        && compile(source, policy).is_ok_and(|expected| expected == *target)
}
