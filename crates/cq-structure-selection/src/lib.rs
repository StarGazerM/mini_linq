//! IX → Structures: bind every attribute-keyed demand to a Rust type.
//!
//! Rules, from mandatory to advisory:
//!
//! 1. Admissibility. Every IX demand `@r[C]` must end up answered: by the
//!    host's key when `r` renames a host keyed on the columns `C` translates
//!    to (then nothing is built), else by a structure built over `r` keyed
//!    on `C`.
//! 2. Guarantee. Defaults preserve the plan's cost premises: O(1) expected
//!    `seek`/`contains`, so joins get `cq_storage::HashIndex` and semijoins a
//!    `HashSet` of keys. An ordered structure (`BTreeMap`, O(log n)) is
//!    chosen only where a rule needs key order — the `merge` strategy, which
//!    keys both operands on the shared columns so co-iteration is admissible.
//! 3. Advice. The policy states host keys and the strategy. A relation
//!    declared `#[index(Type)]` fixes the type of every structure built over
//!    it (through renames): any Rust type implementing `cq_storage::Build`
//!    and the traits the plan's operations use, which rustc checks.
//!    Query-specific choices for derived results are authored directly in
//!    the Structures module.
//!
//! One structure per (name, columns). Nothing here claims what a type can
//! do; the plan decides the operations and rustc decides the type.

use std::collections::{BTreeMap, BTreeSet};

use cq_ir::capabilities::Operation;
use cq_ir::policy::Policy;
use cq_ir::{index_requirements as ix, relational_plan as ra, structures, symbol_name};
use proc_macro2::TokenStream;
use quote::quote;

/// What a rule needs on a key: the strongest need picks the default type.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
enum Need {
    Contains,
    Seek,
    Ordered,
}

impl Need {
    fn operation(self, columns: &[usize]) -> Operation {
        match self {
            Need::Contains => Operation::Contains(columns.to_vec()),
            Need::Seek => Operation::Seek(columns.to_vec()),
            Need::Ordered => Operation::Ordered(columns.to_vec()),
        }
    }

    fn default_type(self) -> syn::Type {
        match self {
            Need::Contains => syn::parse_quote!(::std::collections::HashSet<_>),
            Need::Seek => syn::parse_quote!(::cq_storage::HashIndex<_, _>),
            Need::Ordered => {
                syn::parse_quote!(::std::collections::BTreeMap<_, ::std::vec::Vec<_>>)
            }
        }
    }
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

    // Hosts, exactly as the policy states them: key column names resolved
    // to sorted-heading ordinals.
    let mut host_operations: BTreeMap<String, Vec<Operation>> = BTreeMap::new();
    let mut hosts = TokenStream::new();
    for host in &policy.hosts {
        let name = symbol_name(&host.name);
        let is_input = ra_module
            .program
            .inputs()
            .any(|input| symbol_name(&input.name) == name);
        if !is_input {
            return Err(syn::Error::new_spanned(
                &host.name,
                "policy names an undeclared host relation",
            ));
        }
        let mut answered = vec![Operation::Iter];
        let mut key_tokens = TokenStream::new();
        if let Some(key) = &host.key {
            let heading = &headings[&name];
            let mut columns = Vec::new();
            for column in &key.columns {
                let Some(ordinal) = heading.iter().position(|h| *h == symbol_name(column)) else {
                    return Err(syn::Error::new_spanned(
                        column,
                        format!("`{name}` has no column `{column}`"),
                    ));
                };
                columns.push(ordinal);
            }
            columns.sort_unstable();
            columns.dedup();
            answered.push(Operation::Seek(columns.clone()));
            answered.push(Operation::Contains(columns.clone()));
            let ordered = key.ordered;
            if ordered.is_some() {
                answered.push(Operation::Ordered(columns.clone()));
            }
            let columns = columns.iter().map(|&c| syn::Index::from(c));
            key_tokens = quote! { keyed [#(#columns),*] #ordered };
        }
        if host_operations.insert(name, answered).is_some() {
            return Err(syn::Error::new_spanned(
                &host.name,
                "host relation configured twice",
            ));
        }
        let ident = &host.name;
        hosts.extend(quote! { #ident #key_tokens; });
    }
    for input in ra_module.program.inputs() {
        if !host_operations.contains_key(&symbol_name(&input.name)) {
            return Err(syn::Error::new_spanned(
                policy,
                format!("policy must list host relation `{}`", input.name),
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

    // Advice declared on the host relation applies to its renames.
    let mut advice: BTreeMap<String, syn::Type> = BTreeMap::new();
    for input in ra_module.program.inputs() {
        if let Some(attr) = input.index_advice()? {
            advice.insert(symbol_name(&input.name), attr.parse_args()?);
        }
    }

    // What the plan requires on every (name, key), unless the host serves it.
    let mut needs: BTreeMap<(String, Vec<usize>), BTreeSet<Need>> = BTreeMap::new();
    let mut choose = |name: &str, columns: Vec<usize>, need: Need| {
        let required = need.operation(&columns);
        if inherited(name).contains(&required) {
            return;
        }
        needs
            .entry((name.to_string(), columns))
            .or_default()
            .insert(need);
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
                choose(&left_name, ordinals(&left_name), Need::Ordered);
                choose(&right_name, ordinals(&right_name), Need::Ordered);
            }
            ra::Operator::NaturalJoin(_) => {
                choose(&right_name, ordinals(&right_name), Need::Seek);
            }
            ra::Operator::Semijoin(_) => {
                choose(&right_name, ordinals(&right_name), Need::Contains);
            }
            _ => unreachable!(),
        }
    }

    let mut entries = TokenStream::new();
    for definition in &plan.definitions {
        let name = symbol_name(&definition.result);
        let ident = &definition.result;
        for ((_, columns), needs) in needs.iter().filter(|((n, _), _)| *n == name) {
            let advised = renames
                .get(&name)
                .and_then(|rename| advice.get(&symbol_name(&rename.relation)));
            let ty = match advised {
                Some(ty) => ty.clone(),
                None => needs.iter().max().expect("nonempty").default_type(),
            };
            let columns = columns.iter().map(|&c| syn::Index::from(c));
            entries.extend(quote! { #ident = #ty [#(#columns),*]; });
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
