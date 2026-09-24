//! IX → Stream: realize every RA result as a typed stream program under a
//! query-independent policy.
//!
//! The kernel owns one rule per RA operator. The policy supplies only what
//! the query cannot determine: the capabilities each host relation admits,
//! and which admissible rule to prefer where more than one applies. Modes
//! follow from use: a result read once through `scan` is a pipe; a result
//! read more than once, or read through `seek`/`contains`/`order`, is
//! materialized with exactly the capabilities its readers require.
//!
//! ```text
//! Policy ::= "policy" "{" (Ident "provides" "{" Cap,* "}" ";")*
//!            ["join" "=" ("seek" | "merge") ";"]
//!            ["semijoin" "=" ("eager" | "prefer_lazy") ";"] "}"
//! ```

use std::collections::{BTreeMap, BTreeSet};

use cq_ir::{index_requirements as ix, relational_plan as ra, stream, symbol_name};
use proc_macro2::{Span, TokenStream};
use quote::{format_ident, quote};
use syn::{Ident, Token, token};

pub mod kw {
    syn::custom_keyword!(policy);
    syn::custom_keyword!(join);
    syn::custom_keyword!(semijoin);
    syn::custom_keyword!(seek);
    syn::custom_keyword!(merge);
    syn::custom_keyword!(eager);
    syn::custom_keyword!(prefer_lazy);
}

#[derive(Clone, Debug, PartialEq, Eq, syn_derive::Parse, syn_derive::ToTokens)]
pub struct Policy {
    pub policy_token: kw::policy,
    #[syn(braced)]
    pub brace: token::Brace,
    #[syn(in = brace)]
    #[parse(parse_hosts)]
    #[to_tokens(|tokens, hosts| quote::TokenStreamExt::append_all(tokens, hosts))]
    pub hosts: Vec<Host>,
    #[syn(in = brace)]
    #[syn(optional, peek = kw::join)]
    pub join: Option<JoinChoice>,
    #[syn(in = brace)]
    #[syn(optional, peek = kw::semijoin)]
    pub semijoin: Option<SemijoinChoice>,
}

fn parse_hosts(input: syn::parse::ParseStream<'_>) -> syn::Result<Vec<Host>> {
    let mut hosts = Vec::new();
    while input.peek(Ident) && !input.peek(kw::join) && !input.peek(kw::semijoin) {
        hosts.push(input.parse()?);
    }
    Ok(hosts)
}

#[derive(Clone, Debug, PartialEq, Eq, syn_derive::Parse, syn_derive::ToTokens)]
pub struct Host {
    pub name: Ident,
    pub provides: stream::Provides,
    pub semi_token: Token![;],
}

#[derive(Clone, Debug, PartialEq, Eq, syn_derive::Parse, syn_derive::ToTokens)]
pub struct JoinChoice {
    pub join_token: kw::join,
    pub eq_token: Token![=],
    pub rule: JoinRule,
    pub semi_token: Token![;],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, syn_derive::Parse, syn_derive::ToTokens)]
pub enum JoinRule {
    #[parse(peek = kw::seek)]
    Seek(kw::seek),
    #[parse(peek = kw::merge)]
    Merge(kw::merge),
}

#[derive(Clone, Debug, PartialEq, Eq, syn_derive::Parse, syn_derive::ToTokens)]
pub struct SemijoinChoice {
    pub semijoin_token: kw::semijoin,
    pub eq_token: Token![=],
    pub rule: SemijoinRule,
    pub semi_token: Token![;],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, syn_derive::Parse, syn_derive::ToTokens)]
pub enum SemijoinRule {
    #[parse(peek = kw::eager)]
    Eager(kw::eager),
    #[parse(peek = kw::prefer_lazy)]
    PreferLazy(kw::prefer_lazy),
}

impl Policy {
    fn merge_joins(&self) -> bool {
        matches!(
            self.join,
            Some(JoinChoice {
                rule: JoinRule::Merge(_),
                ..
            })
        )
    }

    fn prefer_lazy(&self) -> bool {
        matches!(
            self.semijoin,
            Some(SemijoinChoice {
                rule: SemijoinRule::PreferLazy(_),
                ..
            })
        )
    }
}

