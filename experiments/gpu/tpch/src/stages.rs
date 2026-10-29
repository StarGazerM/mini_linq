//! Import the authored compile! dependency graph, including terminal iterator unions.
use crate::{
    manifest::Result,
    source::{Kernel, Relation},
};
use proc_macro2::TokenStream;
use quote::{ToTokens, format_ident, quote};
use std::collections::BTreeMap;
use syn::{
    Ident, Token,
    parse::{Parse, ParseStream},
    visit_mut::VisitMut,
};

pub struct Stage {
    pub program: cq_lang_cq::Program,
    pub evidence: cq_lang_sequence::Evidence,
    pub map: Option<syn::ExprClosure>,
}

pub struct Preprocess {
    pub name: Ident,
    pub columns: Vec<(Ident, syn::Type)>,
    pub expression: syn::Expr,
    pub relations: Vec<Relation>,
}

pub struct Program {
    pub file: syn::File,
    pub stages: Vec<Stage>,
    pub aliases: BTreeMap<String, usize>,
    pub terminal: Vec<usize>,
    pub preprocessing: Vec<Preprocess>,
}

fn binding(pattern: &syn::Pat) -> Option<&Ident> {
    match pattern {
        syn::Pat::Ident(name) => Some(&name.ident),
        syn::Pat::Type(typed) => binding(&typed.pat),
        _ => None,
    }
}

fn compiled(expression: &syn::Expr) -> Result<Option<Stage>> {
    match expression {
        syn::Expr::Macro(mac)
            if mac
                .mac
                .path
                .segments
                .last()
                .is_some_and(|s| s.ident == "compile") =>
        {
            let source = syn::parse2::<crate::source::AuthoredQuery>(mac.mac.tokens.clone())?;
            Ok(Some(Stage {
                program: source.program,
                evidence: source.evidence,
                map: None,
            }))
        }
        syn::Expr::MethodCall(method) if method.method == "map" || method.method == "collect" => {
            let Some(mut stage) = compiled(&method.receiver)? else {
                return Ok(None);
            };
            if method.method == "map" {
                let Some(syn::Expr::Closure(map)) = method.args.first() else {
                    return Err("stage map must be a closure".into());
                };
                if stage.map.is_some() {
                    return Err("multiple stage ownership maps".into());
                }
                stage.map = Some(map.clone());
            }
            Ok(Some(stage))
        }
        _ => Ok(None),
    }
}

fn terminal(
    expression: &syn::Expr,
    aliases: &BTreeMap<String, usize>,
    result: &mut Vec<usize>,
) -> Result<()> {
    match expression {
        syn::Expr::Path(path) if path.path.get_ident().is_some() => {
            result.push(
                *aliases
                    .get(&path.path.get_ident().unwrap().to_string())
                    .ok_or("unknown terminal stage")?,
            );
            Ok(())
        }
        syn::Expr::MethodCall(method) if method.method == "collect" || method.method == "into_iter" => {
            terminal(&method.receiver, aliases, result)
        }
        syn::Expr::MethodCall(method) if method.method == "map" && method.args.len() == 1 => {
            let syn::Expr::Closure(map) = &method.args[0] else {
                return Err("terminal publication map must be a closure".into());
            };
            check_identity_map(&publication_map(map))?;
            terminal(&method.receiver, aliases, result)
        }
        syn::Expr::MethodCall(method) if method.method == "chain" && method.args.len() == 1 => {
            terminal(&method.receiver, aliases, result)?;
            terminal(&method.args[0], aliases, result)
        }
        _ => Err(format!(
            "unsupported terminal stage composition: {}",
            expression.to_token_stream()
        )
        .into()),
    }
}

/// The GPU decoder already publishes dictionary text using recorded origins.
/// Retain the authored tuple projection while removing only that explicit
/// terminal representation conversion, not intermediate-stage computation.
fn publication_map(map: &syn::ExprClosure) -> syn::ExprClosure {
    struct Publication;
    impl VisitMut for Publication {
        fn visit_expr_mut(&mut self, expression: &mut syn::Expr) {
            if let syn::Expr::MethodCall(method) = expression
                && method.method == "publish"
                && method.args.len() == 2
                && matches!(&method.args[0], syn::Expr::Lit(syn::ExprLit { lit: syn::Lit::Str(_), .. }))
                && (matches!(method.receiver.as_ref(), syn::Expr::Path(path) if path.path.is_ident("dictionaries"))
                    || matches!(method.receiver.as_ref(), syn::Expr::Field(field)
                        if matches!(&field.member, syn::Member::Named(name) if name == "dictionaries")
                            && matches!(field.base.as_ref(), syn::Expr::Path(path) if path.path.is_ident("data"))))
            {
                *expression = method.args[1].clone();
            }
            syn::visit_mut::visit_expr_mut(self, expression);
        }
    }
    let mut map = map.clone();
    Publication.visit_expr_closure_mut(&mut map);
    map
}

