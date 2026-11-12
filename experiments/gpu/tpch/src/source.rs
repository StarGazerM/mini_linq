//! Lower checked authored stage records to CQ input and representation facts.
//! Rust payloads are preserved; only relational syntax and declared codecs
//! participate in representation adaptation.
use crate::{
    manifest::{Manifest, Result},
    stages,
};
use proc_macro2::{TokenStream, TokenTree};
use quote::{ToTokens, quote};
use std::collections::BTreeMap;
use syn::Ident;

macro_rules! sources {
    ($($name:literal),* $(,)?) => { pub const SOURCES: &[(&str, &str)] = &[$(
        ($name, include_str!(concat!("../queries/", $name, ".stages"))),
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

pub type Origin = (String, String);

#[derive(Clone)]
pub struct Relation {
    pub name: Ident,
    pub table: Ident,
    pub columns: Vec<(Ident, syn::Type)>,
}

pub struct Kernel {
    pub evidence: cq_lang_sequence::Evidence,
    pub relations: Vec<Relation>,
    pub answer: Vec<(Ident, syn::Type)>,
    pub published: Vec<Relation>,
    pub published_origins: BTreeMap<String, Vec<Option<Origin>>>,
    pub domains: Vec<stages::Domain>,
    pub operands: Vec<stages::Operand>,
    pub program: cq_lang_cq::Program,
}

impl Kernel {
    pub fn from_stage(
        source: &stages::Program,
        index: usize,
        types: &BTreeMap<String, Vec<(Ident, syn::Type)>>,
    ) -> Result<Self> {
        let stage = source
            .stages
            .get(index)
            .ok_or("stage index out of bounds")?;
        let mut relations = Vec::new();
        let mut published = Vec::new();
        let mut published_origins = BTreeMap::new();
        for declaration in stage.program.inputs() {
            let (_, producer) = stage
                .inputs
                .iter()
                .find(|(name, _)| *name == declaration.name)
                .ok_or("checked stage input binding missing")?;
            if let Some(input) = source.inputs.iter().find(|input| input.name == *producer) {
                let columns = input
                    .columns
                    .as_ref()
                    .ok_or("host value is not a relation")?;
                if let stages::Producer::Table(table) = &input.producer {
                    relations.push(Relation {
                        name: declaration.name.clone(),
                        table: table.clone(),
                        columns: columns
                            .iter()
                            .map(|column| (column.name.clone(), column.ty.clone()))
                            .collect(),
                    });
                    continue;
                }
                let origins = columns
                    .iter()
                    .map(|column| {
                        column
                            .domain
                            .as_ref()
                            .map(|name| {
                                source.domain(name).map(|domain| {
                                    (domain.table.to_string(), domain.column.to_string())
                                })
                            })
                            .transpose()
                    })
                    .collect::<Result<Vec<_>>>()?;
                published_origins.insert(declaration.name.to_string(), origins);
            }
            published.push(Relation {
                name: declaration.name.clone(),
                table: declaration.name.clone(),
                columns: types
                    .get(&declaration.name.to_string())
                    .cloned()
                    .ok_or_else(|| format!("missing publication schema {}", declaration.name))?,
            });
        }
        let answer = stage
            .program
            .output()
            .ok_or("stage requires an output schema")?
            .columns
            .iter()
            .map(|column| (column.name.clone(), column.ty.clone()))
            .collect();
        let operands = stage
            .captures
            .iter()
            .map(|name| {
                source
                    .operands
                    .iter()
                    .find(|operand| operand.name == *name)
                    .cloned()
                    .ok_or_else(|| format!("missing operand {name}").into())
            })
            .collect::<Result<Vec<_>>>()?;
        Ok(Self {
            evidence: stage.evidence.clone(),
            relations,
            answer,
            published,
            published_origins,
            domains: source.domains.clone(),
            operands,
            program: stage.program.clone(),
        })
    }

    pub fn cpu_query(&self) -> TokenStream {
        self.program.to_token_stream()
    }

    pub fn dictionary_types(&self, manifest: &Manifest) -> Result<BTreeMap<Origin, syn::Type>> {
        let mut storage = BTreeMap::new();
        for domain in &self.domains {
            let origin = (domain.table.to_string(), domain.column.to_string());
            if manifest.column(&origin.0, &origin.1)?.encoding != "dictionary" {
                return Err(
                    format!("declared domain {} is not a dictionary column", domain.name).into(),
                );
            }
            storage.insert(origin, syn::parse_quote!(u32));
        }
        Ok(storage)
    }

    pub fn integer_key_types(&self) -> BTreeMap<Origin, syn::Type> {
        self.relations.iter().flat_map(|relation| {
            relation.columns.iter().map(|(column, _)| (relation.table.to_string(), column.to_string()))
        }).chain(self.published_origins.values().flatten().filter_map(Clone::clone))
            .filter(|(table, column)| crate::key_domains::id(table, column).is_some())
            .map(|origin| (origin, syn::parse_quote!(u32))).collect()
    }

    /// Producer origin and consumer residency are independent facts.
    pub fn computed_publication_inputs(&self) -> Result<std::collections::BTreeSet<(String, usize)>> {
        let computed = self.published_origins.iter().flat_map(|(name, origins)| {
            origins.iter().enumerate().filter_map(move |(slot, origin)| {
                origin.is_none().then(|| (name.clone(), slot))
            })
        }).collect();
        self.check_computed_publication_inputs(&computed)?;
        Ok(computed)
    }

    pub fn check_computed_publication_inputs(
        &self,
        computed: &std::collections::BTreeSet<(String, usize)>,
    ) -> Result<()> {
        for input in &self.published {
            let origins = self.published_origins.get(&input.name.to_string())
                .ok_or("published input lacks producer origin records")?;
            if origins.len() != input.columns.len() {
                return Err("published input origin arity differs from its schema".into());
            }
            for (slot, origin) in origins.iter().enumerate() {
                if computed.contains(&(input.name.to_string(), slot)) != origin.is_none() {
                    return Err(format!("publication origin mismatch for {} column {slot}: computed and domain origins cannot be interchanged", input.name).into());
                }
            }
        }
        if computed.iter().any(|(name, slot)| {
            !self.published.iter().any(|input| input.name == name.as_str() && *slot < input.columns.len())
        }) {
            return Err("computed origin names an absent publication column".into());
        }
        Ok(())
    }

    /// A declared scalar codec can replace relational reads, never opaque Rust.
    pub fn scalar_codec_types(&self, manifest: &Manifest) -> Result<BTreeMap<Origin, syn::Type>> {
        let variables = self.variable_origins()?;
        let mut storage = BTreeMap::new();
        for relation in &self.relations {
            for (column, _) in &relation.columns {
                let origin = (relation.table.to_string(), column.to_string());
                let metadata = manifest.column(&origin.0, &origin.1)?;
                if matches!(metadata.encoding.as_str(), "days_since_epoch" | "scaled_integer")
                    && variables.iter().all(|(variable, source)| source != &origin
                        || (scalar_codec_body(&self.program.query.body, variable)
                            && self.program.query.head.args.iter().all(|argument| {
                                matches!(argument, syn::Expr::Path(path) if path.path.is_ident(variable))
                                    || closed_to(argument, variable)
                            })))
                {
                    storage.insert(origin, syn::parse_str(&metadata.ty)?);
                }
            }
        }
        Ok(storage)
    }

    pub fn variable_origins(&self) -> Result<BTreeMap<String, Origin>> {
        let mut inputs = self.published_origins.clone();
        for relation in &self.relations {
            inputs.insert(
                relation.name.to_string(),
                relation
                    .columns
                    .iter()
                    .map(|(column, _)| Some((relation.table.to_string(), column.to_string())))
                    .collect(),
            );
        }
        let mut variables = BTreeMap::new();
        collect_variables(&self.program.query.body, &inputs, &mut variables)?;
        Ok(variables)
    }

    pub fn setup_bindings(
        &self,
        manifest: &Manifest,
        captures: &TokenStream,
    ) -> Result<TokenStream> {
        let mut bindings = TokenStream::new();
        for domain in &self.domains {
            let name = &domain.name;
            // The shared capture IR records demand. Output-only domain
            // declarations must not embed or evaluate their dictionaries.
            if !mentions(captures.clone(), name)
                && !self
                    .operands
                    .iter()
                    .any(|operand| mentions(operand.expression.clone(), name))
            {
                continue;
            }
            let values = manifest.dictionary(
                manifest.column(&domain.table.to_string(), &domain.column.to_string())?,
            )?;
            if !values.windows(2).all(|pair| pair[0] < pair[1]) {
                return Err(format!("dictionary domain {name} must be sorted and unique").into());
            }
            bindings
                .extend(quote!(let #name = crate::manifest::DictionaryDomain(&[#(#values),*]);));
        }
        for operand in &self.operands {
            let stages::Operand {
                name,
                ty,
                expression,
                ..
            } = operand;
            bindings.extend(quote!(let #name: #ty = { #expression };));
        }
        Ok(bindings)
    }

    pub fn encoded_query_with_types(
        &self,
        manifest: &Manifest,
        storage: &BTreeMap<Origin, syn::Type>,
    ) -> Result<TokenStream> {
        let mut program = self.program.clone();
        let origins = self.variable_origins()?;
        self.check_key_expressions(storage, &origins)?;
        encode_scalar_body(&mut program.query.body, manifest, storage, &origins)?;
        restore_key_body(&mut program.query.body, storage, &origins);
        let output_origins = self.answer_columns()?;
        let output_name = program.query.head.relation.clone();
        // Declaration order is a recorded loader ABI: table inputs, then
        // explicit publications. The relational body is never reordered.
        let mut declarations = Vec::new();
        for relation in self.relations.iter().chain(&self.published) {
            let original = self
                .program
                .inputs()
                .find(|input| input.name == relation.name)
                .ok_or("missing authored input schema")?;
            let mut declaration = original.clone();
            for (index, column) in declaration.columns.iter_mut().enumerate() {
                let origin = if self
                    .relations
                    .iter()
                    .any(|input| input.name == relation.name)
                {
                    Some((
                        relation.table.to_string(),
                        relation.columns[index].0.to_string(),
                    ))
                } else {
                    self.published_origins
                        .get(&relation.name.to_string())
                        .and_then(|columns| columns[index].clone())
                };
                column.ty = origin
                    .as_ref()
                    .and_then(|origin| storage.get(origin))
                    .unwrap_or(&relation.columns[index].1)
                    .clone();
                if let Some(id) = origin.as_ref().filter(|origin| storage.contains_key(*origin))
                    .and_then(|(table, column)| crate::key_domains::id(table, column))
                {
                    let domain = quote::format_ident!("__tpch_domain_{id}");
                    column.domain = Some(syn::parse2(quote!(in (#domain)))?);
                }
                if let Some(domain) = origin.as_ref().and_then(|origin| {
                    self.domains.iter().find(|domain| {
                        domain.table == origin.0.as_str() && domain.column == origin.1.as_str()
                    })
                }) {
                    let name = &domain.name;
                    column.domain = Some(syn::parse2(quote!(in (#name)))?);
                }
            }
            declarations.push(declaration);
        }
        let mut output = self
            .program
            .output()
            .ok_or("missing authored output schema")?
            .clone();
        for (column, origin) in output.columns.iter_mut().zip(output_origins) {
            if let Some(ty) = origin.as_ref().and_then(|origin| storage.get(origin)) {
                column.ty = ty.clone();
            }
            // Output domains are recorded in Publication, not new input-domain captures.
            column.domain = None;
        }
        if output.name != output_name {
            return Err("output schema differs from query head".into());
        }
        declarations.push(output);
        // This is a new representation-level declaration boundary. The same
        // authored i64 identity can now describe u32 keys and i64 measures;
        // carrying that old ID forward would conflate their device carriers.
        for column in declarations.iter_mut().flat_map(|declaration| &mut declaration.columns) {
            column.logical_type_id = None;
        }
        cq_lang_cq::syntax::intern_logical_types(&mut declarations)?;
        program.declarations = declarations;
        Ok(program.to_token_stream())
    }

    fn check_key_expressions(
        &self,
        storage: &BTreeMap<Origin, syn::Type>,
        origins: &BTreeMap<String, Origin>,
    ) -> Result<()> {
        for item in cq_lang_cq::blocks::walk(&self.program.query.body) {
            let cq_lang_cq::BodyItem::Aggregate(aggregate) = item else { continue; };
            let relation = self.relations.iter().find(|relation| relation.name == aggregate.atom.relation);
            for (slot, argument) in aggregate.atom.args.iter().enumerate() {
                let origin = relation.and_then(|relation| relation.columns.get(slot).map(|(column, _)|
                    (relation.table.to_string(), column.to_string())))
                    .or_else(|| self.published_origins.get(&aggregate.atom.relation.to_string())
                        .and_then(|columns| columns.get(slot)).cloned().flatten());
                let Some(origin) = origin else { continue; };
                if !storage.contains_key(&origin) || crate::key_domains::id(&origin.0, &origin.1).is_none() {
                    continue;
                }
                if let Some(name) = projected_variable(argument) {
                    if aggregate.arguments.iter().any(|argument| argument == name) {
                        return Err(format!("unsupported domain-key read `{name}` in aggregate item; restore the logical key in an explicit let binding before aggregation").into());
                    }
                }
            }
        }
        for (name, origin) in origins {
            if !storage.contains_key(origin) || crate::key_domains::id(&origin.0, &origin.1).is_none() {
                continue;
            }
            for argument in &self.program.query.head.args {
                if projected_variable(argument).is_some_and(|variable| variable == name.as_str()) {
                    continue;
                }
                if !closed_to(argument, name) {
                    return Err(format!("unsupported domain-key read `{name}` in head expression; restore the logical key in an explicit let binding before projection or aggregation").into());
                }
            }
            for item in cq_lang_cq::blocks::walk(&self.program.query.body) {
                if let cq_lang_cq::BodyItem::Aggregate(aggregate) = item {
                    if aggregate.arguments.iter().any(|argument| argument == name.as_str())
                        || mentions(aggregate.aggregator.to_token_stream(), &quote::format_ident!("{name}"))
                    {
                        return Err(format!("unsupported domain-key read `{name}` in aggregate item; restore the logical key in an explicit let binding before aggregation").into());
                    }
                }
            }
        }
        Ok(())
    }

    pub fn answer_columns(&self) -> Result<Vec<Option<Origin>>> {
        let variables = self.variable_origins()?;
        Ok(self
            .program
            .query
            .head
            .args
            .iter()
            .map(|argument| {
                if let Some(name) = projected_variable(argument) {
                    variables.get(&name.to_string()).cloned()
                } else {
                    None
                }
            })
            .collect())
    }
}

fn projected_variable(expression: &syn::Expr) -> Option<&Ident> {
    match expression {
        syn::Expr::Path(path) => path.path.get_ident(),
        syn::Expr::Unary(unary) if matches!(unary.op, syn::UnOp::Deref(_)) => {
            if let syn::Expr::Path(path) = &*unary.expr { path.path.get_ident() } else { None }
        }
        _ => None,
    }
}

fn restore_key_body(
    body: &mut cq_lang_cq::CommaList<cq_lang_cq::BodyItem>,
    storage: &BTreeMap<Origin, syn::Type>,
    origins: &BTreeMap<String, Origin>,
) {
    use cq_lang_cq::BodyItem;
    for item in body {
        if let BodyItem::Block(block) = item {
            for branch in &mut block.branches { restore_key_body(&mut branch.body, storage, origins); }
            continue;
        }
        let expression = match item {
            BodyItem::Filter(filter) => &mut filter.condition,
            BodyItem::Let(binding) => &mut binding.expression,
            BodyItem::For(binding) => &mut binding.expression,
            _ => continue,
        };
        let reads = cq_lang_cq::rust::free_variables(expression);
        let mut inverses = TokenStream::new();
        for (name, origin) in origins {
            if !storage.contains_key(origin) || (!reads.opaque && !reads.names.contains(name)) { continue; }
            let Some(id) = crate::key_domains::id(&origin.0, &origin.1) else { continue; };
            let variable = quote::format_ident!("{name}");
            let view = quote::format_ident!("__tpch_domain_{id}");
            let logical: syn::Type = if matches!(id, 4 | 5 | 8) { syn::parse_quote!(i32) } else { syn::parse_quote!(i64) };
            // DECODE R: only authored opaque Rust reads restore integer keys.
            inverses.extend(quote!(let #variable = &((unsafe { #view.value(*#variable) }) as #logical);));
        }
        if !inverses.is_empty() {
            *expression = syn::parse_quote!({ #inverses #expression });
        }
    }
}

fn closed_to(expression: &syn::Expr, variable: &str) -> bool {
    let free = cq_lang_cq::rust::free_variables(expression);
    !free.opaque && !free.names.contains(variable)
}

fn mentions(tokens: TokenStream, name: &Ident) -> bool {
    tokens.into_iter().any(|token| match token {
        TokenTree::Ident(ident) => ident == *name,
        TokenTree::Group(group) => mentions(group.stream(), name),
        _ => false,
    })
}

fn scalar_codec_body(body: &cq_lang_cq::CommaList<cq_lang_cq::BodyItem>, variable: &str) -> bool {
    use cq_lang_cq::BodyItem;
    body.iter().all(|item| match item {
        _ if item.rust_step().is_some_and(|step| {
            step.free_variables.opaque || step.free_variables.names.contains(variable)
        }) =>
        {
            false
        }
        BodyItem::Block(block) => block
            .branches
            .iter()
            .all(|branch| scalar_codec_body(&branch.body, variable)),
        BodyItem::Equality(_) | BodyItem::Constraint(_) | BodyItem::Positive { .. } => true,
        _ => !mentions(item.to_token_stream(), &quote::format_ident!("{variable}")),
    })
}

fn collect_variables(
    body: &cq_lang_cq::CommaList<cq_lang_cq::BodyItem>,
    inputs: &BTreeMap<String, Vec<Option<Origin>>>,
    variables: &mut BTreeMap<String, Origin>,
) -> Result<()> {
    for item in body {
        match item {
            cq_lang_cq::BodyItem::Positive { atom } => {
                if let Some(origins) = inputs.get(&atom.relation.to_string()) {
                    for (argument, origin) in atom.args.iter().zip(origins) {
                        if let (syn::Expr::Path(path), Some(origin)) = (argument, origin) {
                            if let Some(name) = path.path.get_ident() {
                                // Join-column source aliases are legal. Dictionary-domain
                                // identity is separately checked by the shared CQ language.
                                variables.insert(name.to_string(), origin.clone());
                            }
                        }
                    }
                }
            }
            cq_lang_cq::BodyItem::Block(block) => {
                for branch in &block.branches {
                    collect_variables(&branch.body, inputs, variables)?;
                }
            }
            _ => {}
        }
    }
    Ok(())
}

/// Adapt only relational operands using recorded scalar encodings. Dictionary
/// operands belong exclusively to shared CQ A-Domain normalization.
fn encode_scalar_body(
    body: &mut cq_lang_cq::CommaList<cq_lang_cq::BodyItem>,
    manifest: &Manifest,
    storage: &BTreeMap<Origin, syn::Type>,
    variables: &BTreeMap<String, Origin>,
) -> Result<()> {
    use cq_lang_cq::syntax::constraint::Term;
    use cq_lang_cq::{BodyItem, Constraint};
    fn operand(term: &mut Term) -> Option<&mut syn::Expr> {
        match term {
            Term::Literal(expression) => Some(expression.as_mut()),
            Term::Closed(expression) => Some(expression),
            Term::Variable(_) => None,
        }
    }
    fn encode(
        term: &Term,
        value: &mut syn::Expr,
        manifest: &Manifest,
        storage: &BTreeMap<Origin, syn::Type>,
        variables: &BTreeMap<String, Origin>,
    ) -> Result<()> {
        if let Term::Variable(name) = term {
            if let Some(origin) = variables
                .get(&name.to_string())
                .filter(|origin| storage.contains_key(*origin))
            {
                match manifest.column(&origin.0, &origin.1)?.encoding.as_str() {
                    "days_since_epoch" => *value = syn::parse_quote!((#value).days_since_epoch()),
                    "scaled_integer" => *value = syn::parse_quote!((#value).scaled()),
                    _ => {}
                }
            }
        }
        Ok(())
    }
    fn constraint(
        predicate: &mut Constraint,
        manifest: &Manifest,
        storage: &BTreeMap<Origin, syn::Type>,
        variables: &BTreeMap<String, Origin>,
    ) -> Result<()> {
        match predicate {
            Constraint::Compare { left, right, .. } => {
                if let Some(value) = operand(right) {
                    encode(left, value, manifest, storage, variables)?;
                }
                if let Some(value) = operand(left) {
                    encode(right, value, manifest, storage, variables)?;
                }
            }
            Constraint::In { term, literals } => {
                for value in literals {
                    encode(term, value, manifest, storage, variables)?;
                }
            }
            Constraint::Between { term, low, high } => {
                for bound in [low, high] {
                    if let Some(value) = operand(bound) {
                        encode(term, value, manifest, storage, variables)?;
                    }
                }
            }
            Constraint::Or(arms) => {
                for predicate in arms.iter_mut().flatten() {
                    constraint(predicate, manifest, storage, variables)?;
                }
            }
        }
        Ok(())
    }
    for item in body {
        match item {
            BodyItem::Block(block) => {
                for branch in &mut block.branches {
                    encode_scalar_body(&mut branch.body, manifest, storage, variables)?;
                }
            }
            BodyItem::Equality(equality) => {
                let mut predicate = Constraint::from_equality(equality);
                constraint(&mut predicate, manifest, storage, variables)?;
                *equality = predicate
                    .equality()
                    .expect("scalar operand adaptation preserves equality");
            }
            BodyItem::Constraint(predicate) => constraint(predicate, manifest, storage, variables)?,
            _ => {}
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn authored(name: &str) -> stages::Program {
        stages::Program::import(SOURCES.iter().find(|(query, _)| *query == name).unwrap().1)
            .unwrap()
    }

    fn kernels(program: &stages::Program, manifest: &Manifest) -> Vec<Kernel> {
        let mut outputs: Vec<Vec<(Ident, syn::Type)>> = Vec::new();
        let mut domains: Vec<Vec<Option<Origin>>> = Vec::new();
        let mut kernels = Vec::new();
        for (index, stage) in program.stages.iter().enumerate() {
            let mut types = BTreeMap::new();
            let mut origins = BTreeMap::new();
            for input in stage.program.inputs() {
                let producer = &stage
                    .inputs
                    .iter()
                    .find(|(name, _)| *name == input.name)
                    .unwrap()
                    .1;
                if let Some(producer) = program.publication_index(producer).unwrap() {
                    types.insert(
                        input.name.to_string(),
                        input
                            .columns
                            .iter()
                            .zip(&outputs[producer])
                            .map(|(column, (_, ty))| (column.name.clone(), ty.clone()))
                            .collect(),
                    );
                    origins.insert(input.name.to_string(), domains[producer].clone());
                } else {
                    let source = program.input(producer).unwrap();
                    if !matches!(source.producer, stages::Producer::Table(_)) {
                        types.insert(
                            input.name.to_string(),
                            program.input_columns(source).unwrap(),
                        );
                        origins.insert(
                            input.name.to_string(),
                            program.input_origins(source).unwrap(),
                        );
                    }
                }
            }
            let mut kernel = program.kernel(index, &types).unwrap();
            assert_eq!(kernel.evidence, stage.evidence);
            assert_eq!(
                kernel.cpu_query().to_string(),
                stage.program.to_token_stream().to_string()
            );
            kernel.published_origins = origins;
            let mut storage = kernel.scalar_codec_types(manifest).unwrap();
            storage.extend(kernel.dictionary_types(manifest).unwrap());
            let encoded: cq_lang_cq::Program =
                syn::parse2(kernel.encoded_query_with_types(manifest, &storage).unwrap()).unwrap();
            outputs.push(
                encoded
                    .output()
                    .unwrap()
                    .columns
                    .iter()
                    .map(|column| (column.name.clone(), column.ty.clone()))
                    .collect(),
            );
            domains.push(kernel.answer_columns().unwrap());
            kernels.push(kernel);
        }
        kernels
    }

    #[test]
    fn rejects_domain_keys_in_head_expressions_and_aggregates() {
        let manifest = Manifest::load(&crate::data_root().join("sf1")).unwrap();
        for clause in [
            "answer(i64::from(*key)) :- Orders(key)",
            "answer(max(key)) :- Orders(key)",
            "answer(value) :- agg value = max(key) in Orders(key)",
        ] {
            let mut kernel = authored("h05").kernel(0, &BTreeMap::new()).unwrap();
            kernel.program = syn::parse_str(&format!(
                "struct Refusal; relation Orders(key:i64); relation answer(value:i64); {clause};"
            )).unwrap();
            kernel.relations = vec![Relation {
                name: syn::parse_quote!(Orders),
                table: syn::parse_quote!(orders),
                columns: vec![(syn::parse_quote!(o_orderkey), syn::parse_quote!(i64))],
            }];
            let error = kernel.encoded_query_with_types(&manifest, &kernel.integer_key_types()).unwrap_err();
            assert!(error.to_string().contains("unsupported domain-key read `key`"), "{error}");
        }
    }

    #[test]
    fn publication_origin_policy_rejects_both_misclassifications() {
        let manifest = Manifest::load(&crate::data_root().join("sf1")).unwrap();
        let kernel = kernels(&authored("h15"), &manifest).remove(1);
        let origins = kernel.computed_publication_inputs().unwrap();
        assert!(origins.contains(&("Revenue".into(), 1)));
        assert!(!origins.contains(&("Revenue".into(), 0)));
        let mut swapped = origins.clone();
        swapped.insert(("Revenue".into(), 0));
        assert!(kernel.check_computed_publication_inputs(&swapped).is_err());
        let mut omitted = origins;
        omitted.remove(&("Revenue".into(), 1));
        assert!(kernel.check_computed_publication_inputs(&omitted).is_err());
    }

    #[test]
    fn integer_members_use_shared_captures_and_restore_only_rust_reads() {
        let manifest = Manifest::load(&crate::data_root().join("sf1")).unwrap();
        for (name, _) in SOURCES {
            for kernel in kernels(&authored(name), &manifest) {
                let mut storage = kernel.scalar_codec_types(&manifest).unwrap();
                storage.extend(kernel.dictionary_types(&manifest).unwrap());
                storage.extend(kernel.integer_key_types());
                let encoded: cq_lang_cq::Program = syn::parse2(
                    kernel.encoded_query_with_types(&manifest, &storage).unwrap(),
                ).unwrap();
                for relation in &kernel.relations {
                    let declaration = encoded.inputs().find(|input| input.name == relation.name).unwrap();
                    for ((column, _), actual) in relation.columns.iter().zip(&declaration.columns) {
                        if let Some(id) = crate::key_domains::id(&relation.table.to_string(), &column.to_string()) {
                            assert_eq!(actual.ty, syn::parse_quote!(u32), "{name}:{column}");
                            let expected = quote::format_ident!("__tpch_domain_{id}");
                            assert!(mentions(actual.domain.as_ref().unwrap().to_token_stream(), &expected));
                        }
                    }
                }
            }
        }
        let kernel = kernels(&authored("h20"), &manifest).remove(1);
        let mut storage = kernel.scalar_codec_types(&manifest).unwrap();
        storage.extend(kernel.integer_key_types());
        let encoded: cq_lang_cq::Program = syn::parse2(kernel.encoded_query_with_types(&manifest, &storage).unwrap()).unwrap();
        let reads: Vec<_> = encoded.query.body.iter().filter_map(|item| item.rust_step()).collect();
        assert!(reads.iter().any(|step| step.free_variables.names.contains("__tpch_domain_7")));
        assert!(!reads.iter().any(|step| step.free_variables.names.contains("__tpch_domain_0")));
    }

    #[test]
    fn all_stage_schemas_domains_and_opaque_rust_survive_representation_adaptation() {
        for sf in [1, 10] {
            let manifest = Manifest::load(&crate::data_root().join(format!("sf{sf}"))).unwrap();
            for (name, _) in SOURCES {
                let program = authored(name);
                for kernel in kernels(&program, &manifest) {
                    let mut storage = kernel.scalar_codec_types(&manifest).unwrap();
                    storage.extend(kernel.dictionary_types(&manifest).unwrap());
                    let encoded: cq_lang_cq::Program = syn::parse2(
                        kernel
                            .encoded_query_with_types(&manifest, &storage)
                            .unwrap(),
                    )
                    .unwrap();
                    assert_eq!(
                        encoded.inputs().count(),
                        kernel.relations.len() + kernel.published.len(),
                        "{name}"
                    );
                    assert_eq!(
                        encoded.output().unwrap().columns.len(),
                        kernel.answer.len(),
                        "{name}"
                    );
                    assert_eq!(encoded.query.head, kernel.program.query.head, "{name}");
                    fn rust_steps(
                        body: &cq_lang_cq::CommaList<cq_lang_cq::BodyItem>,
                        result: &mut Vec<String>,
                    ) {
                        for item in body {
                            if item.rust_step().is_some() {
                                result.push(item.to_token_stream().to_string());
                            }
                            if let cq_lang_cq::BodyItem::Block(block) = item {
                                for branch in &block.branches {
                                    rust_steps(&branch.body, result);
                                }
                            }
                        }
                    }
                    let mut actual = Vec::new();
                    let mut expected = Vec::new();
                    rust_steps(&encoded.query.body, &mut actual);
                    rust_steps(&kernel.program.query.body, &mut expected);
                    assert_eq!(actual, expected, "{name}");
                    for relation in &kernel.relations {
                        let actual = encoded
                            .inputs()
                            .find(|input| input.name == relation.name)
                            .unwrap();
                        for ((column, _), encoded) in relation.columns.iter().zip(&actual.columns) {
                            let origin = (relation.table.to_string(), column.to_string());
                            if manifest.column(&origin.0, &origin.1).unwrap().encoding
                                == "dictionary"
                            {
                                assert_eq!(encoded.ty, syn::parse_quote!(u32));
                                assert!(encoded.domain.is_some(), "{name}: {column}");
                            }
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn published_string_domains_survive_all_h12_aggregations() {
        let manifest = Manifest::load(&crate::data_root().join("sf1")).unwrap();
        for (index, kernel) in kernels(&authored("h12"), &manifest).iter().enumerate() {
            let mode = if index == 0 { 2 } else { 0 };
            assert_eq!(
                kernel.answer_columns().unwrap()[mode],
                Some(("lineitem".into(), "l_shipmode".into()))
            );
        }
    }

    #[test]
    fn scalar_codecs_never_rewrite_opaque_rust_reads() {
        let manifest = Manifest::load(&crate::data_root().join("sf1")).unwrap();
        let program = authored("h01");
        let kernel = program.kernel(0, &BTreeMap::new()).unwrap();
        let storage = kernel.scalar_codec_types(&manifest).unwrap();
        assert!(storage.contains_key(&("lineitem".into(), "l_shipdate".into())));
        assert!(!storage.contains_key(&("lineitem".into(), "l_quantity".into())));
        assert!(!storage.contains_key(&("lineitem".into(), "l_extendedprice".into())));
        let h05 = authored("h05").kernel(0, &BTreeMap::new()).unwrap();
        let storage = h05.scalar_codec_types(&manifest).unwrap();
        assert!(storage.contains_key(&("orders".into(), "o_orderdate".into())));
        assert!(
            h05.encoded_query_with_types(&manifest, &storage)
                .unwrap()
                .to_string()
                .contains("days_since_epoch")
        );
    }

    #[test]
    fn output_only_domains_are_not_loaded_and_date_payloads_remain_host_expressions() {
        let kernel = authored("h01").kernel(0, &BTreeMap::new()).unwrap();
        let manifest = Manifest {
            schema_version: 2,
            sf: 1,
            byte_order: "little".into(),
            relations: BTreeMap::new(),
            root: Default::default(),
        };
        let bindings = kernel
            .setup_bindings(&manifest, &TokenStream::new())
            .unwrap()
            .to_string();
        assert!(!bindings.contains("DictionaryDomain"));
        for operand in &kernel.operands {
            assert_eq!(bindings.matches(&operand.expression.to_string()).count(), 1);
        }
        assert!(bindings.contains("Date :: load"));
        assert!(!bindings.contains("from_days"));
    }

    #[test]
    fn declared_dictionary_domains_match_both_exported_scales() {
        let program = authored("h19");
        for sf in [1, 10] {
            let manifest = Manifest::load(&crate::data_root().join(format!("sf{sf}"))).unwrap();
            for domain in &program.domains {
                let values = manifest
                    .dictionary(
                        manifest
                            .column(&domain.table.to_string(), &domain.column.to_string())
                            .unwrap(),
                    )
                    .unwrap();
                assert!(values.windows(2).all(|pair| pair[0] < pair[1]));
                let authored = cq_load::StringDictionary::new(values.iter().map(String::as_str));
                let entries: Vec<_> = values.iter().map(String::as_str).collect();
                let exported = crate::manifest::DictionaryDomain(&entries);
                for (code, value) in values.iter().enumerate() {
                    assert_eq!(authored.lookup(value), Some(code as u32));
                    assert_eq!(cq_storage::domain::exact(&exported, value), code as u32);
                }
                assert_eq!(
                    cq_storage::domain::exact(&exported, &"__absent__"),
                    u32::MAX
                );
            }
        }
    }
}
