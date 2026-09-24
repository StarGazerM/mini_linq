//! Stream: IX with every result realized as a typed indexed-stream program.
//!
//! This is the first language whose meaning includes representation. Each RA
//! result becomes a comprehension over cursors of earlier names. `⟦d⟧` is a
//! bag over the RA heading with `support ⟦d⟧ = ⟦r⟧_RA`; the result's mode says
//! whether that bag is a stream consumed once (`pipe`) or a value bound with
//! declared capabilities (`let`). Capabilities are the operations a cursor
//! over a name admits, each with a law against the name's abstraction
//! function: `seek @r[C](k) = σ_{C=k}`, `contains` its nonemptiness,
//! `ordered` a key-ordered `seek`, `iter` enumeration of the support.
//!
//! Nothing here names a data structure, a Rust type, or an execution form.
//! `co` is a comprehension form because its premise (both cursors ordered on
//! the shared key) is a type; below this language it is one cursor combinator.
//!
//! ```text
//! stream {
//!     r  = let provides { seek @r[0] } for (a, b) in scan R { emit (a, b) };
//!     j  = pipe for (a) in scan l { for (b) in seek r[0](a) { emit (a, b) } };
//!     p  = pipe for (a, b) in scan j { support { emit (a) } };
//!     return p;
//! }
//! ```
//!
//! ```text
//! Module     ::= Structures "stream" "{" Definition* Return "}"
//! Definition ::= Ident "=" Realization ";"
//! Realization ::= "alias" Ident | ["lazy"] Mode Body
//! Mode       ::= "pipe" | "let" "provides" "{" [Cap ("," Cap)* [","]] "}"
//! Cap        ::= "iter" | ("seek" | "contains" | "ordered") "@" Ident "[" Column,* "]"
//! Body       ::= "for" Pat "in" Cursor "{" Body "}"
//!              | "co" "(" (Pat "in" Cursor),+ ")" "on" Pat "{" Body "}"
//!              | "if" Guard "{" Body "}"
//!              | "let" Pat [":" Type] "=" Expr ";" Body
//!              | "support" "{" Body "}"
//!              | "unit" "{" Body "}"
//!              | "emit" "(" [Ident ("," Ident)* [","]] ")"
//! Guard      ::= "(" Expr ")" | "contains" Ident "[" Column,* "]" "(" Ident,* ")"
//! Cursor     ::= "scan" Ident ["within" "[" Column "]" "(" Limit "," Limit ")"]
//! Limit      ::= "_" | "(" Expr ")"
//!              | "seek" Ident "[" Column,* "]" "(" Ident,* ")"
//!              | "order" Ident "[" Column,* "]"
//! Return     ::= "return" Ident ";"
//! ```

use std::collections::{BTreeMap, BTreeSet};

use syn::{Expr, Ident, Pat, Token, token};

pub use crate::capabilities::{Capability, Columns, Descriptor, Operation, Provides, operations};
use crate::{Column, CommaList, cq, relational_plan as ra, structures, symbol_name};

pub mod kw {
    syn::custom_keyword!(stream);
    pub use crate::capabilities::kw::provides;
    syn::custom_keyword!(pipe);
    syn::custom_keyword!(lazy);
    syn::custom_keyword!(alias);
    pub use crate::capabilities::kw::{contains, iter, ordered, seek};
    syn::custom_keyword!(co);
    syn::custom_keyword!(on);
    syn::custom_keyword!(scan);
    syn::custom_keyword!(within);
    syn::custom_keyword!(order);
    syn::custom_keyword!(support);
    syn::custom_keyword!(unit);
    syn::custom_keyword!(emit);
}

#[derive(Clone, Debug, PartialEq, Eq, syn_derive::Parse, syn_derive::ToTokens)]
pub struct Module {
    pub structures: structures::Module,
    pub stream_token: kw::stream,
    #[syn(braced)]
    pub stream_brace: token::Brace,
    #[syn(in = stream_brace)]
    pub program: Program,
}

#[derive(Clone, Debug, PartialEq, Eq, syn_derive::Parse, syn_derive::ToTokens)]
pub struct Program {
    #[syn(repeat, until = Token![return])]
    pub definitions: Vec<Definition>,
    pub return_: Return,
}

#[derive(Clone, Debug, PartialEq, Eq, syn_derive::Parse, syn_derive::ToTokens)]
pub struct Definition {
    pub name: Ident,
    pub eq_token: Token![=],
    pub realization: Realization,
    pub semi_token: Token![;],
}

