//! Dataset evidence is recorded input, never an instruction to select a plan.
//!
//! `declared_evidence "<JSON>" { R = "logical_R" full [a = "logical_a"]; }`
//! keeps the checked document and explicit column correspondence in the IR.
//! File acquisition is an explicit build-adapter operation, not parser I/O.
use crate::{RelationDecl, evidence, symbol_name};
use proc_macro2::TokenStream;
use quote::{ToTokens, quote};
use std::{collections::BTreeSet, path::Path, sync::Arc};
use syn::{Ident, LitStr, Token, parse::{Parse, ParseStream}};

pub mod kw {
    syn::custom_keyword!(declared_evidence);
}

/// Which relation version a compiler input denotes. A subset does not inherit
/// an equality statistic from its full source; costing must cite that premise.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EvidenceView {
    Full,
    Delta,
    Subset,
}
impl Parse for EvidenceView {
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        let name: Ident = input.parse()?;
        match name.to_string().as_str() {
            "full" => Ok(Self::Full),
            "delta" => Ok(Self::Delta),
            "subset" => Ok(Self::Subset),
            _ => Err(syn::Error::new_spanned(name, "expected full, delta, or subset evidence view")),
        }
    }
}
impl ToTokens for EvidenceView {
    fn to_tokens(&self, tokens: &mut TokenStream) {
        tokens.extend(match self {
            Self::Full => quote!(full),
            Self::Delta => quote!(delta),
            Self::Subset => quote!(subset),
        });
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ColumnBinding {
    pub column: Ident,
    pub source: String,
}
impl Parse for ColumnBinding {
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        let column = input.parse()?;
        input.parse::<Token![=]>()?;
        let source = input.parse::<LitStr>()?.value();
        Ok(Self { column, source })
    }
}
impl ToTokens for ColumnBinding {
    fn to_tokens(&self, tokens: &mut TokenStream) {
        let column = &self.column;
        let source = &self.source;
        tokens.extend(quote!(#column = #source));
    }
}

/// Explicit compiler-input to dataset relation/column correspondence.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RelationBinding {
    pub relation: Ident,
    pub source: String,
    pub view: EvidenceView,
    pub columns: Vec<ColumnBinding>,
}
impl Parse for RelationBinding {
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        let relation = input.parse()?;
        input.parse::<Token![=]>()?;
        let source = input.parse::<LitStr>()?.value();
        let view = input.parse()?;
        let columns;
        syn::bracketed!(columns in input);
        let columns = columns.parse_terminated(ColumnBinding::parse, Token![,])?
            .into_iter().collect();
        input.parse::<Token![;]>()?;
        Ok(Self { relation, source, view, columns })
    }
}
impl ToTokens for RelationBinding {
    fn to_tokens(&self, tokens: &mut TokenStream) {
        let Self { relation, source, view, columns } = self;
        tokens.extend(quote!(#relation = #source #view [#(#columns),*];));
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeclaredEvidence {
    /// Shared across normalized programs and per-variant holes.
    pub document: Arc<evidence::DatasetEvidence>,
    pub bindings: Vec<RelationBinding>,
}
impl DeclaredEvidence {
    pub fn from_json(json: &str, bindings: Vec<RelationBinding>) -> syn::Result<Self> {
        let document = evidence::parse_json(json).map_err(|error| {
            syn::Error::new(proc_macro2::Span::call_site(), error.to_string())
        })?;
        Ok(Self { document: Arc::new(document), bindings })
    }

    /// Build-script input. The same file is registered as a Cargo dependency;
    /// consumers pass this recorded value to the compiler, not a runtime path.
    pub fn load_build_input(path: &Path, bindings: Vec<RelationBinding>) -> syn::Result<Self> {
        let path = path.canonicalize().map_err(|error| {
            syn::Error::new(proc_macro2::Span::call_site(), format!("evidence {}: {error}", path.display()))
        })?;
        let path_text = path.to_str().ok_or_else(|| {
            syn::Error::new(proc_macro2::Span::call_site(), "evidence path must be UTF-8")
        })?;
        if path_text.contains('\n') || path_text.contains('\r') {
            return Err(syn::Error::new(proc_macro2::Span::call_site(), "evidence path contains a line break"));
        }
        println!("cargo:rerun-if-changed={path_text}");
        let json = std::fs::read_to_string(&path).map_err(|error| {
            syn::Error::new(proc_macro2::Span::call_site(), format!("evidence {path_text}: {error}"))
        })?;
        Self::from_json(&json, bindings)
    }

    /// Validate names and complete target-column coverage. Rust types remain
    /// opaque: this never invents a domain identity from a type spelling.
    pub fn check(&self, declarations: &[RelationDecl]) -> syn::Result<()> {
        self.document.check().map_err(|error| syn::Error::new_spanned(self, error.to_string()))?;
        let mut seen = BTreeSet::new();
        for binding in &self.bindings {
            let name = symbol_name(&binding.relation);
            if !seen.insert(name.clone()) {
                return Err(syn::Error::new_spanned(binding, "duplicate evidence input binding"));
            }
            let declaration = declarations.iter().find(|d| symbol_name(&d.name) == name)
                .ok_or_else(|| syn::Error::new_spanned(binding, "evidence binds an undeclared compiler relation"))?;
            let relation = self.document.relations.get(&binding.source)
                .ok_or_else(|| syn::Error::new_spanned(binding, "evidence binds an unknown dataset relation"))?;
            let mut targets = BTreeSet::new();
            let mut sources = BTreeSet::new();
            for column in &binding.columns {
                let target = symbol_name(&column.column);
                if !targets.insert(target.clone()) || !sources.insert(&column.source) {
                    return Err(syn::Error::new_spanned(column, "duplicate evidence column binding"));
                }
                if !declaration.columns.iter().any(|c| symbol_name(&c.name) == target)
                    || !relation.columns.contains_key(&column.source)
                {
                    return Err(syn::Error::new_spanned(column, "unknown compiler or dataset evidence column"));
                }
            }
            if targets.len() != declaration.columns.len() {
                return Err(syn::Error::new_spanned(binding, "evidence binding must cover every compiler input column"));
            }
        }
        Ok(())
    }

    /// Preserve a view's identity when execution renames its compiler input.
    pub fn rename_relation(&mut self, old: &Ident, new: &Ident) {
        for binding in &mut self.bindings {
            if binding.relation == *old {
                binding.relation = new.clone();
            }
        }
    }

    /// Project one recorded source binding to a renamed/reordered input view.
    /// Column names come from the checked source declaration, never Rust code.
    pub fn project_relation(
        &self,
        source: &Ident,
        target: &RelationDecl,
        view: EvidenceView,
    ) -> syn::Result<Option<RelationBinding>> {
        let Some(binding) = self.bindings.iter().find(|b| b.relation == *source) else {
            return Ok(None);
        };
        let columns = target.columns.iter().map(|column| {
            binding.columns.iter().find(|b| b.column == column.name).cloned()
                .ok_or_else(|| syn::Error::new_spanned(column, "projected input has no evidence column correspondence"))
        }).collect::<syn::Result<Vec<_>>>()?;
        let view = match (binding.view, view) {
            (EvidenceView::Subset, _) => EvidenceView::Subset,
            (EvidenceView::Delta, EvidenceView::Full) => EvidenceView::Delta,
            (_, requested) => requested,
        };
        Ok(Some(RelationBinding {
            relation: target.name.clone(), source: binding.source.clone(), view, columns,
        }))
    }
}
impl Parse for DeclaredEvidence {
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        input.parse::<kw::declared_evidence>()?;
        let json: LitStr = input.parse()?;
        let body;
        syn::braced!(body in input);
        let mut bindings = Vec::new();
        while !body.is_empty() {
            bindings.push(body.parse()?);
        }
        Self::from_json(&json.value(), bindings).map_err(|error| syn::Error::new_spanned(json, error))
    }
}
impl ToTokens for DeclaredEvidence {
    fn to_tokens(&self, tokens: &mut TokenStream) {
        // Serialization is total for this string/integer data model. Do not
        // call check here: even malformed records must print in diagnostics.
        let json = serde_json::to_string(self.document.as_ref()).expect("evidence is JSON-serializable");
        let bindings = &self.bindings;
        tokens.extend(quote!(declared_evidence #json { #(#bindings)* }));
    }
}
pub fn parse_optional(input: ParseStream<'_>) -> syn::Result<Option<DeclaredEvidence>> {
    if input.peek(kw::declared_evidence) { input.parse().map(Some) } else { Ok(None) }
}
