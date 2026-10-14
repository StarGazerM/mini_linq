//! Constant/parameter windows use ordered hosts or a scan, never a build.
use crate::{Context, expressions::term_reference};
use cq_lang_structures::syntax::{self, symbol_name};
use proc_macro2::TokenStream;
use quote::quote;

impl Context<'_> {
    /// Formal rule IF-Range: Leading ordered host windows are direct; other reads scan/filter with no build.
    pub fn host_range(&self, name: &str, select: &syntax::algebra::Select) -> Option<TokenStream> {
        let input = symbol_name(&select.input);
        if !self.analysis.host_read(&input) {
            return None;
        }
        let (column, bounds) = select
            .condition
            .range_bounds()
            .expect("checked range demand");
        let condition = self.relational_condition(&select.condition);
        let pattern = self.borrowed_pattern(&input);
        let rows = self.profile_stream(&input, self.host_scan(&input));
        let rows = self.profile_input(name, rows);
        let filtered = quote!(::core::iter::Iterator::filter(#rows, move |#[allow(unused_variables)] #pattern| ::cq_storage::residual(#condition)));
        let access = self.host_access(&input, std::slice::from_ref(&column));
        if !access.prefix {
            return Some(filtered);
        }
        if bounds.iter().any(|b| matches!(&b.value, syntax::constraint::Term::Literal(l) if matches!(l.as_ref(), syn::Expr::Lit(syn::ExprLit { lit: syn::Lit::Str(_), .. })))) {
            return Some(filtered);
        }
        let ty = &self.infos[&input].types[&column];
        let host = &access.value;
        let fallback_ty = self.fallback_type(&input, name, &access.key);
        let bound = |lower: bool| {
            let mut result = quote!(::core::ops::Bound::Unbounded);
            for b in bounds.iter().filter(|b| b.lower == lower) {
                let v = term_reference(&b.value, &self.parameters);
                let entry = if b.inclusive {
                    quote!(::core::ops::Bound::Included((#v,)))
                } else {
                    quote!(::core::ops::Bound::Excluded((#v,)))
                };
                result = if lower {
                    quote!(::cq_storage::intersect_lower(#result, #entry))
                } else {
                    quote!(::cq_storage::intersect_upper(#result, #entry))
                };
            }
            result
        };
        let low = bound(true);
        let high = bound(false);
        let fields = access
            .key
            .iter()
            .chain(&access.payload)
            .map(|c| self.ident(&input, c));
        let row = self.pattern(name);
        let read = quote!({
            use ::cq_storage::dispatch::RangeRead as _;
            ::core::iter::Iterator::map(
                (&&&::cq_storage::dispatch::RangeHost::<_, (&#ty,), _>::new(
                    #host, move || { let __cq_scan: #fallback_ty = ::core::iter::Iterator::map(::cq_storage::rows::Rows::rows(#host), ::core::convert::identity); __cq_scan }
                )).range(#low, #high),
                move |(#(#fields,)*)| #row
            )
        });
        let read = self.profile_stream(&input, read);
        let read = self.profile_input(name, read);
        Some(
            quote!(::core::iter::Iterator::filter(#read, move |#[allow(unused_variables)] #pattern| ::cq_storage::residual(#condition))),
        )
    }
}