impl Definition {
    pub fn body(&self) -> Option<&Body> {
        match &self.realization {
            Realization::Alias { .. } => None,
            Realization::Stream(streamed) => Some(&streamed.body),
        }
    }
}

/// A rename of a host relation may be the host value itself, with its
/// capabilities translated through the rename; every other result is a
/// stream program in a mode.
#[derive(Clone, Debug, PartialEq, Eq, syn_derive::Parse, syn_derive::ToTokens)]
pub enum Realization {
    #[parse(peek = kw::alias)]
    Alias {
        alias_token: kw::alias,
        host: Ident,
    },
    Stream(Box<Streamed>),
}

#[derive(Clone, Debug, PartialEq, Eq, syn_derive::Parse, syn_derive::ToTokens)]
pub struct Streamed {
    pub lazy: Option<kw::lazy>,
    pub mode: Mode,
    pub body: Body,
}

#[derive(Clone, Debug, PartialEq, Eq, syn_derive::Parse, syn_derive::ToTokens)]
pub enum Mode {
    #[parse(peek = kw::pipe)]
    Pipe { pipe_token: kw::pipe },
    #[parse(peek = Token![let])]
    Let {
        let_token: Token![let],
        provides: Provides,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, syn_derive::Parse, syn_derive::ToTokens)]
pub enum Body {
    #[parse(peek = Token![for])]
    For(Box<For>),
    #[parse(peek = kw::co)]
    Co(Box<Co>),
    #[parse(peek = Token![if])]
    If(Box<If>),
    #[parse(peek = Token![let])]
    Let(Box<Let>),
    #[parse(peek = kw::support)]
    Support(Box<Support>),
    #[parse(peek = kw::unit)]
    Unit(Box<Unit>),
    #[parse(peek = kw::emit)]
    Emit(Emit),
}

#[derive(Clone, Debug, PartialEq, Eq, syn_derive::Parse, syn_derive::ToTokens)]
pub struct For {
    pub for_token: Token![for],
    #[parse(Pat::parse_multi_with_leading_vert)]
    pub pattern: Pat,
    pub in_token: Token![in],
    pub cursor: Cursor,
    #[syn(braced)]
    pub brace: token::Brace,
    #[syn(in = brace)]
    pub body: Body,
}

#[derive(Clone, Debug, PartialEq, Eq, syn_derive::Parse, syn_derive::ToTokens)]
pub struct Co {
    pub co_token: kw::co,
    #[syn(parenthesized)]
    pub paren: token::Paren,
    #[syn(in = paren)]
    #[parse(CommaList::parse_terminated)]
    pub cursors: CommaList<Bound>,
    pub on_token: kw::on,
    #[parse(Pat::parse_multi_with_leading_vert)]
    pub key: Pat,
    #[syn(braced)]
    pub brace: token::Brace,
    #[syn(in = brace)]
    pub body: Body,
}

#[derive(Clone, Debug, PartialEq, Eq, syn_derive::Parse, syn_derive::ToTokens)]
pub struct Bound {
    #[parse(Pat::parse_multi_with_leading_vert)]
    pub pattern: Pat,
    pub in_token: Token![in],
    pub cursor: Cursor,
}

#[derive(Clone, Debug, PartialEq, Eq, syn_derive::Parse, syn_derive::ToTokens)]
pub struct If {
    pub if_token: Token![if],
    pub guard: Guard,
    #[syn(braced)]
    pub brace: token::Brace,
    #[syn(in = brace)]
    pub body: Body,
}

#[derive(Clone, Debug, PartialEq, Eq, syn_derive::Parse, syn_derive::ToTokens)]
pub enum Guard {
    #[parse(peek = token::Paren)]
    Expr(Parenthesized),
    #[parse(peek = kw::contains)]
    Contains {
        contains_token: kw::contains,
        relation: Ident,
        columns: Columns,
        keys: Keys,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, syn_derive::Parse, syn_derive::ToTokens)]
pub struct Parenthesized {
    #[syn(parenthesized)]
    pub paren: token::Paren,
    #[syn(in = paren)]
    pub expr: Expr,
}

#[derive(Clone, Debug, PartialEq, Eq, syn_derive::Parse, syn_derive::ToTokens)]
pub struct Keys {
    #[syn(parenthesized)]
    pub paren: token::Paren,
    #[syn(in = paren)]
    #[parse(CommaList::parse_terminated)]
    pub variables: CommaList<Ident>,
}

