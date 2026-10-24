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

struct Compile(cq_lang_cq::Program);
impl Parse for Compile {
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        let keyword: Ident = input.parse()?;
        if keyword != "pipeline" {
            return Err(input.error("expected pipeline"));
        }
        let pipeline;
        syn::braced!(pipeline in input);
        if !pipeline.is_empty() {
            return Err(pipeline.error("expected authored default pipeline"));
        }
        Ok(Self(input.parse()?))
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
            Ok(Some(Stage {
                program: syn::parse2::<Compile>(mac.mac.tokens.clone())?.0,
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
        syn::Expr::MethodCall(method) if method.method == "collect" => {
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

impl Program {
    pub fn import(source: &str) -> Result<Option<Self>> {
        let file = syn::parse_file(source)?;
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
        // Inputs not exported and not published are authored one-time host
        // preprocessing. Preserve its expression; do not replicate predicates.
        let mut preprocessing = Vec::new();
        let declarations = loader_declarations(&file)?;
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
                            if matches!(local.pat, syn::Pat::Type(_))
                                && binding(&local.pat) == Some(&input.name) =>
                        {
                            local.init.as_ref()
                        }
                        _ => None,
                    })
                    .ok_or_else(|| format!("unbound stage input {}", input.name))?;
                let mut expression = (*initializer.expr).clone();
                struct Fields<'a> {
                    declarations: &'a [Relation],
                    used: Vec<Relation>,
                }
                impl VisitMut for Fields<'_> {
                    fn visit_expr_mut(&mut self, expression: &mut syn::Expr) {
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
                        syn::visit_mut::visit_expr_mut(self, expression);
                    }
                }
                let mut fields = Fields {
                    declarations: &declarations,
                    used: Vec::new(),
                };
                fields.visit_expr_mut(&mut expression);
                preprocessing.push(Preprocess {
                    name: input.name.clone(),
                    columns: input
                        .columns
                        .iter()
                        .map(|c| (c.name.clone(), c.ty.clone()))
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
        Kernel::standalone_program(&self.file, self.stages[index].program.clone(), types)
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
                let table = format_ident!("{}", name.to_string().to_lowercase());
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
                    .is_some_and(|s| s.ident == "relation_loaders") =>
            {
                Some(item.mac.tokens.clone())
            }
            _ => None,
        })
        .ok_or("staged source requires relation_loaders declarations")?;
    Ok(syn::parse2::<Declarations>(tokens)?.0)
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
