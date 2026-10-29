//! Import authored CPU queries, including standalone grouped compile! programs.
//! Standalone sources retain aggregate heads, Rust expressions and result
//! declarations. EXPORTS projections identify loader tables and column domains.
//! Ordinary run bindings to Data code sets resolve to their load-time producers.
//! Dictionary inputs use manifest IDs regardless of Rust type spellings; only
//! relational operands and literal captures are adapted, never authored Rust.
//! Three-argument `dictionary_where` closures run once per dictionary entry at
//! load time. Decimal/Date Rust reads retain their scalar ABI.
//! Staged sources retain every rule, dependency and final iterator composition.
use crate::manifest::{Manifest, Result};
use proc_macro2::{Group, TokenStream, TokenTree};
use quote::{ToTokens, quote};
use std::collections::BTreeMap;
use syn::{
    Ident, Token,
    parse::{Parse, ParseStream},
    punctuated::Punctuated,
};

/// Unwrap the shared CPU configuration shell without expanding its variants.
/// GPU planning still consumes the single authored CQ, not a CPU plan.
pub(crate) fn authored_file(source: &str) -> Result<syn::File> {
    use syn::visit_mut::VisitMut;
    struct Runner(syn::ItemFn);
    impl Parse for Runner {
        fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
            input.parse::<Token![$]>()?;
            input.parse::<Token![;]>()?;
            Ok(Self(input.parse()?))
        }
    }
    struct QueryBody(syn::ItemFn);
    impl Parse for QueryBody {
        fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
            let pattern;
            syn::parenthesized!(pattern in input);
            if !pattern.is_empty() {
                return Err(pattern.error("query_body must have no macro arguments"));
            }
            input.parse::<Token![=>]>()?;
            let body;
            syn::braced!(body in input);
            let runner = body.parse()?;
            if !body.is_empty() {
                return Err(body.error("query_body must contain one runner"));
            }
            if input.peek(Token![;]) { input.parse::<Token![;]>()?; }
            Ok(Self(runner))
        }
    }
    struct Query(TokenStream);
    impl Parse for Query {
        fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
            if input.peek(Ident) && input.fork().parse::<Ident>()? == "plan" {
                let _: Ident = input.parse()?;
                let advice;
                syn::parenthesized!(advice in input);
                let _: TokenStream = advice.parse()?;
                input.parse::<Token![;]>()?;
            }
            Ok(Self(input.parse()?))
        }
    }
    struct Queries(Option<syn::Error>);
    impl VisitMut for Queries {
        fn visit_macro_mut(&mut self, mac: &mut syn::Macro) {
            if mac.path.is_ident("query") {
                match syn::parse2::<Query>(mac.tokens.clone()) {
                    Ok(Query(body)) => {
                        mac.path = syn::parse_quote!(mini_linq::compile);
                        mac.tokens = quote!(pipeline {} #body);
                    }
                    Err(error) => self.0 = Some(error),
                }
            }
        }
    }
    // A query-local configuration shell contains one authored runner. Inspect
    // its declaration, never expand its CPU configuration invocations.
    fn configuration(tokens: TokenStream) -> Option<TokenStream> {
        let tokens: Vec<_> = tokens.into_iter().collect();
        for window in tokens.windows(3) {
            if matches!(&window[0], TokenTree::Ident(name) if name == "configuration")
                && matches!(&window[1], TokenTree::Punct(mark) if mark.as_char() == '!')
                && let TokenTree::Group(arguments) = &window[2]
            {
                return arguments.stream().into_iter().filter_map(|token| match token {
                    TokenTree::Group(group) if group.delimiter() == proc_macro2::Delimiter::Brace =>
                        Some(group.stream()),
                    _ => None,
                }).last();
            }
        }
        tokens.into_iter().find_map(|token| match token {
            TokenTree::Group(group) => configuration(group.stream()),
            _ => None,
        })
    }
    let mut file = syn::parse_file(source)?;
    for item in &mut file.items {
        if let syn::Item::Macro(shell) = item
            && shell.mac.path.segments.last().is_some_and(|s| s.ident == "standalone_configs")
        {
            let Runner(mut runner) = syn::parse2(shell.mac.tokens.clone())?;
            let mut queries = Queries(None);
            queries.visit_block_mut(&mut runner.block);
            if let Some(error) = queries.0 {
                return Err(error.into());
            }
            *item = syn::Item::Fn(runner);
        }
        else if let syn::Item::Macro(shell) = item
            && shell.ident.as_ref().is_some_and(|name| name == "runner")
            && let Some(body) = configuration(shell.mac.tokens.clone())
        {
            let body: syn::File = syn::parse2(body)?;
            let mut runner = body.items.into_iter().find_map(|item| match item {
                syn::Item::Fn(function) if function.sig.ident == "run" => Some(function),
                _ => None,
            }).ok_or("configuration shell has no run function")?;
            let mut queries = Queries(None);
            queries.visit_block_mut(&mut runner.block);
            if let Some(error) = queries.0 { return Err(error.into()); }
            *item = syn::Item::Fn(runner);
        }
        else if let syn::Item::Macro(shell) = item
            && shell.ident.as_ref().is_some_and(|name| name == "query_body")
        {
            let QueryBody(mut runner) = syn::parse2(shell.mac.tokens.clone())?;
            let mut queries = Queries(None);
            queries.visit_block_mut(&mut runner.block);
            if let Some(error) = queries.0 { return Err(error.into()); }
            *item = syn::Item::Fn(runner);
        }
    }
    Ok(file)
}

macro_rules! sources {
    ($($name:literal),* $(,)?) => { pub const SOURCES: &[(&str, &str)] = &[$(
        ($name, include_str!(concat!("../../../../features/workloads/src/queries/retail/tpch/retail_", $name, ".rs"))),
    )*]; };
}
sources!(
    "h01",
    "h02",
    "h06",
    "h11",
    "h13",
    "h15",
    "h18",
    "h20",
    "h10",
    "h12",
    "h19",
    "h03",
    "h04",
    "h04_not_exists",
    "h05",
    "h07",
    "h07_block",
    "h08",
    "h09",
    "h16",
    "h22",
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
    pub plan_advice: TokenStream,
    pub evidence: cq_lang_sequence::Evidence,
    pub setup: syn::Block,
    pub relations: Vec<Relation>,
    pub answer: Vec<(Ident, syn::Type)>,
    pub derived: Vec<(Ident, Vec<(Ident, syn::Type)>)>,
    /// Inputs published by preceding stages, in declaration order.
    pub published: Vec<Relation>,
    /// Source dictionary domains travel independently of published Rust types.
    pub published_origins: BTreeMap<String, Vec<Option<(String, String)>>>,
    pub body: TokenStream,
    /// Declared loader projections, including dictionary-only producers.
    pub exports: Vec<(String, Vec<String>)>,
    /// Standalone programs retain their declared output and aggregate head.
    program: Option<cq_lang_cq::Program>,
}

/// The shared source prefix carries author premises, not CPU planning choices.
pub(crate) struct AuthoredQuery {
    pub program: cq_lang_cq::Program,
    pub evidence: cq_lang_sequence::Evidence,
}

impl Parse for AuthoredQuery {
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
        let evidence = if input.peek(Ident) {
            input.parse()?
        } else {
            Default::default()
        };
        Ok(Self { program: input.parse()?, evidence })
    }
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
        let mut plan_advice = TokenStream::new();
        let setup = loop {
            let key: Ident = input.parse()?;
            if key == "setup" {
                break input.parse()?;
            }
            if key == "advise" {
                let phase: Ident = input.parse()?;
                if phase != "plan" {
                    return Err(syn::Error::new_spanned(phase, "expected plan advice"));
                }
                let arguments;
                syn::parenthesized!(arguments in input);
                plan_advice = arguments.parse()?;
                input.parse::<Token![;]>()?;
                continue;
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
        let mut key: Ident = input.parse()?;
        let mut derived = Vec::new();
        if key == "derived" {
            let declarations;
            syn::braced!(declarations in input);
            while !declarations.is_empty() {
                let keyword: Ident = declarations.parse()?;
                if keyword != "relation" {
                    return Err(syn::Error::new_spanned(keyword, "expected relation"));
                }
                let name = declarations.parse()?;
                let fields;
                syn::parenthesized!(fields in declarations);
                derived.push((name, columns(&fields)?));
                declarations.parse::<Token![;]>()?;
            }
            key = input.parse()?;
        }
        if key != "answer" {
            return Err(syn::Error::new_spanned(key, "expected answer"));
        }
        let fields;
        syn::parenthesized!(fields in input);
        let answer = columns(&fields)?;
        input.parse::<Token![:]>()?;
        input.parse::<Token![-]>()?;
        let exports = relations.iter().map(|relation| (
            relation.table.to_string(),
            relation.columns.iter().map(|(column, _)| column.to_string()).collect(),
        )).collect();
        Ok(Self {
            plan_advice,
            evidence: Default::default(),
            setup,
            relations,
            derived,
            published: Vec::new(),
            published_origins: BTreeMap::new(),
            exports,
            answer,
            program: None,
            body: input.parse()?,
        })
    }
}

fn local_name(pattern: &syn::Pat) -> Option<&Ident> {
    match pattern {
        syn::Pat::Ident(name) => Some(&name.ident),
        syn::Pat::Type(typed) => local_name(&typed.pat),
        _ => None,
    }
}