#[derive(Clone, Debug, PartialEq, Eq, syn_derive::Parse, syn_derive::ToTokens)]
pub struct Let {
    pub let_token: Token![let],
    #[parse(Pat::parse_multi_with_leading_vert)]
    pub pattern: Pat,
    #[syn(optional, peek = Token![:])]
    pub annotation: Option<cq::TypeAnnotation>,
    pub eq_token: Token![=],
    pub value: Expr,
    pub semi_token: Token![;],
    pub body: Body,
}

#[derive(Clone, Debug, PartialEq, Eq, syn_derive::Parse, syn_derive::ToTokens)]
pub struct Support {
    pub support_token: kw::support,
    #[syn(braced)]
    pub brace: token::Brace,
    #[syn(in = brace)]
    pub body: Body,
}

#[derive(Clone, Debug, PartialEq, Eq, syn_derive::Parse, syn_derive::ToTokens)]
pub struct Unit {
    pub unit_token: kw::unit,
    #[syn(braced)]
    pub brace: token::Brace,
    #[syn(in = brace)]
    pub body: Body,
}

#[derive(Clone, Debug, PartialEq, Eq, syn_derive::Parse, syn_derive::ToTokens)]
pub struct Emit {
    pub emit_token: kw::emit,
    #[syn(parenthesized)]
    pub paren: token::Paren,
    #[syn(in = paren)]
    #[parse(CommaList::parse_terminated)]
    pub variables: CommaList<Ident>,
}

#[derive(Clone, Debug, PartialEq, Eq, syn_derive::Parse, syn_derive::ToTokens)]
pub enum Cursor {
    #[parse(peek = kw::scan)]
    Scan {
        scan_token: kw::scan,
        relation: Ident,
        #[syn(optional, peek = kw::within)]
        within: Option<Box<Within>>,
    },
    #[parse(peek = kw::seek)]
    Seek {
        seek_token: kw::seek,
        relation: Ident,
        columns: Columns,
        keys: Keys,
    },
    #[parse(peek = kw::order)]
    Order {
        order_token: kw::order,
        relation: Ident,
        columns: Columns,
    },
}

/// A key-bounded scan: `seek(lo)` then every pair while `key <= hi`. Its
/// premise is `ordered` on the column; an open bound is written `_`.
#[derive(Clone, Debug, PartialEq, Eq, syn_derive::Parse, syn_derive::ToTokens)]
pub struct Within {
    pub within_token: kw::within,
    #[syn(bracketed)]
    pub bracket: token::Bracket,
    #[syn(in = bracket)]
    pub column: Column,
    #[syn(parenthesized)]
    pub paren: token::Paren,
    #[syn(in = paren)]
    pub low: Limit,
    #[syn(in = paren)]
    pub comma: Token![,],
    #[syn(in = paren)]
    pub high: Limit,
}

#[derive(Clone, Debug, PartialEq, Eq, syn_derive::Parse, syn_derive::ToTokens)]
pub enum Limit {
    #[parse(peek = Token![_])]
    Open(Token![_]),
    #[parse(peek = token::Paren)]
    Value(Parenthesized),
}

impl Limit {
    pub fn value(&self) -> Option<&Expr> {
        match self {
            Limit::Open(_) => None,
            Limit::Value(p) => Some(&p.expr),
        }
    }
}

impl Cursor {
    pub fn relation(&self) -> &Ident {
        match self {
            Cursor::Scan { relation, .. }
            | Cursor::Seek { relation, .. }
            | Cursor::Order { relation, .. } => relation,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, syn_derive::Parse, syn_derive::ToTokens)]
pub struct Return {
    pub return_token: Token![return],
    pub name: Ident,
    pub semi_token: Token![;],
}

/// The Stream language contract in diagnostic and predicate form.
pub mod contract {
    /// Check Structures, then Stream's own well-formedness: names, scope,
    /// capability discharge against the chosen structures, affine use of
    /// pipes, the support obligation, and strategy.
    pub fn check(module: &super::Module) -> syn::Result<super::Analysis> {
        let structures = super::structures::contract::check(&module.structures)?;
        super::analyze(module, structures)
    }

