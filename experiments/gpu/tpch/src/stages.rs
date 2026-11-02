//! Authored GPU stage language. Embedded Rust is an opaque token payload.
//! DESIGN contract7-stage-source
//! Contract 7 and DR-StageSource make stage topology, producers and publication
//! authored facts. Never infer them from
//! Rust names, types, closure bodies or ownership-looking expression shapes.
use crate::{manifest::Result, source::Kernel};
use proc_macro2::TokenStream;
use quote::{ToTokens, quote};
use std::collections::{BTreeMap, BTreeSet};
use syn::{
    Ident, Token,
    parse::{Parse, ParseStream},
};

#[derive(Clone)]
pub struct Domain {
    pub name: Ident,
    pub table: Ident,
    pub column: Ident,
}

#[derive(Clone)]
pub struct Column {
    pub name: Ident,
    pub ty: syn::Type,
    pub domain: Option<Ident>,
}

#[derive(Clone)]
pub enum Producer {
    Table(Ident),
    Dictionary {
        domain: Ident,
        predicate: TokenStream,
    },
    Host {
        dependencies: Vec<Ident>,
        expression: TokenStream,
    },
}

#[derive(Clone)]
pub struct Input {
    pub name: Ident,
    /// None denotes a host value, rather than a relation.
    pub columns: Option<Vec<Column>>,
    pub producer: Producer,
}

#[derive(Clone)]
pub enum OperandKind {
    Rust,
    Date,
}

#[derive(Clone)]
pub struct Operand {
    pub name: Ident,
    pub ty: syn::Type,
    pub kind: OperandKind,
    pub expression: TokenStream,
}

#[derive(Clone)]
pub struct Stage {
    pub name: Ident,
    pub inputs: Vec<(Ident, Ident)>,
    pub captures: Vec<Ident>,
    pub program: cq_lang_cq::Program,
    pub evidence: cq_lang_sequence::Evidence,
}

pub struct Publication {
    pub name: Ident,
    pub stage: Ident,
}

pub enum Projection {
    Column(usize),
    Value(TokenStream),
}

pub struct Terminal {
    pub stage: Ident,
    pub projection: Vec<Projection>,
    /// Output slots, after projection, whose code values publish dictionary text.
    pub dictionaries: Vec<(usize, Ident)>,
    /// Recorded frontend lowering fact: reuse the decoded row without copying.
    pub direct: bool,
}

pub struct Program {
    pub name: Ident,
    pub domains: Vec<Domain>,
    pub inputs: Vec<Input>,
    pub operands: Vec<Operand>,
    pub stages: Vec<Stage>,
    pub publications: Vec<Publication>,
    pub terminal: Vec<Terminal>,
}

fn keyword(input: ParseStream<'_>, expected: &str) -> syn::Result<()> {
    let actual: Ident = input.parse()?;
    if actual == expected {
        Ok(())
    } else {
        Err(syn::Error::new_spanned(
            actual,
            format!("expected {expected}"),
        ))
    }
}

fn payload(input: ParseStream<'_>) -> syn::Result<TokenStream> {
    let body;
    syn::braced!(body in input);
    body.parse()
}

fn names(input: ParseStream<'_>) -> syn::Result<Vec<Ident>> {
    let body;
    syn::bracketed!(body in input);
    Ok(body
        .parse_terminated(Ident::parse, Token![,])?
        .into_iter()
        .collect())
}

fn one_name(input: ParseStream<'_>) -> syn::Result<Ident> {
    let body;
    syn::parenthesized!(body in input);
    let name = body.parse()?;
    if !body.is_empty() {
        return Err(body.error("expected one name"));
    }
    Ok(name)
}

fn columns(input: ParseStream<'_>) -> syn::Result<Vec<Column>> {
    let body;
    syn::parenthesized!(body in input);
    let mut columns = Vec::new();
    while !body.is_empty() {
        let name = body.parse()?;
        body.parse::<Token![:]>()?;
        let ty = body.parse()?;
        let domain = if body.peek(Token![in]) {
            body.parse::<Token![in]>()?;
            Some(body.parse()?)
        } else {
            None
        };
        columns.push(Column { name, ty, domain });
        if !body.is_empty() {
            body.parse::<Token![,]>()?;
        }
    }
    Ok(columns)
}