/// Explicit authored input declaration; its predicate is never inspected.
pub fn dictionary_predicate(expression: &syn::Expr) -> Result<Option<(String, &syn::ExprClosure)>> {
    let syn::Expr::Call(call) = expression else { return Ok(None) };
    let syn::Expr::Path(path) = call.func.as_ref() else { return Ok(None) };
    if !path.path.segments.last().is_some_and(|part| part.ident == "dictionary_where") {
        return Ok(None);
    }
    if call.args.len() != 3 {
        return Err("dictionary_where requires dictionaries, column and predicate".into());
    }
    let syn::Expr::Lit(syn::ExprLit { lit: syn::Lit::Str(column), .. }) = &call.args[1] else {
        return Err("dictionary_where domain must use a column literal".into());
    };
    let syn::Expr::Closure(predicate) = &call.args[2] else {
        return Err("dictionary_where requires an authored predicate closure".into());
    };
    Ok(Some((column.value(), predicate)))
}

pub fn dictionary_initializer(
    expression: &syn::Expr,
    exports: &[(String, Vec<String>)],
) -> Result<Option<TokenStream>> {
    let Some((column, predicate)) = dictionary_predicate(expression)? else {
        return Ok(None);
    };
    let table = column_table(exports, &column)?;
    Ok(Some(quote!({
        let dictionary = manifest.dictionary(manifest.column(#table, #column)?)?;
        let mut predicate = #predicate;
        // DECODE R: explicit authored dictionary_where closure reads each dictionary value once; its derived code set feeds relational joins.
        dictionary.iter().enumerate().filter_map(|(code, value)| {
            predicate(value.as_str()).then(|| (
                u32::try_from(code).expect("validated dictionary size"),
            ))
        }).collect::<Vec<(u32,)>>()
    })))
}

fn column_table<'a>(exports: &'a [(String, Vec<String>)], column: &str) -> Result<&'a str> {
    let mut matches = exports.iter().filter(|(_, columns)| columns.iter().any(|name| name == column));
    let table = matches.next().ok_or_else(|| format!("no EXPORTS provenance for {column}"))?;
    if matches.next().is_some() {
        return Err(format!("ambiguous EXPORTS provenance for {column}").into());
    }
    Ok(&table.0)
}

/// Follow an ordinary run binding to its explicit load-time Data producer.
pub(crate) fn load_initializer(file: &syn::File, expression: &syn::Expr) -> Result<syn::Expr> {
    fn data_field(expression: &syn::Expr) -> Option<&Ident> {
        match expression {
            syn::Expr::Reference(reference) => data_field(&reference.expr),
            syn::Expr::MethodCall(method) if method.method == "as_slice" =>
                data_field(&method.receiver),
            syn::Expr::Field(field)
                if matches!(field.base.as_ref(), syn::Expr::Path(path) if path.path.is_ident("data")) =>
                match &field.member { syn::Member::Named(name) => Some(name), _ => None },
            _ => None,
        }
    }
    let Some(field) = data_field(expression) else { return Ok(expression.clone()) };
    Ok(load_field_initializer(file, field)?.unwrap_or_else(|| expression.clone()))
}

pub(crate) fn load_field_initializer(file: &syn::File, field: &Ident) -> Result<Option<syn::Expr>> {
    use syn::visit_mut::VisitMut;
    let Some(load) = file.items.iter().find_map(|item| match item {
        syn::Item::Fn(function) if function.sig.ident == "load" => Some(function),
        _ => None,
    }) else { return Ok(None) };
    struct Fields(BTreeMap<String, syn::Expr>);
    impl VisitMut for Fields {
        fn visit_expr_struct_mut(&mut self, value: &mut syn::ExprStruct) {
            if value.path.is_ident("Data") {
                for field in &value.fields {
                    if let syn::Member::Named(name) = &field.member {
                        self.0.insert(name.to_string(), field.expr.clone());
                    }
                }
            }
            syn::visit_mut::visit_expr_struct_mut(self, value);
        }
    }
    let mut fields = Fields(BTreeMap::new());
    fields.visit_block_mut(&mut load.block.clone());
    let Some(mut producer) = fields.0.remove(&field.to_string()) else {
        return Ok(None);
    };
    // Data { codes } and Data { codes: selected } both retain the authored
    // producer; only load locals are followed, never row-dependent Rust.
    let mut seen = std::collections::BTreeSet::new();
    while let syn::Expr::Path(path) = &producer {
        let Some(name) = path.path.get_ident() else { break };
        if !seen.insert(name.to_string()) {
            return Err("cyclic load initializer".into());
        }
        let Some(value) = load.block.stmts.iter().find_map(|statement| match statement {
            syn::Stmt::Local(local) if local_name(&local.pat) == Some(name) =>
                local.init.as_ref().map(|init| init.expr.as_ref()),
            _ => None,
        }) else { break };
        producer = value.clone();
    }
    Ok(Some(producer))
}

fn dictionary_literal(expression: &syn::Expr) -> Option<&syn::ExprMethodCall> {
    match expression {
        syn::Expr::MethodCall(method) if method.method == "literal" => Some(method),
        _ => None,
    }
}

pub(crate) fn export_tables(file: &syn::File) -> Result<Vec<(String, Vec<String>)>> {
    let exports = file.items.iter().find_map(|item| match item {
        syn::Item::Const(item) if item.ident == "EXPORTS" => Some(item.expr.as_ref()),
        _ => None,
    }).ok_or("CPU source has no EXPORTS")?;
    let syn::Expr::Reference(exports) = exports else {
        return Err("EXPORTS must be a literal slice".into());
    };
    let syn::Expr::Array(exports) = exports.expr.as_ref() else {
        return Err("EXPORTS must be a literal slice".into());
    };
    exports.elems.iter().map(|export| {
        let syn::Expr::Tuple(pair) = export else {
            return Err("EXPORTS entries must be table/projection pairs".into());
        };
        let strings = pair.elems.iter().map(|value| match value {
            syn::Expr::Lit(syn::ExprLit { lit: syn::Lit::Str(value), .. }) => Ok(value.value()),
            _ => Err("EXPORTS table/projection must be literals"),
        }).collect::<std::result::Result<Vec<_>, _>>()?;
        let [table, projection] = strings.as_slice() else {
            return Err("EXPORTS entries must have two fields".into());
        };
        Ok((table.clone(), projection.split(',').map(|column| column.trim().to_owned()).collect()))
    }).collect()
}

impl Kernel {
    /// Reuse the authored one-time input preprocessing, not a second predicate.
    pub fn derived_input(&self, name: &Ident) -> Result<(&syn::Expr, Vec<&Relation>)> {
        let expression = self
            .setup
            .stmts
            .iter()
            .find_map(|statement| {
                let syn::Stmt::Local(local) = statement else {
                    return None;
                };
                (local_name(&local.pat) == Some(name))
                    .then(|| local.init.as_ref().map(|init| init.expr.as_ref()))
                    .flatten()
            })
            .ok_or_else(|| format!("derived relation {name} lacks an authored initializer"))?;
        if dictionary_predicate(expression)?.is_some() {
            return Ok((expression, Vec::new()));
        }
        fn mentions(tokens: TokenStream, name: &Ident) -> bool {
            tokens.into_iter().any(|token| match token {
                TokenTree::Ident(ident) => ident == *name,
                TokenTree::Group(group) => mentions(group.stream(), name),
                _ => false,
            })
        }
        let inputs = self
            .relations
            .iter()
            .filter(|relation| mentions(expression.to_token_stream(), &relation.name))
            .collect();
        Ok((expression, inputs))
    }

    pub fn import(source: &str) -> Result<Self> {
        let mut file = authored_file(source)?;
        for item in &file.items {
            if let syn::Item::Macro(item) = item
                && item
                    .mac
                    .path
                    .segments
                    .last()
                    .is_some_and(|s| s.ident == "retail_kernel")
            {
                return Ok(syn::parse2(item.mac.tokens.clone())?);
            }
        }
        Self::standalone(&mut file)
    }

    fn standalone(file: &mut syn::File) -> Result<Self> {
        use syn::visit_mut::VisitMut;
        struct Compiles(Vec<TokenStream>);
        impl VisitMut for Compiles {
            fn visit_macro_mut(&mut self, mac: &mut syn::Macro) {
                if mac
                    .path
                    .segments
                    .last()
                    .is_some_and(|s| s.ident == "compile")
                {
                    self.0.push(mac.tokens.clone());
                }
            }
        }
        let mut compiles = Compiles(Vec::new());
        let run = file
            .items
            .iter_mut()
            .find_map(|item| match item {
                syn::Item::Fn(function) if function.sig.ident == "run" => Some(function),
                _ => None,
            })
            .ok_or("standalone CPU source has no run function")?;
        compiles.visit_block_mut(&mut run.block);
        if compiles.0.len() != 1 {
            return Err(format!(
                "standalone run contains {} compile! stages; use the staged importer to retain materialization and iterator composition",
                compiles.0.len()
            ).into());
        }
        let source = syn::parse2::<AuthoredQuery>(compiles.0.remove(0))?;
        let mut kernel = Self::standalone_program(file, source.program, &BTreeMap::new())?;
        kernel.evidence = source.evidence;
        Ok(kernel)
    }