    pub fn well_formed(module: &super::Module) -> bool {
        check(module).is_ok()
    }
}

/// The reader name recorded for the returned result.
pub const RETURN: &str = "return";

/// Facts the contract establishes, consumed by later stages.
#[derive(Clone, Debug, Default)]
pub struct Analysis {
    /// Sorted RA heading of every input and definition.
    pub headings: BTreeMap<String, Vec<String>>,
    /// Operations every host, alias, or `let` provides.
    pub provided: BTreeMap<String, Vec<Operation>>,
    /// Operations each definition requires of each name it reads.
    pub required: BTreeMap<String, BTreeMap<String, BTreeSet<Operation>>>,
    /// Readers of every name, in definition order.
    pub readers: BTreeMap<String, Vec<String>>,
    /// Names that are read exactly once, by that reader (or [`RETURN`]), and
    /// never again: a later stage may consume them in place.
    pub unique: BTreeMap<String, String>,
    /// Alias definitions and the host relation each stands for.
    pub aliases: BTreeMap<String, String>,
}

fn error<T>(spanned: &impl quote::ToTokens, message: impl std::fmt::Display) -> syn::Result<T> {
    Err(syn::Error::new_spanned(spanned, message))
}

fn check_columns(
    columns: &[usize],
    arity: usize,
    spanned: &impl quote::ToTokens,
) -> syn::Result<()> {
    if columns.windows(2).any(|w| w[0] >= w[1]) {
        return error(
            spanned,
            "columns must be strictly increasing heading ordinals",
        );
    }
    if let Some(c) = columns.iter().find(|&&c| c >= arity) {
        return error(spanned, format!("column {c} exceeds heading arity {arity}"));
    }
    Ok(())
}

struct Checker<'a> {
    module: &'a Module,
    analysis: Analysis,
    structures: structures::Analysis,
    operators: BTreeMap<String, &'a ra::Operator>,
    modes: BTreeMap<String, bool>,
    set_valued: BTreeMap<String, bool>,
}

fn analyze(module: &Module, structures: structures::Analysis) -> syn::Result<Analysis> {
    let ra_module = &module.structures.index_requirements.relational_plan;
    let mut checker = Checker {
        module,
        analysis: Analysis {
            headings: structures.headings.clone(),
            ..Analysis::default()
        },
        structures,
        operators: BTreeMap::new(),
        modes: BTreeMap::new(),
        set_valued: BTreeMap::new(),
    };
    for definition in &ra_module.plan.definitions {
        let name = symbol_name(&definition.result);
        checker.operators.insert(name, &definition.operator);
    }
    for input in ra_module.program.inputs() {
        let name = symbol_name(&input.name);
        let provided = checker.structures.provided[&name].clone();
        checker.analysis.provided.insert(name, provided);
    }
    checker.check_definitions()?;
    checker.check_return()?;
    Ok(checker.analysis)
}

