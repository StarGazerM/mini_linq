//! A constant-supplied keyed read makes one probe and never builds a structure.
use proc_macro2::TokenStream;
use quote::quote;

use crate::Context;
use cq_lang_structures::syntax::{self, symbol_name};

impl Context<'_> {
    /// Emit a host point read. Native leading-prefix capability dispatches to
    /// lookup; unsupported layouts/types scan and filter. Non-host inputs return
    /// `None` so the printer can consume their stream at its chain position.
    pub fn host_point(&self, name: &str, select: &syntax::algebra::Select) -> Option<TokenStream> {
        let input_name = symbol_name(&select.input);
        if !self.analysis.host_read(&input_name) {
            return None;
        }
        let keys = select.condition.constant_keys();
        let condition = self.relational_condition(&select.condition);
        let columns = crate::columns_used(&select.condition)
            .into_iter()
            .collect::<Vec<_>>();
        let pattern = self.bind_columns(&input_name, &columns, true);
        let rows = self.profile_stream(&input_name, self.host_scan(&input_name));
        let rows = self.profile_input(name, rows);
        let filtered = quote!(::core::iter::Iterator::filter(#rows, move |#[allow(unused_variables)] #pattern| ::cq_storage::residual(#condition)));
        let Some(keys) = keys else {
            return Some(filtered);
        };
        let access = self.host_access(&input_name, &keys.keys().cloned().collect::<Vec<_>>());
        if !access.prefix {
            return Some(filtered);
        }
        let host = &access.value;
        let fallback_ty = self.fallback_type(
            &input_name,
            cq_lang_structures::ReaderRef::Result(name.into()),
            &access.key,
        );
        let types = access.key.iter().map(|c| &self.infos[&input_name].types[c]);
        let values = access.key.iter().map(|c| &keys[c]);
        let fields = access.key.iter().chain(&access.payload).map(|c| {
            if self.carried(&input_name).contains(c) {
                self.access(&input_name, c)
            } else {
                quote!(_)
            }
        });
        let row = self.carrier(&input_name, |c| self.access(&input_name, c));
        let looked_up = quote!({
            use ::cq_storage::dispatch::PointRead as _;
            let __cq_key = (#(#values,)*);
            ::core::iter::Iterator::map(
                (&&::cq_storage::dispatch::PointHost::<_, (#(#types,)*), _, _>::new(
                    #host, &__cq_key, move || { let __cq_scan: #fallback_ty = ::core::iter::Iterator::map(::cq_storage::rows::Rows::rows(#host), ::core::convert::identity); __cq_scan }
                )).point(&__cq_key),
                move |(#(#fields,)*)| #row
            )
        });
        let looked_up = self.profile_stream(&input_name, looked_up);
        let looked_up = self.profile_input(name, looked_up);
        // Retain every conjunct even when repeated columns give conflicting keys.
        Some(
            quote!(::core::iter::Iterator::filter(#looked_up, move |#[allow(unused_variables)] #pattern| ::cq_storage::residual(#condition))),
        )
    }
}