impl Program {
    pub fn import(source: &str) -> Result<Option<Self>> {
        let file = crate::source::authored_file(source)?;
        let Some(run) = file.items.iter().find_map(|item| match item {
            syn::Item::Fn(function) if function.sig.ident == "run" => Some(function),
            _ => None,
        }) else {
            return Ok(None);
        };
        let mut stages = Vec::new();
        let mut aliases = BTreeMap::new();
        let mut terminal_stages = Vec::new();
        for statement in &run.block.stmts {
            match statement {
                syn::Stmt::Local(local) => {
                    let Some(name) = binding(&local.pat) else {
                        continue;
                    };
                    let Some(init) = &local.init else {
                        continue;
                    };
                    if let Some(stage) = compiled(&init.expr)? {
                        aliases.insert(name.to_string(), stages.len());
                        stages.push(stage);
                    } else if let syn::Expr::MethodCall(method) = init.expr.as_ref()
                        && method.method == "as_slice"
                        && let syn::Expr::Path(path) = method.receiver.as_ref()
                        && let Some(original) = path.path.get_ident()
                        && let Some(&index) = aliases.get(&original.to_string())
                    {
                        aliases.insert(name.to_string(), index);
                    }
                }
                syn::Stmt::Expr(expression, None) => {
                    if let Some(stage) = compiled(expression)? {
                        terminal_stages.push(stages.len());
                        stages.push(stage);
                    } else if stages.len() > 1 {
                        terminal(expression, &aliases, &mut terminal_stages)?;
                    }
                }
                _ => {}
            }
        }
        if stages.len() <= 1 {
            return Ok(None);
        }
        if terminal_stages.is_empty() {
            return Err("staged source has no terminal expression".into());
        }
        for stage in &stages {
            for input in stage.program.inputs() {
                if let Some(&producer) = aliases.get(&input.name.to_string())
                    && let Some(map) = &stages[producer].map
                {
                    check_identity_map(map)?;
                }
            }
        }
        for &index in &terminal_stages {
            if let Some(map) = &mut stages[index].map {
                *map = publication_map(map);
            }
        }
        // Inputs not exported and not published are authored one-time host
        // preprocessing. Preserve its expression; do not replicate predicates.
        let mut preprocessing = Vec::new();
        let declarations = loader_declarations(&file)?;
        let exports = crate::source::export_tables(&file)?;
        for stage in &stages {
            for input in stage.program.inputs() {
                if aliases.contains_key(&input.name.to_string())
                    || declarations.iter().any(|r| r.name == input.name)
                    || preprocessing
                        .iter()
                        .any(|p: &Preprocess| p.name == input.name)
                {
                    continue;
                }
                let initializer = run
                    .block
                    .stmts
                    .iter()
                    .find_map(|statement| match statement {
                        syn::Stmt::Local(local)
                            if binding(&local.pat) == Some(&input.name) =>
                        {
                            local.init.as_ref()
                        }
                        _ => None,
                    })
                    .ok_or_else(|| format!("unbound stage input {}", input.name))?;
                let mut expression = crate::source::load_initializer(&file, &initializer.expr)?;
                let dictionary = crate::source::dictionary_initializer(&expression, &exports)?;
                let dictionary_input = dictionary.is_some();
                if let Some(initializer) = dictionary {
                    if input.columns.len() != 1 {
                        return Err("dictionary_where input must have one code column".into());
                    }
                    expression = syn::parse2(initializer)?;
                }
                struct Fields<'a> {
                    declarations: &'a [Relation],
                    file: &'a syn::File,
                    exports: &'a [(String, Vec<String>)],
                    prepared: Vec<(Ident, syn::Expr)>,
                    error: Option<Box<dyn std::error::Error>>,
                    used: Vec<Relation>,
                }
                impl VisitMut for Fields<'_> {
                    fn visit_expr_mut(&mut self, expression: &mut syn::Expr) {
                        match crate::source::dictionary_initializer(expression, self.exports) {
                            Ok(Some(initializer)) => {
                                // Authored host preprocessing uses the cq-load code
                                // carrier; device relation publications remain u32.
                                *expression = syn::parse_quote!({
                                    (#initializer).into_iter()
                                        .map(|(code,)| (cq_load::StringCode(code),))
                                        .collect::<Vec<_>>()
                                });
                                return;
                            }
                            Err(error) => { self.error = Some(error); return; }
                            Ok(None) => {}
                        }
                        if let syn::Expr::Field(field) = expression
                            && let syn::Expr::Path(base) = field.base.as_ref()
                            && base.path.is_ident("data")
                            && let syn::Member::Named(name) = &field.member
                            && let Some(relation) =
                                self.declarations.iter().find(|r| r.table == *name)
                        {
                            if !self.used.iter().any(|r| r.name == relation.name) {
                                self.used.push(relation.clone());
                            }
                            let name = &relation.name;
                            *expression = syn::parse_quote!(#name);
                            return;
                        }
                        if let syn::Expr::Field(field) = expression
                            && let syn::Expr::Path(base) = field.base.as_ref()
                            && base.path.is_ident("data")
                            && let syn::Member::Named(name) = &field.member
                        {
                            let prepared = format_ident!("__cq_prepared_{}", name);
                            if !self.prepared.iter().any(|(name, _)| *name == prepared) {
                                match crate::source::load_field_initializer(self.file, name) {
                                    Ok(Some(mut producer)) => {
                                        self.visit_expr_mut(&mut producer);
                                        self.prepared.push((prepared.clone(), producer));
                                    }
                                    Ok(None) => {
                                        self.error = Some(format!("unbound preprocessing data field {name}").into());
                                        return;
                                    }
                                    Err(error) => { self.error = Some(error); return; }
                                }
                            }
                            *expression = syn::parse_quote!(#prepared);
                            return;
                        }
                        syn::visit_mut::visit_expr_mut(self, expression);
                    }
                }
                let mut fields = Fields {
                    declarations: &declarations,
                    file: &file,
                    exports: &exports,
                    prepared: Vec::new(),
                    error: None,
                    used: Vec::new(),
                };
                fields.visit_expr_mut(&mut expression);
                if let Some(error) = fields.error { return Err(error); }
                if !fields.prepared.is_empty() {
                    let declarations = fields.prepared.iter().map(|(name, producer)|
                        quote!(let #name = #producer;));
                    expression = syn::parse_quote!({ #(#declarations)* #expression });
                }
                preprocessing.push(Preprocess {
                    name: input.name.clone(),
                    columns: input
                        .columns
                        .iter()
                        .map(|c| (c.name.clone(), if dictionary_input {
                            syn::parse_quote!(u32)
                        } else {
                            c.ty.clone()
                        }))
                        .collect(),
                    expression,
                    relations: fields.used,
                });
            }
        }
        Ok(Some(Self {
            file,
            stages,
            aliases,
            terminal: terminal_stages,
            preprocessing,
        }))
    }

    pub fn kernel(
        &self,
        index: usize,
        types: &BTreeMap<String, Vec<(Ident, syn::Type)>>,
    ) -> Result<Kernel> {
        let mut kernel =
            Kernel::standalone_program(&self.file, self.stages[index].program.clone(), types)?;
        kernel.evidence = self.stages[index].evidence.clone();
        Ok(kernel)
    }

    pub fn preprocessing_tokens(&self) -> TokenStream {
        let bindings = self.preprocessing.iter().map(|preprocess| {
            let name = &preprocess.name;
            let types = preprocess.columns.iter().map(|(_, ty)| ty);
            let expression = &preprocess.expression;
            let inputs = preprocess.relations.iter().map(|relation| {
                let input = &relation.name;
                let table = relation.table.to_string();
                let columns: Vec<_> = (0..relation.columns.len()).map(|i| format_ident!("column_{i}")).collect();
                let reads = relation.columns.iter().zip(&columns).map(|((name, ty), variable)| {
                    let name = name.to_string();
                    quote!(let mut #variable = <#ty as crate::manifest::HostValue>::read(manifest, #table, #name)?.into_iter();)
                });
                quote! {
                    #(#reads)*
                    let #input: Vec<_> = (0..manifest.relations[#table].rows)
                        .map(|_| (#(#columns.next().expect("validated manifest row count"),)* )).collect();
                }
            });
            quote!(let #name: Vec<(#(#types,)*)> = { #(#inputs)* #expression };)
        });
        quote!(#(#bindings)*)
    }
}

fn loader_declarations(file: &syn::File) -> Result<Vec<Relation>> {
    struct Declarations(Vec<Relation>);
    impl Parse for Declarations {
        fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
            let mut relations = Vec::new();
            while !input.is_empty() {
                let keyword: Ident = input.parse()?;
                if keyword != "relation" {
                    return Err(input.error("expected relation"));
                }
                let name: Ident = input.parse()?;
                let fields;
                syn::parenthesized!(fields in input);
                let mut columns = Vec::new();
                while !fields.is_empty() {
                    let name = fields.parse()?;
                    fields.parse::<Token![:]>()?;
                    columns.push((name, fields.parse()?));
                    if !fields.is_empty() {
                        fields.parse::<Token![,]>()?;
                    }
                }
                input.parse::<Token![;]>()?;
                let table = name.clone();
                relations.push(Relation {
                    name,
                    table,
                    columns,
                });
            }
            Ok(Self(relations))
        }
    }
    let tokens = file
        .items
        .iter()
        .find_map(|item| match item {
            syn::Item::Macro(item)
                if item
                    .mac
                    .path
                    .segments
                    .last()
                    .is_some_and(|s| s.ident == "relation_loaders" || s.ident == "interned_relation_loaders") =>
            {
                Some(item.mac.tokens.clone())
            }
            _ => None,
        })
        .ok_or("staged source requires relation_loaders declarations")?;
    let mut declarations = syn::parse2::<Declarations>(tokens)?.0;
    let exports = crate::source::export_tables(file)?;
    for relation in &mut declarations {
        let projection: Vec<_> = relation.columns.iter().map(|(name, _)| name.to_string()).collect();
        let matches: Vec<_> = exports.iter().filter(|(_, columns)| *columns == projection).collect();
        if matches.len() != 1 {
            return Err(format!("loader {} requires exactly one EXPORTS projection", relation.name).into());
        }
        relation.table = syn::parse_str(&matches[0].0)?;
    }
    Ok(declarations)
}

/// Ownership maps borrow keys from Sequential's row carrier. Publications
/// already own every field, so remove only dereferences of closure bindings.
pub fn owned_map(map: &syn::ExprClosure) -> syn::ExprClosure {
    struct Own;
    impl VisitMut for Own {
        fn visit_expr_mut(&mut self, expression: &mut syn::Expr) {
            if let syn::Expr::Unary(unary) = expression
                && matches!(unary.op, syn::UnOp::Deref(_))
                && matches!(unary.expr.as_ref(), syn::Expr::Path(_))
            {
                *expression = (*unary.expr).clone();
            }
            syn::visit_mut::visit_expr_mut(self, expression);
        }
    }
    let mut map = map.clone();
    Own.visit_expr_closure_mut(&mut map);
    map
}

pub fn check_identity_map(map: &syn::ExprClosure) -> Result<()> {
    let map = owned_map(map);
    let Some(syn::Pat::Tuple(pattern)) = map.inputs.first() else {
        return Err("published ownership map must destructure its row".into());
    };
    let body = match map.body.as_ref() {
        syn::Expr::Block(block) => match block.block.stmts.last() {
            Some(syn::Stmt::Expr(expression, None)) if block.block.stmts.len() == 1 => expression,
            _ => return Err("published ownership map contains computation".into()),
        },
        expression => expression,
    };
    let syn::Expr::Tuple(tuple) = body else {
        return Err("published ownership map must return its row".into());
    };
    if pattern.elems.len() != tuple.elems.len() {
        return Err("published ownership map changes the row schema".into());
    }
    for (pattern, expression) in pattern.elems.iter().zip(&tuple.elems) {
        let Some(name) = binding(pattern) else {
            return Err("unsupported ownership binding".into());
        };
        let expression = match expression {
            syn::Expr::MethodCall(method) if method.method == "clone" && method.args.is_empty() => {
                method.receiver.as_ref()
            }
            expression => expression,
        };
        if !matches!(expression, syn::Expr::Path(path) if path.path.is_ident(name)) {
            return Err("published ownership map changes a field value or position".into());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn terminal_publication_preserves_stage_order_and_rejects_computation() {
        let aliases = BTreeMap::from([("left".into(), 0), ("right".into(), 1)]);
        let expression = syn::parse_quote!(
            left.chain(right).map(|(code, count)| {
                (data.dictionaries.publish("label", code), count)
            }).collect()
        );
        let mut stages = Vec::new();
        terminal(&expression, &aliases, &mut stages).unwrap();
        assert_eq!(stages, [0, 1]);
        let computation = syn::parse_quote!(
            left.map(|(code, count)| (data.dictionaries.publish("label", code), count + 1))
        );
        assert!(terminal(&computation, &aliases, &mut Vec::new()).is_err());
    }

    #[test]
    fn terminal_stage_maps_defer_dictionary_publication_to_the_gpu_decoder() {
        for name in ["h02", "h15", "h18"] {
            let source = crate::source::SOURCES.iter().find(|(query, _)| *query == name).unwrap().1;
            let program = Program::import(source).unwrap().unwrap();
            for &index in &program.terminal {
                let map = program.stages[index].map.as_ref().unwrap();
                check_identity_map(map).unwrap();
                assert!(!map.to_token_stream().to_string().contains("publish"));
            }
        }
        let projection: syn::ExprClosure = syn::parse_quote!(
            |(code, count)| (data.dictionaries.publish("label", *code), *count, 0)
        );
        let mapped = owned_map(&publication_map(&projection));
        assert_eq!(mapped.to_token_stream().to_string(),
            quote!(|(code, count)| (code, count, 0)).to_string());
        let unrelated: syn::ExprClosure = syn::parse_quote!(|(code,)| (other.publish("label", code),));
        assert_eq!(publication_map(&unrelated), unrelated);
    }
}
