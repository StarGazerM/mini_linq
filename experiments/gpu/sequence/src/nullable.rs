//! Refine resident optional fields to ordinary scalar relations before planning.
//! Thus all provider permutations, dispatch stores and Rust read sets see both
//! lanes, without an Option-specific path in any index or kernel emitter.
use cq_lang_cq::syntax as cq_syntax;
use cq_lang_cq::{Atom, BodyItem, CommaList, Program};
use proc_macro2::{TokenStream, TokenTree};
use quote::{format_ident, quote, ToTokens};
use std::collections::{BTreeMap, BTreeSet};
use syn::{Expr, Ident, Type};

pub use cq_lang_sequence::gpu::publication::{check_presence_slots as check, presence_slots};

pub fn row_type(types: &[&Type], presence: &[usize]) -> TokenStream {
    let values = types.iter().enumerate().map(|(i, ty)| {
        if presence.contains(&i) { quote!(i64) } else { quote!(#ty) }
    });
    let flags = presence.iter().map(|_| quote!(bool));
    quote!((#(#values,)* #(#flags,)*))
}

/// Expand a logical host row only for the independent Sequential plan's input.
pub fn pack(width: usize, presence: &[usize]) -> TokenStream {
    let values = (0..width).map(|i| {
        let slot = syn::Index::from(i);
        if presence.contains(&i) {
            quote!(::cq_gpu_sequence_runtime::group::Nullable::value_lane(&row.#slot))
        } else {
            quote!(row.#slot.clone())
        }
    });
    let flags = presence.iter().map(|&i| {
        let slot = syn::Index::from(i);
        quote!(::cq_gpu_sequence_runtime::group::Nullable::present(&row.#slot))
    });
    quote!((#(#values,)* #(#flags,)*))
}

/// Restore logical tuples at an explicit host boundary. `flags` maps logical
/// output columns to the positions of the physical presence fields.
pub fn unpack(types: &[&Type], flags: &BTreeMap<usize, usize>) -> TokenStream {
    let values = types.iter().enumerate().map(|(i, ty)| {
        let slot = syn::Index::from(i);
        if let Some(&flag) = flags.get(&i) {
            let flag = syn::Index::from(flag);
            quote!(<#ty as ::cq_gpu_sequence_runtime::group::Nullable>::from_lanes(
                *::core::borrow::Borrow::<i64>::borrow(&row.#slot),
                *::core::borrow::Borrow::<bool>::borrow(&row.#flag)))
        } else {
            quote!(<#ty as ::core::clone::Clone>::clone(::core::borrow::Borrow::<#ty>::borrow(&row.#slot)))
        }
    });
    quote!((#(#values,)*))
}

struct Fresh {
    taken: BTreeSet<String>,
    variables: BTreeMap<String, Ident>,
}
impl Fresh {
    fn new(tokens: TokenStream) -> Self {
        fn collect(tokens: TokenStream, taken: &mut BTreeSet<String>) {
            for token in tokens {
                match token {
                    TokenTree::Ident(id) => { taken.insert(cq_syntax::symbol_name(&id)); }
                    TokenTree::Group(g) => collect(g.stream(), taken),
                    _ => (),
                }
            }
        }
        let mut names = BTreeSet::new();
        collect(tokens, &mut names);
        Self { taken: names, variables: BTreeMap::new() }
    }
    fn flag(&mut self, name: &Ident) -> Ident {
        let mut id = format!("__cq_present_{}", cq_syntax::symbol_name(name));
        while !self.taken.insert(id.clone()) { id.push('_'); }
        format_ident!("{id}")
    }
    // Shared block outputs must have the same physical name in every arm.
    fn variable(&mut self, id: &Ident) -> Ident {
        let name = cq_syntax::symbol_name(id);
        if let Some(existing) = self.variables.get(&name) { return existing.clone(); }
        let flag = self.flag(id);
        self.variables.insert(name, flag.clone());
        flag
    }
}

type Vars = BTreeMap<String, (Ident, Type)>;
type Fields = BTreeMap<String, Vec<(usize, Type)>>;
fn restore(expression: &mut Expr, vars: &Vars) {
    let reads = cq_syntax::rust::free_variables(expression);
    let mut bindings = TokenStream::new();
    for (name, (flag, ty)) in vars {
        if reads.opaque || reads.names.contains(name) {
            let id = format_ident!("{name}");
            bindings.extend(quote!(
                let #id = &<#ty as ::cq_gpu_sequence_runtime::group::Nullable>::from_lanes(*#id, *#flag);
            ));
        }
    }
    if !bindings.is_empty() {
        *expression = syn::parse_quote!({ #bindings #expression });
    }
}

/// Input refinement returns the forwarded presence slots in the native head;
/// genuine extremum presence is emitted by the aggregate, not a body variable.
pub fn lower<'a>(
    program: &mut Program,
    resident_inputs: impl IntoIterator<Item = (&'a str, &'a [bool], &'a [usize])>,
) -> syn::Result<BTreeMap<usize, usize>> {
    let mut schemas = BTreeMap::new();
    let mut fresh = None;
    for (name, nullable, presence) in resident_inputs {
        check(nullable, presence)?;
        if presence.is_empty() { continue; }
        let fresh = fresh.get_or_insert_with(|| Fresh::new(program.to_token_stream()));
        let decl = program.declarations.iter_mut().find(|d| d.name == name)
            .ok_or_else(|| syn::Error::new(proc_macro2::Span::call_site(), "DR-NullableLanes: missing input relation"))?;
        if decl.columns.len() != nullable.len() {
            return Err(syn::Error::new_spanned(decl, "DR-NullableLanes: stale input width"));
        }
        let mut slots = Vec::with_capacity(presence.len());
        for &slot in presence {
            let column = &mut decl.columns[slot];
            let flag = fresh.flag(&column.name);
            slots.push((slot, std::mem::replace(&mut column.ty, syn::parse_quote!(i64))));
            column.logical_type_id = None;
            decl.columns.push(syn::parse_quote!(#flag: bool));
        }
        schemas.insert(decl.name.to_string(), slots);
    }
    let mut forwarded = BTreeMap::new();
    let Some(mut fresh) = fresh else { return Ok(forwarded) };
    fn atom(atom: &mut Atom, schemas: &Fields, vars: &mut Vars, fresh: &mut Fresh) -> syn::Result<()> {
        let Some(slots) = schemas.get(&atom.relation.to_string()) else { return Ok(()) };
        for (i, ty) in slots {
            let arg = atom.args.get(*i).ok_or_else(|| syn::Error::new_spanned(&*atom, "nullable operand outside atom"))?;
            let arg = if let Some(id) = cq_syntax::rust::variable(arg) {
                let (flag, _) = vars.entry(id.to_string()).or_insert_with(|| (fresh.variable(id), ty.clone()));
                syn::parse_quote!(#flag)
            } else if matches!(arg, Expr::Infer(_)) {
                syn::parse_quote!(_)
            } else {
                return Err(syn::Error::new_spanned(arg, "nullable atom operand requires a binder or wildcard; use where for comparisons"));
            };
            atom.args.push(arg);
        }
        Ok(())
    }
    fn body(items: &mut CommaList<BodyItem>, schemas: &Fields, vars: &mut Vars, fresh: &mut Fresh) -> syn::Result<()> {
        for item in items {
            if let BodyItem::Equality(eq) = item {
                *item = BodyItem::Constraint(cq_syntax::constraint::Constraint::from_equality(eq));
            }
            match item {
            BodyItem::Positive { atom: a } => atom(a, schemas, vars, fresh)?,
            BodyItem::Negation(n) => atom(&mut n.atom, schemas, vars, fresh)?,
            BodyItem::Block(b) => {
                let mut exported: Option<Vars> = None;
                for branch in &mut b.branches {
                    let mut arm = vars.clone();
                    body(&mut branch.body, schemas, &mut arm, fresh)?;
                    match &mut exported {
                        Some(common) => common.retain(|name, binding| arm.get(name) == Some(binding)),
                        None => exported = Some(arm),
                    }
                }
                if let Some(common) = exported { *vars = common; }
            }
            BodyItem::Filter(f) => restore(&mut f.condition, vars),
            BodyItem::For(f) => restore(&mut f.expression, vars),
            BodyItem::Let(l) => restore(&mut l.expression, vars),
            BodyItem::Aggregate(aggregate) => {
                let mut inner = vars.clone();
                atom(&mut aggregate.atom, schemas, &mut inner, fresh)?;
                if aggregate.arguments.iter().any(|arg| inner.contains_key(&arg.to_string())) {
                    return Err(syn::Error::new_spanned(&aggregate.arguments, "nullable input to an opaque aggregator requires host rows"));
                }
            }
            BodyItem::Constraint(c) if c.variables().any(|v| vars.contains_key(&v.to_string())) => {
                // A closed operand is an owned callable capture, never a host
                // reference copied into device code. Take its borrow inside
                // the per-row expression. Repeated operands share one value.
                let mut parameters = BTreeMap::new();
                let mut bindings = TokenStream::new();
                for operand in c.parameters() {
                    parameters.entry(operand.to_token_stream().to_string()).or_insert_with(|| {
                        let value = fresh.flag(&format_ident!("operand"));
                        let reference = fresh.flag(&value);
                        bindings.extend(quote! {
                            let #value = (#operand).clone();
                            let #reference = &#value;
                        });
                        reference
                    });
                }
                let test = cq_print::expressions::relational_constraint(c, &parameters);
                let mut expr = syn::parse_quote!({ #bindings #test });
                restore(&mut expr, vars);
                *item = syn::parse_quote!(if #expr);
            }
            _ => (),
            }
        }
        Ok(())
    }
    let mut vars = Vars::new();
    body(&mut program.query.body, &schemas, &mut vars, &mut fresh)?;
    let width = program.query.head.args.len();
    let decl = program.declarations.iter_mut().find(|d| d.name == program.query.head.relation)
        .ok_or_else(|| syn::Error::new_spanned(&program.query.head, "nullable stage needs a declared output"))?;
    for i in 0..width {
        let expr = &mut program.query.head.args[i];
        let var = match expr {
            Expr::Unary(u) if matches!(u.op, syn::UnOp::Deref(_)) => cq_syntax::rust::variable(&u.expr),
            _ => cq_syntax::rust::variable(expr),
        };
        if let Some((flag, _)) = var.and_then(|v| vars.get(&v.to_string())) {
            forwarded.insert(i, program.query.head.args.len());
            program.query.head.args.push(syn::parse_quote!(#flag));
            let col = &mut decl.columns[i];
            let flag_col = fresh.flag(&col.name);
            col.ty = syn::parse_quote!(i64);
            col.logical_type_id = None;
            decl.columns.push(syn::parse_quote!(#flag_col: bool));
        } else if cq_lang_cq::group::aggregate(expr)?.is_some() {
            let Expr::Call(call) = expr else { unreachable!() };
            for arg in &mut call.args { restore(arg, &vars); }
        } else {
            restore(expr, &vars);
        }
    }
    cq_syntax::intern_logical_types(&mut program.declarations)?;
    Ok(forwarded)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn forwarded_alias_and_opaque_read_use_the_same_pair() {
        let mut p: Program = syn::parse_quote! {
            struct Query;
            relation R(k: u32, v: Maybe);
            relation answer(out: Maybe);
            answer(v) :- R(k, v), if v.is_none() || *v == Some(*k as i64);
        };
        let flags = lower(&mut p, [("R", &[false, true][..], &[1][..])]).unwrap();
        assert_eq!(flags, [(0, 1)].into_iter().collect());
        let query = &p.query;
        let expected: cq_lang_cq::Query = syn::parse_quote! {
            answer(v, __cq_present_v_) :- R(k, v, __cq_present_v_), if {
                let v = &<Maybe as ::cq_gpu_sequence_runtime::group::Nullable>::from_lanes(*v, *__cq_present_v_);
                v.is_none() || *v == Some(*k as i64)
            };
        };
        assert_eq!(query, &expected);
        let ty = p.inputs().next().unwrap().columns.iter().map(|c| &c.ty);
        assert_eq!(quote!(#(#ty),*).to_string(), quote!(u32, i64, bool).to_string());
    }

    #[test]
    fn type_aliases_do_not_hide_incomplete_or_stale_presence_layouts() {
        let program: Program = syn::parse_quote! {
            struct Query;
            relation R(a: Maybe, b: Maybe);
            relation answer(ok: bool);
            answer(true) :- R(x, _), if x.is_none();
        };
        for (nullable, presence) in [
            (&[true, true][..], &[1][..]),
            (&[true, true, false][..], &[0, 1][..]),
        ] {
            assert!(lower(&mut program.clone(), [("R", nullable, presence)]).is_err());
        }
    }

    #[test]
    fn nullable_where_reads_both_lanes() {
        let mut p: Program = syn::parse_quote! {
            struct Query;
            relation R(a: Maybe, b: Maybe);
            relation answer(ok: bool);
            answer(true) :- R(left, right), where left <= right;
        };
        lower(&mut p, [("R", &[true, true][..], &[0, 1][..])]).unwrap();
        let BodyItem::Filter(filter) = &p.query.body[1] else { panic!("expected logical comparison") };
        let reads = cq_syntax::rust::free_variables(&filter.condition).names;
        assert_eq!(reads, ["left", "right", "__cq_present_left", "__cq_present_right"]
            .map(String::from).into_iter().collect());
    }
}
