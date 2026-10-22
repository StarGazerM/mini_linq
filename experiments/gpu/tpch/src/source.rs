//! Import the CPU retail_kernel invocations themselves, never a second query body.
use crate::manifest::{Manifest, Result};
use proc_macro2::{Group, TokenStream, TokenTree};
use quote::{ToTokens, quote};
use std::collections::BTreeMap;
use syn::{
    Ident, Token,
    parse::{Parse, ParseStream},
    punctuated::Punctuated,
};

macro_rules! sources {
    ($($name:literal),* $(,)?) => { pub const SOURCES: &[(&str, &str)] = &[$(
        ($name, include_str!(concat!("../../../../features/workloads/src/queries/retail/tpch/retail_", $name, ".rs"))),
    )*]; };
}
sources!(
    "h03",
    "h05",
    "h07",
    "h07_block",
    "h08",
    "h09",
    "h21_positive",
    "d19",
    "d27",
    "d72_inner",
    "d85"
);

#[derive(Clone)]
pub struct Relation {
    pub name: Ident,
    pub table: Ident,
    pub columns: Vec<(Ident, syn::Type)>,
}

pub struct Kernel {
    pub setup: syn::Block,
    pub relations: Vec<Relation>,
    pub answer: Vec<(Ident, syn::Type)>,
    pub body: TokenStream,
}

fn columns(input: ParseStream<'_>) -> syn::Result<Vec<(Ident, syn::Type)>> {
    let mut result = Vec::new();
    while !input.is_empty() {
        let name = input.parse()?;
        input.parse::<Token![:]>()?;
        result.push((name, input.parse()?));
        if !input.is_empty() {
            input.parse::<Token![,]>()?;
        }
    }
    Ok(result)
}

impl Parse for Kernel {
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        input.parse::<Token![$]>()?;
        input.parse::<Token![;]>()?;
        let setup = loop {
            let key: Ident = input.parse()?;
            if key == "setup" {
                break input.parse()?;
            }
            while !input.peek(Token![;]) {
                input.parse::<TokenTree>()?;
            }
            input.parse::<Token![;]>()?;
        };
        let key: Ident = input.parse()?;
        if key != "relations" {
            return Err(syn::Error::new_spanned(key, "expected relations"));
        }
        let declarations;
        syn::braced!(declarations in input);
        let mut relations = Vec::new();
        while !declarations.is_empty() {
            let key: Ident = declarations.parse()?;
            if key != "relation" {
                return Err(syn::Error::new_spanned(key, "expected relation"));
            }
            let name = declarations.parse()?;
            declarations.parse::<Token![=>]>()?;
            let table = declarations.parse()?;
            let fields;
            syn::parenthesized!(fields in declarations);
            relations.push(Relation {
                name,
                table,
                columns: columns(&fields)?,
            });
            declarations.parse::<Token![;]>()?;
        }
        let key: Ident = input.parse()?;
        if key != "answer" {
            return Err(syn::Error::new_spanned(key, "expected answer"));
        }
        let fields;
        syn::parenthesized!(fields in input);
        let answer = columns(&fields)?;
        input.parse::<Token![:]>()?;
        input.parse::<Token![-]>()?;
        Ok(Self {
            setup,
            relations,
            answer,
            body: input.parse()?,
        })
    }
}

impl Kernel {
    pub fn import(source: &str) -> Result<Self> {
        let file = syn::parse_file(source)?;
        let invocation = file
            .items
            .into_iter()
            .find_map(|item| match item {
                syn::Item::Macro(item)
                    if item
                        .mac
                        .path
                        .segments
                        .last()
                        .is_some_and(|s| s.ident == "retail_kernel") =>
                {
                    Some(item.mac.tokens)
                }
                _ => None,
            })
            .ok_or("CPU source has no retail_kernel invocation")?;
        Ok(syn::parse2(invocation)?)
    }

