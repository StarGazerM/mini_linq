//! GPU Round physical choice, under the publication's set-support contract.
//! The certificate binds CQ occurrence names after Storage's permutations and
//! scan substitutions, not a rule label, ordinal in a READS list, or Rust type
//! alias spelling. It travels in the terminal invocation alongside its arena.
use cq_lang_datalog_round::{self as round, Admission, HoleInterface, Program, RuleHole};
use cq_lang_structures::Provider;
use proc_macro2::TokenStream;
use quote::{ToTokens, quote};
use std::collections::BTreeMap;
use syn::{Ident, Token, parse::{Parse, ParseStream}};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct UnionProject {
    pub driver: Ident,
    pub lookup: Ident,
    pub witness_second: bool,
    pub group_first: bool,
}

impl UnionProject {
    /// Formal rule DL-UnionProject: only the support of this whole rule is
    /// consumed; its exactly two binary accesses share just the RHS prefix,
    /// and its declared output columns are precisely the other two variables.
    pub(crate) fn recognize(p: &Program, rule: &RuleHole, providers: &BTreeMap<String, Provider>) -> Option<Self> {
        let h = &p.holes[rule.hole];
        let q = &h.cq.query;
        if h.interface != HoleInterface::Batch || rule.known != Admission::Admit ||
            p.context.observer.is_some() || !q.captures.items.is_empty() || q.body.len() != 2 {
            return None;
        }
        let mut body = q.body.iter();
        let (round::cq::BodyItem::Positive { atom: a }, round::cq::BodyItem::Positive { atom: b }) =
            (body.next()?, body.next()?) else { return None };
        // The first physical atom need not be the changing occurrence after
        // an authored permutation. Bind the driver by its checked origin;
        // seeds have no delta and retain their chosen scan. The prefix test
        // below also applies to the reversed lookup; never assume an index.
        let planning = h.planning.as_ref()?;
        let (a, b) = match planning.delta {
            Some(d) if planning.order.get(1) == Some(&d) => (b, a),
            Some(d) if planning.order.first() != Some(&d) => return None,
            _ => (a, b),
        };
        let schema = |name: &Ident| {
            let input = providers.get(&name.to_string())?;
            (input.token == crate::storage::RADIX_RELATION).then_some(input.input.as_ref()?)
        };
        let (left, right) = (schema(&a.relation)?, schema(&b.relation)?);
        let head = &p.storage.device.as_ref()?.relations.get(rule.head)?.input;
        if left.columns.len() != 2 || right.columns.len() != 2 || head.columns.len() != 2 {
            return None;
        }
        let vars = |args: &cq_syntax::CommaList<syn::Expr>, head: bool| -> Option<[String; 2]> {
            if args.len() != 2 { return None; }
            let name = |expr: &syn::Expr| {
                // A positive binder is &T. In a head, the native copy *v
                // projects that same scalar. No user expression is evaluated.
                let term = match expr {
                    syn::Expr::Unary(u) if head && matches!(u.op, syn::UnOp::Deref(_)) => &u.expr,
                    other => other,
                };
                cq_syntax::rust::variable(term).map(cq_syntax::symbol_name)
            };
            Some([name(&args[0])?, name(&args[1])?])
        };
        let (av, bv, out) = (vars(&a.args, false)?, vars(&b.args, false)?, vars(&q.head.args, true)?);
        let i = av.iter().position(|v| *v == bv[0])?;
        let g = &av[1 - i];
        if av[0] == av[1] || bv[0] == bv[1] || g == &bv[1] { return None; }
        let first = out[0] == *g && out[1] == bv[1];
        if !first && !(out[1] == *g && out[0] == bv[1]) { return None; }
        // Equality includes logical identity and codec ID/type, not just lane width.
        if left.columns[i] != right.columns[0] || left.columns[1 - i] != head.columns[usize::from(!first)] ||
            right.columns[1] != head.columns[usize::from(first)] { return None; }
        Some(Self { driver: a.relation.clone(), lookup: b.relation.clone(), witness_second: i == 1, group_first: first })
    }
}

impl ToTokens for UnionProject {
    fn to_tokens(&self, out: &mut TokenStream) {
        let Self { driver, lookup, witness_second, group_first } = self;
        out.extend(quote!(union_project(#driver, #lookup, #witness_second, #group_first);));
    }
}

// The ordinary terminal remains the else arm and owns its original argument.
struct Choice(Option<UnionProject>, TokenStream);
impl Parse for Choice {
    fn parse(s: ParseStream<'_>) -> syn::Result<Self> {
        let marker = s.peek(Ident) && s.fork().parse::<Ident>()? == "union_project";
        let plan = if marker {
            s.parse::<Ident>()?;
            let a; syn::parenthesized!(a in s);
            let driver = a.parse()?; a.parse::<Token![,]>()?;
            let lookup = a.parse()?; a.parse::<Token![,]>()?;
            let witness_second = a.parse::<syn::LitBool>()?.value; a.parse::<Token![,]>()?;
            let group_first = a.parse::<syn::LitBool>()?.value;
            if !a.is_empty() { return Err(a.error("unexpected UnionProject operands")); }
            s.parse::<Token![;]>()?;
            Some(UnionProject { driver, lookup, witness_second, group_first })
        } else { None };
        Ok(Self(plan, s.parse()?))
    }
}

pub(crate) fn split(args: &TokenStream) -> syn::Result<(Option<UnionProject>, TokenStream)> {
    let Choice(plan, ordinary) = syn::parse2(args.clone())?;
    Ok((plan, ordinary))
}

pub(crate) fn plans(program: &Program, providers: &BTreeMap<String, Provider>) -> BTreeMap<usize, UnionProject> {
    program.context.strata.iter().flat_map(|s| s.seeds.iter().chain(&s.steps))
        .filter_map(|r| UnionProject::recognize(program, r, providers).map(|plan| (r.hole, plan))).collect()
}
