//! Import authored CPU queries, including standalone grouped compile! programs.
//! Standalone sources retain aggregate heads and result declarations. Their
//! EXPORTS projections identify loader tables; scale-two Decimal::scaled()
//! reads and ISO date captures use the manifest's coefficient/day encodings.
//! Dictionary-coded group keys are decoded only at the comparison boundary.
//! String methods retain their authored host representation and remain subject
//! to checked planning/translation. Staged sources retain every compiled rule,
//! materialized dependency and final iterator composition.
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
    pub setup: syn::Block,
    pub relations: Vec<Relation>,
    pub answer: Vec<(Ident, syn::Type)>,
    pub derived: Vec<(Ident, Vec<(Ident, syn::Type)>)>,
    /// Inputs published by preceding stages, in declaration order.
    pub published: Vec<Relation>,
    pub body: TokenStream,
    /// Standalone programs retain their declared output and aggregate head.
    program: Option<cq_lang_cq::Program>,
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
        Ok(Self {
            plan_advice,
            setup,
            relations,
            derived,
            published: Vec::new(),
            answer,
            program: None,
            body: input.parse()?,
        })
    }
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
                let syn::Pat::Type(typed) = &local.pat else {
                    return None;
                };
                let syn::Pat::Ident(binding) = typed.pat.as_ref() else {
                    return None;
                };
                (binding.ident == *name)
                    .then(|| local.init.as_ref().map(|init| init.expr.as_ref()))
                    .flatten()
            })
            .ok_or_else(|| format!("derived relation {name} lacks an authored initializer"))?;
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
        let mut file = syn::parse_file(source)?;
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
        struct Compile(cq_lang_cq::Program);
        impl Parse for Compile {
            fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
                let keyword: Ident = input.parse()?;
                if keyword != "pipeline" {
                    return Err(syn::Error::new_spanned(keyword, "expected pipeline"));
                }
                let pipeline;
                syn::braced!(pipeline in input);
                if !pipeline.is_empty() {
                    return Err(pipeline
                        .error("standalone importer requires the authored default pipeline"));
                }
                Ok(Self(input.parse()?))
            }
        }
        let program = syn::parse2::<Compile>(compiles.0.remove(0))?.0;
        Self::standalone_program(file, program, &BTreeMap::new())
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
            if let syn::Stmt::Local(local) = statement {
                let name = match &local.pat {
                    syn::Pat::Ident(name) => Some(&name.ident),
                    syn::Pat::Type(typed) => match typed.pat.as_ref() {
                        syn::Pat::Ident(name) => Some(&name.ident),
                        _ => None,
                    },
                    _ => None,
                };
                if let Some(name) = name
                    && !program.inputs().any(|input| input.name == *name)
                    && mentions(body.clone(), name)
                {
                    setup.stmts.push(statement.clone());
                }
            }
        }
        let exports = file
            .items
            .iter()
            .find_map(|item| match item {
                syn::Item::Const(item) if item.ident == "EXPORTS" => Some(item.expr.as_ref()),
                _ => None,
            })
            .ok_or("standalone CPU source has no EXPORTS")?;
        let syn::Expr::Reference(exports) = exports else {
            return Err("EXPORTS must be a literal slice".into());
        };
        let syn::Expr::Array(exports) = exports.expr.as_ref() else {
            return Err("EXPORTS must be a literal slice".into());
        };
        let mut tables = Vec::new();
        for export in &exports.elems {
            let syn::Expr::Tuple(pair) = export else {
                return Err("EXPORTS entries must be table/projection pairs".into());
            };
            let strings = pair
                .elems
                .iter()
                .map(|value| match value {
                    syn::Expr::Lit(syn::ExprLit {
                        lit: syn::Lit::Str(value),
                        ..
                    }) => Ok(value.value()),
                    _ => Err("EXPORTS table/projection must be literals"),
                })
                .collect::<std::result::Result<Vec<_>, _>>()?;
            if strings.len() != 2 {
                return Err("EXPORTS entries must have two fields".into());
            }
            tables.push((
                strings[0].clone(),
                strings[1]
                    .split(',')
                    .map(|s| s.trim().to_owned())
                    .collect::<Vec<_>>(),
            ));
        }
        let mut relations = Vec::new();
        let mut published = Vec::new();
        for input in program.inputs() {
            if let Some(columns) = publications.get(&input.name.to_string()) {
                published.push(Relation {
                    name: input.name.clone(),
                    table: input.name.clone(),
                    columns: columns.clone(),
                });
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
            .ok_or("standalone program requires an answer declaration")?
            .columns
            .iter()
            .map(|column| (column.name.clone(), column.ty.clone()))
            .collect();
        Ok(Self {
            plan_advice: TokenStream::new(),
            setup,
            relations,
            answer,
            derived: Vec::new(),
            published,
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

    /// Column-local dictionary codes preserve literal equality, including
    /// disjunctions and finite membership. Opaque or ordered String uses still
    /// require the authored host representation.
    pub fn dictionary_types(
        &self,
        manifest: &Manifest,
    ) -> Result<BTreeMap<(String, String), syn::Type>> {
        let program = self.cpu_program()?;
        let mut variables = BTreeMap::new();
        collect_variables(self.body.clone(), &self.relations, &mut variables)?;
        let mut storage = BTreeMap::new();
        for relation in &self.relations {
            for (column, logical) in &relation.columns {
                let origin = (relation.table.to_string(), column.to_string());
                if logical.to_token_stream().to_string() == "String"
                    && manifest.column(&origin.0, &origin.1)?.encoding == "dictionary"
                    && variables.iter().all(|(variable, source)| {
                        source != &origin
                            || (dictionary_body(&program.query.body, variable)
                                && program.query.head.args.iter().all(|argument| {
                                    matches!(argument, syn::Expr::Path(path) if path.path.is_ident(variable))
                                        || dictionary_expression(argument, variable)
                                }))
                    })
                {
                    storage.insert(origin, syn::parse_quote!(i32));
                }
            }
        }
        Ok(storage)
    }

    #[cfg(test)]
    /// Storage adaptation only: types and constants, not order/providers/plans.
    pub fn encoded_query(&self, manifest: &Manifest) -> Result<TokenStream> {
        self.encoded_query_with_types(manifest, &BTreeMap::new())
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
        let dictionary_storage = self.dictionary_types(manifest)?;
        for relation in &mut relations {
            for (column, ty) in &mut relation.columns {
                if let Some(stored) = storage.get(&(relation.table.to_string(), column.to_string()))
                {
                    *ty = stored.clone();
                } else if self.is_standalone() {
                    let metadata =
                        manifest.column(&relation.table.to_string(), &column.to_string())?;
                    let declared = ty.to_token_stream().to_string();
                    let keep_string = declared == "String"
                        && !dictionary_storage.contains_key(&(relation.table.to_string(), column.to_string()));
                    let authored_code = declared == "u32" && metadata.encoding == "dictionary";
                    if !keep_string && !authored_code {
                        *ty = syn::parse_str(&metadata.ty)?;
                    }
                }
            }
        }
        let constants = setup_constants(&self.setup, &self.derived, manifest)?;
        let mut variables = BTreeMap::new();
        collect_variables(self.body.clone(), &relations, &mut variables)?;
        let mut body = self.cpu_program()?.query.body;
        encode_dictionary_body(&mut body, &relations, manifest, &variables)?;
        let body = encode_tokens(
            body.to_token_stream(),
            &relations,
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
                let declared = logical.to_token_stream().to_string();
                if metadata.logical_type != declared
                    && metadata.ty != declared
                    && !(declared == "u32" && metadata.encoding == "dictionary")
                    && !(self.is_standalone()
                        && declared == "i64"
                        && metadata.logical_type == "Integer")
                {
                    return Err(format!(
                        "{}.{}: CPU type {} differs from manifest logical type {}",
                        relation.table,
                        column,
                        logical.to_token_stream(),
                        metadata.logical_type
                    )
                    .into());
                }
                let ty = &relations
                    .iter()
                    .find(|encoded| encoded.name == relation.name)
                    .expect("cloned input relation")
                    .columns
                    .iter()
                    .find(|(name, _)| name == column)
                    .expect("cloned input column")
                    .1;
                let ty: syn::Type = if self.is_standalone()
                    || storage.contains_key(&(relation.table.to_string(), column.to_string()))
                {
                    ty.clone()
                } else if declared == "String"
                    || (declared == "u32" && metadata.encoding == "dictionary")
                {
                    logical.clone()
                } else {
                    syn::parse_str(&metadata.ty)?
                };
                fields.push(quote!(#column: #ty));
            }
            declarations.extend(quote!(relation #name(#(#fields),*);));
        }
        for (name, columns) in &self.derived {
            let fields = columns.iter().map(|(column, ty)| quote!(#column: #ty));
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
            let mut head = program.query.head.clone();
            for argument in &mut head.args {
                encode_dictionary_expression(argument, &relations, manifest, &variables)?;
            }
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
        let mut variables = BTreeMap::new();
        collect_variables(self.body.clone(), &self.relations, &mut variables)?;
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
// A coded string may occur inside an arithmetic/conditional expression only
// through literal equality. Any other use keeps the entire column as String.
fn dictionary_expression(expression: &syn::Expr, variable: &str) -> bool {
    use syn::visit::Visit;
    struct Check<'a> { variable: &'a str, safe: bool }
    impl<'ast> Visit<'ast> for Check<'_> {
        fn visit_expr(&mut self, expression: &'ast syn::Expr) {
            if let syn::Expr::Binary(binary) = expression
                && matches!(binary.op, syn::BinOp::Eq(_) | syn::BinOp::Ne(_))
                && [(&*binary.left, &*binary.right), (&*binary.right, &*binary.left)]
                    .iter().any(|(variable, literal)| {
                        matches!(variable, syn::Expr::Path(path) if path.path.is_ident(self.variable))
                            && matches!(literal, syn::Expr::Lit(lit) if matches!(lit.lit, syn::Lit::Str(_)))
                    })
            {
                return;
            }
            if matches!(expression, syn::Expr::Path(path) if path.path.is_ident(self.variable))
                || matches!(expression, syn::Expr::Macro(mac) if mentions(mac.mac.tokens.clone(), &quote::format_ident!("{}", self.variable)))
            {
                self.safe = false;
            }
            syn::visit::visit_expr(self, expression);
        }
    }
    let mut check = Check { variable, safe: true };
    check.visit_expr(expression);
    check.safe
}

fn encode_dictionary_expression(
    expression: &mut syn::Expr,
    relations: &[Relation],
    manifest: &Manifest,
    variables: &BTreeMap<String, (String, String)>,
) -> Result<()> {
    use syn::visit_mut::VisitMut;
    struct Encode<'a> {
        relations: &'a [Relation],
        manifest: &'a Manifest,
        variables: &'a BTreeMap<String, (String, String)>,
        error: Option<Box<dyn std::error::Error>>,
    }
    impl VisitMut for Encode<'_> {
        fn visit_expr_mut(&mut self, expression: &mut syn::Expr) {
            if let syn::Expr::Binary(binary) = expression
                && matches!(binary.op, syn::BinOp::Eq(_) | syn::BinOp::Ne(_))
            {
                let pair = if matches!(&*binary.left, syn::Expr::Path(_)) {
                    (&*binary.left, &mut *binary.right)
                } else {
                    (&*binary.right, &mut *binary.left)
                };
                if let syn::Expr::Path(path) = pair.0
                    && let Some(name) = path.path.get_ident()
                    && let Some((table, column)) = self.variables.get(&name.to_string())
                    && self.relations.iter().any(|relation| relation.table == table.as_str()
                        && relation.columns.iter().any(|(name, ty)| name == column.as_str()
                            && ty.to_token_stream().to_string() != "String"))
                    && let syn::Expr::Lit(syn::ExprLit { lit: syn::Lit::Str(text), .. }) = pair.1
                {
                    match self.manifest.text_id(table, column, &text.value()) {
                        Ok(code) => *pair.1 = syn::parse_quote!(#code),
                        Err(error) => self.error = Some(error),
                    }
                }
            }
            syn::visit_mut::visit_expr_mut(self, expression);
        }
    }
    let mut encode = Encode { relations, manifest, variables, error: None };
    encode.visit_expr_mut(expression);
    match encode.error { Some(error) => Err(error), None => Ok(()) }
}

fn dictionary_constraint(predicate: &cq_lang_cq::Constraint, variable: &str) -> bool {
    use cq_lang_cq::syntax::constraint::{Constraint, Term};
    let is_variable = |term: &Term| matches!(term, Term::Variable(name) if name == variable);
    let string = |expression: &syn::Expr| matches!(expression, syn::Expr::Lit(literal) if matches!(literal.lit, syn::Lit::Str(_)));
    if !predicate.variables().any(|name| name == variable) {
        return true;
    }
    match predicate {
        Constraint::Compare {
            left,
            comparison,
            right,
        } if !comparison.ordered() => {
            matches!(right, Term::Literal(literal) if is_variable(left) && string(literal))
                || matches!(left, Term::Literal(literal) if is_variable(right) && string(literal))
        }
        Constraint::In { term, literals } => is_variable(term) && literals.iter().all(string),
        Constraint::Or(arms) => arms
            .iter()
            .flatten()
            .all(|arm| dictionary_constraint(arm, variable)),
        _ => false,
    }
}

fn dictionary_body(body: &cq_lang_cq::CommaList<cq_lang_cq::BodyItem>, variable: &str) -> bool {
    use cq_lang_cq::{BodyItem, Constraint};
    body.iter().all(|item| match item {
        BodyItem::Block(block) => block
            .branches
            .iter()
            .all(|branch| dictionary_body(&branch.body, variable)),
        BodyItem::Equality(equality) => {
            dictionary_constraint(&Constraint::from_equality(equality), variable)
        }
        BodyItem::Constraint(predicate) => dictionary_constraint(predicate, variable),
        BodyItem::Let(binding) => dictionary_expression(&binding.expression, variable),
        BodyItem::Positive { atom } => atom.args.iter().all(|argument| {
            matches!(
                argument,
                syn::Expr::Path(_) | syn::Expr::Lit(_) | syn::Expr::Infer(_)
            ) || !mentions(
                argument.to_token_stream(),
                &quote::format_ident!("{variable}"),
            )
        }),
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
            && let syn::Expr::Lit(syn::ExprLit {
                lit: syn::Lit::Str(text),
                ..
            }) = expression
            && relations.iter().any(|relation| {
                relation.table == table.as_str()
                    && relation.columns.iter().any(|(name, ty)| {
                        name == column.as_str() && ty.to_token_stream().to_string() != "String"
                    })
            })
        {
            let code = manifest.text_id(table, column, &text.value())?;
            *expression = syn::parse_quote!(#code);
        }
        Ok(())
    }

    fn constraint(
        predicate: &mut Constraint,
        relations: &[Relation],
        manifest: &Manifest,
        variables: &BTreeMap<String, (String, String)>,
    ) -> Result<()> {
        match predicate {
            Constraint::Compare {
                left,
                comparison,
                right,
            } if !comparison.ordered() => {
                if let Term::Literal(expression) = right {
                    literal(left, expression, relations, manifest, variables)?;
                }
                if let Term::Literal(expression) = left {
                    literal(right, expression, relations, manifest, variables)?;
                }
            }
            Constraint::In { term, literals } => {
                for expression in literals {
                    literal(term, expression, relations, manifest, variables)?;
                }
            }
            Constraint::Or(arms) => {
                for predicate in arms.iter_mut().flatten() {
                    constraint(predicate, relations, manifest, variables)?;
                }
            }
            _ => {}
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
            BodyItem::Let(binding) => {
                encode_dictionary_expression(&mut binding.expression, relations, manifest, variables)?;
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
    manifest: &Manifest,
) -> Result<BTreeMap<String, TokenStream>> {
    let mut constants = BTreeMap::new();
    for statement in &setup.stmts {
        let syn::Stmt::Local(local) = statement else {
            return Err("unsupported workload setup statement".into());
        };
        if let syn::Pat::Type(typed) = &local.pat
            && let syn::Pat::Ident(name) = typed.pat.as_ref()
            && derived.iter().any(|(relation, _)| *relation == name.ident)
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
            syn::Expr::Call(call) if call.func.to_token_stream().to_string() == "intern" => {
                let arguments: Vec<_> = call.args.iter().map(|argument| {
                    match argument {
                        syn::Expr::Lit(syn::ExprLit { lit: syn::Lit::Str(value), .. }) =>
                            Ok(value.value()),
                        _ => Err("intern needs table, column and text literals"),
                    }
                }).collect::<std::result::Result<_, _>>()?;
                let [table, column, text] = arguments.as_slice() else {
                    return Err("intern needs table, column and text literals".into());
                };
                if manifest.column(table, column)?.encoding != "dictionary" {
                    return Err(format!("intern requires a dictionary column: {table}.{column}").into());
                }
                let code = manifest.text_id(table, column, text)?;
                // The authored interner reserves MAX for absent literals.
                let code = if code < 0 { u32::MAX } else { u32::try_from(code)? };
                quote!(#code)
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
                quote!(#days)
            }
            syn::Expr::MethodCall(method) if method.method == "expect" => {
                let syn::Expr::Call(call) = method.receiver.as_ref() else {
                    return Err("unsupported setup initializer".into());
                };
                if call.func.to_token_stream().to_string() == "Date :: load" {
                    let Some(syn::Expr::Lit(syn::ExprLit {
                        lit: syn::Lit::Str(date),
                        ..
                    })) = call.args.first()
                    else {
                        return Err("Date::load needs an ISO literal".into());
                    };
                    let days = date.value().parse::<cq_load::Date>()?.days_since_epoch();
                    constants.insert(name.ident.to_string(), quote!(#days));
                    continue;
                }
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
            syn::Expr::Lit(_) => expression.to_token_stream(),
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
    let mut skip_until = 0;
    for (index, token) in tokens.iter().enumerate() {
        if index < skip_until {
            continue;
        }
        if let TokenTree::Ident(name) = token
            && let Some((table, column)) = variables.get(&name.to_string())
            && manifest.column(table, column)?.logical_type == "Decimal"
            && let Some(
                [
                    TokenTree::Punct(dot),
                    TokenTree::Ident(method),
                    TokenTree::Group(args),
                ],
            ) = tokens.get(index + 1..index + 4)
            && dot.as_char() == '.'
            && method == "scaled"
            && args.stream().is_empty()
        {
            encoded.extend(quote!((*#name)));
            skip_until = index + 4;
            continue;
        }
        if let TokenTree::Ident(name) = token
            && let Some((table, column)) = variables.get(&name.to_string())
            && manifest.column(table, column)?.logical_type == "String"
            && relations
                .iter()
                .find(|relation| relation.table == table.as_str())
                .and_then(|relation| {
                    relation
                        .columns
                        .iter()
                        .find(|(name, _)| name == column.as_str())
                })
                .is_some_and(|(_, ty)| ty.to_token_stream().to_string() != "String")
            && matches!(tokens.get(index + 1..index + 3),
                Some([TokenTree::Punct(a), TokenTree::Punct(b)])
                    if matches!(a.as_char(), '=' | '!') && b.as_char() == '=')
            && !matches!(index.checked_sub(1).and_then(|index| tokens.get(index)),
                Some(TokenTree::Punct(punct)) if punct.as_char() == '*')
        {
            encoded.extend(quote!(*#name));
            continue;
        }
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
                    for (arg, (column, ty)) in args.iter_mut().zip(&relation.columns) {
                        if ty.to_token_stream().to_string() == "String" {
                            continue;
                        }
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
            _ => encoded.extend([token.clone()]),
        }
    }
    Ok(encoded)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn imported_plan_hints_are_not_discarded() {
        let source = SOURCES
            .iter()
            .find(|(name, _)| *name == "d72_inner")
            .unwrap()
            .1;
        let kernel = Kernel::import(source).unwrap();
        assert_eq!(
            kernel.plan_advice.to_string(),
            quote!(skip(Lineitem with PartSupp), reduce(PartSupp with Lineitem)).to_string()
        );
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
        for name in ["h01", "h06", "h10", "h12", "h19"] {
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
            ("h13", 2, vec![0, 1]),
            ("h15", 3, vec![2]),
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
                assert_eq!(program.preprocessing[0].relations[0].table, "orders");
            }
        }
    }

    #[test]
    fn negation_import_retains_authored_preprocessing_and_polarity() {
        let manifest = Manifest::load(&crate::data_root().join("sf1")).unwrap();
        for (query, derived, dependency, negatives) in [
            ("h04", None, None, 0),
            ("h04_not_exists", Some("LateLineitem"), Some("Lineitem"), 1),
            ("h16", Some("BadSupplier"), Some("Supplier"), 1),
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
                        let value: u32 = intern(#table, #column, #text);
                    });
                    let constants = setup_constants(&setup, &[], &manifest).unwrap();
                    assert_eq!(constants["value"].to_string(), format!("{code}u32"));
                }
            }
            let setup = syn::parse_quote!({
                let absent: u32 = intern("part", "p_brand", "__absent__");
            });
            let constants = setup_constants(&setup, &[], &manifest).unwrap();
            assert_eq!(constants["absent"].to_string(), format!("{}u32", u32::MAX));
            let encoded = kernel.encoded_query(&manifest).unwrap();
            assert!(!encoded.to_string().contains("intern"));
            let program: cq_lang_cq::Program = syn::parse2(encoded).unwrap();
            assert_eq!(program.inputs().flat_map(|relation| &relation.columns)
                .filter(|column| column.ty.to_token_stream().to_string() == "u32").count(), 4);
        }
    }

    #[test]
    fn grouped_storage_adaptation_preserves_folds_and_scalar_encodings() {
        let manifest = Manifest::load(&crate::data_root().join("sf1")).unwrap();
        for name in ["h01", "h06", "h10", "h12", "h19"] {
            let source = SOURCES.iter().find(|(query, _)| *query == name).unwrap().1;
            let kernel = Kernel::import(source).unwrap();
            let encoded = kernel.encoded_query(&manifest).unwrap();
            let spelling = encoded.to_string();
            assert!(!spelling.contains("scaled"), "{name}: {spelling}");
            assert!(!spelling.contains("Date"), "{name}: {spelling}");
            assert!(!spelling.contains("Decimal"), "{name}: {spelling}");
            let program: cq_lang_cq::Program = syn::parse2(encoded).unwrap();
            assert_eq!(program.query.head, kernel.cpu_program().unwrap().query.head);
            let normal = cq_lang_cq::normal::normalize(&program).unwrap();
            assert!(normal.query.group.is_some(), "{name}");
            if name == "h01" {
                assert!(spelling.contains("10471i32"));
                let types: Vec<_> = program
                    .output()
                    .unwrap()
                    .columns
                    .iter()
                    .map(|column| column.ty.to_token_stream().to_string())
                    .collect();
                assert_eq!(types, ["i32", "i32", "i64", "i64", "usize"]);
            }
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
        let strings = String::read(&manifest, "part", "p_name").unwrap();
        assert_eq!(strings, ["green part", "red", "green part"]);
        assert!(strings[0].contains("green"));
        assert_eq!(i32::read(&manifest, "part", "p_name").unwrap(), [1, 0, 1]);
        for (ty, expected) in [("String", "\"green part\""), ("i32", "1i32")] {
            let kernel: Kernel = syn::parse_str(&format!(
                "$; setup {{}} relations {{ relation Part => part(p_name: {ty}); }}
                 answer(name: {ty}) :- Part(name), if name == \"green part\";"
            ))
            .unwrap();
            let encoded = kernel.encoded_query(&manifest).unwrap().to_string();
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
            let encoded = literal.encoded_query(&manifest).unwrap().to_string();
            assert!(encoded.contains(&format!("Part ({expected})")), "{encoded}");
        }
        for (guard, dictionary) in [
            ("name == \"red\"", true),
            ("name == \"red\" || name == \"green part\"", true),
            (
                "(name == \"red\" && other == \"green part\") || (name == \"green part\" && other == \"red\")",
                true,
            ),
            ("\"red\" == name || \"green part\" == name", true),
            ("name.contains(\"red\")", false),
            ("name == \"red\" || name.contains(\"green\")", false),
            ("name == \"red\" && name.contains(\"green\")", false),
            ("name < \"red\"", false),
        ] {
            let kernel: Kernel = syn::parse_str(&format!(
                "$; setup {{}} relations {{ relation Part => part(p_name: String); }}
                 answer(name: String) :- Part(name), Part(other), if {guard};"
            ))
            .unwrap();
            let storage = kernel.dictionary_types(&manifest).unwrap();
            assert_eq!(!storage.is_empty(), dictionary, "{guard}");
            let encoded = kernel
                .encoded_query_with_types(&manifest, &storage)
                .unwrap();
            let spelling = encoded.to_string();
            assert_eq!(spelling.contains("p_name : i32"), dictionary, "{spelling}");
            if dictionary {
                assert!(!spelling.contains('"'), "{spelling}");
                assert!(spelling.contains("0i32"), "{spelling}");
            } else {
                assert!(spelling.contains("p_name : String"), "{spelling}");
                assert!(spelling.contains('"'), "{spelling}");
            }
            let program: cq_lang_cq::Program = syn::parse2(encoded).unwrap();
            cq_lang_cq::normal::normalize(&program).unwrap();
        }
        for (expression, coded) in [
            ("if name == \"red\" || name == \"green part\" { 1i64 } else { 0i64 }", true),
            ("2i64 * (if \"red\" != name { 1i64 } else { 0i64 })", true),
            ("if name.contains(\"red\") { 1i64 } else { 0i64 }", false),
            ("if name < \"red\" { 1i64 } else { 0i64 }", false),
        ] {
            let mut kernel: Kernel = syn::parse_str(&format!(
                "$; setup {{}} relations {{ relation Part => part(p_name: String); }}
                 answer(value: i64) :- Part(name), let value: i64 = {expression};"
            )).unwrap();
            let storage = kernel.dictionary_types(&manifest).unwrap();
            assert_eq!(!storage.is_empty(), coded, "{expression}");
            let encoded = kernel.encoded_query_with_types(&manifest, &storage).unwrap();
            let spelling = encoded.to_string();
            assert_eq!(spelling.contains("p_name : i32"), coded, "{spelling}");
            assert_eq!(spelling.contains('"'), !coded, "{spelling}");
            let program: cq_lang_cq::Program = syn::parse2(encoded).unwrap();
            cq_lang_cq::normal::normalize(&program).unwrap();
            // Aggregate heads are normalized into Bind expressions later;
            // their literals must already use the input dictionary here.
            kernel.program = Some(syn::parse_str(&format!(
                "struct Query; relation Part(p_name: String); relation answer(value: i64);
                 answer(sum({expression})) :- Part(name);"
            )).unwrap());
            kernel.body = quote!(Part(name));
            let storage = kernel.dictionary_types(&manifest).unwrap();
            assert_eq!(!storage.is_empty(), coded, "{expression}");
            let encoded = kernel.encoded_query_with_types(&manifest, &storage).unwrap();
            let spelling = encoded.to_string();
            assert_eq!(spelling.contains("p_name : i32"), coded, "{spelling}");
            assert_eq!(spelling.contains('"'), !coded, "{spelling}");
            let program: cq_lang_cq::Program = syn::parse2(encoded).unwrap();
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
                let encoded = kernel.encoded_query(&manifest).unwrap();
                let program: cq_lang_cq::Program = syn::parse2(encoded.clone()).unwrap();
                assert_eq!(
                    program.inputs().count(),
                    kernel.relations.len() + kernel.derived.len(),
                    "{name}"
                );
                for relation in &program.declarations {
                    for column in &relation.columns {
                        assert!(matches!(
                            column.ty.to_token_stream().to_string().as_str(),
                            "i32" | "i64" | "u32" | "String" | "usize"
                        ));
                    }
                }
            }
            let kernel =
                Kernel::import(SOURCES.iter().find(|(name, _)| *name == "h03").unwrap().1).unwrap();
            let encoded = kernel.encoded_query(&manifest).unwrap().to_string();
            assert!(encoded.contains("9204i32"));
            assert!(encoded.contains("BUILDING"));
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
