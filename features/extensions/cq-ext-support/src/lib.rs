//! Surface contexts over the public pass protocol.
//!
//! Each surface pass receives raw text and a remaining [`cq_stage::Pipeline`].
//! Unowned text continues unchanged. An owned quotation emits a Rust context
//! with nested calls to that remaining pipeline, before contracted CQ parsing.
//! Contexts use concrete set collectors written into their generated Rust;
//! customizing inner execution means composing passes in the caller's pipeline.
//! Structural checks preserve leaves, declaration order, and fresh binders.
//! Runtime evidence assumes finite immutable hosts, Leibniz equality and pure
//! opaque Rust expressions, with the core's written-order refinement meaning.
use std::collections::BTreeSet;

use cq_lang_cq::{BodyItem, Program, Query};
use cq_syntax::{RelationDecl, rust, symbol_name};
use proc_macro2::{Span, TokenStream, TokenTree};
use quote::{ToTokens, quote};
use syn::{Expr, Ident, Token, parse::Parse};

mod kw {
    syn::custom_keyword!(arguments);
}

/// Public surface packet; the queue is already past the current pass.
pub struct SurfaceInput {
    pub arguments: TokenStream,
    pub pipeline: cq_stage::Pipeline,
    pub program: TokenStream,
}
impl Parse for SurfaceInput {
    fn parse(input: syn::parse::ParseStream<'_>) -> syn::Result<Self> {
        let arguments = if input.peek(kw::arguments) {
            input.parse::<kw::arguments>()?;
            let contents;
            syn::braced!(contents in input);
            let arguments = contents.parse()?;
            input.parse::<Token![;]>()?;
            arguments
        } else {
            TokenStream::new()
        };
        let pipeline = input.parse()?;
        input.parse::<Token![;]>()?;
        Ok(Self {
            arguments,
            pipeline,
            program: input.parse()?,
        })
    }
}

/// Expand an owned surface or forward raw text. A context finishes this macro
/// invocation; its nested queries each resume the same remaining queue.
pub fn surface(
    input: TokenStream,
    expand: impl FnOnce(&TokenStream, &cq_stage::Pipeline) -> syn::Result<Option<TokenStream>>,
) -> TokenStream {
    (|| {
        let input: SurfaceInput = syn::parse2(input)?;
        ensure(
            input.arguments.is_empty(),
            &input.arguments,
            "this surface pass accepts no arguments",
        )?;
        ensure(
            !input.pipeline.passes.is_empty(),
            &input.program,
            "surface pass requires a remaining pipeline with a Rust printer",
        )?;
        if let Some(context) = expand(&input.program, &input.pipeline)? {
            Ok(context)
        } else {
            input.pipeline.start(input.program)
        }
    })()
    .unwrap_or_else(syn::Error::into_compile_error)
}

/// Match only a whole owned quotation. Other surfaces stay opaque.
pub fn quotation(tokens: &TokenStream, owner: &str) -> syn::Result<Option<TokenStream>> {
    let mut trees = tokens.clone().into_iter();
    let Some(TokenTree::Ident(name)) = trees.next() else {
        return Ok(None);
    };
    if symbol_name(&name) != owner {
        return Ok(None);
    }
    if !matches!(trees.next(), Some(TokenTree::Punct(p)) if p.as_char() == '!') {
        return Ok(None);
    }
    let Some(TokenTree::Group(contents)) = trees.next() else {
        return Err(syn::Error::new_spanned(
            tokens,
            "expected a surface quotation",
        ));
    };
    ensure(
        trees.next().is_none(),
        tokens,
        "unexpected text after surface quotation",
    )?;
    Ok(Some(contents.stream()))
}

/// Detect ordinary CQ without trying to parse another extension's quotation.
pub fn is_cq(tokens: &TokenStream) -> bool {
    matches!(tokens.clone().into_iter().next(), Some(TokenTree::Ident(id)) if id == "struct" || id == "pub")
}

pub fn same_symbol(left: &Ident, right: &Ident) -> bool {
    symbol_name(left) == symbol_name(right)
}

