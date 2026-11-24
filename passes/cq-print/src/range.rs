//! Constant/parameter windows use the recorded native range or ordered read,
//! or a scan, never a build.
use crate::{Context, expressions::term_reference};
use cq_lang_structures::syntax::{self, symbol_name};
use proc_macro2::TokenStream;
use quote::quote;

impl Context<'_> {
    /// Formal rule IF-Range: a recorded native `range` read seeks, a native
    /// `ordered` read cuts a window; other reads scan/filter with no build.
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
        let columns = crate::columns_used(&select.condition)
            .into_iter()
            .collect::<Vec<_>>();
        let pattern = self.bind_columns(&input, &columns, true);
        let rows = self.profile_stream(&input, self.host_scan(&input));
        let rows = self.profile_input(name, rows);
        let filtered = quote!(::core::iter::Iterator::filter(#rows, move |#[allow(unused_variables)] #pattern| ::cq_storage::residual(#condition)));
        let access = self.host_access(
            &input,
            Some(cq_lang_structures::ReaderRef::Result(name.into())),
            std::slice::from_ref(&column),
        );
        let Some(native) = &access.native else {
            return Some(filtered);
        };
        let ty = &self.infos[&input].types[&column];
        let host = &access.value;
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
        let fields = access.key.iter().chain(&access.payload).map(|c| {
            if self.carried(&input).contains(c) {
                self.access(&input, c)
            } else {
                quote!(_)
            }
        });
        let row = self.carrier(&input, |c| self.access(&input, c));
        let window = if native.read == syntax::HostRead::Range {
            quote!(::cq_storage::dispatch::range::<_, (&#ty,)>(#host, #low, #high))
        } else {
            quote!(::cq_storage::dispatch::window::<_, (&#ty,)>(&#host, #low, #high))
        };
        let read = quote!(::core::iter::Iterator::map(#window, move |(#(#fields,)*)| #row));
        let read = self.profile_stream(&input, read);
        let read = self.profile_input(name, read);
        Some(
            quote!(::core::iter::Iterator::filter(#read, move |#[allow(unused_variables)] #pattern| ::cq_storage::residual(#condition))),
        )
    }
}