impl Parse for Program {
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        keyword(input, "program")?;
        let name = input.parse()?;
        let body;
        syn::braced!(body in input);
        let mut program = Self {
            name,
            domains: Vec::new(),
            inputs: Vec::new(),
            operands: Vec::new(),
            stages: Vec::new(),
            publications: Vec::new(),
            terminal: Vec::new(),
        };
        while !body.is_empty() {
            let declaration: Ident = body.parse()?;
            match declaration.to_string().as_str() {
                "domain" => {
                    let name = body.parse()?;
                    body.parse::<Token![=]>()?;
                    keyword(&body, "dictionary")?;
                    let origin;
                    syn::parenthesized!(origin in body);
                    let table = origin.parse()?;
                    origin.parse::<Token![.]>()?;
                    let column = origin.parse()?;
                    if !origin.is_empty() {
                        return Err(origin.error("expected table.column"));
                    }
                    body.parse::<Token![;]>()?;
                    program.domains.push(Domain {
                        name,
                        table,
                        column,
                    });
                }
                "input" | "host" => {
                    let name = body.parse()?;
                    let is_relation = declaration == "input";
                    let columns = if is_relation {
                        let columns = columns(&body)?;
                        body.parse::<Token![=]>()?;
                        Some(columns)
                    } else {
                        None
                    };
                    let kind: Ident = body.parse()?;
                    let producer = match kind.to_string().as_str() {
                        "table" if is_relation => Producer::Table(one_name(&body)?),
                        "dictionary" if is_relation => {
                            let domain = one_name(&body)?;
                            body.parse::<Token![where]>()?;
                            Producer::Dictionary {
                                domain,
                                predicate: payload(&body)?,
                            }
                        }
                        "host" if is_relation => {
                            keyword(&body, "uses")?;
                            let dependencies = names(&body)?;
                            keyword(&body, "rust")?;
                            Producer::Host {
                                dependencies,
                                expression: payload(&body)?,
                            }
                        }
                        "uses" if !is_relation => {
                            let dependencies = names(&body)?;
                            keyword(&body, "rust")?;
                            Producer::Host {
                                dependencies,
                                expression: payload(&body)?,
                            }
                        }
                        _ => {
                            return Err(syn::Error::new_spanned(
                                kind,
                                "expected an explicit input producer",
                            ));
                        }
                    };
                    body.parse::<Token![;]>()?;
                    program.inputs.push(Input {
                        name,
                        columns,
                        producer,
                    });
                }
                "operand" => {
                    let name = body.parse()?;
                    body.parse::<Token![:]>()?;
                    let ty = body.parse()?;
                    body.parse::<Token![=]>()?;
                    let kind: Ident = body.parse()?;
                    let kind = match kind.to_string().as_str() {
                        "rust" => OperandKind::Rust,
                        "date" => OperandKind::Date,
                        _ => {
                            return Err(syn::Error::new_spanned(
                                kind,
                                "expected rust or date operand",
                            ));
                        }
                    };
                    let expression = payload(&body)?;
                    body.parse::<Token![;]>()?;
                    program.operands.push(Operand {
                        name,
                        ty,
                        kind,
                        expression,
                    });
                }
                "stage" => {
                    let name = body.parse()?;
                    let stage;
                    syn::braced!(stage in body);
                    keyword(&stage, "inputs")?;
                    let bindings;
                    syn::braced!(bindings in stage);
                    let mut inputs = Vec::new();
                    while !bindings.is_empty() {
                        let input = bindings.parse()?;
                        bindings.parse::<Token![=]>()?;
                        let producer = bindings.parse()?;
                        bindings.parse::<Token![;]>()?;
                        inputs.push((input, producer));
                    }
                    keyword(&stage, "captures")?;
                    let captures = names(&stage)?;
                    stage.parse::<Token![;]>()?;
                    keyword(&stage, "evidence")?;
                    let evidence = payload(&stage)?;
                    let evidence = if evidence.is_empty() {
                        Default::default()
                    } else {
                        syn::parse2(quote!(evidence { #evidence }))?
                    };
                    keyword(&stage, "cq")?;
                    let cq = syn::parse2(payload(&stage)?)?;
                    if !stage.is_empty() {
                        return Err(stage.error("unexpected stage declaration"));
                    }
                    program.stages.push(Stage {
                        name,
                        inputs,
                        captures,
                        program: cq,
                        evidence,
                    });
                }
                "publish" => {
                    let name = body.parse()?;
                    body.parse::<Token![=]>()?;
                    let stage = body.parse()?;
                    keyword(&body, "owned")?;
                    body.parse::<Token![;]>()?;
                    program.publications.push(Publication { name, stage });
                }
                "terminal" => {
                    let terminals;
                    syn::braced!(terminals in body);
                    while !terminals.is_empty() {
                        keyword(&terminals, "from")?;
                        let stage = terminals.parse()?;
                        keyword(&terminals, "project")?;
                        let projection;
                        syn::bracketed!(projection in terminals);
                        let mut fields = Vec::new();
                        while !projection.is_empty() {
                            let kind: Ident = projection.parse()?;
                            fields.push(match kind.to_string().as_str() {
                                "column" => {
                                    let slot;
                                    syn::parenthesized!(slot in projection);
                                    let value = slot.parse::<syn::LitInt>()?.base10_parse()?;
                                    if !slot.is_empty() {
                                        return Err(slot.error("expected one column slot"));
                                    }
                                    Projection::Column(value)
                                }
                                "value" => Projection::Value(payload(&projection)?),
                                _ => {
                                    return Err(syn::Error::new_spanned(
                                        kind,
                                        "expected column or value",
                                    ));
                                }
                            });
                            if !projection.is_empty() {
                                projection.parse::<Token![,]>()?;
                            }
                        }
                        keyword(&terminals, "dictionaries")?;
                        let domains;
                        syn::bracketed!(domains in terminals);
                        let mut dictionaries = Vec::new();
                        while !domains.is_empty() {
                            let slot = domains.parse::<syn::LitInt>()?.base10_parse()?;
                            domains.parse::<Token![=]>()?;
                            dictionaries.push((slot, domains.parse()?));
                            if !domains.is_empty() {
                                domains.parse::<Token![,]>()?;
                            }
                        }
                        terminals.parse::<Token![;]>()?;
                        program.terminal.push(Terminal {
                            stage,
                            projection: fields,
                            dictionaries,
                            direct: false,
                        });
                    }
                }
                _ => {
                    return Err(syn::Error::new_spanned(
                        declaration,
                        "unknown stage-language declaration",
                    ));
                }
            }
        }
        for terminal in &mut program.terminal {
            terminal.direct = program.stages.iter().find(|stage| stage.name == terminal.stage)
                .and_then(|stage| stage.program.output())
                .is_some_and(|output| output.columns.len() == terminal.projection.len())
                && terminal.projection.iter().enumerate()
                    .all(|(index, projection)| matches!(projection, Projection::Column(slot) if *slot == index));
        }
        Ok(program)
    }
}

impl Program {
    pub fn import(source: &str) -> Result<Self> {
        let program: Self = syn::parse_str(source)?;
        program.check()?;
        Ok(program)
    }

    pub fn stage_index(&self, name: &Ident) -> Result<usize> {
        self.stages
            .iter()
            .position(|stage| stage.name == *name)
            .ok_or_else(|| format!("unknown stage {name}").into())
    }

    pub fn publication_index(&self, name: &Ident) -> Result<Option<usize>> {
        self.publications
            .iter()
            .find(|publication| publication.name == *name)
            .map(|publication| self.stage_index(&publication.stage))
            .transpose()
    }

    pub fn domain(&self, name: &Ident) -> Result<&Domain> {
        self.domains
            .iter()
            .find(|domain| domain.name == *name)
            .ok_or_else(|| format!("unknown dictionary domain {name}").into())
    }

    /// Formal rule DR-StageSource: Check authored records without recovering facts from Rust.
    pub fn check(&self) -> Result<()> {
        let mut names = BTreeSet::new();
        for name in self
            .domains
            .iter()
            .map(|item| &item.name)
            .chain(self.inputs.iter().map(|item| &item.name))
            .chain(self.operands.iter().map(|item| &item.name))
            .chain(self.stages.iter().map(|item| &item.name))
            .chain(self.publications.iter().map(|item| &item.name))
        {
            if !names.insert(name.to_string()) {
                return Err(format!("duplicate stage-language name {name}").into());
            }
        }
        let mut origins = BTreeSet::new();
        for domain in &self.domains {
            if !origins.insert((domain.table.to_string(), domain.column.to_string())) {
                return Err("one source dictionary must have one domain identity".into());
            }
        }
        let mut available = BTreeSet::new();
        for input in &self.inputs {
            if let Some(columns) = &input.columns {
                let mut fields = BTreeSet::new();
                for column in columns {
                    if !fields.insert(column.name.to_string()) {
                        return Err(
                            format!("duplicate column {}.{}", input.name, column.name).into()
                        );
                    }
                    if let Some(name) = &column.domain {
                        let domain = self.domain(name)?;
                        if let Producer::Table(table) = &input.producer {
                            if domain.table != *table || domain.column != column.name {
                                return Err(format!(
                                    "domain {name} does not describe {}.{}",
                                    table, column.name
                                )
                                .into());
                            }
                        }
                    }
                }
            }
            match &input.producer {
                Producer::Dictionary { domain, .. } => {
                    self.domain(domain)?;
                    let columns = input
                        .columns
                        .as_ref()
                        .ok_or("dictionary producer must be a relation")?;
                    if columns.len() != 1 || columns[0].domain.as_ref() != Some(domain) {
                        return Err(
                            "dictionary producer needs one column in its declared domain".into(),
                        );
                    }
                }
                Producer::Host { dependencies, .. } => {
                    let mut seen = BTreeSet::new();
                    for dependency in dependencies {
                        if !available.contains(&dependency.to_string())
                            || !seen.insert(dependency.to_string())
                        {
                            return Err(format!(
                                "unknown, forward or duplicate host dependency {dependency}"
                            )
                            .into());
                        }
                    }
                }
                Producer::Table(_) => {}
            }
            available.insert(input.name.to_string());
        }
        for publication in &self.publications {
            self.stage_index(&publication.stage)?;
        }
        for (index, stage) in self.stages.iter().enumerate() {
            let mut bound = BTreeSet::new();
            for (name, producer) in &stage.inputs {
                if !bound.insert(name.to_string()) {
                    return Err(format!("duplicate input binding {}.{name}", stage.name).into());
                }
                let declaration = stage
                    .program
                    .inputs()
                    .find(|input| input.name == *name)
                    .ok_or_else(|| format!("{} has no CQ input {name}", stage.name))?;
                let columns: Vec<_> =
                    if let Some(producer_index) = self.publication_index(producer)? {
                        if producer_index >= index {
                            return Err(format!("forward stage dependency {producer}").into());
                        }
                        self.stages[producer_index]
                            .program
                            .output()
                            .ok_or("stage requires an output schema")?
                            .columns
                            .iter()
                            .map(|column| &column.ty)
                            .collect()
                    } else {
                        self.inputs
                            .iter()
                            .find(|input| input.name == *producer)
                            .and_then(|input| input.columns.as_ref())
                            .ok_or_else(|| format!("unknown relation producer {producer}"))?
                            .iter()
                            .map(|column| &column.ty)
                            .collect()
                    };
                if columns.len() != declaration.columns.len()
                    || columns
                        .iter()
                        .zip(&declaration.columns)
                        .any(|(ty, column)| {
                            ty.to_token_stream().to_string()
                                != column.ty.to_token_stream().to_string()
                        })
                {
                    return Err(format!("input schema mismatch {}.{name}", stage.name).into());
                }
            }
            if stage
                .program
                .inputs()
                .any(|input| !bound.contains(&input.name.to_string()))
            {
                return Err(format!("{} has an unbound CQ input", stage.name).into());
            }
            stage
                .program
                .output()
                .ok_or("stage requires an authored output schema")?;
            let mut captures = BTreeSet::new();
            for capture in &stage.captures {
                if !self.operands.iter().any(|operand| operand.name == *capture)
                    || !captures.insert(capture.to_string())
                {
                    return Err(format!("unknown or duplicate capture {capture}").into());
                }
            }
        }
        if self.stages.is_empty() || self.terminal.is_empty() {
            return Err("program requires stages and terminal publications".into());
        }
        let width = self.terminal[0].projection.len();
        let mut terminal_stages = BTreeSet::new();
        for terminal in &self.terminal {
            if !terminal_stages.insert(terminal.stage.to_string()) {
                return Err("terminal publication consumes a stage twice".into());
            }
            let stage = &self.stages[self.stage_index(&terminal.stage)?];
            let columns = &stage
                .program
                .output()
                .ok_or("terminal stage requires an output schema")?
                .columns;
            if terminal.projection.len() != width {
                return Err("terminal output widths differ".into());
            }
            if terminal.direct {
                if terminal.projection.len() != columns.len()
                    || terminal.projection.iter().enumerate()
                        .any(|(index, projection)| !matches!(projection, Projection::Column(slot) if *slot == index))
                { return Err("direct terminal publication changes its row".into()); }
            }
            let mut used = BTreeSet::new();
            for projection in &terminal.projection {
                if let Projection::Column(slot) = projection {
                    if *slot >= columns.len() || !used.insert(*slot) {
                        return Err("terminal column is out of bounds or repeated".into());
                    }
                }
            }
            let mut decoded = BTreeSet::new();
            for (slot, domain) in &terminal.dictionaries {
                self.domain(domain)?;
                if !matches!(terminal.projection.get(*slot), Some(Projection::Column(_)))
                    || !decoded.insert(*slot)
                {
                    return Err("dictionary publication requires a unique projected column".into());
                }
            }
        }
        Ok(())
    }

    pub fn kernel(
        &self,
        index: usize,
        types: &BTreeMap<String, Vec<(Ident, syn::Type)>>,
    ) -> Result<Kernel> {
        Kernel::from_stage(self, index, types)
    }

    pub fn input(&self, name: &Ident) -> Result<&Input> {
        self.inputs
            .iter()
            .find(|input| input.name == *name)
            .ok_or_else(|| format!("unknown input {name}").into())
    }

    pub fn input_columns(&self, input: &Input) -> Result<Vec<(Ident, syn::Type)>> {
        Ok(input
            .columns
            .as_ref()
            .ok_or("host value has no relation schema")?
            .iter()
            .map(|column| {
                (
                    column.name.clone(),
                    if column.domain.is_some() {
                        syn::parse_quote!(u32)
                    } else {
                        column.ty.clone()
                    },
                )
            })
            .collect())
    }

    pub fn input_origins(&self, input: &Input) -> Result<Vec<Option<crate::source::Origin>>> {
        input
            .columns
            .as_ref()
            .ok_or("host value has no relation schema")?
            .iter()
            .map(|column| {
                column
                    .domain
                    .as_ref()
                    .map(|name| {
                        self.domain(name)
                            .map(|domain| (domain.table.to_string(), domain.column.to_string()))
                    })
                    .transpose()
            })
            .collect()
    }

    pub fn prepared_name(input: &Input) -> Ident {
        quote::format_ident!("__cq_prepared_{}", input.name)
    }

    pub fn preprocessing_tokens(&self) -> Result<TokenStream> {
        let mut needed = BTreeSet::new();
        for stage in &self.stages {
            for (_, producer) in &stage.inputs {
                if let Some(input) = self.inputs.iter().find(|input| input.name == *producer) {
                    if !matches!(input.producer, Producer::Table(_)) {
                        needed.insert(input.name.to_string());
                    }
                }
            }
        }
        for input in self.inputs.iter().rev() {
            if needed.contains(&input.name.to_string()) {
                if let Producer::Host { dependencies, .. } = &input.producer {
                    needed.extend(dependencies.iter().map(ToString::to_string));
                }
            }
        }
        let mut tokens = TokenStream::new();
        for input in &self.inputs {
            if !needed.contains(&input.name.to_string()) {
                continue;
            }
            let name = &input.name;
            let annotation = input.columns.as_ref().map(|columns| {
                let types = columns.iter().map(|column| &column.ty);
                quote!(: Vec<(#(#types,)*)>)
            });
            let expression = match &input.producer {
                Producer::Table(table) => {
                    let table = table.to_string();
                    let columns = input
                        .columns
                        .as_ref()
                        .ok_or("table input requires columns")?;
                    let variables: Vec<_> = (0..columns.len())
                        .map(|slot| quote::format_ident!("column_{slot}"))
                        .collect();
                    // DECODE ?: explicit host producer receives declared logical rows; column-level Rust demand is not recorded.
                    let reads = columns.iter().zip(&variables).map(|(column, variable)| {
                        let name = column.name.to_string();
                        let ty = &column.ty;
                        quote!(let mut #variable = <#ty as crate::manifest::HostValue>::read(manifest, #table, #name)?.into_iter();)
                    });
                    quote!({
                        #(#reads)*
                        (0..manifest.relations[#table].rows)
                            .map(|_| (#(#variables.next().expect("validated manifest row count"),)*))
                            .collect()
                    })
                }
                Producer::Dictionary { domain, predicate } => {
                    let origin = self.domain(domain)?;
                    let table = origin.table.to_string();
                    let column = origin.column.to_string();
                    let ty = &input
                        .columns
                        .as_ref()
                        .ok_or("dictionary input requires columns")?[0]
                        .ty;
                    quote!({
                        let dictionary = manifest.dictionary(manifest.column(#table, #column)?)?;
                        let mut predicate = { #predicate };
                        // DECODE R: explicit dictionary producer evaluates its authored predicate once per dictionary value.
                        dictionary.iter().enumerate().filter_map(|(code, value)| {
                            predicate(value.as_str()).then(|| (
                                <#ty as crate::manifest::DictionaryCode>::from_code(
                                    u32::try_from(code).expect("validated dictionary size")),
                            ))
                        }).collect()
                    })
                }
                Producer::Host {
                    dependencies,
                    expression,
                } => {
                    let bindings = dependencies.iter().map(|name| quote!(let #name = &#name;));
                    quote!({ #(#bindings)* #expression })
                }
            };
            tokens.extend(quote!(let #name #annotation = #expression;));
        }
        // Adapt the explicitly declared domain columns at the publication
        // boundary, after every opaque host producer has consumed logical rows.
        for input in &self.inputs {
            if matches!(input.producer, Producer::Table(_))
                || !self.stages.iter().any(|stage| {
                    stage
                        .inputs
                        .iter()
                        .any(|(_, producer)| *producer == input.name)
                })
            {
                continue;
            }
            let name = &input.name;
            let prepared = Self::prepared_name(input);
            let columns = input
                .columns
                .as_ref()
                .ok_or("stage input requires columns")?;
            if columns.iter().all(|column| column.domain.is_none()) {
                tokens.extend(quote!(let #prepared = #name;));
            } else {
                let fields = columns.iter().enumerate().map(|(index, column)| {
                    let slot = syn::Index::from(index);
                    if column.domain.is_some() {
                        quote!(crate::manifest::DictionaryCode::into_code(row.#slot))
                    } else {
                        quote!(row.#slot)
                    }
                });
                tokens.extend(quote!(let #prepared: Vec<_> = #name.into_iter().map(|row| (#(#fields,)*)).collect();));
            }
        }
        Ok(tokens)
    }
}

impl ToTokens for Program {
    fn to_tokens(&self, tokens: &mut TokenStream) {
        let name = &self.name;
        let domains = self.domains.iter().map(|domain| {
            let Domain {
                name,
                table,
                column,
            } = domain;
            quote!(domain #name = dictionary(#table.#column);)
        });
        let inputs = self.inputs.iter().map(|input| {
            let name = &input.name;
            let prefix = match &input.columns {
                Some(columns) => {
                    let columns = columns.iter().map(|column| {
                        let Column { name, ty, domain } = column;
                        let domain = domain.as_ref().map(|name| quote!(in #name));
                        quote!(#name: #ty #domain)
                    });
                    quote!(input #name(#(#columns),*) =)
                }
                None => quote!(host #name),
            };
            let producer = match &input.producer {
                Producer::Table(table) => quote!(table(#table)),
                Producer::Dictionary { domain, predicate } => {
                    quote!(dictionary(#domain) where { #predicate })
                }
                Producer::Host {
                    dependencies,
                    expression,
                } => {
                    let kind = input.columns.as_ref().map(|_| quote!(host));
                    quote!(#kind uses [#(#dependencies),*] rust { #expression })
                }
            };
            quote!(#prefix #producer;)
        });
        let operands = self.operands.iter().map(|operand| {
            let Operand {
                name,
                ty,
                kind,
                expression,
            } = operand;
            let kind = match kind {
                OperandKind::Rust => quote!(rust),
                OperandKind::Date => quote!(date),
            };
            quote!(operand #name: #ty = #kind { #expression };)
        });
        let stages = self.stages.iter().map(|stage| {
            let Stage { name, inputs, captures, program, evidence } = stage;
            let inputs = inputs.iter().map(|(input, producer)| quote!(#input = #producer;));
            let facts = &evidence.facts;
            quote!(stage #name { inputs { #(#inputs)* } captures [#(#captures),*]; evidence { #(#facts)* } cq { #program } })
        });
        let publications = self.publications.iter().map(|publication| {
            let Publication { name, stage } = publication;
            quote!(publish #name = #stage owned;)
        });
        let terminal = self.terminal.iter().map(|terminal| {
            let stage = &terminal.stage;
            let projection = terminal
                .projection
                .iter()
                .map(|projection| match projection {
                    Projection::Column(slot) => quote!(column(#slot)),
                    Projection::Value(value) => quote!(value { #value }),
                });
            let dictionaries = terminal
                .dictionaries
                .iter()
                .map(|(slot, domain)| quote!(#slot = #domain));
            quote!(from #stage project [#(#projection),*] dictionaries [#(#dictionaries),*];)
        });
        tokens.extend(quote!(program #name { #(#domains)* #(#inputs)* #(#operands)* #(#stages)* #(#publications)* terminal { #(#terminal)* } }));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn simple() -> &'static str {
        r#"program sample {
            domain Labels = dictionary(items.label);
            input Items(key: CallerType, label: CallerCode in Labels) = table(items);
            operand cutoff: CallerType = date { unrelated::date("not an ISO date").clone() };
            stage s0 {
                inputs { Items = Items; }
                captures [cutoff];
                evidence {}
                cq {
                    struct Query;
                    relation Items(key: CallerType, label: CallerCode);
                    relation answer(key: CallerType, label: CallerCode);
                    answer(key, label) :- Items(key, label), where key > cutoff;
                }
            }
            terminal { from s0 project [column(0), column(1)] dictionaries [1 = Labels]; }
        }"#
    }

    #[test]
    fn every_authored_program_is_checked_and_printable() {
        assert_eq!(crate::source::SOURCES.len(), 26);
        for (name, source) in crate::source::SOURCES {
            let program = Program::import(source).unwrap_or_else(|error| panic!("{name}: {error}"));
            assert_eq!(program.name.to_string(), *name);
            let printed = program.to_token_stream();
            let roundtrip = Program::import(&printed.to_string()).unwrap();
            assert_eq!(
                printed.to_string(),
                roundtrip.to_token_stream().to_string(),
                "{name}"
            );
            assert_eq!(program.stages.len(), roundtrip.stages.len(), "{name}");
        }
    }

    #[test]
    fn rust_payloads_are_neither_recognized_nor_evaluated() {
        let program = Program::import(simple()).unwrap();
        assert_eq!(
            program.operands[0].expression.to_string(),
            quote!(unrelated::date("not an ISO date").clone()).to_string()
        );
        let source = simple().replace(
            "operand cutoff: CallerType = date { unrelated::date(\"not an ISO date\").clone() };",
            r#"operand cutoff: CallerType = rust {
                data.publish("x", values).map(|x| (*x).clone()).into_iter()
                    .chain(other.as_slice()).collect()
            };"#,
        );
        let program = Program::import(&source).unwrap();
        assert_eq!(
            program.operands[0].expression.to_string(),
            quote! {
                data.publish("x", values).map(|x| (*x).clone()).into_iter()
                    .chain(other.as_slice()).collect()
            }
            .to_string()
        );
        let printed = program.to_token_stream().to_string();
        assert_eq!(
            Program::import(&printed).unwrap().operands[0]
                .expression
                .to_string(),
            program.operands[0].expression.to_string()
        );
    }

    #[test]
    fn malformed_authored_graphs_are_rejected() {
        for (from, to) in [
            ("Items = Items;", "Items = Unknown;"),
            ("Items = Items;", "Items = Items; Items = Items;"),
            ("Items = Items;", ""),
            ("captures [cutoff]", "captures [unknown]"),
            ("captures [cutoff]", "captures [cutoff, cutoff]"),
            (
                "dictionaries [1 = Labels];",
                "dictionaries [1 = Labels]; from s0 project [column(0), column(1)] dictionaries [1 = Labels];",
            ),
            ("column(0), column(1)", "column(0), column(0)"),
            ("column(0), column(1)", "column(0), column(2)"),
            ("dictionaries [1 = Labels]", "dictionaries [1 = Missing]"),
            ("dictionaries [1 = Labels]", "dictionaries [2 = Labels]"),
            ("from s0", "from missing"),
            (
                "relation Items(key: CallerType",
                "relation Items(key: OtherType",
            ),
            ("in Labels", "in Unknown"),
        ] {
            assert!(
                Program::import(&simple().replace(from, to)).is_err(),
                "{from} -> {to}"
            );
        }
        let forward = simple()
            .replace("Items = Items;", "Items = Later;")
            .replace("terminal {", "publish Later = s0 owned; terminal {");
        assert!(Program::import(&forward).is_err());
        let host_forward = simple().replace(
            "stage s0",
            "host Prepared uses [Later] rust { Later }; stage s0",
        );
        assert!(Program::import(&host_forward).is_err());
        let dictionary_width = simple().replace(
            "= table(items)",
            "= dictionary(Labels) where { |text| true }",
        );
        assert!(Program::import(&dictionary_width).is_err());
    }

    #[test]
    fn stage_counts_publications_and_terminal_order_are_preserved() {
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
            let source = crate::source::SOURCES
                .iter()
                .find(|(query, _)| *query == name)
                .unwrap()
                .1;
            let program = Program::import(source).unwrap();
            assert_eq!(program.stages.len(), count, "{name}");
            let actual: Vec<_> = program
                .terminal
                .iter()
                .map(|terminal| program.stage_index(&terminal.stage).unwrap())
                .collect();
            assert_eq!(actual, terminals, "{name}");
        }
        let h13 = Program::import(
            crate::source::SOURCES
                .iter()
                .find(|(name, _)| *name == "h13")
                .unwrap()
                .1,
        )
        .unwrap();
        assert!(matches!(
            h13.terminal[1].projection[1],
            Projection::Value(_)
        ));
        assert_eq!(
            h13.publication_index(&syn::parse_quote!(CountedCustomers))
                .unwrap(),
            Some(0)
        );
        let host = h13.preprocessing_tokens().unwrap().to_string();
        assert_eq!(host.matches("from_dense_keys").count(), 1);
        assert_eq!(host.matches("split_once").count(), 1);
        assert!(host.find("from_dense_keys").unwrap() < host.find("filter (").unwrap());
    }

    #[test]
    fn key_evidence_is_preserved_as_an_authored_record() {
        let h01 = Program::import(
            crate::source::SOURCES
                .iter()
                .find(|(name, _)| *name == "h01")
                .unwrap()
                .1,
        )
        .unwrap();
        assert!(
            h01.stages[0]
                .evidence
                .has_key("Lineitem", &["l_orderkey".into(), "l_linenumber".into()])
        );
        let h13 = Program::import(
            crate::source::SOURCES
                .iter()
                .find(|(name, _)| *name == "h13")
                .unwrap()
                .1,
        )
        .unwrap();
        assert!(
            h13.stages
                .iter()
                .all(|stage| stage.evidence.facts.is_empty())
        );
    }
}