/// Several rules following one ordinary CQ header and declaration block.
#[derive(Clone, Debug)]
pub struct Rules {
    pub first: Program,
    pub rest: Vec<Query>,
}
impl Parse for Rules {
    fn parse(input: syn::parse::ParseStream<'_>) -> syn::Result<Self> {
        let first = input.parse()?;
        let mut rest = Vec::new();
        while !input.is_empty() {
            rest.push(input.parse()?);
        }
        Ok(Self { first, rest })
    }
}
impl Rules {
    pub fn queries(&self) -> impl Iterator<Item = &Query> {
        std::iter::once(&self.first.query).chain(&self.rest)
    }
}
impl ToTokens for Rules {
    fn to_tokens(&self, tokens: &mut TokenStream) {
        self.first.to_tokens(tokens);
        for query in &self.rest {
            query.to_tokens(tokens);
        }
    }
}

/// Fresh local names also avoid identifiers hidden in opaque Rust/extension text.
pub struct Fresh {
    used: BTreeSet<String>,
}
impl Fresh {
    pub fn new(source: impl ToTokens) -> Self {
        fn collect(tokens: TokenStream, used: &mut BTreeSet<String>) {
            for token in tokens {
                match token {
                    TokenTree::Ident(id) => {
                        used.insert(symbol_name(&id));
                    }
                    TokenTree::Group(group) => collect(group.stream(), used),
                    _ => {}
                }
            }
        }
        let mut used = BTreeSet::new();
        collect(source.into_token_stream(), &mut used);
        Self { used }
    }
    pub fn id(&mut self, role: &str) -> Ident {
        for ordinal in 0.. {
            let name = format!("__cq_ext_{role}_{ordinal}");
            if self.used.insert(name.clone()) {
                return Ident::new(&name, Span::mixed_site());
            }
        }
        unreachable!()
    }
}

