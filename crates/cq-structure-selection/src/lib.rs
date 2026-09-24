//! IX → Structures: bind every attribute-keyed demand to a Rust type.
//!
//! Rules, from mandatory to advisory:
//!
//! 1. Admissibility. Every IX demand `@r[C]` must end up served by a structure
//!    on `r` claiming `seek[C]` (join) or `contains[C]` (semijoin). Host
//!    capabilities, declared by the policy, serve demands on renames through
//!    the rename's column translation; then no structure is built.
//! 2. Guarantee. Defaults preserve the plan's cost premises: O(1) expected
//!    `seek`/`contains`, so joins get `cq_storage::HashIndex` and semijoins a
//!    `HashSet` of keys. An ordered structure (`BTreeMap`, O(log n)) is
//!    chosen only where a rule needs key order — the `merge` strategy, which
//!    orders both operands on the shared columns so co-iteration is
//!    admissible.
//! 3. Advice. The policy supplies host capabilities and the join strategy. A
//!    relation declared `#[index(Type provides { … })]` fixes the type of
//!    every structure built over it (through renames): any Rust type
//!    implementing `cq_storage::Build` and the traits it claims. A type whose
//!    claims do not cover what the plan requires on that key is an error,
//!    never silently replaced. Query-specific choices for derived results are
//!    authored directly in the Structures module.
//!
//! One structure per (name, columns), claiming the union of what the plan
//! requires there; without advice the weakest default whose claims cover it.

use std::collections::{BTreeMap, BTreeSet};

use cq_ir::capabilities::{Claims, Operation, operations};
use cq_ir::policy::Policy;
use cq_ir::{index_requirements as ix, relational_plan as ra, structures, symbol_name};
use proc_macro2::TokenStream;
use quote::quote;
use syn::Ident;

/// One of the three operations a structure may claim on its key.
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

    fn name(self) -> &'static str {
        match self {
            Need::Contains => "contains",
            Need::Seek => "seek",
            Need::Ordered => "ordered",
        }
    }

    fn claimed(claims: &Claims) -> BTreeSet<Need> {
        use cq_ir::capabilities::Claim;
        claims
            .claims
            .iter()
            .map(|c| match c {
                Claim::Seek(_) => Need::Seek,
                Claim::Contains(_) => Need::Contains,
                Claim::Ordered(_) => Need::Ordered,
            })
            .collect()
    }
}

/// A structure type with the claims it is known to honour.
#[derive(Clone)]
struct Choice {
    ty: syn::Type,
    claims: Claims,
}

impl Choice {
    fn covers(&self, needs: &BTreeSet<Need>) -> bool {
        needs.is_subset(&Need::claimed(&self.claims))
    }
}

/// The defaults, weakest first: the first whose claims cover the needs wins.
fn defaults() -> [Choice; 3] {
    [
        Choice {
            ty: syn::parse_quote!(::std::collections::HashSet<_>),
            claims: syn::parse_quote!(provides { contains }),
        },
        Choice {
            ty: syn::parse_quote!(::cq_storage::HashIndex<_, _>),
            claims: syn::parse_quote!(provides { seek, contains }),
        },
        Choice {
            ty: syn::parse_quote!(::std::collections::BTreeMap<_, ::std::vec::Vec<_>>),
            claims: syn::parse_quote!(provides {
                ordered,
                seek,
                contains
            }),
        },
    ]
}

fn parse_advice(attr: &syn::Attribute) -> syn::Result<Choice> {
    attr.parse_args_with(|input: syn::parse::ParseStream<'_>| {
        Ok(Choice {
            ty: input.parse()?,
            claims: input.parse()?,
        })
    })
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

    // Advice declared on the host relation applies to its renames.
    let mut advice: BTreeMap<String, Choice> = BTreeMap::new();
    for input in ra_module.program.inputs() {
        if let Some(attr) = input.index_advice()? {
            advice.insert(symbol_name(&input.name), parse_advice(attr)?);
        }
    }

    // What the plan requires on every (name, key), unless the host serves it.
    let mut needs: BTreeMap<(String, Vec<usize>), (BTreeSet<Need>, Ident)> = BTreeMap::new();
    let mut choose = |name: &str, columns: Vec<usize>, need: Need, site: &Ident| {
        let required = need.operation(&columns);
        if inherited(name).contains(&required) {
            return;
        }
        needs
            .entry((name.to_string(), columns))
            .or_insert_with(|| (BTreeSet::new(), site.clone()))
            .0
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
        let site = &definition.result;
        match &definition.operator {
            ra::Operator::NaturalJoin(_) if policy.merge_joins() => {
                choose(&left_name, ordinals(&left_name), Need::Ordered, site);
                choose(&right_name, ordinals(&right_name), Need::Ordered, site);
            }
            ra::Operator::NaturalJoin(_) => {
                choose(&right_name, ordinals(&right_name), Need::Seek, site);
            }
            ra::Operator::Semijoin(_) => {
                choose(&right_name, ordinals(&right_name), Need::Contains, site);
            }
            _ => unreachable!(),
        }
    }

    let mut entries = TokenStream::new();
    for definition in &plan.definitions {
        let name = symbol_name(&definition.result);
        let ident = &definition.result;
        for ((_, columns), (needs, site)) in needs.iter().filter(|((n, _), _)| *n == name) {
            let advised = renames
                .get(&name)
                .and_then(|rename| advice.get(&symbol_name(&rename.relation)));
            let choice = match advised {
                Some(choice) if choice.covers(needs) => choice.clone(),
                Some(choice) => {
                    let missing: Vec<&str> = needs
                        .difference(&Need::claimed(&choice.claims))
                        .map(|n| n.name())
                        .collect();
                    return Err(syn::Error::new_spanned(
                        site,
                        format!(
                            "relation `{}` advises a structure that does not claim {} needed by `{name}` on {columns:?}",
                            renames[&name].relation,
                            missing.join(", "),
                        ),
                    ));
                }
                None => defaults()
                    .into_iter()
                    .find(|d| d.covers(needs))
                    .expect("BTreeMap covers every need"),
            };
            let Choice { ty, claims } = choice;
            let columns = columns.iter().map(|&c| syn::Index::from(c));
            entries.extend(quote! { #ident = #ty [#(#columns),*] #claims; });
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
