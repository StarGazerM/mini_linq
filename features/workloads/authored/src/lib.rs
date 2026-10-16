//! Tool-time access to the one literal query in each executable case.
//! Dataset generators, explain/oracle tools and catalog consistency tests use
//! this reader. Executable golden tests use typed schemas and never read Rust.
//! Paths are supplied by the caller, so standalone fixtures can inspect the
//! corpus without compiling the executable workload library.
use proc_macro2::{Span, TokenStream, TokenTree};
use std::path::Path;

fn query(path: &Path) -> syn::Result<TokenStream> {
    let source = std::fs::read_to_string(path).map_err(|error| {
        syn::Error::new(Span::call_site(), format!("{}: {error}", path.display()))
    })?;
    let mut queries = Vec::new();
    fn visit(tokens: TokenStream, queries: &mut Vec<TokenStream>) {
        let tokens: Vec<_> = tokens.into_iter().collect();
        let mut i = 0;
        while i < tokens.len() {
            if let [
                TokenTree::Ident(name),
                TokenTree::Punct(bang),
                TokenTree::Group(body),
                ..,
            ] = &tokens[i..]
                && name == "compile"
                && bang.as_char() == '!'
            {
                queries.push(body.stream());
                i += 3;
                continue;
            }
            if let TokenTree::Group(group) = &tokens[i] {
                visit(group.stream(), queries);
            }
            i += 1;
        }
    }
    visit(source.parse()?, &mut queries);
    if queries.len() != 1 {
        return Err(syn::Error::new(
            Span::call_site(),
            format!(
                "{} must contain exactly one literal compile! query; found {}",
                path.display(),
                queries.len()
            ),
        ));
    }
    let mut tokens: Vec<_> = queries.remove(0).into_iter().collect();
    if matches!(tokens.first(), Some(TokenTree::Ident(name)) if name == "pipeline") {
        if !matches!(tokens.get(1), Some(TokenTree::Group(_))) {
            return Err(syn::Error::new(
                Span::call_site(),
                "expected a pipeline block",
            ));
        }
        tokens.drain(..2);
    }
    if let [
        TokenTree::Ident(name),
        TokenTree::Punct(bang),
        TokenTree::Group(body),
    ] = tokens.as_slice()
        && (name == "recursion" || name == "aggregate")
        && bang.as_char() == '!'
    {
        return Ok(body.stream());
    }
    Ok(tokens.into_iter().collect())
}

pub fn program(path: impl AsRef<Path>) -> syn::Result<cq_lang_cq::Program> {
    let program = syn::parse2(query(path.as_ref())?)?;
    cq_lang_cq::contract::check(&program)?;
    Ok(program)
}

pub fn schema(path: impl AsRef<Path>) -> syn::Result<Vec<cq_syntax::RelationDecl>> {
    let tokens = query(path.as_ref())?;
    // The aggregate surface puts its grouping clauses before the CQ struct.
    let tokens: TokenStream = tokens
        .into_iter()
        .skip_while(|token| !matches!(token, TokenTree::Ident(name) if name == "struct"))
        .collect();
    let rules: cq_ext_support::Rules = syn::parse2(tokens)?;
    let mut declarations: Vec<_> = rules.first.inputs().cloned().collect();
    for input in &mut declarations {
        input.value = None;
    }
    Ok(declarations)
}
