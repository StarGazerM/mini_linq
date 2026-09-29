//! An independent RA surface with theta_join. It produces IX directly; it
//! neither parses reference RA nor calls the reference index-policy pass.
use cq_lang_ix::{
    self as ix, Operation,
    syntax::{self, symbol_name},
};
use std::collections::{BTreeMap, BTreeSet};
use syn::{
    Ident, Token,
    parse::{Parse, ParseStream},
};

mod kw {
    syn::custom_keyword!(theta_join);
    syn::custom_keyword!(with);
    syn::custom_keyword!(output);
}
struct Binding {
    host: Ident,
    // Target attribute -> source column, preserving caller identifier spans.
    mapping: BTreeMap<Ident, Ident>,
}
impl Parse for Binding {
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        let host = input.parse()?;
        let content;
        syn::braced!(content in input);
        let mut mapping = BTreeMap::new();
        while !content.is_empty() {
            let source: Ident = content.parse()?;
            content.parse::<Token![->]>()?;
            let target: Ident = content.parse()?;
            if mapping.insert(target.clone(), source).is_some() {
                return Err(syn::Error::new_spanned(
                    target,
                    "duplicate target attribute",
                ));
            }
            if content.is_empty() {
                break;
            }
            content.parse::<Token![,]>()?;
        }
        Ok(Self { host, mapping })
    }
}

/// The dialect's checked query is the source environment for its IX section.
pub struct Query {
    inputs: Vec<syntax::RelationDecl>,
    operations: Vec<(Ident, Operation)>,
    headings: BTreeMap<String, Vec<String>>,
    head: syntax::Atom,
}
impl Parse for Query {
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        let mut inputs: Vec<syntax::RelationDecl> = Vec::new();
        while input.peek(syntax::kw::relation) {
            inputs.push(input.parse()?);
        }
        syntax::check_input_declarations(&inputs)?;
        input.parse::<kw::theta_join>()?;
        let left: Binding = input.parse()?;
        input.parse::<kw::with>()?;
        let right: Binding = input.parse()?;
        input.parse::<Token![if]>()?;
        let condition: syn::Expr = input.parse()?;
        input.parse::<Token![;]>()?;
        input.parse::<kw::output>()?;
        let head: syntax::Atom = input.parse()?;
        input.parse::<Token![;]>()?;
        let types = |binding: &Binding| -> syn::Result<BTreeMap<String, syn::Type>> {
            let relation = inputs
                .iter()
                .find(|r| r.name == binding.host)
                .ok_or_else(|| syn::Error::new_spanned(&binding.host, "undeclared host"))?;
            let mut types = BTreeMap::new();
            let mut sources = BTreeSet::new();
            for (target, source) in &binding.mapping {
                let column = relation
                    .columns
                    .iter()
                    .find(|c| c.name == *source)
                    .ok_or_else(|| syn::Error::new_spanned(source, "unknown source column"))?;
                if !sources.insert(symbol_name(source)) {
                    return Err(syn::Error::new_spanned(
                        source,
                        "source column renamed twice",
                    ));
                }
                types.insert(symbol_name(target), column.ty.clone());
            }
            if sources.len() != relation.columns.len() {
                return Err(syn::Error::new_spanned(
                    &binding.host,
                    "rename must cover every host column",
                ));
            }
            Ok(types)
        };
        let left_types = types(&left)?;
        let right_types = types(&right)?;
        for (name, ty) in &left_types {
            if right_types.get(name).is_some_and(|right| right != ty) {
                return Err(input.error("shared attributes need equal types"));
            }
        }
        let joined: BTreeSet<_> = left_types
            .keys()
            .chain(right_types.keys())
            .cloned()
            .collect();
        if !head.is_normalized() || head.variables().any(|v| !joined.contains(&symbol_name(v))) {
            return Err(syn::Error::new_spanned(
                &head,
                "output must name joined attributes",
            ));
        }
        let mut headings = BTreeMap::new();
        headings.insert("l".into(), left_types.into_keys().collect());
        headings.insert("r".into(), right_types.into_keys().collect());
        headings.insert("j".into(), joined.iter().cloned().collect());
        headings.insert("s".into(), joined.into_iter().collect());
        let kept: BTreeSet<_> = head.variables().map(symbol_name).collect();
        headings.insert("p".into(), kept.into_iter().collect());
        let operations = vec![
            (
                syn::parse_quote!(l),
                Operation::Rename {
                    host: left.host,
                    mapping: left.mapping,
                },
            ),
            (
                syn::parse_quote!(r),
                Operation::Rename {
                    host: right.host,
                    mapping: right.mapping,
                },
            ),
            (
                syn::parse_quote!(j),
                Operation::Join {
                    left: syn::parse_quote!(l),
                    right: syn::parse_quote!(r),
                },
            ),
            (
                syn::parse_quote!(s),
                Operation::Select {
                    input: syn::parse_quote!(j),
                    condition,
                },
            ),
            (
                syn::parse_quote!(p),
                Operation::Project {
                    input: syn::parse_quote!(s),
                },
            ),
        ];
        Ok(Self {
            inputs,
            operations,
            headings,
            head,
        })
    }
}
impl ix::RaEnv for Query {
    fn heading(&self, result: &str) -> Option<&[String]> {
        self.headings.get(result).map(Vec::as_slice)
    }
    fn joins(&self) -> Vec<ix::Join> {
        self.operations
            .iter()
            .filter_map(|(name, op)| match op {
                Operation::Join { left, right } => Some(ix::Join {
                    result: symbol_name(name),
                    left: symbol_name(left),
                    right: symbol_name(right),
                    semi: false,
                }),
                _ => None,
            })
            .collect()
    }
}
impl ix::Env for Query {
    fn inputs(&self) -> Vec<syntax::RelationDecl> {
        self.inputs.clone()
    }
    fn operations(&self) -> Vec<(Ident, Operation)> {
        self.operations.clone()
    }
    fn returned(&self) -> Ident {
        syn::parse_quote!(p)
    }
    fn head(&self) -> &syntax::Atom {
        &self.head
    }
}
impl Query {
    /// Own demand derivation: theta_join's natural-join component needs the
    /// right result indexed by its shared heading; selection adds no demand.
    pub fn indexes(&self) -> syn::Result<ix::Section> {
        let mut indexes = syntax::SemicolonList::new();
        let mut seen = BTreeSet::new();
        for (_, op) in &self.operations {
            let Operation::Join { left, right } = op else {
                continue;
            };
            let columns = ix::result_join_columns(
                &self.headings[&symbol_name(left)],
                &self.headings[&symbol_name(right)],
            );
            if !seen.insert((symbol_name(right), columns.clone())) {
                continue;
            }
            indexes.push(ix::IndexDefinition {
                name: Ident::new(&format!("theta_key_{}", indexes.len()), right.span()),
                eq_token: Default::default(),
                requirement: ix::IndexRequirement {
                    at_token: Default::default(),
                    relation: right.clone(),
                    bracket_token: Default::default(),
                    columns: columns.into_iter().map(syn::Index::from).collect(),
                },
            });
            indexes.push_punct(Default::default());
        }
        let section = ix::Section {
            indexes_token: Default::default(),
            indexes_brace: Default::default(),
            indexes,
        };
        ix::contract::check(&section, self)?;
        Ok(section)
    }
}