/// Context binders must not capture source identifiers or alias each other.
pub fn check_fresh<'a>(
    source: impl ToTokens,
    bindings: impl IntoIterator<Item = &'a Ident>,
) -> syn::Result<()> {
    let source_names = Fresh::new(source).used;
    let mut introduced = BTreeSet::new();
    for binding in bindings {
        let name = symbol_name(binding);
        ensure(
            !source_names.contains(&name) && introduced.insert(name),
            binding,
            "context binding captures a source name or aliases another binding",
        )?;
    }
    Ok(())
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Leaf {
    pub binding: Ident,
    pub expression: Expr,
}
impl ToTokens for Leaf {
    fn to_tokens(&self, tokens: &mut TokenStream) {
        let Self {
            binding,
            expression,
        } = self;
        tokens.extend(quote!(let #binding = #expression;));
    }
}

/// Evaluate authored input leaves once in the context, then borrow them in holes.
pub fn hoist(source: &Program, fresh: &mut Fresh) -> (Vec<RelationDecl>, Vec<Leaf>) {
    let mut declarations = source.declarations.clone();
    let mut leaves = Vec::new();
    for declaration in &mut declarations {
        if same_symbol(&declaration.name, &source.query.head.relation) {
            continue;
        }
        if let Some(value) = &mut declaration.value {
            let binding = fresh.id("leaf");
            leaves.push(Leaf {
                binding: binding.clone(),
                expression: value.expression.clone(),
            });
            value.expression = syn::parse_quote!(&#binding);
        }
    }
    (declarations, leaves)
}

/// Independent declaration/leaf correspondence; it does not rerun hoisting.
pub fn check_leaves(
    source: &Program,
    declarations: &[RelationDecl],
    leaves: &[Leaf],
) -> syn::Result<()> {
    ensure(
        declarations.len() == source.declarations.len(),
        source,
        "declaration count changed",
    )?;
    let mut remaining = leaves.iter();
    let mut names = BTreeSet::new();
    for (original, target) in source.declarations.iter().zip(declarations) {
        let mut erased = target.clone();
        if !same_symbol(&original.name, &source.query.head.relation) {
            if let Some(value) = &original.value {
                let leaf = remaining
                    .next()
                    .ok_or_else(|| syn::Error::new_spanned(original, "missing leaf binding"))?;
                ensure(
                    names.insert(symbol_name(&leaf.binding)),
                    &leaf.binding,
                    "duplicate leaf binding",
                )?;
                ensure(
                    leaf.expression == value.expression,
                    &leaf.expression,
                    "leaf expression changed",
                )?;
                let binding = &leaf.binding;
                ensure(
                    target.value.as_ref().map(|v| &v.expression)
                        == Some(&syn::parse_quote!(&#binding)),
                    target,
                    "hole must borrow its leaf binding",
                )?;
                erased.value = original.value.clone();
            }
        }
        ensure(
            &erased == original,
            target,
            "declaration schema or order changed",
        )?;
    }
    ensure(remaining.next().is_none(), source, "extra leaf binding")
}

pub fn ensure(condition: bool, source: impl ToTokens, message: &str) -> syn::Result<()> {
    if condition {
        Ok(())
    } else {
        Err(syn::Error::new_spanned(source, message))
    }
}

/// A context's final answer values form a set even when computed heads map
/// several core valuations to the same value. Its concrete type is in the context.
pub fn distinct(stream: TokenStream, fresh: &mut Fresh, role: &str) -> TokenStream {
    let state = fresh.id(role);
    let row = fresh.id("answer_value");
    quote!({
        let mut #state = ::std::collections::HashSet::new();
        #stream.filter(move |#row| #state.insert(::core::clone::Clone::clone(#row)))
    })
}

pub fn bound(item: &BodyItem) -> Vec<Ident> {
    match item {
        BodyItem::Positive { atom } => atom.variables().cloned().collect(),
        BodyItem::Let(binding) => rust::bindings(&binding.pattern),
        _ => Vec::new(),
    }
}

/// Preserve the input block while changing a hole's head/output schema.
pub fn with_query(
    source: &Program,
    declarations: &[RelationDecl],
    query: Query,
    output: Option<RelationDecl>,
) -> Program {
    let mut hole = source.clone();
    hole.declarations = declarations
        .iter()
        .filter(|d| !same_symbol(&d.name, &source.query.head.relation))
        .cloned()
        .collect();
    if let Some(output) = output {
        hole.declarations.push(output);
    }
    hole.query = query;
    hole
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn forwarding_keeps_next_arguments_and_the_rest_of_the_queue() {
        let result = surface(
            quote!(pipeline { next::pass(alpha + beta), last::print }; foreign!{opaque}),
            |_, _| Ok(None),
        );
        assert_eq!(
            result.to_string(),
            quote!(next::pass! {
                arguments { alpha + beta }; pipeline { last::print }; foreign!{opaque}
            })
            .to_string()
        );
    }
    #[test]
    fn quotation_ownership_and_missing_printer_are_checked() {
        assert!(
            quotation(&quote!(other! {opaque}), "union")
                .unwrap()
                .is_none()
        );
        assert!(quotation(&quote!(union!{opaque} extra), "union").is_err());
        let result = surface(quote!(pipeline {}; struct Q;), |_, _| Ok(None));
        assert!(result.to_string().contains("requires a remaining pipeline"));
    }
    #[test]
    fn source_capture_and_aliases_are_rejected() {
        let source = quote!(hidden! {occupied});
        let occupied = Ident::new("occupied", Span::mixed_site());
        assert!(check_fresh(&source, [&occupied]).is_err());
        let fresh = Ident::new("fresh", Span::mixed_site());
        assert!(check_fresh(&source, [&fresh, &fresh]).is_err());
        check_fresh(source, [&fresh]).unwrap();
    }
    #[test]
    fn fresh_names_see_opaque_text_and_leaf_check_rejects_mutations() {
        let source: Program = syn::parse_quote! {
            struct Q; relation R(x:i32) = (make(__cq_ext_leaf_0));
            answer(x) :- R(x), opaque!{ __cq_ext_leaf_1 };
        };
        let (mut declarations, mut leaves) = hoist(&source, &mut Fresh::new(&source));
        assert_eq!(leaves[0].binding, "__cq_ext_leaf_2");
        check_leaves(&source, &declarations, &leaves).unwrap();
        leaves[0].expression = syn::parse_quote!(different());
        assert!(check_leaves(&source, &declarations, &leaves).is_err());
        let (_, leaves) = hoist(&source, &mut Fresh::new(&source));
        declarations[0].columns[0].ty = syn::parse_quote!(u64);
        assert!(check_leaves(&source, &declarations, &leaves).is_err());
    }
}
