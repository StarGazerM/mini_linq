//! Structures → Stream: realize every RA result as a typed stream program.
//!
//! The kernel owns one rule per RA operator and is structure-directed: which
//! rule applies follows from the operations the operands' chosen structures
//! provide. Both operands ordered on the shared columns → co-iteration; the
//! right operand seekable → probe; a reducer with membership → guard; a
//! comparison on a column the host keeps ordered → bounded scan. Nothing is
//! chosen here that Structures did not make admissible, and nothing falls
//! back: an operator no structure serves is an error.
//!
//! Modes follow from use: a result read once through `scan` is a pipe; a
//! result read more than once, or through `seek`/`contains`/`order`, is
//! materialized with exactly the operations its readers require. The policy
//! contributes only the semijoin strategy.

use std::collections::{BTreeMap, BTreeSet};

use cq_ir::capabilities::Operation;
use cq_ir::policy::Policy;
use cq_ir::{relational_plan as ra, stream, structures, symbol_name};
use proc_macro2::{Span, TokenStream};
use quote::{format_ident, quote};
use syn::Ident;

/// The access rule the kernel derived for one binary operator.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Rule {
    /// `for t in l { for u in seek r[C](t.C) … }`
    Seek,
    /// `co (t in order l[Cl], u in order r[Cr]) on C …`
    Merge,
    /// `for t in l { if contains r[C](t.C) … }`
    Contains,
}

/// Facts about one RA result used to choose its mode and body.
struct Result_ {
    name: String,
    heading: Vec<String>,
    /// Readers in RA order, each with the operation it needs on this result.
    needs: Vec<(String, Operation)>,
    bag: bool,
}