impl<'a> Checker<'a> {
    fn program(&self) -> &'a Program {
        &self.module.program
    }

    fn arity(&self, name: &Ident) -> syn::Result<usize> {
        self.analysis
            .headings
            .get(&symbol_name(name))
            .map(Vec::len)
            .ok_or_else(|| syn::Error::new_spanned(name, "unknown relation name"))
    }

    /// A `let` may claim only operations some chosen structure on the name
    /// provides (`iter` is materialized rows and always available).
    fn check_provides(&self, name: &Ident, provides: &Provides) -> syn::Result<Vec<Operation>> {
        let arity = self.arity(name)?;
        for capability in &provides.capabilities {
            if let Capability::Seek { descriptor: d, .. }
            | Capability::Contains { descriptor: d, .. }
            | Capability::Ordered { descriptor: d, .. } = capability
            {
                if symbol_name(&d.relation) != symbol_name(name) {
                    return error(d, format!("capability must describe `{name}` itself"));
                }
                check_columns(&d.columns.ordinals(), arity, d)?;
            }
        }
        let operations = operations(provides);
        let mut seen = BTreeSet::new();
        let chosen = self
            .structures
            .provided
            .get(&symbol_name(name))
            .cloned()
            .unwrap_or_default();
        for operation in &operations {
            if !seen.insert(operation.clone()) {
                return error(provides, "duplicate capability");
            }
            if *operation != Operation::Iter && !chosen.contains(operation) {
                return error(
                    provides,
                    format!("no structure chosen on `{name}` provides {operation:?}"),
                );
            }
        }
        Ok(operations)
    }

    fn check_definitions(&mut self) -> syn::Result<()> {
        let ra_definitions = &self
            .module
            .structures
            .index_requirements
            .relational_plan
            .plan
            .definitions;
        let definitions = &self.program().definitions;
        if definitions.len() != ra_definitions.len() {
            return error(
                &self.module.stream_token,
                "stream must define exactly the RA results, in RA order",
            );
        }
        for (definition, ra_definition) in definitions.iter().zip(ra_definitions) {
            let name = symbol_name(&definition.name);
            if name != symbol_name(&ra_definition.result) {
                return error(
                    &definition.name,
                    format!(
                        "expected definition of `{}` here, in RA order",
                        ra_definition.result
                    ),
                );
            }
            let streamed: &Streamed = match &definition.realization {
                Realization::Alias { alias_token, host } => {
                    self.check_alias(&name, alias_token, host, &ra_definition.operator)?;
                    continue;
                }
                Realization::Stream(streamed) => streamed,
            };
            let is_let = match &streamed.mode {
                Mode::Pipe { .. } => false,
                Mode::Let { provides, .. } => {
                    let operations = self.check_provides(&definition.name, provides)?;
                    self.analysis.provided.insert(name.clone(), operations);
                    true
                }
            };
            let mut scope = Scope {
                checker: self,
                definition: &name,
                bound: Vec::new(),
                required: BTreeMap::new(),
                reads: Vec::new(),
                emitted: None,
                supported: false,
            };
            scope.body(&streamed.body, 0)?;
            let Scope {
                required,
                reads,
                emitted,
                supported,
                ..
            } = scope;
            let emitted = emitted.expect("every body path ends in emit");
            let arity = self.analysis.headings[&name].len();
            if emitted.len() != arity {
                return error(
                    &streamed.body,
                    format!(
                        "emits {} variables but `{name}` has arity {arity}",
                        emitted.len()
                    ),
                );
            }
            for read in &reads {
                self.analysis
                    .readers
                    .entry(read.clone())
                    .or_default()
                    .push(name.clone());
            }
            self.analysis.required.insert(name.clone(), required);
            self.modes.insert(name.clone(), is_let);
            self.check_strategy(definition, streamed, &reads)?;
            self.check_set_valued(name, &streamed.body, &reads, supported);
        }
        self.check_usage()?;
        self.check_ix_discharged()
    }

    /// An alias stands for the host value; its capabilities are the host's,
    /// with column ordinals translated through the rename.
    fn check_alias(
        &mut self,
        name: &str,
        alias_token: &kw::alias,
        host: &Ident,
        operator: &ra::Operator,
    ) -> syn::Result<()> {
        let ra::Operator::Rename(rename) = operator else {
            return error(
                alias_token,
                "`alias` realizes only a rename of a host relation",
            );
        };
        if symbol_name(&rename.relation) != symbol_name(host) {
            return error(
                host,
                format!("`{name}` renames `{}`, not `{host}`", rename.relation),
            );
        }
        let host_name = symbol_name(host);
        let Some(host_operations) = self.analysis.provided.get(&host_name).cloned() else {
            return error(host, "alias must name an opened host relation");
        };
        let host_heading = self.analysis.headings[&host_name].clone();
        let heading = self.analysis.headings[name].clone();
        let translate = |ordinals: &[usize]| -> Vec<usize> {
            let mut translated: Vec<usize> = ordinals
                .iter()
                .map(|&column| {
                    let source = &host_heading[column];
                    let target = rename
                        .mapping
                        .iter()
                        .find(|m| symbol_name(&m.source) == *source)
                        .map(|m| symbol_name(&m.target))
                        .expect("RA checked the rename is complete");
                    heading
                        .iter()
                        .position(|h| *h == target)
                        .expect("target in heading")
                })
                .collect();
            translated.sort_unstable();
            translated
        };
        let operations = host_operations
            .iter()
            .map(|op| match op {
                Operation::Iter => Operation::Iter,
                Operation::Contains(c) => Operation::Contains(translate(c)),
                Operation::Seek(c) => Operation::Seek(translate(c)),
                Operation::Ordered(c) => Operation::Ordered(translate(c)),
            })
            .collect();
        self.analysis.provided.insert(name.to_string(), operations);
        self.analysis
            .readers
            .entry(host_name.clone())
            .or_default()
            .push(name.to_string());
        self.analysis
            .required
            .insert(name.to_string(), BTreeMap::new());
        self.analysis.aliases.insert(name.to_string(), host_name);
        self.modes.insert(name.to_string(), true);
        self.set_valued.insert(name.to_string(), true);
        Ok(())
    }

    fn check_strategy(
        &self,
        definition: &Definition,
        streamed: &Streamed,
        reads: &[String],
    ) -> syn::Result<()> {
        let Some(lazy) = &streamed.lazy else {
            return Ok(());
        };
        let name = symbol_name(&definition.name);
        let Some(ra::Operator::Semijoin(semijoin)) = self.operators.get(&name) else {
            return error(lazy, "`lazy` applies only to semijoin results");
        };
        if matches!(streamed.mode, Mode::Let { .. }) {
            return error(
                lazy,
                "a lazy semijoin is a pipe; `let` materializes eagerly",
            );
        }
        let reducer = symbol_name(&semijoin.right);
        if !reads.contains(&reducer) {
            return error(lazy, "a lazy semijoin must still test its reducer");
        }
        // The consumer must re-test the reducer so that lazily surviving
        // dangling tuples are removed: (l ⋉' r) ⋈ r = l ⋈ r.
        let consumer = self
            .program()
            .definitions
            .iter()
            .find(|d| d.body().is_some_and(|b| reads_name(b, &name)))
            .ok_or_else(|| syn::Error::new_spanned(lazy, "a lazy semijoin needs a consumer"))?;
        if !consumer.body().is_some_and(|b| reads_name(b, &reducer)) {
            return error(
                lazy,
                format!(
                    "lazy is admissible only when the consumer `{}` also reads `{reducer}`",
                    consumer.name
                ),
            );
        }
        Ok(())
    }

    /// A definition denotes a set when it applies `support`, or when it binds
    /// exactly the variables it emits over set-valued operands.
    fn check_set_valued(&mut self, name: String, body: &Body, reads: &[String], supported: bool) {
        let set_valued = supported
            || (emits_all_bound(body, 0)
                && reads
                    .iter()
                    .all(|read| self.set_valued.get(read).copied().unwrap_or(true)));
        self.set_valued.insert(name, set_valued);
    }

    fn check_usage(&mut self) -> syn::Result<()> {
        let returned = symbol_name(&self.program().return_.name);
        for definition in &self.program().definitions {
            let name = symbol_name(&definition.name);
            let mut readers = self
                .analysis
                .readers
                .get(&name)
                .cloned()
                .unwrap_or_default();
            if name == returned {
                readers.push(RETURN.to_string());
            }
            match &definition.realization {
                Realization::Stream(streamed)
                    if matches!(streamed.mode, Mode::Pipe { .. }) && readers.len() != 1 =>
                {
                    return error(
                        &definition.name,
                        format!(
                            "a pipe must be read exactly once; `{name}` is read {} times",
                            readers.len()
                        ),
                    );
                }
                _ => {}
            }
            if readers.len() == 1 {
                self.analysis.unique.insert(name, readers[0].clone());
            }
        }
        Ok(())
    }

    fn check_ix_discharged(&self) -> syn::Result<()> {
        let ix = &self.module.structures.index_requirements;
        for index_use in &ix.uses {
            let index = ix
                .indexes
                .iter()
                .find(|i| symbol_name(&i.name) == symbol_name(&index_use.index))
                .expect("IX contract validated uses");
            let (relation, columns) = crate::index_requirements::signature(&index.requirement);
            let consumer = symbol_name(&index_use.result);
            let required = Operation::Contains(columns.clone());
            let discharged = self.analysis.required[&consumer]
                .get(&relation)
                .is_some_and(|ops| ops.iter().any(|op| op.discharges(&required)));
            if !discharged {
                return error(
                    index_use,
                    format!("`{consumer}` must access `@{relation}{columns:?}` as IX demands"),
                );
            }
        }
        Ok(())
    }

    fn check_return(&self) -> syn::Result<()> {
        let output = &self
            .module
            .structures
            .index_requirements
            .relational_plan
            .plan
            .output;
        let name = symbol_name(&self.program().return_.name);
        if name != symbol_name(&output.input) {
            return error(
                &self.program().return_,
                "return must name the RA output result",
            );
        }
        if !self.set_valued.get(&name).copied().unwrap_or(false) {
            return error(
                &self.program().return_,
                "the returned result must be set-valued: apply `support` on the path to it",
            );
        }
        Ok(())
    }
}