    /// The exact macro substitution made by tpch_runtime::retail_kernel.
    pub fn cpu_query(&self) -> TokenStream {
        let declarations = self.relations.iter().map(|relation| {
            let name = &relation.name;
            let fields = relation.columns.iter().map(|(name, ty)| quote!(#name: #ty));
            quote!(relation #name(#(#fields),*);)
        });
        let outputs = self.answer.iter().map(|(name, _)| name);
        let body = &self.body;
        quote!(struct Query; #(#declarations)* answer(#(#outputs),*) :- #body)
    }

    /// Storage adaptation only: types and constants, not order/providers/plans.
    pub fn encoded_query(&self, manifest: &Manifest) -> Result<TokenStream> {
        let constants = setup_constants(&self.setup)?;
        let mut variables = BTreeMap::new();
        collect_variables(self.body.clone(), &self.relations, &mut variables)?;
        let body = encode_tokens(
            self.body.clone(),
            &self.relations,
            manifest,
            &constants,
            &variables,
        )?;
        let mut declarations = TokenStream::new();
        for relation in &self.relations {
            let name = &relation.name;
            let mut fields = Vec::new();
            for (column, logical) in &relation.columns {
                let metadata = manifest.column(&relation.table.to_string(), &column.to_string())?;
                if metadata.logical_type != logical.to_token_stream().to_string() {
                    return Err(format!(
                        "{}.{}: CPU type {} differs from manifest logical type {}",
                        relation.table,
                        column,
                        logical.to_token_stream(),
                        metadata.logical_type
                    )
                    .into());
                }
                let ty: syn::Type = syn::parse_str(&metadata.ty)?;
                fields.push(quote!(#column: #ty));
            }
            declarations.extend(quote!(relation #name(#(#fields),*);));
        }
        let outputs = self.answer.iter().map(|(name, _)| name);
        Ok(quote!(struct Query; #declarations answer(#(#outputs),*) :- #body))
    }
}

fn setup_constants(setup: &syn::Block) -> Result<BTreeMap<String, TokenStream>> {
    let mut constants = BTreeMap::new();
    for statement in &setup.stmts {
        let syn::Stmt::Local(local) = statement else {
            return Err("unsupported workload setup statement".into());
        };
        let syn::Pat::Ident(name) = &local.pat else {
            return Err("unsupported workload setup pattern".into());
        };
        let expression = &local
            .init
            .as_ref()
            .ok_or("setup local needs initializer")?
            .expr;
        let value = match expression.as_ref() {
            syn::Expr::Call(call) if call.func.to_token_stream().to_string() == "date" => {
                let Some(syn::Expr::Lit(syn::ExprLit {
                    lit: syn::Lit::Str(date),
                    ..
                })) = call.args.first()
                else {
                    return Err("date needs an ISO literal".into());
                };
                let days = date.value().parse::<cq_load::Date>()?.days_since_epoch();
                quote!(#days)
            }
            syn::Expr::MethodCall(method) if method.method == "expect" => {
                let syn::Expr::Call(call) = method.receiver.as_ref() else {
                    return Err("unsupported setup initializer".into());
                };
                if call.func.to_token_stream().to_string() != "Decimal :: from_scaled" {
                    return Err("unsupported setup initializer".into());
                }
                let Some(syn::Expr::Lit(syn::ExprLit {
                    lit: syn::Lit::Int(number),
                    ..
                })) = call.args.first()
                else {
                    return Err("scaled decimal needs integer literal".into());
                };
                let value = number.base10_parse::<i64>()?;
                quote!(#value)
            }
            _ => {
                return Err(format!(
                    "unsupported setup initializer: {}",
                    expression.to_token_stream()
                )
                .into());
            }
        };
        constants.insert(name.ident.to_string(), value);
    }
    Ok(constants)
}

fn arguments(group: &Group) -> syn::Result<Punctuated<syn::Expr, Token![,]>> {
    use syn::parse::Parser;
    Punctuated::parse_terminated.parse2(group.stream())
}

fn collect_variables(
    tokens: TokenStream,
    relations: &[Relation],
    variables: &mut BTreeMap<String, (String, String)>,
) -> Result<()> {
    let tokens: Vec<_> = tokens.into_iter().collect();
    for (index, token) in tokens.iter().enumerate() {
        if let TokenTree::Group(group) = token {
            if let Some(TokenTree::Ident(name)) = index.checked_sub(1).and_then(|i| tokens.get(i))
                && let Some(relation) = relations.iter().find(|r| r.name == *name)
            {
                for (argument, (column, _)) in arguments(group)?.iter().zip(&relation.columns) {
                    if let syn::Expr::Path(path) = argument
                        && let Some(name) = path.path.get_ident()
                    {
                        variables.insert(
                            name.to_string(),
                            (relation.table.to_string(), column.to_string()),
                        );
                    }
                }
            }
            collect_variables(group.stream(), relations, variables)?;
        }
    }
    Ok(())
}

fn encode_tokens(
    tokens: TokenStream,
    relations: &[Relation],
    manifest: &Manifest,
    constants: &BTreeMap<String, TokenStream>,
    variables: &BTreeMap<String, (String, String)>,
) -> Result<TokenStream> {
    let tokens: Vec<_> = tokens.into_iter().collect();
    let mut encoded = TokenStream::new();
    for (index, token) in tokens.iter().enumerate() {
        match token {
            TokenTree::Ident(name) if constants.contains_key(&name.to_string()) => {
                encoded.extend(constants[&name.to_string()].clone())
            }
            TokenTree::Group(group) => {
                let relation = index
                    .checked_sub(1)
                    .and_then(|i| tokens.get(i))
                    .and_then(|token| {
                        if let TokenTree::Ident(name) = token {
                            relations.iter().find(|r| r.name == *name)
                        } else {
                            None
                        }
                    });
                let contents = if let Some(relation) = relation {
                    let mut args = arguments(group)?;
                    for (arg, (column, _)) in args.iter_mut().zip(&relation.columns) {
                        if let syn::Expr::Lit(syn::ExprLit {
                            lit: syn::Lit::Str(text),
                            ..
                        }) = arg
                        {
                            let id = manifest.text_id(
                                &relation.table.to_string(),
                                &column.to_string(),
                                &text.value(),
                            )?;
                            *arg = syn::parse_quote!(#id);
                        }
                    }
                    args.to_token_stream()
                } else {
                    encode_tokens(group.stream(), relations, manifest, constants, variables)?
                };
                let mut replacement = Group::new(group.delimiter(), contents);
                replacement.set_span(group.span());
                encoded.extend([TokenTree::Group(replacement)]);
            }
            TokenTree::Literal(literal) if index >= 3 => {
                // Equality guards have the same dictionary as their bound column.
                // Method arguments (e.g. contains) remain untouched for the checked
                // translator to accept or reject; dictionary codes are not strings.
                let comparison = matches!((&tokens[index-2], &tokens[index-1]),
                    (TokenTree::Punct(a), TokenTree::Punct(b)) if (a.as_char() == '=' || a.as_char() == '!') && b.as_char() == '=');
                if comparison
                    && let (TokenTree::Ident(name), Ok(text)) = (
                        &tokens[index - 3],
                        syn::parse_str::<syn::LitStr>(&literal.to_string()),
                    )
                    && let Some((table, column)) = variables.get(&name.to_string())
                {
                    let id = manifest.text_id(table, column, &text.value())?;
                    encoded.extend(quote!(#id));
                    continue;
                }
                encoded.extend([token.clone()]);
            }
            _ => encoded.extend([token.clone()]),
        }
    }
    Ok(encoded)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn every_cpu_query_is_imported_without_a_second_body() {
        assert_eq!(SOURCES.len(), 11);
        for (name, source) in SOURCES {
            let kernel = Kernel::import(source).unwrap();
            let query: cq_lang_cq::Program = syn::parse2(kernel.cpu_query()).unwrap();
            assert_eq!(query.declarations.len(), kernel.relations.len(), "{name}");
            assert!(!kernel.body.is_empty(), "{name}");
        }
    }
}

#[cfg(test)]
mod encoding_tests {
    use super::*;

    #[test]
    fn both_scales_encode_literals_from_their_own_dictionaries() {
        for sf in [1, 10] {
            let manifest = Manifest::load(&crate::data_root().join(format!("sf{sf}"))).unwrap();
            for &(name, source) in SOURCES {
                let kernel = Kernel::import(source).unwrap();
                let encoded = kernel.encoded_query(&manifest).unwrap();
                let program: cq_lang_cq::Program = syn::parse2(encoded.clone()).unwrap();
                assert_eq!(program.declarations.len(), kernel.relations.len(), "{name}");
                for relation in &program.declarations {
                    for column in &relation.columns {
                        assert!(matches!(
                            column.ty.to_token_stream().to_string().as_str(),
                            "i32" | "i64"
                        ));
                    }
                }
            }
            let kernel =
                Kernel::import(SOURCES.iter().find(|(name, _)| *name == "h03").unwrap().1).unwrap();
            let encoded = kernel.encoded_query(&manifest).unwrap().to_string();
            let code = manifest
                .text_id("customer", "c_mktsegment", "BUILDING")
                .unwrap();
            assert!(encoded.contains(&format!("Customer (customer_key , {code}i32)")));
            assert!(encoded.contains("9204i32"));
            assert!(!encoded.contains("BUILDING"));
            assert!(!encoded.contains("cutoff"));
            assert_eq!(
                manifest
                    .text_id("customer", "c_mktsegment", "__not_in_dictionary__")
                    .unwrap(),
                -1
            );
        }
    }
}
