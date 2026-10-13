//! Scoped disjunctions. The interface is the intersection of branch binders;
//! prefix names used inside are free variables. Other branch binders are local.
//! Union projects to the common heading and combines sets; fallback selects the
//! first nonempty branch. A branch panic is bottom. Union order and witness
//! short circuiting may remove bottom; fallback order is part of its meaning.

use crate::{BodyItem, CommaList, rust, symbol_name};
use quote::{ToTokens, quote};
use std::collections::{BTreeMap, BTreeSet};
use syn::{Ident, Token, token};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Union,
    Fallback,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Branch {
    pub brace: token::Brace,
    pub body: CommaList<BodyItem>,
}

impl syn::parse::Parse for Branch {
    fn parse(input: syn::parse::ParseStream<'_>) -> syn::Result<Self> {
        let contents;
        let brace = syn::braced!(contents in input);
        Ok(Self {
            brace,
            body: CommaList::parse_separated_nonempty(&contents)?,
        })
    }
}
impl ToTokens for Branch {
    fn to_tokens(&self, tokens: &mut proc_macro2::TokenStream) {
        self.brace
            .surround(tokens, |tokens| self.body.to_tokens(tokens));
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Block {
    pub kind: Kind,
    pub branches: Vec<Branch>,
}
impl syn::parse::Parse for Block {
    fn parse(input: syn::parse::ParseStream<'_>) -> syn::Result<Self> {
        let mut branches = vec![input.parse()?];
        let kind = if input.peek(Token![|]) {
            Kind::Union
        } else if input.peek(Token![else]) {
            Kind::Fallback
        } else {
            return Err(
                input.error("a block requires at least two branches joined by `|` or `else`")
            );
        };
        while input.peek(Token![|]) || input.peek(Token![else]) {
            match kind {
                Kind::Union if input.peek(Token![|]) => {
                    input.parse::<Token![|]>()?;
                }
                Kind::Fallback if input.peek(Token![else]) => {
                    input.parse::<Token![else]>()?;
                }
                _ => return Err(input.error("mixing `|` and `else` requires a nested block")),
            }
            branches.push(input.parse()?);
        }
        Ok(Self { kind, branches })
    }
}
impl ToTokens for Block {
    fn to_tokens(&self, tokens: &mut proc_macro2::TokenStream) {
        for (j, branch) in self.branches.iter().enumerate() {
            if j != 0 {
                tokens.extend(match self.kind {
                    Kind::Union => quote!(|),
                    Kind::Fallback => quote!(else),
                });
            }
            branch.to_tokens(tokens);
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Analysis {
    pub free: BTreeMap<String, Ident>,
    pub interface: BTreeMap<String, Ident>,
    pub locals: Vec<BTreeSet<String>>,
    pub rust: bool,
}

impl Block {
    /// Facts depend on the written context, never on a chosen plan.
    pub fn analyze(&self, prefix: &BTreeSet<String>) -> Analysis {
        let mut result = Analysis::default();
        let mut common: Option<BTreeMap<String, Ident>> = None;
        for branch in &self.branches {
            let bindings = bound(&branch.body);
            common = Some(match common.take() {
                None => bindings.clone(),
                Some(mut names) => {
                    names.retain(|name, _| bindings.contains_key(name));
                    names
                }
            });
            for item in walk(&branch.body) {
                let uses: BTreeMap<String, Ident> = match item {
                    BodyItem::Positive { atom } => atom
                        .variables()
                        .map(|id| (symbol_name(id), id.clone()))
                        .collect(),
                    BodyItem::Equality(e) => e
                        .variables()
                        .map(|id| (symbol_name(id), id.clone()))
                        .collect(),
                    BodyItem::Constraint(c) => c
                        .variables()
                        .map(|id| (symbol_name(id), id.clone()))
                        .collect(),
                    _ => BTreeMap::new(),
                };
                result
                    .free
                    .extend(uses.into_iter().filter(|(name, _)| prefix.contains(name)));
                if let Some(step) = item.rust_step() {
                    result.rust = true;
                    for name in step.free_variables.dependencies(prefix) {
                        result
                            .free
                            .entry(name.clone())
                            .or_insert_with(|| Ident::new(&name, proc_macro2::Span::call_site()));
                    }
                }
                if let BodyItem::Positive { atom } = item {
                    for argument in &atom.args {
                        let free = rust::free_variables(argument);
                        for name in free.dependencies(prefix) {
                            result.free.entry(name.clone()).or_insert_with(|| {
                                Ident::new(&name, proc_macro2::Span::call_site())
                            });
                        }
                    }
                }
            }
        }
        result.interface = common
            .unwrap_or_default()
            .into_iter()
            .filter(|(name, _)| !prefix.contains(name))
            .collect();
        result.locals = self
            .branches
            .iter()
            .map(|branch| {
                bound(&branch.body)
                    .into_keys()
                    .filter(|name| !prefix.contains(name) && !result.interface.contains_key(name))
                    .collect()
            })
            .collect();
        result
    }
}

/// Visible binders at the end of a body, respecting nested block interfaces.
pub fn bound(body: &CommaList<BodyItem>) -> BTreeMap<String, Ident> {
    let mut names = BTreeMap::new();
    for item in body {
        match item {
            BodyItem::Positive { atom } => {
                names.extend(atom.variables().map(|id| (symbol_name(id), id.clone())))
            }
            BodyItem::Let(binding) => names.extend(
                rust::bindings(&binding.pattern)
                    .into_iter()
                    .map(|id| (symbol_name(&id), id)),
            ),
            BodyItem::Block(block) => {
                names.extend(block.analyze(&names.keys().cloned().collect()).interface)
            }
            _ => (),
        }
    }
    names
}

/// Preorder over clauses; branch boundaries remain in the returned block items.
pub fn walk(body: &CommaList<BodyItem>) -> Vec<&BodyItem> {
    let mut items = Vec::new();
    for item in body {
        items.push(item);
        if let BodyItem::Block(block) = item {
            for branch in &block.branches {
                items.extend(walk(&branch.body));
            }
        }
    }
    items
}

pub(crate) fn split_guards(body: &mut CommaList<BodyItem>, logical: &BTreeSet<String>) {
    let mut prefix = BTreeSet::new();
    let mut forbidden = BTreeSet::new();
    let mut result = CommaList::new();
    for mut item in std::mem::take(body) {
        match &mut item {
            BodyItem::Block(block) => {
                let facts = block.analyze(&prefix);
                for branch in &mut block.branches {
                    let branch_logical = logical
                        .iter()
                        .cloned()
                        .chain(bound(&branch.body).into_keys())
                        .collect();
                    split_guards(&mut branch.body, &branch_logical);
                }
                forbidden.extend(facts.locals.into_iter().flatten());
                prefix.extend(facts.interface.into_keys());
            }
            BodyItem::Positive { atom } => prefix.extend(atom.variables().map(symbol_name)),
            BodyItem::Let(binding) => {
                prefix.extend(rust::bindings(&binding.pattern).iter().map(symbol_name))
            }
            BodyItem::Filter(filter) => {
                if filter.total.is_some() {
                    result.push(BodyItem::Constraint(crate::Constraint::total(
                        filter.condition.clone(), logical,
                    )));
                    continue;
                }
                let names = logical.union(&forbidden).cloned().collect();
                let split = cq_syntax::guard::split(&filter.condition, &names);
                for c in split.constraints {
                    result.push(match c.equality() {
                        Some(e) => BodyItem::Equality(e),
                        None => BodyItem::Constraint(c),
                    });
                }
                if let Some(condition) = split.residual {
                    result.push(BodyItem::Filter(crate::Filter {
                        condition,
                        ..filter.clone()
                    }));
                }
                continue;
            }
            _ => (),
        }
        result.push(item);
    }
    *body = result;
}

/// Infer declared visible types and check each branch's common interface.
/// Inferred let types deliberately remain `_` for rustc.
pub fn types(
    schema: &BTreeMap<String, &crate::RelationDecl>,
    body: &CommaList<BodyItem>,
    prefix: &BTreeMap<String, syn::Type>,
) -> syn::Result<BTreeMap<String, syn::Type>> {
    let mut result = prefix.clone();
    for item in body {
        match item {
            BodyItem::Positive { atom } => {
                let declaration = crate::check_input_atom(schema, atom)?;
                for (argument, column) in atom.args.iter().zip(&declaration.columns) {
                    if let Some(id) = rust::variable(argument) {
                        result
                            .entry(symbol_name(id))
                            .or_insert_with(|| column.ty.clone());
                    }
                }
            }
            BodyItem::Let(binding) => result.extend(
                binding
                    .binding_types()
                    .into_iter()
                    .map(|(id, ty)| (symbol_name(&id), ty)),
            ),
            BodyItem::Block(block) => {
                let facts = block.analyze(&result.keys().cloned().collect());
                let branches = block
                    .branches
                    .iter()
                    .map(|b| types(schema, &b.body, &result))
                    .collect::<syn::Result<Vec<_>>>()?;
                for (name, id) in &facts.interface {
                    let declared: Vec<_> = branches
                        .iter()
                        .zip(&block.branches)
                        .filter(|(_, branch)| {
                            walk(&branch.body)
                                .into_iter()
                                .filter_map(BodyItem::atom)
                                .flat_map(crate::Atom::variables)
                                .any(|id| symbol_name(id) == *name)
                        })
                        .filter_map(|(b, _)| b.get(name))
                        .filter(|ty| !matches!(ty, syn::Type::Infer(_)))
                        .collect();
                    if declared.windows(2).any(|pair| pair[0] != pair[1]) {
                        return Err(syn::Error::new_spanned(
                            id,
                            format!(
                                "block interface variable `{name}` has different declared types across branches"
                            ),
                        ));
                    }
                    result.insert(
                        name.clone(),
                        declared
                            .first()
                            .map(|ty| (*ty).clone())
                            .or_else(|| branches.first().and_then(|b| b.get(name)).cloned())
                            .unwrap_or_else(|| syn::parse_quote!(_)),
                    );
                }
            }
            _ => (),
        }
    }
    Ok(result)
}