/// Body walk for one definition: binding scope, reads, and requirements.
struct Scope<'c, 'a> {
    checker: &'c Checker<'a>,
    definition: &'c str,
    bound: Vec<String>,
    required: BTreeMap<String, BTreeSet<Operation>>,
    reads: Vec<String>,
    emitted: Option<Vec<String>>,
    supported: bool,
}

fn reads_name(body: &Body, name: &str) -> bool {
    match body {
        Body::For(f) => symbol_name(f.cursor.relation()) == name || reads_name(&f.body, name),
        Body::Co(c) => {
            c.cursors
                .iter()
                .any(|b| symbol_name(b.cursor.relation()) == name)
                || reads_name(&c.body, name)
        }
        Body::If(i) => {
            matches!(&i.guard, Guard::Contains { relation, .. } if symbol_name(relation) == name)
                || reads_name(&i.body, name)
        }
        Body::Let(l) => reads_name(&l.body, name),
        Body::Support(b) => reads_name(&b.body, name),
        Body::Unit(b) => reads_name(&b.body, name),
        Body::Emit(_) => false,
    }
}

/// True when every binding introduced on the path is emitted: no projection.
fn emits_all_bound(body: &Body, bound: usize) -> bool {
    match body {
        Body::For(f) => emits_all_bound(&f.body, bound + cq::rust::bindings(&f.pattern).len()),
        Body::Co(c) => {
            let introduced = cq::rust::bindings(&c.key).len()
                + c.cursors
                    .iter()
                    .map(|b| cq::rust::bindings(&b.pattern).len())
                    .sum::<usize>();
            emits_all_bound(&c.body, bound + introduced)
        }
        Body::If(i) => emits_all_bound(&i.body, bound),
        Body::Let(l) => emits_all_bound(&l.body, bound + cq::rust::bindings(&l.pattern).len()),
        Body::Support(b) => emits_all_bound(&b.body, bound),
        Body::Unit(b) => emits_all_bound(&b.body, bound),
        Body::Emit(e) => e.variables.len() == bound,
    }
}