    pub fn standalone_program(
        file: &syn::File,
        program: cq_lang_cq::Program,
        publications: &BTreeMap<String, Vec<(Ident, syn::Type)>>,
    ) -> Result<Self> {
        let run = file
            .items
            .iter()
            .find_map(|item| match item {
                syn::Item::Fn(function) if function.sig.ident == "run" => Some(function),
                _ => None,
            })
            .ok_or("standalone CPU source has no run function")?;
        let body = program.query.body.to_token_stream();
        let mut setup: syn::Block = syn::parse_quote!({});
        for statement in &run.block.stmts {
            let syn::Stmt::Local(local) = statement else { continue };
            let Some(name) = local_name(&local.pat) else { continue };
            if !mentions(body.clone(), name) { continue; }
            let mut local = local.clone();
            if let Some(init) = &mut local.init {
                let producer = load_initializer(file, &init.expr)?;
                if program.inputs().any(|input| input.name == *name)
                    && dictionary_predicate(&producer)?.is_none()
                {
                    continue;
                }
                init.expr = Box::new(producer);
            }
            setup.stmts.push(syn::Stmt::Local(local));
        }
        let tables = export_tables(file)?;
        let mut relations = Vec::new();
        let mut published = Vec::new();
        let mut derived = Vec::new();
        for input in program.inputs() {
            if let Some(columns) = publications.get(&input.name.to_string()) {
                published.push(Relation {
                    name: input.name.clone(),
                    table: input.name.clone(),
                    columns: columns.clone(),
                });
                continue;
            }
            if setup.stmts.iter().any(|statement| matches!(statement,
                syn::Stmt::Local(local) if local_name(&local.pat) == Some(&input.name)
            )) {
                derived.push((input.name.clone(), input.columns.iter()
                    .map(|column| (column.name.clone(), column.ty.clone())).collect()));
                continue;
            }
            let projection: Vec<_> = input
                .columns
                .iter()
                .map(|column| column.name.to_string())
                .collect();
            let matches: Vec<_> = tables
                .iter()
                .filter(|(_, columns)| *columns == projection)
                .collect();
            if matches.len() != 1 {
                return Err(format!(
                    "standalone input {} requires exactly one matching EXPORTS projection",
                    input.name
                )
                .into());
            }
            relations.push(Relation {
                name: input.name.clone(),
                table: syn::parse_str(&matches[0].0)?,
                columns: input
                    .columns
                    .iter()
                    .map(|column| (column.name.clone(), column.ty.clone()))
                    .collect(),
            });
        }
        let answer = program
            .output()
            .ok_or("authored query requires an answer relation declaration")?
            .columns
            .iter()
            .map(|column| (column.name.clone(), column.ty.clone()))
            .collect();
        Ok(Self {
            plan_advice: TokenStream::new(),
            evidence: Default::default(),
            setup,
            relations,
            answer,
            derived,
            published,
            published_origins: BTreeMap::new(),
            exports: tables,
            body,
            program: Some(program),
        })
    }

    pub fn is_standalone(&self) -> bool {
        self.program.is_some()
    }

    fn cpu_program(&self) -> Result<cq_lang_cq::Program> {
        Ok(syn::parse2(self.cpu_query())?)
    }

