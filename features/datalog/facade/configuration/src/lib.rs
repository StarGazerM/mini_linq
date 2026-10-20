//! Datalog-owned named phases, resolved into the ordinary checked pass queue.
use cq_stage::{Configuration, Invocation, Pipeline};
use quote::quote;
use syn::parse::{Parse, ParseStream};
use syn::{Token, braced, bracketed};

mod kw {
    syn::custom_keyword!(pipeline);
}

/// Named Datalog pipeline choices. `None` inherits a default; an explicit empty
/// list disables that phase's defaults. CQ choices belong to the fork arguments.
#[derive(Clone, Debug, Default)]
pub struct DatalogConfiguration {
    pub surface: Option<Vec<Invocation>>,
    pub stratify: Option<Invocation>,
    pub incremental: Option<Invocation>,
    pub storage: Option<Invocation>,
    pub storage_rewrites: Option<Vec<Invocation>>,
    pub fork: Option<Invocation>,
    pub round: Option<Vec<Invocation>>,
    pub print: Option<Invocation>,
    pub cq: Option<Configuration>,
}

impl Parse for DatalogConfiguration {
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        input.parse::<kw::pipeline>()?;
        let body;
        braced!(body in input);
        let mut config = Self::default();
        let mut seen = std::collections::BTreeSet::new();
        while !body.is_empty() {
            let name: syn::Ident = body.parse()?;
            if !seen.insert(name.to_string()) {
                return Err(syn::Error::new_spanned(
                    name,
                    "duplicate Datalog pipeline phase",
                ));
            }
            body.parse::<Token![=]>()?;
            match name.to_string().as_str() {
                "stratify" => config.stratify = Some(body.parse()?),
                "incremental" => config.incremental = Some(body.parse()?),
                "storage" => config.storage = Some(body.parse()?),
                "fork" => config.fork = Some(body.parse()?),
                "print" => config.print = Some(body.parse()?),
                "cq" => config.cq = Some(body.parse()?),
                "surface" | "storage_rewrites" | "round" => {
                    let list;
                    bracketed!(list in body);
                    let entries = Some(
                        list.parse_terminated(Invocation::parse, Token![,])?
                            .into_iter()
                            .collect(),
                    );
                    match name.to_string().as_str() {
                        "surface" => config.surface = entries,
                        "storage_rewrites" => config.storage_rewrites = entries,
                        _ => config.round = entries,
                    }
                }
                _ => {
                    return Err(syn::Error::new_spanned(
                        name,
                        "unknown Datalog pipeline phase",
                    ));
                }
            }
            body.parse::<Token![;]>()?;
        }
        Ok(config)
    }
}

impl DatalogConfiguration {
    /// The base surface expander, checked translations and sequential printer.
    pub fn defaults() -> Self {
        let mut defaults: Self = syn::parse_quote! { pipeline {
            surface = [::cq_datalog::passes::surface_if_let];
            stratify = ::cq_datalog::passes::stratify;
            incremental = ::cq_datalog::passes::incremental;
            storage = ::cq_datalog::passes::storage;
            fork = ::cq_datalog::passes::fork;
            print = ::cq_datalog::passes::print_rounds;
        } };
        if cfg!(feature = "phase-timing") {
            defaults.fork.as_mut().unwrap().arguments = quote!(timing = phases;);
        }
        defaults
    }

    /// Resolve all phases and record the resolved CQ continuation in fork args.
    /// Replacement fork passes receive the same opaque `cq = pipeline { ... };`
    /// argument, after any arguments authored on their invocation.
    pub fn resolve(self, defaults: Self) -> syn::Result<Pipeline> {
        let missing = || {
            syn::Error::new(
                proc_macro2::Span::call_site(),
                "missing Datalog pipeline default",
            )
        };
        let cq = self
            .cq
            .or(defaults.cq)
            .unwrap_or_default()
            .resolve(cq_datalog_fork::cq_defaults())?;
        let mut fork = self.fork.or(defaults.fork).ok_or_else(missing)?;
        fork.arguments.extend(quote!(cq = #cq;));
        let mut passes = self.surface.or(defaults.surface).unwrap_or_default();
        passes.push(self.stratify.or(defaults.stratify).ok_or_else(missing)?);
        passes.push(
            self.incremental
                .or(defaults.incremental)
                .ok_or_else(missing)?,
        );
        passes.push(self.storage.or(defaults.storage).ok_or_else(missing)?);
        passes.extend(
            self.storage_rewrites
                .or(defaults.storage_rewrites)
                .unwrap_or_default(),
        );
        passes.push(fork);
        passes.extend(self.round.or(defaults.round).unwrap_or_default());
        passes.push(self.print.or(defaults.print).ok_or_else(missing)?);
        Ok(Pipeline { passes })
    }
}

#[cfg(test)]
mod tests;