/// Facts about one RA result used to choose its mode and body.
struct Result_ {
    name: String,
    heading: Vec<String>,
    /// Readers in RA order, each with the operation it needs on this result.
    needs: Vec<(String, stream::Operation)>,
    bag: bool,
}

/// Lower IX to Stream under `policy`, then check the Stream contract.
pub fn compile(source: &ix::Module, policy: &Policy) -> syn::Result<stream::Module> {
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

    // Host capabilities in each host's sorted column ordinals.
    let host_operations: BTreeMap<String, Vec<stream::Operation>> = policy
        .hosts
        .iter()
        .map(|host| (symbol_name(&host.name), operations(&host.provides)))
        .collect();
    let renames: BTreeMap<String, &ra::Rename> = plan
        .definitions
        .iter()
        .filter_map(|d| match &d.operator {
            ra::Operator::Rename(r) => Some((symbol_name(&d.result), &**r)),
            _ => None,
        })
        .collect();

    // Requirements each result places on its operands.
    let mut results: BTreeMap<String, Result_> = plan
        .definitions
        .iter()
        .map(|d| {
            let name = symbol_name(&d.result);
            (
                name.clone(),
                Result_ {
                    heading: headings[&name].clone(),
                    name,
                    needs: Vec::new(),
                    bag: false,
                },
            )
        })
        .collect();
    let ordinals = |relation: &str, attributes: &[String]| -> Vec<usize> {
        let heading = &headings[relation];
        attributes
            .iter()
            .map(|a| {
                heading
                    .iter()
                    .position(|h| h == a)
                    .expect("attribute in heading")
            })
            .collect()
    };
    for definition in &plan.definitions {
        let reader = symbol_name(&definition.result);
        let mut need = |operand: &Ident, operation: stream::Operation| {
            results
                .get_mut(&symbol_name(operand))
                .expect("RA operands are results")
                .needs
                .push((reader.clone(), operation));
        };
        match &definition.operator {
            ra::Operator::Unit(_) | ra::Operator::Rename(_) => {}
            ra::Operator::NaturalJoin(join) => {
                let shared = shared_attributes(&headings, &join.left, &join.right);
                if policy.merge_joins() {
                    need(
                        &join.left,
                        stream::Operation::Ordered(ordinals(&symbol_name(&join.left), &shared)),
                    );
                    need(
                        &join.right,
                        stream::Operation::Ordered(ordinals(&symbol_name(&join.right), &shared)),
                    );
                } else {
                    need(&join.left, stream::Operation::Iter);
                    need(
                        &join.right,
                        stream::Operation::Seek(ordinals(&symbol_name(&join.right), &shared)),
                    );
                }
            }
            ra::Operator::Semijoin(join) => {
                let shared = shared_attributes(&headings, &join.left, &join.right);
                need(&join.left, stream::Operation::Iter);
                need(
                    &join.right,
                    stream::Operation::Contains(ordinals(&symbol_name(&join.right), &shared)),
                );
            }
            ra::Operator::Project(p) => need(&p.input, stream::Operation::Iter),
            ra::Operator::Select(s) => {
                let input = symbol_name(&s.input);
                match bounded_scan(&s.condition, &headings[&input]) {
                    Some(window)
                        if renames.get(&input).is_some_and(|rename| {
                            host_provides(rename, &headings[&input], &headings, &host_operations)
                                .iter()
                                .any(|p| {
                                    p.satisfies(&stream::Operation::Ordered(vec![window.column]))
                                })
                        }) =>
                    {
                        need(&s.input, stream::Operation::Ordered(vec![window.column]));
                    }
                    _ => need(&s.input, stream::Operation::Iter),
                }
            }
            ra::Operator::Extend(e) => need(&e.input, stream::Operation::Iter),
        }
    }
    let output = symbol_name(&plan.output.input);
    results
        .get_mut(&output)
        .expect("output names a result")
        .needs
        .push((stream::RETURN.to_string(), stream::Operation::Iter));

    // Bag-ness: projection introduces multiplicity; everything else preserves it.
    for definition in &plan.definitions {
        let name = symbol_name(&definition.result);
        let bag = match &definition.operator {
            ra::Operator::Project(_) => true,
            ra::Operator::Unit(_) | ra::Operator::Rename(_) => false,
            ra::Operator::NaturalJoin(j) => {
                results[&symbol_name(&j.left)].bag || results[&symbol_name(&j.right)].bag
            }
            ra::Operator::Semijoin(j) => results[&symbol_name(&j.left)].bag,
            ra::Operator::Select(s) => results[&symbol_name(&s.input)].bag,
            ra::Operator::Extend(e) => results[&symbol_name(&e.input)].bag,
        };
        results.get_mut(&name).unwrap().bag = bag;
    }

    // Host inputs from the policy.
    let declared: BTreeSet<_> = ra_module
        .program
        .inputs()
        .map(|d| symbol_name(&d.name))
        .collect();
    let mut inputs = TokenStream::new();
    let mut seen = BTreeSet::new();
    for host in &policy.hosts {
        let name = symbol_name(&host.name);
        if !declared.contains(&name) {
            return Err(syn::Error::new_spanned(
                &host.name,
                "policy names an undeclared host relation",
            ));
        }
        if !seen.insert(name) {
            return Err(syn::Error::new_spanned(
                &host.name,
                "host relation configured twice",
            ));
        }
        let host_name = &host.name;
        let provides = &host.provides;
        inputs.extend(quote! { open #host_name #provides; });
    }
    if let Some(missing) = declared.difference(&seen).next() {
        return Err(syn::Error::new_spanned(
            policy,
            format!("policy must declare capabilities for host relation `{missing}`"),
        ));
    }

    let mut definitions = TokenStream::new();
    for definition in &plan.definitions {
        let name = symbol_name(&definition.result);
        let result = &results[&name];
        let ident = &definition.result;
        if let ra::Operator::Rename(rename) = &definition.operator {
            if host_serves(rename, result, &headings, &host_operations) {
                let host = &rename.relation;
                definitions.extend(quote! { #ident = alias #host; });
                continue;
            }
        }
        let mode = mode(result);
        let lazy = lazy(policy, definition, result, plan);
        let body = body(
            definition,
            &headings,
            &results,
            policy,
            &renames,
            &host_operations,
        );
        let body = if name == output && result.bag {
            quote! { support { #body } }
        } else {
            body
        };
        definitions.extend(quote! { #ident = #lazy #mode #body; });
    }
    let return_name = &plan.output.input;
    let target: stream::Module = syn::parse2(quote! {
        #source
        stream { #inputs #definitions return #return_name; }
    })?;
    stream::contract::check(&target)?;
    Ok(target)
}

fn operations(provides: &stream::Provides) -> Vec<stream::Operation> {
    provides
        .capabilities
        .iter()
        .map(|c| match c {
            stream::Capability::Iter { .. } => stream::Operation::Iter,
            stream::Capability::Contains { descriptor, .. } => {
                stream::Operation::Contains(descriptor.columns.ordinals())
            }
            stream::Capability::Seek { descriptor, .. } => {
                stream::Operation::Seek(descriptor.columns.ordinals())
            }
            stream::Capability::Ordered { descriptor, .. } => {
                stream::Operation::Ordered(descriptor.columns.ordinals())
            }
        })
        .collect()
}

/// A rename needs no realization of its own when the host, read through the
/// rename's column translation, already provides every operation required.
fn host_serves(
    rename: &ra::Rename,
    result: &Result_,
    headings: &BTreeMap<String, Vec<String>>,
    host_operations: &BTreeMap<String, Vec<stream::Operation>>,
) -> bool {
    let provided = host_provides(rename, &result.heading, headings, host_operations);
    result
        .needs
        .iter()
        .all(|(_, need)| provided.iter().any(|p| p.satisfies(need)))
}

/// The host's declared operations, read through the rename's column
/// translation into the renamed result's sorted heading ordinals.
fn host_provides(
    rename: &ra::Rename,
    heading: &[String],
    headings: &BTreeMap<String, Vec<String>>,
    host_operations: &BTreeMap<String, Vec<stream::Operation>>,
) -> Vec<stream::Operation> {
    let host = symbol_name(&rename.relation);
    let host_heading = &headings[&host];
    let translate = |ordinals: &[usize]| -> Vec<usize> {
        let mut translated: Vec<usize> = ordinals
            .iter()
            .map(|&column| {
                let target = rename
                    .mapping
                    .iter()
                    .find(|m| symbol_name(&m.source) == host_heading[column])
                    .map(|m| symbol_name(&m.target))
                    .expect("complete rename");
                heading
                    .iter()
                    .position(|h| *h == target)
                    .expect("target in heading")
            })
            .collect();
        translated.sort_unstable();
        translated
    };
    host_operations[&host]
        .iter()
        .map(|op| match op {
            stream::Operation::Iter => stream::Operation::Iter,
            stream::Operation::Contains(c) => stream::Operation::Contains(translate(c)),
            stream::Operation::Seek(c) => stream::Operation::Seek(translate(c)),
            stream::Operation::Ordered(c) => stream::Operation::Ordered(translate(c)),
        })
        .collect()
}

/// A comparison chain on one attribute against integer literals, as a key
/// window plus the conjuncts it does not cover.
struct Window {
    column: usize,
    low: Option<i32>,
    high: Option<i32>,
    residual: Vec<syn::Expr>,
}

fn conjuncts(expr: &syn::Expr) -> Vec<&syn::Expr> {
    match cq_ir::cq::rust::unparen(expr) {
        syn::Expr::Binary(syn::ExprBinary {
            op: syn::BinOp::And(_),
            left,
            right,
            ..
        }) => {
            let mut all = conjuncts(left);
            all.extend(conjuncts(right));
            all
        }
        other => vec![other],
    }
}

fn bounded_scan(condition: &syn::Expr, heading: &[String]) -> Option<Window> {
    use cq_ir::cq::{Comparison, Predicate, Term};
    let parts = conjuncts(condition);
    // The window attribute is the first one compared against a literal.
    let attribute = parts.iter().find_map(|part| {
        let predicate = Predicate::from_expr(part)?;
        match (&predicate.left, &predicate.right) {
            (Term::Variable(v), Term::Integer(_)) | (Term::Integer(_), Term::Variable(v)) => {
                Some(symbol_name(v))
            }
            _ => None,
        }
    })?;
    let column = heading.iter().position(|h| *h == attribute)?;
    let mut window = Window {
        column,
        low: None,
        high: None,
        residual: Vec::new(),
    };
    let tighten_low = |low: &mut Option<i32>, v: i32| *low = Some(low.map_or(v, |l| l.max(v)));
    let tighten_high = |high: &mut Option<i32>, v: i32| *high = Some(high.map_or(v, |h| h.min(v)));
    for part in parts {
        let covered = Predicate::from_expr(part).and_then(|predicate| {
            // Normalize to `attribute cmp literal`.
            let (comparison, literal) = match (&predicate.left, &predicate.right) {
                (Term::Variable(v), Term::Integer(k)) if symbol_name(v) == attribute => {
                    (predicate.comparison, *k)
                }
                (Term::Integer(k), Term::Variable(v)) if symbol_name(v) == attribute => {
                    let flipped = match predicate.comparison {
                        Comparison::Lt(t) => Comparison::Gt(syn::Token![>](t.span)),
                        Comparison::Le(t) => Comparison::Ge(syn::Token![>=](t.spans[0])),
                        Comparison::Gt(t) => Comparison::Lt(syn::Token![<](t.span)),
                        Comparison::Ge(t) => Comparison::Le(syn::Token![<=](t.spans[0])),
                        same => same,
                    };
                    (flipped, *k)
                }
                _ => return None,
            };
            match comparison {
                Comparison::Eq(_) => {
                    tighten_low(&mut window.low, literal);
                    tighten_high(&mut window.high, literal);
                }
                Comparison::Ge(_) => tighten_low(&mut window.low, literal),
                Comparison::Gt(_) => tighten_low(&mut window.low, literal.checked_add(1)?),
                Comparison::Le(_) => tighten_high(&mut window.high, literal),
                Comparison::Lt(_) => tighten_high(&mut window.high, literal.checked_sub(1)?),
                Comparison::Ne(_) => return None,
            }
            Some(())
        });
        if covered.is_none() {
            window.residual.push(part.clone());
        }
    }
    (window.low.is_some() || window.high.is_some()).then_some(window)
}

fn shared_attributes(
    headings: &BTreeMap<String, Vec<String>>,
    left: &Ident,
    right: &Ident,
) -> Vec<String> {
    let left = &headings[&symbol_name(left)];
    headings[&symbol_name(right)]
        .iter()
        .filter(|a| left.contains(a))
        .cloned()
        .collect()
}

fn mode(result: &Result_) -> TokenStream {
    let materialized = result.needs.len() > 1
        || result
            .needs
            .iter()
            .any(|(_, op)| *op != stream::Operation::Iter);
    if !materialized {
        return quote!(pipe);
    }
    let name = ident(&result.name);
    let mut provided = BTreeSet::new();
    for (_, operation) in &result.needs {
        provided.insert(operation.clone());
    }
    let capabilities = provided.iter().map(|op| match op {
        stream::Operation::Iter => quote!(iter),
        stream::Operation::Contains(c) => {
            let c = c.iter().map(|&i| syn::Index::from(i));
            quote!(contains @#name[#(#c),*])
        }
        stream::Operation::Seek(c) => {
            let c = c.iter().map(|&i| syn::Index::from(i));
            quote!(seek @#name[#(#c),*])
        }
        stream::Operation::Ordered(c) => {
            let c = c.iter().map(|&i| syn::Index::from(i));
            quote!(ordered @#name[#(#c),*])
        }
    });
    quote!(let provides { #(#capabilities),* })
}

/// A semijoin may be lazy when its single consumer also reads the reducer.
fn lazy(
    policy: &Policy,
    definition: &ra::Definition,
    result: &Result_,
    plan: &ra::Plan,
) -> TokenStream {
    let ra::Operator::Semijoin(semijoin) = &definition.operator else {
        return TokenStream::new();
    };
    if !policy.prefer_lazy() || result.needs.len() != 1 {
        return TokenStream::new();
    }
    let reducer = symbol_name(&semijoin.right);
    let consumer = &result.needs[0].0;
    let consumer_reads_reducer = plan
        .definitions
        .iter()
        .find(|d| symbol_name(&d.result) == *consumer)
        .is_some_and(|d| {
            operands(&d.operator)
                .iter()
                .any(|o| symbol_name(o) == reducer)
        });
    if consumer_reads_reducer {
        quote!(lazy)
    } else {
        TokenStream::new()
    }
}

fn operands(operator: &ra::Operator) -> Vec<&Ident> {
    match operator {
        ra::Operator::Unit(_) => vec![],
        ra::Operator::Rename(_) => vec![],
        ra::Operator::NaturalJoin(j) => vec![&j.left, &j.right],
        ra::Operator::Semijoin(j) => vec![&j.left, &j.right],
        ra::Operator::Project(p) => vec![&p.input],
        ra::Operator::Select(s) => vec![&s.input],
        ra::Operator::Extend(e) => vec![&e.input],
    }
}

fn ident(name: &str) -> Ident {
    syn::parse_str::<Ident>(name)
        .unwrap_or_else(|_| format_ident!("r#{}", name, span = Span::call_site()))
}

fn tuple(names: &[String]) -> TokenStream {
    let names = names.iter().map(|n| ident(n));
    quote!((#(#names,)*))
}

fn columns(ordinals: &[usize]) -> TokenStream {
    let ordinals = ordinals.iter().map(|&i| syn::Index::from(i));
    quote!([#(#ordinals),*])
}

fn body(
    definition: &ra::Definition,
    headings: &BTreeMap<String, Vec<String>>,
    results: &BTreeMap<String, Result_>,
    policy: &Policy,
    renames: &BTreeMap<String, &ra::Rename>,
    host_operations: &BTreeMap<String, Vec<stream::Operation>>,
) -> TokenStream {
    let name = symbol_name(&definition.result);
    let heading = &results[&name].heading;
    let emit = tuple(heading);
    match &definition.operator {
        ra::Operator::Unit(_) => quote! { unit { emit () } },
        ra::Operator::Rename(rename) => {
            // Bind each source column, in the host's sorted column order, to
            // the query variable it renames to; emit in result heading order.
            let source = &rename.relation;
            let mapping: BTreeMap<String, String> = rename
                .mapping
                .iter()
                .map(|m| (symbol_name(&m.source), symbol_name(&m.target)))
                .collect();
            let pattern: Vec<String> = headings[&symbol_name(source)]
                .iter()
                .map(|column| mapping[column].clone())
                .collect();
            let pattern = tuple(&pattern);
            quote! { for #pattern in scan #source { emit #emit } }
        }
        ra::Operator::Select(select) => {
            let input = &select.input;
            let input_heading = &results[&symbol_name(input)].heading;
            let pattern = tuple(input_heading);
            let condition = &select.condition;
            let ordered = |window: &Window| {
                renames.get(&symbol_name(input)).is_some_and(|rename| {
                    host_provides(rename, input_heading, headings, host_operations)
                        .iter()
                        .any(|p| p.satisfies(&stream::Operation::Ordered(vec![window.column])))
                })
            };
            match bounded_scan(condition, input_heading) {
                Some(window) if ordered(&window) => {
                    let column = syn::Index::from(window.column);
                    let limit = |value: Option<i32>| match value {
                        Some(v) => quote!((#v)),
                        None => quote!(_),
                    };
                    let (low, high) = (limit(window.low), limit(window.high));
                    let residual = &window.residual;
                    let guarded = if residual.is_empty() {
                        quote! { emit #emit }
                    } else {
                        quote! { if (#(#residual)&&*) { emit #emit } }
                    };
                    quote! { for #pattern in scan #input within [#column](#low, #high) { #guarded } }
                }
                _ => quote! { for #pattern in scan #input { if (#condition) { emit #emit } } },
            }
        }
        ra::Operator::Extend(extend) => {
            let input = &extend.input;
            let pattern = tuple(&results[&symbol_name(input)].heading);
            let fresh: Vec<_> = extend.columns.iter().map(|c| &c.name).collect();
            let value = &extend.value;
            quote! { for #pattern in scan #input { let (#(#fresh,)*) = #value; emit #emit } }
        }
        ra::Operator::Project(project) => {
            let input = &project.input;
            let pattern = tuple(&results[&symbol_name(input)].heading);
            quote! { for #pattern in scan #input { emit #emit } }
        }
        ra::Operator::Semijoin(join) => {
            let (left, right) = (&join.left, &join.right);
            let left_heading = &results[&symbol_name(left)].heading;
            let right_heading = &results[&symbol_name(right)].heading;
            let shared: Vec<String> = right_heading
                .iter()
                .filter(|a| left_heading.contains(a))
                .cloned()
                .collect();
            let right_columns = columns(&ordinals_of(right_heading, &shared));
            let pattern = tuple(left_heading);
            let keys = tuple(&shared);
            quote! { for #pattern in scan #left { if contains #right #right_columns #keys { emit #emit } } }
        }
        ra::Operator::NaturalJoin(join) => {
            let (left, right) = (&join.left, &join.right);
            let left_heading = &results[&symbol_name(left)].heading;
            let right_heading = &results[&symbol_name(right)].heading;
            let shared: Vec<String> = right_heading
                .iter()
                .filter(|a| left_heading.contains(a))
                .cloned()
                .collect();
            let right_payload: Vec<String> = right_heading
                .iter()
                .filter(|a| !shared.contains(a))
                .cloned()
                .collect();
            if policy.merge_joins() {
                let left_payload: Vec<String> = left_heading
                    .iter()
                    .filter(|a| !shared.contains(a))
                    .cloned()
                    .collect();
                let left_columns = columns(&ordinals_of(left_heading, &shared));
                let right_columns = columns(&ordinals_of(right_heading, &shared));
                let (lp, rp, key) = (tuple(&left_payload), tuple(&right_payload), tuple(&shared));
                quote! { co (#lp in order #left #left_columns, #rp in order #right #right_columns) on #key { emit #emit } }
            } else {
                let right_columns = columns(&ordinals_of(right_heading, &shared));
                let (pattern, payload, keys) =
                    (tuple(left_heading), tuple(&right_payload), tuple(&shared));
                quote! { for #pattern in scan #left { for #payload in seek #right #right_columns #keys { emit #emit } } }
            }
        }
    }
}

fn ordinals_of(heading: &[String], attributes: &[String]) -> Vec<usize> {
    attributes
        .iter()
        .map(|a| {
            heading
                .iter()
                .position(|h| h == a)
                .expect("attribute in heading")
        })
        .collect()
}

/// IX → Stream correspondence: IX preserved, the Stream contract holds, and
/// the program is exactly the kernel's realization under `policy`.
pub fn check(source: &ix::Module, target: &stream::Module, policy: &Policy) -> bool {
    target.index_requirements == *source
        && stream::contract::well_formed(target)
        && compile(source, policy).is_ok_and(|expected| expected == *target)
}