    /// The exact macro substitution made by tpch_runtime::retail_kernel.
    pub fn cpu_query(&self) -> TokenStream {
        if let Some(program) = &self.program {
            return program.to_token_stream();
        }
        let declarations = self.relations.iter().map(|relation| {
            let name = &relation.name;
            let fields = relation.columns.iter().map(|(name, ty)| quote!(#name: #ty));
            quote!(relation #name(#(#fields),*);)
        });
        let outputs = self.answer.iter().map(|(name, _)| name);
        let derived = self.derived.iter().map(|(name, columns)| {
            let fields = columns.iter().map(|(column, ty)| quote!(#column: #ty));
            quote!(relation #name(#(#fields),*);)
        });
        let body = &self.body;
        quote!(struct Query; #(#declarations)* #(#derived)* answer(#(#outputs),*) :- #body)
    }

    /// Dictionary IDs are a loader representation fact, not a Rust capability.
    /// Use the authored interner's unsigned code domain, retaining manifest order.
    pub fn dictionary_types(
        &self,
        manifest: &Manifest,
    ) -> Result<BTreeMap<(String, String), syn::Type>> {
        let mut storage = BTreeMap::new();
        let dictionaries = self.dictionary_relations()?;
        for relation in self.relations.iter().chain(&dictionaries) {
            for (column, _) in &relation.columns {
                let origin = (relation.table.to_string(), column.to_string());
                if manifest.column(&origin.0, &origin.1)?.encoding == "dictionary" {
                    storage.insert(origin, syn::parse_quote!(u32));
                }
            }
        }
        for origin in self.published_origins.values().flatten().flatten() {
            if manifest.column(&origin.0, &origin.1)?.encoding == "dictionary" {
                storage.insert(origin.clone(), syn::parse_quote!(u32));
            }
        }
        Ok(storage)
    }

    /// Provider codecs may adapt relational-only columns, never Rust reads.
    pub fn scalar_codec_types(&self, manifest: &Manifest) -> Result<BTreeMap<(String, String), syn::Type>> {
        let program = self.cpu_program()?;
        let variables = self.variable_origins()?;
        let mut storage = BTreeMap::new();
        for relation in &self.relations {
            for (column, _) in &relation.columns {
                let origin = (relation.table.to_string(), column.to_string());
                let metadata = manifest.column(&origin.0, &origin.1)?;
                if matches!(metadata.encoding.as_str(), "days_since_epoch" | "scaled_integer")
                    && variables.iter().all(|(variable, source)| source != &origin
                        || (scalar_codec_body(&program.query.body, variable)
                            && program.query.head.args.iter().all(|argument| {
                                matches!(argument, syn::Expr::Path(path) if path.path.is_ident(variable))
                                    || dictionary_expression(argument, variable)
                            })))
                {
                    storage.insert(origin, syn::parse_str(&metadata.ty)?);
                }
            }
        }
        Ok(storage)
    }

    fn dictionary_relations(&self) -> Result<Vec<Relation>> {
        let mut relations = Vec::new();
        for statement in &self.setup.stmts {
            let syn::Stmt::Local(local) = statement else { continue };
            let Some(name) = local_name(&local.pat) else { continue };
            let Some(init) = &local.init else { continue };
            if let Some((column, _)) = dictionary_predicate(&init.expr)? {
                let table = column_table(&self.exports, &column)?;
                relations.push(Relation {
                    name: name.clone(),
                    table: syn::parse_str(table)?,
                    columns: vec![(syn::parse_str(&column)?, syn::parse_quote!(u32))],
                });
            }
        }
        Ok(relations)
    }

    pub fn variable_origins(&self) -> Result<BTreeMap<String, (String, String)>> {
        let mut variables = BTreeMap::new();
        collect_variables(self.body.clone(), &self.relations, &mut variables)?;
        collect_variables(self.body.clone(), &self.dictionary_relations()?, &mut variables)?;
        fn published(
            body: &cq_lang_cq::CommaList<cq_lang_cq::BodyItem>,
            inputs: &BTreeMap<String, Vec<Option<(String, String)>>>,
            variables: &mut BTreeMap<String, (String, String)>,
        ) {
            for item in body {
                match item {
                    cq_lang_cq::BodyItem::Positive { atom } => {
                        if let Some(origins) = inputs.get(&atom.relation.to_string()) {
                            for (argument, origin) in atom.args.iter().zip(origins) {
                                if let Some(origin) = origin
                                    && let syn::Expr::Path(path) = argument
                                    && let Some(name) = path.path.get_ident()
                                {
                                    variables.insert(name.to_string(), origin.clone());
                                }
                            }
                        }
                    }
                    cq_lang_cq::BodyItem::Block(block) => {
                        for branch in &block.branches {
                            published(&branch.body, inputs, variables);
                        }
                    }
                    _ => {}
                }
            }
        }
        published(&self.cpu_program()?.query.body, &self.published_origins, &mut variables);
        Ok(variables)
    }

    pub fn setup_bindings(&self, manifest: &Manifest) -> Result<TokenStream> {
        let constants = setup_constants(&self.setup, &self.derived, &self.exports, manifest)?;
        let mut bindings = TokenStream::new();
        for statement in &self.setup.stmts {
            let syn::Stmt::Local(local) = statement else { continue };
            let name = match &local.pat {
                syn::Pat::Ident(name) => Some(&name.ident),
                syn::Pat::Type(typed) => match &*typed.pat {
                    syn::Pat::Ident(name) => Some(&name.ident),
                    _ => None,
                },
                _ => None,
            };
            if let Some(value) = name.and_then(|name| constants.get(&name.to_string())) {
                let pattern = if local.init.as_ref().is_some_and(|init| dictionary_literal(&init.expr).is_some()) {
                    local_name(&local.pat).unwrap().to_token_stream()
                } else {
                    local.pat.to_token_stream()
                };
                bindings.extend(quote!(let #pattern = #value;));
            }
        }
        Ok(bindings)
    }

    #[cfg(test)]
    /// Storage adaptation only: types and constants, not order/providers/plans.
    pub fn encoded_query(&self, manifest: &Manifest) -> Result<TokenStream> {
        self.encoded_query_with_types(manifest, &self.dictionary_types(manifest)?)
    }

    /// A loader profile may replace stored scalar types; predicates and atom
    /// order remain the imported source. Dictionary literals use that column's
    /// manifest dictionary, never an assumed global ID.
    pub fn encoded_query_with_types(
        &self,
        manifest: &Manifest,
        storage: &BTreeMap<(String, String), syn::Type>,
    ) -> Result<TokenStream> {
        let mut relations = self.relations.clone();
        for relation in &mut relations {
            for (column, ty) in &mut relation.columns {
                if let Some(stored) = storage.get(&(relation.table.to_string(), column.to_string()))
                {
                    *ty = stored.clone();
                }
            }
        }
        let variables = self.variable_origins()?;
        relations.extend(self.dictionary_relations()?);
        let mut body = self.cpu_program()?.query.body;
        let dictionary_relations: Vec<_> = relations.iter().cloned().map(|mut relation| {
            relation.columns.retain(|(column, _)| storage.contains_key(&(relation.table.to_string(), column.to_string())));
            relation
        }).collect();
        encode_dictionary_body(&mut body, &dictionary_relations, manifest, &variables)?;
        encode_atom_body(&mut body, &relations, &dictionary_relations, manifest)?;
        let body = body.to_token_stream();
        let mut declarations = TokenStream::new();
        for relation in &self.relations {
            let name = &relation.name;
            let mut fields = Vec::new();
            for (column, logical) in &relation.columns {
                let _ = logical;
                let ty = &relations
                    .iter()
                    .find(|encoded| encoded.name == relation.name)
                    .expect("cloned input relation")
                    .columns
                    .iter()
                    .find(|(name, _)| name == column)
                    .expect("cloned input column")
                    .1;
                let ty = ty.clone();
                fields.push(quote!(#column: #ty));
            }
            declarations.extend(quote!(relation #name(#(#fields),*);));
        }
        for (name, columns) in &self.derived {
            let dictionary = relations.iter().any(|relation| relation.name == *name);
            if dictionary && columns.len() != 1 {
                return Err("dictionary_where input must have one code column".into());
            }
            let fields = columns.iter().map(|(column, ty)| {
                if dictionary { quote!(#column: u32) } else { quote!(#column: #ty) }
            });
            declarations.extend(quote!(relation #name(#(#fields),*);));
        }
        for relation in &self.published {
            let name = &relation.name;
            let fields = relation
                .columns
                .iter()
                .map(|(column, ty)| quote!(#column: #ty));
            declarations.extend(quote!(relation #name(#(#fields),*);));
        }
        if let Some(program) = &self.program {
            let head = program.query.head.clone();
            let answer_name = &head.relation;
            let origins = self.answer_columns()?;
            let fields = self
                .answer
                .iter()
                .zip(origins)
                .map(|((name, logical), origin)| {
                    let ty = origin
                        .as_ref()
                        .and_then(|(table, column)| {
                            relations
                                .iter()
                                .find(|relation| relation.table == table.as_str())
                                .and_then(|relation| {
                                    relation
                                        .columns
                                        .iter()
                                        .find(|(name, _)| name == column.as_str())
                                })
                                .map(|(_, ty)| ty)
                        })
                        .or_else(|| origin.as_ref().and_then(|origin| storage.get(origin)))
                        .unwrap_or(logical);
                    quote!(#name: #ty)
                });
            return Ok(
                quote!(struct Query; #declarations relation #answer_name(#(#fields),*); #head :- #body;),
            );
        }
        let outputs = self.answer.iter().map(|(name, _)| name);
        Ok(quote!(struct Query; #declarations answer(#(#outputs),*) :- #body;))
    }

    pub fn answer_columns(&self) -> Result<Vec<Option<(String, String)>>> {
        let variables = self.variable_origins()?;
        Ok(self
            .cpu_program()?
            .query
            .head
            .args
            .iter()
            .map(|argument| {
                if let syn::Expr::Path(path) = argument {
                    path.path
                        .get_ident()
                        .and_then(|name| variables.get(&name.to_string()))
                        .cloned()
                } else {
                    None
                }
            })
            .collect())
    }
}
// Scalar codecs may adapt relational operands without inspecting Rust.
fn dictionary_expression(expression: &syn::Expr, variable: &str) -> bool {
    let free = cq_lang_cq::rust::free_variables(expression);
    !free.opaque && !free.names.contains(variable)
}


fn scalar_codec_body(body: &cq_lang_cq::CommaList<cq_lang_cq::BodyItem>, variable: &str) -> bool {
    use cq_lang_cq::BodyItem;
    body.iter().all(|item| match item {
        _ if item.rust_step().is_some_and(|step|
            step.free_variables.opaque || step.free_variables.names.contains(variable)) => false,
        BodyItem::Block(block) => block.branches.iter()
            .all(|branch| scalar_codec_body(&branch.body, variable)),
        BodyItem::Equality(_) | BodyItem::Constraint(_) | BodyItem::Positive { .. } => true,
        _ => !mentions(item.to_token_stream(), &quote::format_ident!("{variable}")),
    })
}

fn encode_dictionary_body(
    body: &mut cq_lang_cq::CommaList<cq_lang_cq::BodyItem>,
    relations: &[Relation],
    manifest: &Manifest,
    variables: &BTreeMap<String, (String, String)>,
) -> Result<()> {
    use cq_lang_cq::syntax::constraint::Term;
    use cq_lang_cq::{BodyItem, Constraint};

    fn literal(
        term: &Term,
        expression: &mut syn::Expr,
        relations: &[Relation],
        manifest: &Manifest,
        variables: &BTreeMap<String, (String, String)>,
    ) -> Result<()> {
        if let Term::Variable(name) = term
            && let Some((table, column)) = variables.get(&name.to_string())
            && relations.iter().any(|relation| {
                relation.table == table.as_str()
                    && relation.columns.iter().any(|(column_name, _)| column_name == column.as_str())
            })
        {
            match manifest.column(table, column)?.encoding.as_str() {
                "days_since_epoch" => {
                    *expression = syn::parse_quote!((#expression).days_since_epoch());
                    return Ok(());
                }
                "scaled_integer" => {
                    *expression = syn::parse_quote!((#expression).scaled());
                    return Ok(());
                }
                "dictionary" => {}
                _ => return Ok(()),
            }
            if let syn::Expr::Lit(syn::ExprLit { lit: syn::Lit::Str(text), .. }) = expression {
                let code = manifest.text_id(table, column, &text.value())?;
                let code = if code < 0 { u32::MAX } else { u32::try_from(code)? };
                *expression = syn::parse_quote!(#code);
            } else {
                let dictionary = manifest.dictionary(manifest.column(table, column)?)?;
                *expression = syn::parse_quote!({
                    let value = #expression;
                    let dictionary: &[&str] = &[#(#dictionary),*];
                    crate::manifest::DictionaryLiteral::dictionary_code(&value, dictionary)
                });
            }
        }
        Ok(())
    }

    fn operand(term: &mut Term) -> Option<&mut syn::Expr> {
        match term {
            Term::Literal(expression) => Some(expression.as_mut()),
            Term::Closed(expression) => Some(expression),
            Term::Variable(_) => None,
        }
    }

    fn constraint(
        predicate: &mut Constraint,
        relations: &[Relation],
        manifest: &Manifest,
        variables: &BTreeMap<String, (String, String)>,
    ) -> Result<()> {
        let dictionary = |term: &Term| -> Result<bool> {
            let Term::Variable(name) = term else { return Ok(false) };
            let Some((table, column)) = variables.get(&name.to_string()) else { return Ok(false) };
            Ok(relations.iter().any(|relation| relation.table == table.as_str()
                && relation.columns.iter().any(|(name, _)| name == column.as_str()))
                && manifest.column(table, column)?.encoding == "dictionary")
        };
        match predicate {
            Constraint::Compare {
                left,
                comparison,
                right,
            } => {
                if comparison.ordered() && (dictionary(left)? || dictionary(right)?) {
                    return Err("dictionary IDs do not preserve lexical order; use an explicit dictionary predicate".into());
                }
                if let Some(expression) = operand(right) {
                    literal(left, expression, relations, manifest, variables)?;
                }
                if let Some(expression) = operand(left) {
                    literal(right, expression, relations, manifest, variables)?;
                }
            }
            Constraint::In { term, literals } => {
                for expression in literals {
                    literal(term, expression, relations, manifest, variables)?;
                }
            }
            Constraint::Between { term, low, high } => {
                if dictionary(term)? {
                    return Err("dictionary IDs do not preserve lexical order; use an explicit dictionary predicate".into());
                }
                for bound in [low, high] {
                    if let Some(expression) = operand(bound) {
                        literal(term, expression, relations, manifest, variables)?;
                    }
                }
            }
            Constraint::Or(arms) => {
                for predicate in arms.iter_mut().flatten() {
                    constraint(predicate, relations, manifest, variables)?;
                }
            }
        }
        Ok(())
    }

    for item in body {
        match item {
            BodyItem::Block(block) => {
                for branch in &mut block.branches {
                    encode_dictionary_body(&mut branch.body, relations, manifest, variables)?;
                }
            }
            BodyItem::Equality(equality) => {
                let mut predicate = Constraint::from_equality(equality);
                constraint(&mut predicate, relations, manifest, variables)?;
                *equality = predicate
                    .equality()
                    .expect("literal adaptation preserves equality");
            }
            BodyItem::Constraint(predicate) => {
                constraint(predicate, relations, manifest, variables)?
            }
            _ => {}
        }
    }
    Ok(())
}

fn mentions(tokens: TokenStream, name: &Ident) -> bool {
    tokens.into_iter().any(|token| match token {
        TokenTree::Ident(ident) => ident == *name,
        TokenTree::Group(group) => mentions(group.stream(), name),
        _ => false,
    })
}


fn setup_constants(
    setup: &syn::Block,
    derived: &[(Ident, Vec<(Ident, syn::Type)>)],
    exports: &[(String, Vec<String>)],
    manifest: &Manifest,
) -> Result<BTreeMap<String, TokenStream>> {
    let mut constants = BTreeMap::new();
    for statement in &setup.stmts {
        let syn::Stmt::Local(local) = statement else {
            return Err("unsupported workload setup statement".into());
        };
        if local_name(&local.pat).is_some_and(|name| derived.iter().any(|(relation, _)| relation == name))
            || local.init.as_ref().is_some_and(|init|
                dictionary_predicate(&init.expr).is_ok_and(|value| value.is_some()))
        {
            continue;
        }
        let name = match &local.pat {
            syn::Pat::Ident(name) => name,
            syn::Pat::Type(typed) => match typed.pat.as_ref() {
                syn::Pat::Ident(name) => name,
                _ => return Err("unsupported workload setup pattern".into()),
            },
            _ => return Err("unsupported workload setup pattern".into()),
        };
        let expression = &local
            .init
            .as_ref()
            .ok_or("setup local needs initializer")?
            .expr;
        let value = match expression.as_ref() {
            expression if dictionary_literal(expression).is_some() => {
                let method = dictionary_literal(expression).unwrap();
                if method.args.len() != 2 {
                    return Err("dictionary literal requires column and text".into());
                }
                let syn::Expr::Lit(syn::ExprLit { lit: syn::Lit::Str(column), .. }) = &method.args[0] else {
                    return Err("dictionary literal requires a column literal".into());
                };
                let column = column.value();
                let table = column_table(exports, &column)?;
                let dictionary = manifest.dictionary(manifest.column(table, &column)?)?;
                let text = &method.args[1];
                quote!({
                    let value = #text;
                    let dictionary: &[&str] = &[#(#dictionary),*];
                    crate::manifest::DictionaryLiteral::dictionary_code(&value, dictionary)
                })
            }
            syn::Expr::Call(call) if call.func.to_token_stream().to_string() == "date" => {
                let Some(syn::Expr::Lit(syn::ExprLit {
                    lit: syn::Lit::Str(date),
                    ..
                })) = call.args.first()
                else {
                    return Err("date needs an ISO literal".into());
                };
                let days = date.value().parse::<cq_load::Date>()?.days_since_epoch();
                // DECODE R: closed authored date literal is reconstructed for its opaque host expression, not a row-dependent relational decode.
                quote!(::cq_load::Date::from_days(#days))
            }
            _ => expression.to_token_stream(),
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

fn encode_atom_body(
    body: &mut cq_lang_cq::CommaList<cq_lang_cq::BodyItem>,
    relations: &[Relation],
    dictionaries: &[Relation],
    manifest: &Manifest,
) -> Result<()> {
    for item in body {
        match item {
            cq_lang_cq::BodyItem::Block(block) => {
                for branch in &mut block.branches {
                    encode_atom_body(&mut branch.body, relations, dictionaries, manifest)?;
                }
            }
            cq_lang_cq::BodyItem::Positive { atom } => {
                if let Some(relation) = relations.iter().find(|relation| relation.name == atom.relation) {
                    for (argument, (column, _)) in atom.args.iter_mut().zip(&relation.columns) {
                        if !dictionaries.iter().any(|encoded| encoded.name == relation.name
                            && encoded.columns.iter().any(|(name, _)| name == column)) {
                            continue;
                        }
                        let table = relation.table.to_string();
                        let column = column.to_string();
                        let metadata = manifest.column(&table, &column)?;
                        if metadata.encoding != "dictionary" { continue; }
                        if let syn::Expr::Lit(syn::ExprLit { lit: syn::Lit::Str(text), .. }) = argument {
                            let code = manifest.text_id(&table, &column, &text.value())?;
                            let code = if code < 0 { u32::MAX } else { u32::try_from(code)? };
                            *argument = syn::parse_quote!(#code);
                        } else if matches!(argument, syn::Expr::Block(_)) {
                            // Closed atom operands keep their authored position
                            // and Rust body. The declared column domain selects
                            // the literal codec, including already encoded captures.
                            let dictionary = manifest.dictionary(metadata)?;
                            *argument = syn::parse_quote!({
                                let value = #argument;
                                let dictionary: &[&str] = &[#(#dictionary),*];
                                crate::manifest::DictionaryLiteral::dictionary_code(&value, dictionary)
                            });
                        }
                    }
                }
            }
            _ => {}
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn direct_kernel_records_dictionary_domains_from_declared_relations() {
        let kernel: Kernel = syn::parse_str(r#"
            $; setup {
                let Selected = cq_load::dictionary_where(
                    &dictionaries, "label", |text: &str| text.contains("x"));
            }
            relations { relation Item => source_table(label: CallerOwnedAlias); }
            derived { relation Selected(code: CallerOwnedAlias); }
            answer(value: CallerOwnedAlias) :- Item(value), Selected(value);
        "#).unwrap();
        assert_eq!(kernel.exports, vec![("source_table".into(), vec!["label".into()])]);
        let domains = kernel.dictionary_relations().unwrap();
        assert_eq!(domains.len(), 1);
        assert_eq!(domains[0].table, "source_table");
        assert_eq!(domains[0].columns[0].0, "label");
    }

    #[test]
    fn load_dictionary_producers_follow_data_fields_and_declared_origins() {
        for (locals, field) in [
            ("", r#"codes: cq_load::dictionary_where(&dictionaries, "label", |text| text.contains("x"))"#),
            (r#"let codes = cq_load::dictionary_where(&dictionaries, "label", |text| text.contains("x"));"#, "codes"),
            (r#"let selected = cq_load::dictionary_where(&dictionaries, "label", |text| text.contains("x")); let codes = selected;"#, "codes"),
        ] {
            let source = format!(r#"
                const EXPORTS: &[(&str, &str)] = &[("unrelated_table_name", "label")];
                fn load() -> Data {{
                    {locals}
                    Ok(Data {{ {field}, dictionaries }})
                }}
                fn run(data: &Data) {{
                    let Codes = data.codes.as_slice();
                    mini_linq::compile! {{
                        pipeline {{}}
                        struct Query;
                        relation Codes(code: UserAlias);
                        relation answer(value: UserAlias);
                        answer(value) :- Codes(value);
                    }}
                }}
            "#);
            let kernel = Kernel::import(&source).unwrap();
            assert!(kernel.relations.is_empty());
            assert_eq!(kernel.derived.len(), 1);
            assert_eq!(kernel.answer_columns().unwrap(),
                vec![Some(("unrelated_table_name".into(), "label".into()))]);
            let (expression, inputs) = kernel.derived_input(&syn::parse_quote!(Codes)).unwrap();
            assert!(inputs.is_empty());
            let (column, predicate) = dictionary_predicate(expression).unwrap().unwrap();
            assert_eq!(column, "label");
            assert_eq!(predicate.to_token_stream().to_string(), quote!(|text| text.contains("x")).to_string());
            let generated = dictionary_initializer(expression, &kernel.exports).unwrap().unwrap().to_string();
            assert!(generated.contains("unrelated_table_name"));
            assert_eq!(generated.matches("predicate (value . as_str ())").count(), 1);
            assert!(!kernel.cpu_query().to_string().contains("contains"));
        }
    }

    #[test]
    fn dictionary_provenance_rejects_missing_and_ambiguous_columns() {
        let expression = syn::parse_quote!(cq_load::dictionary_where(&dictionaries, "label", |text| true));
        assert!(dictionary_initializer(&expression, &[]).unwrap_err().to_string().contains("no EXPORTS"));
        let exports = vec![("a".into(), vec!["label".into()]), ("b".into(), vec!["label".into()])];
        assert!(dictionary_initializer(&expression, &exports).unwrap_err().to_string().contains("ambiguous"));
        let old = syn::parse_quote!(dictionary_where("a", "label", values, |text| true));
        assert!(dictionary_predicate(&old).is_err());
    }

    #[test]
    fn shared_key_evidence_survives_single_and_staged_imports() {
        let source = |name| SOURCES.iter().find(|(query, _)| *query == name).unwrap().1;
        let h01 = Kernel::import(source("h01")).unwrap();
        assert!(h01.evidence.has_key(
            "Lineitem",
            &["l_orderkey".into(), "l_linenumber".into()],
        ));
        let h13 = crate::stages::Program::import(source("h13")).unwrap().unwrap();
        for index in 0..h13.stages.len() {
            let mut publications: BTreeMap<_, _> = h13.preprocessing.iter()
                .map(|preprocess| (preprocess.name.to_string(), preprocess.columns.clone()))
                .collect();
            for input in h13.stages[index].program.inputs() {
                if h13.aliases.contains_key(&input.name.to_string()) {
                    publications.insert(input.name.to_string(), input.columns.iter()
                        .map(|column| (column.name.clone(), column.ty.clone())).collect());
                }
            }
            let kernel = h13.kernel(index, &publications).unwrap();
            assert_eq!(kernel.evidence, h13.stages[index].evidence);
            assert!(!kernel.evidence.has_key("Customer", &["c_custkey".into()]));
            assert!(!kernel.evidence.has_key("Orders", &["o_orderkey".into()]));
        }
    }

    #[test]
    fn output_schema_comes_only_from_the_authored_relation_declaration() {
        let source = |declaration| format!(r#"
            const EXPORTS: &[(&str, &str)] = &[("source_table", "value")];
            type Answer = (String,);
            fn run(data: &Data) -> Vec<Answer> {{
                let Input = data.input.as_slice();
                mini_linq::compile! {{
                    pipeline {{}}
                    struct Query;
                    relation Input(value: DeclaredScalar);
                    {declaration}
                    answer(value) :- Input(value);
                }}
            }}
        "#);
        let kernel = Kernel::import(&source("relation answer(value: DeclaredScalar);")).unwrap();
        assert_eq!(kernel.answer, vec![(syn::parse_quote!(value), syn::parse_quote!(DeclaredScalar))]);
        let error = Kernel::import(&source("")).err().unwrap();
        assert!(error.to_string().contains("answer relation declaration"));
    }

    #[test]
    fn compact_configuration_shell_keeps_the_single_authored_query() {
        let source = SOURCES.iter().find(|(name, _)| *name == "d72_inner").unwrap().1;
        let file = authored_file(source).unwrap();
        assert_eq!(file.items.iter().filter(|item| matches!(item,
            syn::Item::Fn(function) if function.sig.ident == "run")).count(), 1);
        let kernel = Kernel::import(source).unwrap();
        // CPU provider/configuration choices are not a second GPU plan.
        assert!(kernel.plan_advice.is_empty());
        assert_eq!(kernel.cpu_program().unwrap().query.body.to_token_stream().to_string(),
            kernel.body.to_string());
    }
    #[test]
    fn query_body_shell_imports_the_authored_runner_not_its_dispatcher() {
        let source = SOURCES.iter().find(|(name, _)| *name == "h22").unwrap().1;
        assert!(crate::stages::Program::import(source).unwrap().is_none());
        let kernel = Kernel::import(source).unwrap();
        assert_eq!(kernel.derived.len(), 1);
        assert_eq!(kernel.derived[0].0, "QualifyingPhones");
        let program = kernel.cpu_program().unwrap();
        assert_eq!(program.inputs().count(), 3);
        assert!(program.query.body.iter().any(|item|
            matches!(item, cq_lang_cq::BodyItem::Negation(_))));
        let (expression, inputs) = kernel.derived_input(&syn::parse_quote!(QualifyingPhones)).unwrap();
        assert!(inputs.is_empty());
        assert_eq!(dictionary_predicate(expression).unwrap().unwrap().0, "c_phone");
        assert_eq!(kernel.answer_columns().unwrap(), vec![Some(("customer".into(), "c_custkey".into()))]);
    }

    #[test]
    fn every_cpu_query_is_imported_without_a_second_body() {
        assert_eq!(SOURCES.len(), 26);
        for (name, source) in SOURCES {
            if crate::stages::Program::import(source).unwrap().is_some() {
                continue;
            }
            let kernel = Kernel::import(source).unwrap();
            let query: cq_lang_cq::Program = syn::parse2(kernel.cpu_query()).unwrap();
            assert_eq!(
                query.inputs().count(),
                kernel.relations.len() + kernel.derived.len(),
                "{name}"
            );
            assert!(!kernel.body.is_empty(), "{name}");
        }
    }

    #[test]
    fn standalone_import_retains_aggregate_heads_and_output_types() {
        for name in ["h01", "h06", "h10", "h19"] {
            let source = SOURCES.iter().find(|(query, _)| *query == name).unwrap().1;
            let kernel = Kernel::import(source).unwrap();
            assert!(kernel.is_standalone());
            let program = kernel.cpu_program().unwrap();
            assert_eq!(program.output().unwrap().columns.len(), kernel.answer.len());
            assert!(
                program
                    .query
                    .head
                    .args
                    .iter()
                    .any(|argument| { cq_lang_cq::group::aggregate(argument).unwrap().is_some() })
            );
            let normal = cq_lang_cq::normal::normalize(&program).unwrap();
            assert!(normal.query.group.is_some(), "{name}");
        }
    }

    #[test]
    fn staged_programs_retain_every_compiled_node_and_composition() {
        for (name, count, terminals) in [
            ("h02", 2, vec![1]),
            ("h11", 4, vec![3]),
            ("h12", 5, vec![3, 4]),
            ("h13", 2, vec![0, 1]),
            ("h15", 3, vec![2]),
            ("h16", 2, vec![1]),
            ("h18", 2, vec![1]),
            ("h20", 2, vec![1]),
        ] {
            let source = SOURCES.iter().find(|(query, _)| *query == name).unwrap().1;
            let program = crate::stages::Program::import(source).unwrap().unwrap();
            assert_eq!(program.stages.len(), count, "{name}");
            assert_eq!(program.terminal, terminals, "{name}");
            for (index, stage) in program.stages.iter().enumerate() {
                let mut publications = BTreeMap::new();
                for input in stage.program.inputs() {
                    if program.aliases.contains_key(&input.name.to_string()) {
                        publications.insert(
                            input.name.to_string(),
                            input
                                .columns
                                .iter()
                                .map(|column| (column.name.clone(), column.ty.clone()))
                                .collect(),
                        );
                    }
                }
                for preprocess in &program.preprocessing {
                    publications.insert(preprocess.name.to_string(), preprocess.columns.clone());
                }
                let kernel = program.kernel(index, &publications).unwrap();
                assert_eq!(
                    kernel.relations.len() + kernel.published.len(),
                    stage.program.inputs().count()
                );
            }
            if name == "h11" {
                assert_eq!(program.aliases["Stock"], 0);
                assert_eq!(program.aliases["PartValue"], 1);
                assert_eq!(program.aliases["TotalValue"], 2);
            }
            if name == "h13" {
                assert_eq!(program.preprocessing.len(), 1);
                assert_eq!(program.preprocessing[0].name, "QualifiedOrders");
                assert!(
                    program.preprocessing[0]
                        .expression
                        .to_token_stream()
                        .to_string()
                        .contains("split_once")
                );
                assert!(
                    program
                        .preprocessing_tokens()
                        .to_string()
                        .contains("split_once")
                );
                assert_eq!(program.preprocessing[0].relations.len(), 1);
                assert_eq!(program.preprocessing[0].relations[0].name, "Orders");
                assert_eq!(program.preprocessing[0].columns.len(), 2);
                assert!(program.preprocessing[0].columns.iter().all(|(_, ty)|
                    *ty == syn::parse_quote!(i64)));
                let prepared = program.preprocessing_tokens().to_string();
                assert_eq!(prepared.matches("from_dense_keys").count(), 1);
                assert!(prepared.find("from_dense_keys").unwrap() < prepared.find("filter (").unwrap());
                assert_eq!(program.aliases["CountedCustomers"], 0);
            }
            if name == "h16" {
                assert_eq!(program.aliases["BadSupplier"], 0);
                assert_eq!(program.preprocessing.len(), 2);
                assert!(program.stages[1].program.query.body.iter()
                    .any(|item| matches!(item, cq_lang_cq::BodyItem::Negation(_))));
            }
        }
    }

    #[test]
    fn negation_import_retains_authored_preprocessing_and_polarity() {
        let manifest = Manifest::load(&crate::data_root().join("sf1")).unwrap();
        for (query, derived, dependency, negatives) in [
            ("h04", None, None, 0),
            ("h04_not_exists", Some("LateLineitem"), Some("Lineitem"), 1),
            ("h22", None, None, 1),
        ] {
            let source = SOURCES.iter().find(|(name, _)| *name == query).unwrap().1;
            let kernel = Kernel::import(source).unwrap();
            let encoded: cq_lang_cq::Program =
                syn::parse2(kernel.encoded_query(&manifest).unwrap()).unwrap();
            assert_eq!(
                encoded
                    .query
                    .body
                    .iter()
                    .filter(|item| matches!(item, cq_lang_cq::BodyItem::Negation(_)))
                    .count(),
                negatives
            );
            if let Some(derived) = derived {
                let name = syn::parse_str(derived).unwrap();
                let (_, inputs) = kernel.derived_input(&name).unwrap();
                assert_eq!(inputs.len(), 1);
                assert_eq!(inputs[0].name.to_string(), dependency.unwrap());
            }
        }
    }
}

#[cfg(test)]
mod encoding_tests {
    use super::*;

    #[test]
    fn authored_interning_matches_exported_codes_at_both_scales() {
        let source = SOURCES.iter().find(|(name, _)| *name == "h19").unwrap().1;
        let kernel = Kernel::import(source).unwrap();
        for sf in [1, 10] {
            let manifest = Manifest::load(&crate::data_root().join(format!("sf{sf}"))).unwrap();
            for (table, column) in [
                ("lineitem", "l_shipmode"), ("lineitem", "l_shipinstruct"),
                ("part", "p_brand"), ("part", "p_container"),
            ] {
                let metadata = manifest.column(table, column).unwrap();
                assert_eq!(metadata.encoding, "dictionary");
                assert_eq!(metadata.dictionary_index_base, Some(0));
                let values = manifest.dictionary(metadata).unwrap();
                assert!(values.windows(2).all(|pair| pair[0] < pair[1]));
                let dictionary = cq_load::StringDictionary::new(values.iter().map(String::as_str));
                for (code, text) in values.iter().enumerate() {
                    assert_eq!(dictionary.lookup(text), Some(code as u32));
                    let setup = syn::parse_quote!({
                        let value: Alias = data.dictionaries.literal(#column, #text);
                    });
                    let constants = setup_constants(&setup, &[], &kernel.exports, &manifest).unwrap();
                    assert!(constants["value"].to_string().contains("DictionaryLiteral"));
                    let entries: Vec<_> = values.iter().map(String::as_str).collect();
                    assert_eq!(crate::manifest::DictionaryLiteral::dictionary_code(text, &entries), code as u32);
                }
            }
            let setup = syn::parse_quote!({
                let absent: Alias = dictionaries.literal("p_brand", "__absent__");
            });
            let constants = setup_constants(&setup, &[], &kernel.exports, &manifest).unwrap();
            assert!(constants["absent"].to_string().contains("__absent__"));
            let values = manifest.dictionary(manifest.column("part", "p_brand").unwrap()).unwrap();
            let entries: Vec<_> = values.iter().map(String::as_str).collect();
            assert_eq!(crate::manifest::DictionaryLiteral::dictionary_code("__absent__", &entries), u32::MAX);
            let encoded = kernel.encoded_query(&manifest).unwrap();
            assert!(!encoded.to_string().contains("intern"));
            let program: cq_lang_cq::Program = syn::parse2(encoded).unwrap();
            assert_eq!(program.inputs().flat_map(|relation| &relation.columns)
                .filter(|column| column.ty.to_token_stream().to_string() == "u32").count(), 4);
        }
    }

    #[test]
    fn grouped_storage_adaptation_preserves_authored_rust_and_types() {
        let manifest = Manifest::load(&crate::data_root().join("sf1")).unwrap();
        for name in ["h01", "h06", "h10", "h19"] {
            let source = SOURCES.iter().find(|(query, _)| *query == name).unwrap().1;
            let kernel = Kernel::import(source).unwrap();
            let encoded = kernel.encoded_query(&manifest).unwrap();
            let program: cq_lang_cq::Program = syn::parse2(encoded).unwrap();
            assert_eq!(program.query.head, kernel.cpu_program().unwrap().query.head);
            let normal = cq_lang_cq::normal::normalize(&program).unwrap();
            assert!(normal.query.group.is_some(), "{name}");
            for ((actual, authored), origin) in program.output().unwrap().columns.iter()
                .zip(&kernel.cpu_program().unwrap().output().unwrap().columns)
                .zip(kernel.answer_columns().unwrap())
            {
                if origin.is_some_and(|(table, column)|
                    manifest.column(&table, &column).unwrap().encoding == "dictionary")
                {
                    assert_eq!(actual.ty, syn::parse_quote!(u32));
                } else {
                    assert_eq!(actual, authored);
                }
            }
        }
    }

    #[test]
    fn relational_scalar_codecs_do_not_rewrite_rust_reads() {
        let manifest = Manifest::load(&crate::data_root().join("sf1")).unwrap();
        let source = |name| SOURCES.iter().find(|(query, _)| *query == name).unwrap().1;
        let h05 = Kernel::import(source("h05")).unwrap();
        let storage = h05.scalar_codec_types(&manifest).unwrap();
        assert!(storage.contains_key(&("orders".into(), "o_orderdate".into())));
        let encoded = h05.encoded_query_with_types(&manifest, &storage).unwrap().to_string();
        assert!(encoded.contains("days_since_epoch"), "{encoded}");
        let h01 = Kernel::import(source("h01")).unwrap();
        let storage = h01.scalar_codec_types(&manifest).unwrap();
        assert!(storage.contains_key(&("lineitem".into(), "l_shipdate".into())));
        assert!(!storage.contains_key(&("lineitem".into(), "l_quantity".into())));
        assert!(!storage.contains_key(&("lineitem".into(), "l_extendedprice".into())));
        let program: cq_lang_cq::Program = syn::parse2(
            h01.encoded_query_with_types(&manifest, &storage).unwrap()
        ).unwrap();
        let rust_steps = |program: &cq_lang_cq::Program| program.query.body.iter()
            .filter(|item| item.rust_step().is_some()).map(ToTokens::to_token_stream)
            .map(|tokens| tokens.to_string()).collect::<Vec<_>>();
        assert_eq!(rust_steps(&program), rust_steps(&h01.cpu_program().unwrap()));
    }

    #[test]
    fn staged_symbol_domains_survive_priority_aggregation() {
        let manifest = Manifest::load(&crate::data_root().join("sf1")).unwrap();
        let source = SOURCES.iter().find(|(name, _)| *name == "h12").unwrap().1;
        let program = crate::stages::Program::import(source).unwrap().unwrap();
        let mut outputs: Vec<Vec<(Ident, syn::Type)>> = Vec::new();
        let mut domains: Vec<Vec<Option<(String, String)>>> = Vec::new();
        for (index, stage) in program.stages.iter().enumerate() {
            let mut inputs = BTreeMap::new();
            let mut origins = BTreeMap::new();
            for input in stage.program.inputs() {
                if let Some(&producer) = program.aliases.get(&input.name.to_string()) {
                    inputs.insert(input.name.to_string(), input.columns.iter().zip(&outputs[producer])
                        .map(|(column, (_, ty))| (column.name.clone(), ty.clone())).collect());
                    origins.insert(input.name.to_string(), domains[producer].clone());
                } else if let Some(preprocess) = program.preprocessing.iter()
                    .find(|preprocess| preprocess.name == input.name)
                {
                    inputs.insert(input.name.to_string(), preprocess.columns.clone());
                }
            }
            let mut kernel = program.kernel(index, &inputs).unwrap();
            kernel.published_origins = origins;
            let storage = kernel.dictionary_types(&manifest).unwrap();
            let encoded: cq_lang_cq::Program = syn::parse2(
                kernel.encoded_query_with_types(&manifest, &storage).unwrap(),
            ).unwrap();
            let output = encoded.output().unwrap();
            outputs.push(output.columns.iter().map(|column|
                (column.name.clone(), column.ty.clone())).collect());
            domains.push(kernel.answer_columns().unwrap());
            let mode = if index == 0 { 2 } else { 0 };
            assert_eq!(domains[index][mode], Some(("lineitem".into(), "l_shipmode".into())));
            assert_eq!(output.columns[mode].ty, syn::parse_quote!(u32));
        }
    }

    #[test]
    fn domain_profile_adapts_the_imported_h05_without_reordering_it() {
        let source = SOURCES.iter().find(|(name, _)| *name == "h05").unwrap().1;
        let kernel = Kernel::import(source).unwrap();
        let schema = gpu_compress_host::tpch::schema();
        let mut storage = BTreeMap::new();
        for (table, columns) in schema.tables {
            for (column, kind) in columns {
                storage.insert(
                    (table.clone(), column),
                    match kind {
                        gpu_compress_host::Kind::Id(_) => syn::parse_quote!(u32),
                        gpu_compress_host::Kind::I32 => syn::parse_quote!(i32),
                    },
                );
            }
        }
        for sf in [1, 10] {
            let manifest = Manifest::load(&crate::data_root().join(format!("sf{sf}"))).unwrap();
            let mut storage = storage.clone();
            storage.extend(kernel.dictionary_types(&manifest).unwrap());
            let program: cq_lang_cq::Program = syn::parse2(
                kernel
                    .encoded_query_with_types(&manifest, &storage)
                    .unwrap(),
            )
            .unwrap();
            let atoms: Vec<_> = program
                .query
                .body
                .iter()
                .filter_map(|item| {
                    if let cq_lang_cq::BodyItem::Positive { atom } = item {
                        Some(atom.relation.to_string())
                    } else {
                        None
                    }
                })
                .collect();
            assert_eq!(
                atoms,
                [
                    "Lineitem", "Orders", "Customer", "Nation", "Region", "Supplier"
                ]
            );
            assert!(
                program
                    .declarations
                    .iter()
                    .flat_map(|relation| &relation.columns)
                    .all(|column| matches!(
                        column.ty.to_token_stream().to_string().as_str(),
                        "u32" | "i32"
                    ))
            );
            assert!(!program.to_token_stream().to_string().contains("\"ASIA\""));
        }
    }
    #[test]
    fn synthetic_manifest_loads_declared_strings_and_preserves_codes() {
        use crate::manifest::HostValue;
        use std::{
            fs,
            time::{SystemTime, UNIX_EPOCH},
        };
        let root = std::env::temp_dir().join(format!(
            "cq-tpch-loader-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir(&root).unwrap();
        struct Fixture(std::path::PathBuf);
        impl Drop for Fixture {
            fn drop(&mut self) {
                let _ = fs::remove_dir_all(&self.0);
            }
        }
        let _fixture = Fixture(root.clone());
        fs::write(
            root.join("name.bin"),
            [1_i32, 0, 1]
                .into_iter()
                .flat_map(i32::to_le_bytes)
                .collect::<Vec<_>>(),
        )
        .unwrap();
        fs::write(root.join("name.dict.txt"), "red\ngreen part\n").unwrap();
        fs::write(
            root.join("manifest.json"),
            serde_json::to_vec(&serde_json::json!({
                "schema_version": 2, "sf": 1, "byte_order": "little",
                "relations": {"part": {"rows": 3, "columns": [{
                    "name": "p_name", "type": "i32", "logical_type": "String",
                    "encoding": "dictionary", "file": "name.bin",
                    "dictionary": "name.dict.txt", "dictionary_index_base": 0,
                    "dictionary_entries": 2
                }]}}
            }))
            .unwrap(),
        )
        .unwrap();
        let manifest = Manifest::load(&root).unwrap();
        let literal_source = r#"
            const EXPORTS: &[(&str, &str)] = &[("part", "p_name")];
            fn run(data: &Data) {
                let Part = data.part.as_slice();
                let selected: ArbitraryAlias = data.dictionaries.literal("p_name", { "green part".to_owned() });
                mini_linq::compile! {
                    pipeline {}
                    struct Query;
                    relation Part(p_name: ArbitraryAlias);
                    relation answer(name: ArbitraryAlias);
                    answer(name) :- Part(name), where name == selected;
                }
            }
        "#;
        let literal_kernel = Kernel::import(literal_source).unwrap();
        let bindings = literal_kernel.setup_bindings(&manifest).unwrap().to_string();
        assert!(bindings.contains("DictionaryLiteral"));
        assert!(bindings.contains("to_owned"));
        assert!(!bindings.contains("ArbitraryAlias"));
        assert!(!bindings.contains("data"));
        let encoded = literal_kernel.encoded_query(&manifest).unwrap().to_string();
        assert!(encoded.contains("p_name : u32"));
        assert!(!encoded.contains("ArbitraryAlias"));
        assert_eq!(literal_kernel.answer_columns().unwrap(), vec![Some(("part".into(), "p_name".into()))]);
        for operand in ["{ selected }", "{ { selected } }", "{ \"green part\".to_owned() }"] {
            let source = literal_source.replace(
                "Part(name), where name == selected",
                &format!("Part(name), Part({operand})"),
            );
            let kernel = Kernel::import(&source).unwrap();
            let authored = kernel.cpu_program().unwrap();
            let encoded: cq_lang_cq::Program = syn::parse2(kernel.encoded_query(&manifest).unwrap()).unwrap();
            assert_eq!(encoded.query.body.len(), authored.query.body.len());
            for (actual, original) in encoded.query.body.iter().zip(&authored.query.body) {
                let (cq_lang_cq::BodyItem::Positive { atom: actual },
                    cq_lang_cq::BodyItem::Positive { atom: original }) = (actual, original)
                else { panic!("closed literal adaptation must not introduce relational steps") };
                assert_eq!(actual.relation, original.relation);
                assert_eq!(actual.args.len(), original.args.len());
            }
            let spelling = encoded.to_token_stream().to_string();
            assert!(spelling.contains("DictionaryLiteral"));
            assert!(spelling.contains("\"green part\""));
            assert!(spelling.contains("\"red\""));
            assert!(spelling.contains(&syn::parse_str::<syn::Expr>(operand).unwrap().to_token_stream().to_string()));
            if operand.contains("selected") {
                assert!(kernel.setup_bindings(&manifest).unwrap().to_string().contains("DictionaryLiteral"));
            }
        }
        let strings = String::read(&manifest, "part", "p_name").unwrap();
        assert_eq!(strings, ["green part", "red", "green part"]);
        assert!(strings[0].contains("green"));
        assert_eq!(i32::read(&manifest, "part", "p_name").unwrap(), [1, 0, 1]);
        for (ty, expected) in [("String", "\"green part\""), ("u32", "1u32")] {
            let kernel: Kernel = syn::parse_str(&format!(
                "$; setup {{}} relations {{ relation Part => part(p_name: {ty}); }}
                 answer(name: {ty}) :- Part(name), where name == \"green part\";"
            ))
            .unwrap();
            let storage = if ty == "u32" {
                kernel.dictionary_types(&manifest).unwrap()
            } else { BTreeMap::new() };
            let encoded = kernel.encoded_query_with_types(&manifest, &storage).unwrap().to_string();
            assert!(encoded.contains(&format!("p_name : {ty}")), "{encoded}");
            assert!(
                encoded.contains(&format!("name == {expected}")),
                "{encoded}"
            );
            let literal: Kernel = syn::parse_str(&format!(
                "$; setup {{}} relations {{ relation Part => part(p_name: {ty}); }}
                 answer() :- Part(\"green part\");"
            ))
            .unwrap();
            let encoded = literal.encoded_query_with_types(&manifest, &storage).unwrap().to_string();
            assert!(encoded.contains(&format!("Part ({expected})")), "{encoded}");
        }
        for (guard, relational) in [
            ("where name == \"red\"", true),
            ("where name == \"red\" || name == \"green part\"", true),
            (
                "where (name == \"red\" && other == \"green part\") || (name == \"green part\" && other == \"red\")",
                true,
            ),
            ("if name.contains(\"red\")", false),
            ("if name == \"red\" || name.contains(\"green\")", false),
            ("if name == \"red\" && name.contains(\"green\")", false),
        ] {
            let kernel: Kernel = syn::parse_str(&format!(
                "$; setup {{}} relations {{ relation Part => part(p_name: String); }}
                 answer(name: String) :- Part(name), Part(other), {guard};"
            ))
            .unwrap();
            let storage = kernel.dictionary_types(&manifest).unwrap();
            assert!(!storage.is_empty(), "{guard}");
            let encoded = kernel
                .encoded_query_with_types(&manifest, &storage)
                .unwrap();
            let spelling = encoded.to_string();
            assert!(spelling.contains("p_name : u32"), "{spelling}");
            if relational {
                assert!(!spelling.contains('"'), "{spelling}");
                assert!(spelling.contains("0u32"), "{spelling}");
            } else {
                assert!(spelling.contains('"'), "{spelling}");
            }
            let program: cq_lang_cq::Program = syn::parse2(encoded).unwrap();
            cq_lang_cq::normal::normalize(&program).unwrap();
        }
        let ordered: Kernel = syn::parse_str(
            "$; setup {} relations { relation Part => part(p_name: String); }
             answer(name: String) :- Part(name), where name < \"red\";"
        ).unwrap();
        assert!(ordered.encoded_query_with_types(
            &manifest, &ordered.dictionary_types(&manifest).unwrap(),
        ).unwrap_err().to_string().contains("lexical order"));
        let closed: Kernel = syn::parse_str(
            "$; setup {} relations { relation Part => part(p_name: String); }
             answer(name: String) :- Part(name), where name == { \"red\".to_owned() };"
        ).unwrap();
        let storage = closed.dictionary_types(&manifest).unwrap();
        let encoded = closed.encoded_query_with_types(&manifest, &storage).unwrap().to_string();
        assert!(encoded.contains("DictionaryLiteral"), "{encoded}");
        assert!(encoded.contains("to_owned"), "{encoded}");
        for expression in [
            "if name == \"red\" || name == \"green part\" { 1i64 } else { 0i64 }",
            "2i64 * (if \"red\" != name { 1i64 } else { 0i64 })",
            "if name.contains(\"red\") { 1i64 } else { 0i64 }",
            "if name < \"red\" { 1i64 } else { 0i64 }",
        ] {
            let mut kernel: Kernel = syn::parse_str(&format!(
                "$; setup {{}} relations {{ relation Part => part(p_name: String); }}
                 answer(value: i64) :- Part(name), let value: i64 = {expression};"
            )).unwrap();
            let storage = kernel.dictionary_types(&manifest).unwrap();
            assert!(!storage.is_empty(), "{expression}");
            let encoded = kernel.encoded_query_with_types(&manifest, &storage).unwrap();
            let spelling = encoded.to_string();
            assert!(spelling.contains("p_name : u32"), "{spelling}");
            assert!(spelling.contains('"'), "{spelling}");
            let program: cq_lang_cq::Program = syn::parse2(encoded).unwrap();
            assert_eq!(program.query.body, kernel.cpu_program().unwrap().query.body);
            cq_lang_cq::normal::normalize(&program).unwrap();
            // Aggregate Rust heads remain verbatim, independent of input encoding.
            kernel.program = Some(syn::parse_str(&format!(
                "struct Query; relation Part(p_name: String); relation answer(value: i64);
                 answer(sum({expression})) :- Part(name);"
            )).unwrap());
            kernel.body = quote!(Part(name));
            let storage = kernel.dictionary_types(&manifest).unwrap();
            assert!(!storage.is_empty(), "{expression}");
            let encoded = kernel.encoded_query_with_types(&manifest, &storage).unwrap();
            let spelling = encoded.to_string();
            assert!(spelling.contains("p_name : u32"), "{spelling}");
            assert!(spelling.contains('"'), "{spelling}");
            let program: cq_lang_cq::Program = syn::parse2(encoded).unwrap();
            assert_eq!(program.query.head, kernel.cpu_program().unwrap().query.head);
            cq_lang_cq::normal::normalize(&program).unwrap();
        }
        fs::write(
            root.join("name.bin"),
            [2_i32, 0, 1]
                .into_iter()
                .flat_map(i32::to_le_bytes)
                .collect::<Vec<_>>(),
        )
        .unwrap();
        assert!(String::read(&manifest, "part", "p_name").is_err());
        assert!(i32::read(&manifest, "part", "p_name").is_err());
    }

    #[test]
    fn both_scales_encode_literals_from_their_own_dictionaries() {
        for sf in [1, 10] {
            let manifest = Manifest::load(&crate::data_root().join(format!("sf{sf}"))).unwrap();
            for &(name, source) in SOURCES {
                if crate::stages::Program::import(source).unwrap().is_some() {
                    continue;
                }
                let kernel = Kernel::import(source).unwrap();
                let storage = kernel.dictionary_types(&manifest).unwrap();
                let encoded = kernel.encoded_query_with_types(&manifest, &storage).unwrap();
                let program: cq_lang_cq::Program = syn::parse2(encoded.clone()).unwrap();
                assert_eq!(
                    program.inputs().count(),
                    kernel.relations.len() + kernel.derived.len(),
                    "{name}"
                );
                for (actual, authored) in program.inputs().zip(kernel.cpu_program().unwrap().inputs()) {
                    for (actual, authored) in actual.columns.iter().zip(&authored.columns) {
                        let dictionary = kernel.relations.iter().any(|relation| {
                            relation.columns.iter().any(|(column, _)| column == &authored.name)
                                && storage.contains_key(&(relation.table.to_string(), authored.name.to_string()))
                        }) || kernel.dictionary_relations().unwrap().iter()
                            .any(|relation| program.inputs().any(|input| input.name == relation.name
                                && input.columns.iter().any(|column| column.name == actual.name)));
                        if dictionary {
                            assert_eq!(actual.ty, syn::parse_quote!(u32), "{name}");
                        } else {
                            assert_eq!(actual, authored, "{name}");
                        }
                    }
                }
            }
            let kernel =
                Kernel::import(SOURCES.iter().find(|(name, _)| *name == "h03").unwrap().1).unwrap();
            let encoded = kernel.encoded_query(&manifest).unwrap().to_string();
            assert!(kernel.setup_bindings(&manifest).unwrap().to_string().contains("BUILDING"));
            assert!(encoded.contains("cutoff"));
            assert_eq!(
                manifest
                    .text_id("customer", "c_mktsegment", "__not_in_dictionary__")
                    .unwrap(),
                -1
            );
        }
    }
}
