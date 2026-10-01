//! A constant-supplied keyed read makes one probe and never builds a structure.
use proc_macro2::TokenStream;
use quote::quote;

use crate::{Context, expressions::relational_condition};
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
        let keys = select
            .condition
            .constant_keys()
            .expect("checked const demand");
        let condition = relational_condition(&select.condition);
        let pattern = self.borrowed_pattern(&input_name);
        let access = self.host_access(&input_name, &keys.keys().cloned().collect::<Vec<_>>());
        let rows = self.host_scan(&input_name);
        let filtered = quote!(::core::iter::Iterator::filter(#rows, move |#[allow(unused_variables)] #pattern| #condition));
        if !access.prefix {
            return Some(filtered);
        }
        let host = &access.value;
        let types = access.key.iter().map(|c| &self.infos[&input_name].types[c]);
        let values = access.key.iter().map(|c| &keys[c]);
        let fields = access
            .key
            .iter()
            .chain(&access.payload)
            .map(|c| self.ident(&input_name, c));
        let row = self.pattern(name);
        let looked_up = quote!({
            use ::cq_storage::dispatch::PointRead as _;
            ::core::iter::Iterator::map(
                (&&::cq_storage::dispatch::PointHost::<_, (#(#types,)*), _>::new(
                    #host, move || ::cq_storage::rows::Rows::rows(#host)
                )).point(&(#(#values,)*)),
                move |(#(#fields,)*)| #row
            )
        });
        // Retain every conjunct even when repeated columns give conflicting keys.
        Some(
            quote!(::core::iter::Iterator::filter(#looked_up, move |#[allow(unused_variables)] #pattern| #condition)),
        )
    }
}