impl Scope<'_, '_> {
    fn require(&mut self, relation: &Ident, operation: Operation) -> syn::Result<()> {
        let name = symbol_name(relation);
        if !self.checker.analysis.headings.contains_key(&name) {
            return error(relation, "unknown relation name");
        }
        if name == self.definition {
            return error(relation, "a definition may not read itself");
        }
        let defined_earlier = self
            .checker
            .program()
            .definitions
            .iter()
            .take_while(|d| symbol_name(&d.name) != self.definition)
            .any(|d| symbol_name(&d.name) == name);
        let is_input = !self.checker.operators.contains_key(&name);
        if !defined_earlier && !is_input {
            return error(
                relation,
                "reads must name an opened input or an earlier definition",
            );
        }
        // Capability discharge: pipes admit only one inline scan.
        let provided = match self.checker.modes.get(&name) {
            Some(false) => {
                if operation != Operation::Iter {
                    return error(relation, "a pipe result admits only `scan`");
                }
                vec![Operation::Iter]
            }
            _ => self.checker.analysis.provided[&name].clone(),
        };
        if !provided.contains(&operation) {
            return error(relation, format!("`{name}` does not provide {operation:?}"));
        }
        self.reads.push(name.clone());
        self.required.entry(name).or_default().insert(operation);
        Ok(())
    }

    fn bind(&mut self, pattern: &Pat, expected: usize) -> syn::Result<()> {
        let names = cq::rust::bindings(pattern);
        if names.len() != expected {
            return error(
                pattern,
                format!(
                    "pattern binds {} variables; {expected} expected",
                    names.len()
                ),
            );
        }
        for name in names {
            let name = symbol_name(&name);
            if self.bound.contains(&name) {
                return error(pattern, format!("`{name}` is already bound"));
            }
            self.bound.push(name);
        }
        Ok(())
    }

    fn keys(&self, relation: &Ident, columns: &Columns, keys: &Keys) -> syn::Result<Vec<usize>> {
        let arity = self.checker.arity(relation)?;
        let ordinals = columns.ordinals();
        check_columns(&ordinals, arity, columns)?;
        if keys.variables.len() != ordinals.len() {
            return error(keys, format!("{} key variables expected", ordinals.len()));
        }
        for key in &keys.variables {
            if !self.bound.contains(&symbol_name(key)) {
                return error(key, format!("key variable `{key}` is not bound"));
            }
        }
        Ok(ordinals)
    }