/// Lower Structures to Stream, then check the Stream contract.
pub fn compile(source: &structures::Module, policy: &Policy) -> syn::Result<stream::Module> {
    let analysis = structures::contract::check(source)?;
    let ra_module = &source.index_requirements.relational_plan;
    let plan = &ra_module.plan;
    let headings = &analysis.headings;
    let provided = |name: &str| -> &[Operation] {
        analysis
            .provided
            .get(name)
            .map(Vec::as_slice)
            .unwrap_or(&[])
    };
    let serves = |name: &str, op: &Operation| provided(name).iter().any(|p| p.satisfies(op));

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
    let mut rules: BTreeMap<String, Rule> = BTreeMap::new();
    let mut windows: BTreeMap<String, Window> = BTreeMap::new();
    let ordinals = |relation: &str, attributes: &[String]| -> Vec<usize> {
        ordinals_of(&headings[relation], attributes)
    };

    for definition in &plan.definitions {
        let reader = symbol_name(&definition.result);
        let mut need = |operand: &Ident, operation: Operation| {
            let name = symbol_name(operand);
            if let Some(result) = results.get_mut(&name) {
                result.needs.push((reader.clone(), operation));
            }
        };
        match &definition.operator {
            ra::Operator::Unit(_) | ra::Operator::Rename(_) => {}
            ra::Operator::NaturalJoin(join) => {
                let (l, r) = (symbol_name(&join.left), symbol_name(&join.right));
                let shared = shared_attributes(headings, &l, &r);
                let (cl, cr) = (ordinals(&l, &shared), ordinals(&r, &shared));
                if serves(&l, &Operation::Ordered(cl.clone()))
                    && serves(&r, &Operation::Ordered(cr.clone()))
                {
                    need(&join.left, Operation::Ordered(cl));
                    need(&join.right, Operation::Ordered(cr));
                    rules.insert(reader.clone(), Rule::Merge);
                } else if serves(&r, &Operation::Seek(cr.clone())) {
                    need(&join.left, Operation::Iter);
                    need(&join.right, Operation::Seek(cr));
                    rules.insert(reader.clone(), Rule::Seek);
                } else {
                    return Err(syn::Error::new_spanned(
                        &definition.result,
                        format!(
                            "no chosen structure serves the join `{reader}`: `{r}` needs seek or both operands ordered on {shared:?}"
                        ),
                    ));
                }
            }
            ra::Operator::Semijoin(join) => {
                let (l, r) = (symbol_name(&join.left), symbol_name(&join.right));
                let shared = shared_attributes(headings, &l, &r);
                let cr = ordinals(&r, &shared);
                if !serves(&r, &Operation::Contains(cr.clone())) {
                    return Err(syn::Error::new_spanned(
                        &definition.result,
                        format!(
                            "no chosen structure serves the semijoin `{reader}`: `{r}` needs membership on {shared:?}"
                        ),
                    ));
                }
                need(&join.left, Operation::Iter);
                need(&join.right, Operation::Contains(cr));
                rules.insert(reader.clone(), Rule::Contains);
            }
            ra::Operator::Select(select) => {
                let input = symbol_name(&select.input);
                match bounded_scan(&select.condition, &headings[&input]) {
                    Some(window)
                        if analysis.aliases.contains_key(&input)
                            && serves(&input, &Operation::Ordered(vec![window.column])) =>
                    {
                        need(&select.input, Operation::Ordered(vec![window.column]));
                        windows.insert(reader.clone(), window);
                    }
                    _ => need(&select.input, Operation::Iter),
                }
            }
            ra::Operator::Project(p) => need(&p.input, Operation::Iter),
            ra::Operator::Extend(e) => need(&e.input, Operation::Iter),
        }
    }
    let output = symbol_name(&plan.output.input);
    results
        .get_mut(&output)
        .expect("output names a result")
        .needs
        .push((stream::RETURN.to_string(), Operation::Iter));

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

    let mut definitions = TokenStream::new();
    for definition in &plan.definitions {
        let name = symbol_name(&definition.result);
        let result = &results[&name];
        let ident = &definition.result;
        if let (ra::Operator::Rename(rename), Some(host)) =
            (&definition.operator, analysis.aliases.get(&name))
        {
            // No structure of its own: the host value stands for the rename.
            let unserved = result.needs.iter().find(|(_, need)| !serves(&name, need));
            if let Some((reader, need)) = unserved {
                return Err(syn::Error::new_spanned(
                    &rename.relation,
                    format!("host `{host}` must provide {need:?} for `{reader}` to read `{name}`"),
                ));
            }
            let host = &rename.relation;
            definitions.extend(quote! { #ident = alias #host; });
            continue;
        }
        let mode = mode(result);
        let lazy = lazy(policy, definition, result, plan);
        let body = body(definition, &results, rules.get(&name), windows.get(&name));
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
        stream { #definitions return #return_name; }
    })?;
    stream::contract::check(&target)?;
    Ok(target)
}

fn shared_attributes(
    headings: &BTreeMap<String, Vec<String>>,
    left: &str,
    right: &str,
) -> Vec<String> {
    let left = &headings[left];
    headings[right]
        .iter()
        .filter(|a| left.contains(a))
        .cloned()
        .collect()
}

fn mode(result: &Result_) -> TokenStream {
    let materialized =
        result.needs.len() > 1 || result.needs.iter().any(|(_, op)| *op != Operation::Iter);
    if !materialized {
        return quote!(pipe);
    }
    let name = ident(&result.name);
    // Keep only the strongest operation per column set: `ordered ⊒ seek ⊒ contains`.
    let mut provided = BTreeSet::new();
    for (_, operation) in &result.needs {
        provided.insert(operation.clone());
    }
    let all: Vec<Operation> = provided.iter().cloned().collect();
    provided.retain(|op| !all.iter().any(|other| other != op && other.satisfies(op)));
    let capabilities = provided.iter().map(|op| match op {
        Operation::Iter => quote!(iter),
        Operation::Contains(c) => {
            let c = c.iter().map(|&i| syn::Index::from(i));
            quote!(contains @#name[#(#c),*])
        }
        Operation::Seek(c) => {
            let c = c.iter().map(|&i| syn::Index::from(i));
            quote!(seek @#name[#(#c),*])
        }
        Operation::Ordered(c) => {
            let c = c.iter().map(|&i| syn::Index::from(i));
            quote!(ordered @#name[#(#c),*])
        }
    });
    quote!(let provides { #(#capabilities),* })
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
    results: &BTreeMap<String, Result_>,
    rule: Option<&Rule>,
    window: Option<&Window>,
) -> TokenStream {
    let name = symbol_name(&definition.result);
    let heading = &results[&name].heading;
    let emit = tuple(heading);
    let heading_of = |input: &Ident| -> Vec<String> {
        results
            .get(&symbol_name(input))
            .map(|r| r.heading.clone())
            .expect("operands are results")
    };
    match &definition.operator {
        ra::Operator::Unit(_) => quote! { unit { emit () } },
        ra::Operator::Rename(rename) => {
            // A rename with a structure of its own scans the host, binding
            // each source column (host sorted order) to its query variable.
            let source = &rename.relation;
            let mapping: BTreeMap<String, String> = rename
                .mapping
                .iter()
                .map(|m| (symbol_name(&m.source), symbol_name(&m.target)))
                .collect();
            let mut source_columns: Vec<String> = mapping.keys().cloned().collect();
            source_columns.sort();
            let pattern: Vec<String> = source_columns.iter().map(|c| mapping[c].clone()).collect();
            let pattern = tuple(&pattern);
            quote! { for #pattern in scan #source { emit #emit } }
        }
        ra::Operator::Select(select) => {
            let input = &select.input;
            let pattern = tuple(&heading_of(input));
            let condition = &select.condition;
            match window {
                Some(window) => {
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
                None => quote! { for #pattern in scan #input { if (#condition) { emit #emit } } },
            }
        }
        ra::Operator::Extend(extend) => {
            let input = &extend.input;
            let pattern = tuple(&heading_of(input));
            let fresh: Vec<_> = extend.columns.iter().map(|c| &c.name).collect();
            let value = &extend.value;
            quote! { for #pattern in scan #input { let (#(#fresh,)*) = #value; emit #emit } }
        }
        ra::Operator::Project(project) => {
            let input = &project.input;
            let pattern = tuple(&heading_of(input));
            quote! { for #pattern in scan #input { emit #emit } }
        }
        ra::Operator::Semijoin(join) => {
            let (left, right) = (&join.left, &join.right);
            let (left_heading, right_heading) = (heading_of(left), heading_of(right));
            let shared: Vec<String> = right_heading
                .iter()
                .filter(|a| left_heading.contains(a))
                .cloned()
                .collect();
            let right_columns = columns(&ordinals_of(&right_heading, &shared));
            let pattern = tuple(&left_heading);
            let keys = tuple(&shared);
            quote! { for #pattern in scan #left { if contains #right #right_columns #keys { emit #emit } } }
        }
        ra::Operator::NaturalJoin(join) => {
            let (left, right) = (&join.left, &join.right);
            let (left_heading, right_heading) = (heading_of(left), heading_of(right));
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
            match rule.expect("joins have a derived rule") {
                Rule::Merge => {
                    let left_payload: Vec<String> = left_heading
                        .iter()
                        .filter(|a| !shared.contains(a))
                        .cloned()
                        .collect();
                    let left_columns = columns(&ordinals_of(&left_heading, &shared));
                    let right_columns = columns(&ordinals_of(&right_heading, &shared));
                    let (lp, rp, key) =
                        (tuple(&left_payload), tuple(&right_payload), tuple(&shared));
                    quote! { co (#lp in order #left #left_columns, #rp in order #right #right_columns) on #key { emit #emit } }
                }
                Rule::Seek => {
                    let right_columns = columns(&ordinals_of(&right_heading, &shared));
                    let (pattern, payload, keys) =
                        (tuple(&left_heading), tuple(&right_payload), tuple(&shared));
                    quote! { for #pattern in scan #left { for #payload in seek #right #right_columns #keys { emit #emit } } }
                }
                Rule::Contains => unreachable!("joins are not membership rules"),
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

/// Structures → Stream correspondence: Structures preserved, the Stream
/// contract holds, and the program is exactly the kernel's realization.
pub fn check(source: &structures::Module, target: &stream::Module, policy: &Policy) -> bool {
    target.structures == *source
        && stream::contract::well_formed(target)
        && compile(source, policy).is_ok_and(|expected| expected == *target)
}