    /// Logical attributes of this definition's relations that a Rust
    /// expression mentions must already be bound; other names are Rust's.
    fn free_bound(&self, expr: &Expr, spanned: &impl quote::ToTokens) -> syn::Result<()> {
        let logical: BTreeSet<String> = self
            .checker
            .analysis
            .headings
            .values()
            .flatten()
            .cloned()
            .collect();
        for name in cq::rust::known_uses(expr, &logical) {
            if !self.bound.contains(&name) {
                return error(spanned, format!("`{name}` is used before it is bound"));
            }
        }
        Ok(())
    }

    fn body(&mut self, body: &Body, depth: usize) -> syn::Result<()> {
        match body {
            Body::For(f) => {
                let relation = f.cursor.relation().clone();
                let payload = match &f.cursor {
                    Cursor::Scan { within: None, .. } => {
                        self.require(&relation, Operation::Iter)?;
                        self.checker.arity(&relation)?
                    }
                    Cursor::Scan {
                        within: Some(within),
                        ..
                    } => {
                        let arity = self.checker.arity(&relation)?;
                        let column = within.column.index as usize;
                        check_columns(&[column], arity, &within.column)?;
                        for bound in [&within.low, &within.high] {
                            if let Some(expr) = bound.value() {
                                self.free_bound(expr, bound)?;
                            }
                        }
                        self.require(&relation, Operation::Ordered(vec![column]))?;
                        arity
                    }
                    Cursor::Seek {
                        relation: r,
                        columns,
                        keys,
                        ..
                    } => {
                        let ordinals = self.keys(r, columns, keys)?;
                        self.require(r, Operation::Seek(ordinals.clone()))?;
                        self.checker.arity(r)? - ordinals.len()
                    }
                    Cursor::Order { relation: r, .. } => {
                        return error(r, "`order` cursors appear only inside `co`");
                    }
                };
                self.bind(&f.pattern, payload)?;
                self.body(&f.body, depth + 1)
            }
            Body::Co(c) => {
                if c.cursors.len() < 2 {
                    return error(c, "co-iteration needs at least two cursors");
                }
                let mut key_arity = None;
                for bound in &c.cursors {
                    let Cursor::Order {
                        relation: r,
                        columns,
                        ..
                    } = &bound.cursor
                    else {
                        return error(&bound.cursor, "`co` cursors must be `order`");
                    };
                    let arity = self.checker.arity(r)?;
                    let ordinals = columns.ordinals();
                    check_columns(&ordinals, arity, columns)?;
                    match key_arity {
                        None => key_arity = Some(ordinals.len()),
                        Some(k) if k != ordinals.len() => {
                            return error(columns, "co cursors must share one key arity");
                        }
                        _ => {}
                    }
                    self.require(r, Operation::Ordered(ordinals.clone()))?;
                }
                self.bind(&c.key, key_arity.unwrap_or(0))?;
                for bound in &c.cursors {
                    let Cursor::Order {
                        relation: r,
                        columns,
                        ..
                    } = &bound.cursor
                    else {
                        unreachable!()
                    };
                    let payload = self.checker.arity(r)? - columns.ordinals().len();
                    self.bind(&bound.pattern, payload)?;
                }
                self.body(&c.body, depth + 1)
            }
            Body::If(i) => {
                match &i.guard {
                    Guard::Expr(p) => self.free_bound(&p.expr, p)?,
                    Guard::Contains {
                        relation: r,
                        columns,
                        keys,
                        ..
                    } => {
                        let ordinals = self.keys(r, columns, keys)?;
                        self.require(r, Operation::Contains(ordinals))?;
                    }
                }
                self.body(&i.body, depth + 1)
            }
            Body::Let(l) => {
                self.free_bound(&l.value, &l.value)?;
                let count = cq::rust::bindings(&l.pattern).len();
                self.bind(&l.pattern, count)?;
                self.body(&l.body, depth + 1)
            }
            Body::Support(b) => {
                self.supported = true;
                self.body(&b.body, depth + 1)
            }
            Body::Unit(b) => {
                if depth != 0 {
                    return error(&b.unit_token, "`unit` must be the whole body");
                }
                self.body(&b.body, depth + 1)
            }
            Body::Emit(e) => {
                let mut seen = BTreeSet::new();
                for variable in &e.variables {
                    let name = symbol_name(variable);
                    if !self.bound.contains(&name) {
                        return error(variable, format!("emitted variable `{name}` is not bound"));
                    }
                    if !seen.insert(name) {
                        return error(variable, "emitted variables must be distinct");
                    }
                }
                self.emitted = Some(e.variables.iter().map(symbol_name).collect());
                Ok(())
            }
        }
    }
}
