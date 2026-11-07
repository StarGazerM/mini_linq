//! Checked operational rounds over a storage refinement.
//! Every hole carries its CQ invocation queue and typed interface; the program
//! records the exact macro paths declared to implement the pull interface.
//! Execution certificates may rename one immutable scan input; erasure checks that exact
//! substitution before the ordinary round correspondence is checked.
//!
//! `ScanBinding` is a generic CQ scan input recorded by Round: its source is
//! either an ordinary delta host or a key-zero layout bucket. The default
//! `__cq_datalog_scan_*` binding denotes that complete immutable input and
//! preserves its column order. It does not select an execution strategy;
//! sequential printing uses it without any execution annotations.
//! Execution records specify admission, publication, termination, observer
//! placement, component-local key scope, loop form, and output conversion.
//! Opaque Rust providers carry the Storage laws as explicit premises.
//! CQ holes declare exactly their relational inputs in context order, including
//! borrowed completed negative key sets, followed by any scan binding and the
//! head output. Negation is a CQ `!K(args)` atom, not a Rust capture.
//! Correspondence derives these schemas and clauses independently of the fork
//! producer and rejects unused or misbound inputs.
//! Negative key holes apply constants and recorded repeated-column equalities
//! before projecting one column per variable. Rule holes therefore probe the
//! prepared host with distinct variables, without rebuilding a transformed RHS.
//! Canonical consumers have exactly one relation admission or key-set owner.
//! Each hole records typed context facts and the lowering invocation that
//! introduces them. Raw CQ entry remains fact-free; checked lower arguments
//! establish the exact envelope subsequently carried by the core pass protocol.
use cq_datalog_support::{collect_names, ensure};
pub use cq_lang_cq as cq;
use cq_lang_cq::{BodyItem, Program as CqProgram};
pub use cq_lang_datalog_storage as storage;
use cq_syntax::{RelationDecl, rust, symbol_name};
use proc_macro2::TokenStream;
use quote::{ToTokens, quote};
use std::collections::BTreeSet;
pub use storage::incremental;
use storage::{record, text::Text};
use syn::{Expr, Ident};
mod correspondence;
pub mod evidence;
mod planning;
pub use correspondence::check_storage;
pub mod schedule;
pub use schedule::Schedule;
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Relation {
    pub declaration: RelationDecl,
    pub providers: storage::Providers,
    /// Round-local NEW version of the canonical layout, with its provider type.
    pub next_canonical: Ident,
    pub next: Ident,
    pub delta: Option<Ident>,
    pub delta_host: Option<Ident>,
    /// Rule holes whose complete candidate rows share this admission set.
    pub admit: Vec<usize>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Layout {
    pub relation: usize,
    pub columns: Vec<usize>,
    pub key: usize,
    pub name: Ident,
    pub host: Ident,
    pub provider: storage::Provider,
    pub hasher: storage::Hasher,
    pub maintenance: Ident,
    pub protocol: storage::LayoutProtocol,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KeySet {
    pub relation: usize,
    pub columns: Vec<usize>,
    pub filters: Vec<(usize, Expr)>,
    pub equalities: Vec<(usize, usize)>,
    pub name: Ident,
    /// Copied from the matching Storage `Negative`, over inferred row `_`.
    pub provider: storage::Provider,
    pub hasher: storage::Hasher,
    pub hole: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InputSource {
    Delta(usize),
    Layout(usize),
}

/// A named, whole-input scan view consumed by one CQ occurrence.
/// Its source and column order are checked independently of execution policy.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScanBinding {
    pub source: InputSource,
    pub binding: Ident,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RuleHole {
    pub variant: usize,
    pub head: usize,
    pub hole: usize,
    pub scan: Option<ScanBinding>,
    /// Where this producer discharges the committed anti-join `▷R` (ADM-InPlan).
    pub known: Admission,
    /// Per positive read of the variant, in variant order: whether the read can
    /// multiply rows (DL-Fanout). FK-Known's default consumes it.
    pub fanout: Vec<Fanout>,
}

/// `filter`: the read introduces no variable (a membership test) or binds a
/// recorded key of its relation, so it keeps or drops each incoming row.
/// `multiply`: it introduces a variable (a wildcard is an enumerated fresh
/// witness) through no bound recorded key, so one row may extend to many.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fanout {
    Filter,
    Multiply,
}
impl Text for Fanout {
    fn parse_text(input: syn::parse::ParseStream<'_>) -> syn::Result<Self> {
        let name: Ident = input.parse()?;
        match name.to_string().as_str() {
            "filter" => Ok(Self::Filter),
            "multiply" => Ok(Self::Multiply),
            _ => Err(syn::Error::new_spanned(name, "expected filter or multiply")),
        }
    }
    fn text(&self) -> TokenStream {
        match self {
            Self::Filter => quote!(filter),
            Self::Multiply => quote!(multiply),
        }
    }
}

/// Names bound by one body item, in the order the item binds them.
fn item_bindings(item: &BodyItem, bound: &BTreeSet<String>) -> Vec<String> {
    match item {
        BodyItem::Positive { atom } => atom.variables().map(symbol_name).collect(),
        BodyItem::Let(binding) => rust::bindings(&binding.pattern)
            .iter()
            .map(symbol_name)
            .collect(),
        BodyItem::For(binding) => rust::bindings(&binding.pattern)
            .iter()
            .map(symbol_name)
            .collect(),
        BodyItem::Block(block) => block.analyze(bound).interface.into_keys().collect(),
        _ => Vec::new(),
    }
}

/// The level (count of positive reads, in written order) at which the rule's
/// last head variable binds, and its positive read count. A computed head term
/// binds once its free body variables have; `let`/`for`/block bindings bind at
/// the current level.
pub fn head_level(query: &cq::Query) -> (usize, usize) {
    let mut level = std::collections::BTreeMap::new();
    let mut bound = BTreeSet::new();
    let mut reads = 0;
    for item in &query.body {
        if matches!(item, BodyItem::Positive { .. }) {
            reads += 1;
        }
        for name in item_bindings(item, &bound) {
            bound.insert(name.clone());
            level.entry(name).or_insert(reads);
        }
    }
    let last = query
        .head
        .args
        .iter()
        .flat_map(|argument| match rust::variable(argument) {
            Some(variable) => std::iter::once(symbol_name(variable)).collect(),
            None => rust::free_variables(argument).dependencies(&bound),
        })
        .filter_map(|name| level.get(&name).copied())
        .max()
        .unwrap_or(0);
    (last, reads)
}

/// Formal rule DL-Fanout: classify each positive read of a variant query, in
/// written order. `keys(R)` are R's recorded keys as column positions; a key
/// is bound when each of its arguments is a bound variable or a closed term.
pub fn fanout(query: &cq::Query, keys: &dyn Fn(&Ident) -> Vec<Vec<usize>>) -> Vec<Fanout> {
    let mut bound = BTreeSet::new();
    let mut reads = Vec::new();
    for item in &query.body {
        if let BodyItem::Positive { atom } = item {
            let given = |argument: &Expr| match argument {
                Expr::Infer(_) => false,
                _ => rust::variable(argument).is_none_or(|v| bound.contains(&symbol_name(v))),
            };
            let introduces = !atom.args.iter().all(given);
            let keyed = keys(&atom.relation)
                .iter()
                .any(|key| key.iter().all(|&column| given(&atom.args[column])));
            reads.push(if introduces && !keyed {
                Fanout::Multiply
            } else {
                Fanout::Filter
            });
        }
        let names = item_bindings(item, &bound);
        bound.extend(names);
    }
    reads
}

/// Whether a read after the written head-binding level `last` (at least the
/// first read) is recorded `multiply`: the premise of FK-Known's default.
pub fn fanout_follows(fanout: &[Fanout], last: usize) -> bool {
    fanout
        .get(last.max(1)..)
        .is_some_and(|after| after.contains(&Fanout::Multiply))
}

/// `admit`: admission probes the committed canonical layout. `in_plan`: the
/// hole's CQ ends with `!C(x̄)` over the head's committed canonical layout, so
/// admission keeps only the round-local NEW insert-if-absent (DL-AdmitOnce).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Admission {
    Admit,
    InPlan,
}
impl Text for Admission {
    fn parse_text(input: syn::parse::ParseStream<'_>) -> syn::Result<Self> {
        let name: Ident = input.parse()?;
        match name.to_string().as_str() {
            "admit" => Ok(Self::Admit),
            "in_plan" => Ok(Self::InPlan),
            _ => Err(syn::Error::new_spanned(name, "expected admit or in_plan")),
        }
    }
    fn text(&self) -> TokenStream {
        match self {
            Self::Admit => quote!(admit),
            Self::InPlan => quote!(in_plan),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Stratum {
    pub members: Vec<usize>,
    pub keys: Vec<KeySet>,
    pub seeds: Vec<RuleHole>,
    pub steps: Vec<RuleHole>,
    pub phases: Vec<Ident>,
    pub publication: Vec<Publication>,
    pub termination: Vec<usize>,
    pub termination_test: Ident,
    pub observer: Ident,
    pub key_scope: Ident,
    pub loop_form: Ident,
}

/// An ordered member transaction; maintenance must finish before evaluation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Publication {
    pub relation: usize,
    pub operations: Vec<PublicationOp>,
}

/// Layout indices refer to the full or delta layout vector according to kind.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PublicationOp {
    pub kind: Ident,
    pub layouts: Vec<usize>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Output {
    pub relation: usize,
    pub conversion: Ident,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RoundContext {
    pub relations: Vec<Relation>,
    pub layouts: Vec<Layout>,
    pub delta_layouts: Vec<Layout>,
    pub strata: Vec<Stratum>,
    pub outputs: Vec<Output>,
    pub observer: Option<Expr>,
    pub observer_name: Ident,
    pub round_name: Ident,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Program {
    pub storage: storage::Program,
    pub context: RoundContext,
    pub holes: Vec<Hole>,
    pub terminals: TerminalRegistry,
    pub execution: Vec<Execution>,
    pub policy: RoundPolicy,
    /// Recorded evaluation schedule; checked by WF-DL-Schedule, changes no value.
    pub schedule: Schedule,
}
/// CQ planning facts in original positive-occurrence numbering. Normalization
/// only translates these identities to CQ clause selectors; it chooses nothing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Planning {
    pub order: Vec<usize>,
    pub authored: bool,
    pub steps: Vec<PlanningStep>,
    pub smallest: Vec<Vec<usize>>,
    /// Explicit `Known::At` level pin only; automatic admission follows CQ readiness.
    pub known: Option<usize>,
    /// Ordering inputs handed to the hole's planning invocation as `ordering(...)`
    /// (DL-PlanningOwner): Incremental delta occurrence (none for a seed), driver,
    /// complete source-body permutation and optional stratum index demand.
    pub delta: Option<usize>,
    pub driver: incremental::DriverPolicy,
    pub permutation: Vec<usize>,
    pub shared_demand: Option<Vec<SharedDemand>>,
}
/// A per-stratum index demand `Relation(column, ...)`, recorded only; it ranks nothing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SharedDemand {
    pub relation: Ident,
    pub columns: Vec<usize>,
}
impl syn::parse::Parse for SharedDemand {
    fn parse(input: syn::parse::ParseStream<'_>) -> syn::Result<Self> {
        let relation = input.parse()?;
        let columns;
        syn::parenthesized!(columns in input);
        let columns = columns
            .parse_terminated(<syn::LitInt as syn::parse::Parse>::parse, syn::Token![,])?
            .iter()
            .map(syn::LitInt::base10_parse)
            .collect::<syn::Result<_>>()?;
        Ok(Self { relation, columns })
    }
}
impl ToTokens for SharedDemand {
    fn to_tokens(&self, tokens: &mut TokenStream) {
        let relation = &self.relation;
        let columns = self.columns.iter().copied().map(syn::Index::from);
        tokens.extend(quote!(#relation(#(#columns),*)));
    }
}
impl Text for SharedDemand {
    fn parse_text(input: syn::parse::ParseStream<'_>) -> syn::Result<Self> {
        input.parse()
    }
    fn text(&self) -> TokenStream {
        self.to_token_stream()
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlanningStep {
    pub occurrence: usize,
    pub columns: Option<Vec<Ident>>,
}
#[derive(Clone, Debug)]
pub struct Hole {
    pub cq: CqProgram,
    pub pipeline: cq_stage::Pipeline,
    pub interface: HoleInterface,
    pub facts: cq_stage::Context,
    /// Invocation that introduces this hole's facts through lower arguments.
    pub lower: usize,
    /// Planning invocation owned by this hole; independent of pass spelling.
    pub plan: Option<usize>,
    pub planning: Option<Planning>,
    /// Invocation owning the complete device input binding packet.
    pub binding: Option<usize>,
    /// How each answer column reaches the round printer (DL-AnswerOwnership).
    pub answer: Vec<AnswerColumn>,
}
impl PartialEq for Hole {
    fn eq(&self, other: &Self) -> bool {
        self.cq == other.cq
            && self.interface == other.interface
            && self.facts == other.facts
            && self.lower == other.lower
            && self.plan == other.plan
            && self.planning == other.planning
            && self.binding == other.binding
            && self.answer == other.answer
            && self.pipeline.to_token_stream().to_string()
                == other.pipeline.to_token_stream().to_string()
    }
}
impl Eq for Hole {}
impl Hole {
    /// Rename a certified scan input and its exact lowering argument packet.
    pub fn rename_host_fact(&mut self, old: &Ident, new: &Ident) {
        if let Some(evidence) = &mut self.cq.declared_evidence {
            evidence.rename_relation(old, new);
        }
        if self.facts.discharged_host_sets.remove(&symbol_name(old)) {
            self.facts.discharged_host_sets.insert(symbol_name(new));
            if let Some(lower) = self.pipeline.passes.get_mut(self.lower) {
                lower.arguments = self.facts.to_token_stream();
            }
        }
    }
}

/// Representation of one hole answer column at the host round boundary.
/// A head variable defined by a positive atom is the stored column itself,
/// which a host CQ terminal yields as `&T`; any other head term is a value the
/// hole computes. Batch holes write values into their arena.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AnswerColumn {
    Borrowed,
    Owned,
}
impl syn::parse::Parse for AnswerColumn {
    fn parse(input: syn::parse::ParseStream<'_>) -> syn::Result<Self> {
        let name: Ident = input.parse()?;
        match name.to_string().as_str() {
            "borrowed" => Ok(Self::Borrowed),
            "owned" => Ok(Self::Owned),
            _ => Err(syn::Error::new_spanned(name, "expected borrowed or owned")),
        }
    }
}
impl ToTokens for AnswerColumn {
    fn to_tokens(&self, tokens: &mut TokenStream) {
        tokens.extend(match self {
            Self::Borrowed => quote!(borrowed),
            Self::Owned => quote!(owned),
        });
    }
}
impl Text for AnswerColumn {
    fn parse_text(input: syn::parse::ParseStream<'_>) -> syn::Result<Self> {
        input.parse()
    }
    fn text(&self) -> TokenStream {
        self.to_token_stream()
    }
}

/// Head columns for which a borrow exists: on a host interface, a head
/// variable whose defining binder is a positive atom occurrence of the body.
/// CQ well-formedness forbids rebinding (IF-Environment), so a variable's first
/// top-level binder in written order defines it; later atoms that mention it
/// only join, and a generator- or `let`-defined value has no stored column.
/// Batch holes write values, so none of their columns is borrowable.
pub fn borrowable_columns(cq: &CqProgram, interface: HoleInterface) -> Vec<bool> {
    let mut defined = BTreeSet::new();
    let mut stored = BTreeSet::new();
    for item in &cq.query.body {
        let (names, from_atom): (Vec<String>, bool) = match item {
            BodyItem::Positive { atom } => (atom.variables().map(symbol_name).collect(), true),
            BodyItem::Let(binding) => (
                rust::bindings(&binding.pattern)
                    .iter()
                    .map(symbol_name)
                    .collect(),
                false,
            ),
            BodyItem::For(binding) => (
                rust::bindings(&binding.pattern)
                    .iter()
                    .map(symbol_name)
                    .collect(),
                false,
            ),
            BodyItem::Block(block) => (
                block.analyze(&defined).interface.into_keys().collect(),
                false,
            ),
            _ => continue,
        };
        for name in names {
            if defined.insert(name.clone()) && from_atom {
                stored.insert(name);
            }
        }
    }
    cq.query
        .head
        .args
        .iter()
        .map(|argument| {
            interface != HoleInterface::Batch
                && rust::variable(argument).is_some_and(|v| stored.contains(&symbol_name(v)))
        })
        .collect()
}

/// Formal rule DL-AnswerOwnership: fork records each answer column's
/// representation from the hole's CQ syntax and interface, never from Rust
/// types: `borrowed` exactly where a borrow exists ([`borrowable_columns`]),
/// `owned` elsewhere. The checker admits any sound record (WF-DL-Hole).
pub fn answer_ownership(cq: &CqProgram, interface: HoleInterface) -> Vec<AnswerColumn> {
    borrowable_columns(cq, interface)
        .into_iter()
        .map(|borrowable| {
            if borrowable {
                AnswerColumn::Borrowed
            } else {
                AnswerColumn::Owned
            }
        })
        .collect()
}

/// The calling convention of an embedded CQ result.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HoleInterface {
    Pull,
    Push,
    /// Write candidates into a caller-owned batch arena.
    Batch,
}

impl syn::parse::Parse for HoleInterface {
    fn parse(input: syn::parse::ParseStream<'_>) -> syn::Result<Self> {
        let name: Ident = input.parse()?;
        match name.to_string().as_str() {
            "pull" => Ok(Self::Pull),
            "push" => Ok(Self::Push),
            "batch" => Ok(Self::Batch),
            _ => Err(syn::Error::new_spanned(
                name,
                "expected pull, push or batch interface",
            )),
        }
    }
}
impl ToTokens for HoleInterface {
    fn to_tokens(&self, tokens: &mut TokenStream) {
        tokens.extend(match self {
            Self::Pull => quote!(pull),
            Self::Push => quote!(push),
            Self::Batch => quote!(batch),
        });
    }
}
impl Text for HoleInterface {
    fn parse_text(input: syn::parse::ParseStream<'_>) -> syn::Result<Self> {
        input.parse()
    }
    fn text(&self) -> TokenStream {
        self.to_token_stream()
    }
}

/// Exact terminal paths and their trusted calling-convention assertions.
/// Declarations do not perform Rust alias resolution.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TerminalRegistry(pub Vec<Terminal>);

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Terminal {
    pub path: syn::Path,
    pub interface: HoleInterface,
}
impl syn::parse::Parse for Terminal {
    fn parse(input: syn::parse::ParseStream<'_>) -> syn::Result<Self> {
        let path = syn::Path::parse_mod_style(input)?;
        input.parse::<syn::Token![=>]>()?;
        Ok(Self {
            path,
            interface: input.parse()?,
        })
    }
}
impl ToTokens for Terminal {
    fn to_tokens(&self, tokens: &mut TokenStream) {
        let Self { path, interface } = self;
        tokens.extend(quote!(#path => #interface));
    }
}
impl Default for TerminalRegistry {
    fn default() -> Self {
        syn::parse_quote!([
            ::mini_linq::passes::translate_pull => pull,
            mini_linq::passes::translate_pull => pull,
            ::cq_datalog::passes::print_push => push
        ])
    }
}
impl syn::parse::Parse for TerminalRegistry {
    fn parse(input: syn::parse::ParseStream<'_>) -> syn::Result<Self> {
        let entries;
        syn::bracketed!(entries in input);
        let registry = Self(
            entries
                .parse_terminated(Terminal::parse, syn::Token![,])?
                .into_iter()
                .collect(),
        );
        registry.check()?;
        Ok(registry)
    }
}
impl TerminalRegistry {
    pub fn check(&self) -> syn::Result<()> {
        for (index, terminal) in self.0.iter().enumerate() {
            check_terminal_path(&terminal.path)?;
            ensure(
                !self.0[..index]
                    .iter()
                    .any(|previous| previous.path == terminal.path),
                &terminal.path,
                "duplicate terminal path",
            )?;
        }
        Ok(())
    }

    pub fn interface(&self, pipeline: &cq_stage::Pipeline) -> syn::Result<HoleInterface> {
        let terminal = pipeline.passes.last().ok_or_else(|| {
            syn::Error::new_spanned(pipeline, "hole pipeline needs a terminal printer")
        })?;
        check_terminal_path(&terminal.path)?;
        self.0
            .iter()
            .find(|entry| entry.path == terminal.path)
            .map(|entry| entry.interface)
            .ok_or_else(|| {
                syn::Error::new_spanned(
                    &terminal.path,
                    "hole terminal is missing from the recorded terminal registry",
                )
            })
    }
}
impl ToTokens for TerminalRegistry {
    fn to_tokens(&self, tokens: &mut TokenStream) {
        let entries = &self.0;
        tokens.extend(quote!([#(#entries),*]));
    }
}
impl Text for TerminalRegistry {
    fn parse_text(input: syn::parse::ParseStream<'_>) -> syn::Result<Self> {
        input.parse()
    }
    fn text(&self) -> TokenStream {
        self.to_token_stream()
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Execution {
    pub hole: usize,
    pub occurrence: usize,
    pub binding: Ident,
    pub strategy: Ident,
    /// Optional task-local candidate suppression, never an admission owner.
    /// `per_task` is an erasable realization of a sharded parallel execution.
    pub self_dedup: Option<Ident>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RoundPolicy {
    /// Formal rule DL-Timing: `phases` materializes candidates before admission
    /// so evaluation and admission have separate clocks; `none` streams them.
    /// This is a property of the timing variant, not a printer optimization.
    pub timing: Ident,
}
record!(Program { storage: storage::Program, context: RoundContext, holes: Vec<Hole>, terminals: TerminalRegistry, execution: Vec<Execution>, policy: RoundPolicy, schedule: Schedule });
record!(Hole {
    cq: CqProgram,
    pipeline: cq_stage::Pipeline,
    interface: HoleInterface,
    facts: cq_stage::Context,
    lower: usize,
    plan: Option<usize>,
    planning: Option<Planning>,
    binding: Option<usize>,
    answer: Vec<AnswerColumn>
});
record!(Planning { order: Vec<usize>, authored: bool, steps: Vec<PlanningStep>, smallest: Vec<Vec<usize>>, known: Option<usize>, delta: Option<usize>, driver: incremental::DriverPolicy, permutation: Vec<usize>, shared_demand: Option<Vec<SharedDemand>> });
record!(PlanningStep { occurrence: usize, columns: Option<Vec<Ident>> });
record!(Execution {
    hole: usize,
    occurrence: usize,
    binding: Ident,
    strategy: Ident,
    self_dedup: Option<Ident>
});
record!(RoundPolicy { timing: Ident });
record!(Relation { declaration: RelationDecl, providers: storage::Providers, next_canonical: Ident, next: Ident, delta: Option<Ident>, delta_host: Option<Ident>, admit: Vec<usize> });
record!(Layout { relation: usize, columns: Vec<usize>, key: usize, name: Ident, host: Ident, provider: storage::Provider, hasher: storage::Hasher, maintenance: Ident, protocol: storage::LayoutProtocol });
record!(KeySet { relation: usize, columns: Vec<usize>, filters: Vec<(usize,Expr)>, equalities: Vec<(usize, usize)>, name: Ident, provider: storage::Provider, hasher: storage::Hasher, hole: usize });
record!(ScanBinding {
    source: InputSource,
    binding: Ident
});
record!(RuleHole { variant: usize, head: usize, hole: usize, scan: Option<ScanBinding>, known: Admission, fanout: Vec<Fanout> });
record!(Stratum { members: Vec<usize>, keys: Vec<KeySet>, seeds: Vec<RuleHole>, steps: Vec<RuleHole>, phases: Vec<Ident>, publication: Vec<Publication>, termination: Vec<usize>, termination_test: Ident, observer: Ident, key_scope: Ident, loop_form: Ident });
record!(Publication { relation: usize, operations: Vec<PublicationOp> });
record!(PublicationOp { kind: Ident, layouts: Vec<usize> });
record!(Output {
    relation: usize,
    conversion: Ident
});
record!(RoundContext { relations: Vec<Relation>, layouts: Vec<Layout>, delta_layouts: Vec<Layout>, strata: Vec<Stratum>, outputs: Vec<Output>, observer: Option<Expr>, observer_name: Ident, round_name: Ident });
impl Text for InputSource {
    fn parse_text(input: syn::parse::ParseStream<'_>) -> syn::Result<Self> {
        let kind: Ident = input.parse()?;
        let index = usize::parse_text(input)?;
        match kind.to_string().as_str() {
            "delta" => Ok(Self::Delta(index)),
            "layout" => Ok(Self::Layout(index)),
            _ => Err(syn::Error::new_spanned(kind, "expected delta or layout")),
        }
    }
    fn text(&self) -> TokenStream {
        match self {
            Self::Delta(i) => {
                let i = i.text();
                quote!(delta #i)
            }
            Self::Layout(i) => {
                let i = i.text();
                quote!(layout #i)
            }
        }
    }
}

/// Erase certified input substitutions and optional task-local self-deduplication.
/// Self-deduplication preserves candidate sets; canonical admission remains the
/// sole owner. Neither certificate permits arbitrary CQ or context edits.
pub fn erase(program: &Program) -> syn::Result<Program> {
    let mut erased = program.clone();
    let mut used = BTreeSet::new();
    let mut names = BTreeSet::new();
    collect_names(program.storage.to_token_stream(), &mut names);
    collect_names(program.context.to_token_stream(), &mut names);
    for hole in &program.holes {
        names.insert(symbol_name(&hole.cq.name));
        // Certificate bindings are omitted here; all other declaration names
        // and Rust identifiers remain forbidden to a new runtime binding.
        let mut cq = hole.cq.clone();
        for entry in &program.execution {
            if let Some(rule) = program
                .context
                .strata
                .iter()
                .flat_map(|s| s.seeds.iter().chain(&s.steps))
                .find(|r| r.hole == entry.hole)
            {
                if program
                    .holes
                    .get(entry.hole)
                    .is_some_and(|h| h.cq.name == cq.name)
                {
                    if let Some(scan) = &rule.scan {
                        if let Some(evidence) = &mut cq.declared_evidence {
                            evidence.rename_relation(&entry.binding, &scan.binding);
                        }
                        for declaration in &mut cq.declarations {
                            if declaration.name == entry.binding {
                                declaration.name = scan.binding.clone();
                                let binding = &scan.binding;
                                declaration.value = Some(syn::parse_quote!(= (#binding)));
                            }
                        }
                        if let Some(BodyItem::Positive { atom }) =
                            cq.query.body.iter_mut().nth(entry.occurrence)
                        {
                            if atom.relation == entry.binding {
                                atom.relation = scan.binding.clone();
                            }
                        }
                    }
                }
            }
        }
        collect_names(cq.to_token_stream(), &mut names);
    }
    for entry in &program.execution {
        ensure(
            entry
                .self_dedup
                .as_ref()
                .is_none_or(|mode| mode == "per_task" && entry.strategy == "rayon_sharded"),
            entry,
            "self_dedup requires per_task on a sharded parallel execution",
        )?;
        ensure(
            used.insert(entry.hole),
            program,
            "duplicate execution certificate",
        )?;
        ensure(
            entry.occurrence == 0,
            program,
            "execution input must be the first occurrence",
        )?;
        ensure(
            names.insert(symbol_name(&entry.binding)),
            program,
            "execution binding must be fresh and distinct",
        )?;
        let rule = program
            .context
            .strata
            .iter()
            .flat_map(|s| s.seeds.iter().chain(&s.steps))
            .find(|r| r.hole == entry.hole)
            .ok_or_else(|| {
                syn::Error::new_spanned(program, "execution certificate requires a rule hole")
            })?;
        let scan = rule.scan.as_ref().ok_or_else(|| {
            syn::Error::new_spanned(program, "execution requires an immutable scan")
        })?;
        let hole = erased
            .holes
            .get_mut(entry.hole)
            .ok_or_else(|| syn::Error::new_spanned(program, "missing execution hole"))?;
        let count = hole
            .cq
            .declarations
            .iter()
            .filter(|d| d.name == entry.binding)
            .count();
        ensure(
            count == 1,
            program,
            "execution binding declaration must occur exactly once",
        )?;
        let declaration = hole
            .cq
            .declarations
            .iter_mut()
            .find(|d| d.name == entry.binding)
            .expect("checked declaration");
        let binding = &entry.binding;
        ensure(
            declaration.value == Some(syn::parse_quote!(= (#binding))),
            program,
            "execution host must bind directly",
        )?;
        declaration.name = scan.binding.clone();
        let binding = &scan.binding;
        declaration.value = Some(syn::parse_quote!(= (#binding)));
        let Some(BodyItem::Positive { atom }) = hole.cq.query.body.iter_mut().nth(entry.occurrence)
        else {
            return Err(syn::Error::new_spanned(
                program,
                "execution input must be positive",
            ));
        };
        ensure(
            atom.relation == entry.binding,
            program,
            "execution certificate does not name its occurrence",
        )?;
        atom.relation = scan.binding.clone();
        hole.rename_host_fact(&entry.binding, &scan.binding);
    }
    erased.execution.clear();
    Ok(erased)
}
/// Formal rule WF-DL-Hole: the terminal assertion matches the hole interface,
/// and the recorded answer ownership is sound: one record per head column,
/// `borrowed` only where a borrow exists ([`borrowable_columns`]); `owned`
/// (a clone) is always admitted.
pub fn check_pipeline(hole: &Hole, terminals: &TerminalRegistry) -> syn::Result<()> {
    ensure(
        terminals.interface(&hole.pipeline)? == hole.interface,
        hole,
        "hole interface does not match its registered terminal interface",
    )?;
    let borrowable = borrowable_columns(&hole.cq, hole.interface);
    ensure(
        hole.answer.len() == borrowable.len()
            && hole
                .answer
                .iter()
                .zip(&borrowable)
                .all(|(column, &borrowable)| *column == AnswerColumn::Owned || borrowable),
        hole,
        "hole answer ownership claims a borrow where no positive atom defines the column on a host interface",
    )?;
    ensure(
        hole.plan
            .is_none_or(|plan| plan < hole.lower && plan < hole.pipeline.passes.len()),
        hole,
        "hole planning invocation must precede lowering",
    )?;
    hole.invocation_pipeline()?;
    ensure(
        hole.binding.is_none_or(|binding| {
            binding > hole.lower && binding < hole.pipeline.passes.len().saturating_sub(1)
        }),
        hole,
        "hole binding invocation must follow lowering and precede the terminal",
    )?;
    let reads: BTreeSet<_> = hole
        .cq
        .query
        .body
        .iter()
        .filter_map(|item| match item {
            BodyItem::Positive { atom } => Some(symbol_name(&atom.relation)),
            _ => None,
        })
        .collect();
    ensure(
        hole.facts.discharged_host_sets.is_subset(&reads),
        hole,
        "discharged fact names must be actual positive hole inputs",
    )?;
    let lower = hole
        .pipeline
        .passes
        .get(hole.lower)
        .ok_or_else(|| syn::Error::new_spanned(hole, "hole lowering invocation is missing"))?;
    let packet: cq_stage::Context = syn::parse2(lower.arguments.clone())?;
    ensure(
        packet == hole.facts,
        hole,
        "lower argument packet must exactly match the hole facts",
    )
}

fn check_terminal_path(path: &syn::Path) -> syn::Result<()> {
    ensure(
        path.segments
            .iter()
            .all(|s| matches!(s.arguments, syn::PathArguments::None)),
        path,
        "terminal declarations require macro paths without generic arguments",
    )
}
/// Check execution obligations, not equality with the default fork schedule.
fn check_execution(program: &Program) -> syn::Result<()> {
    let context = &program.context;
    for stratum in &context.strata {
        ensure(
            stratum
                .phases
                .iter()
                .map(symbol_name)
                .eq(["evaluate", "admit", "publish"].map(str::to_owned))
                && stratum.observer == "before_step"
                && stratum.key_scope == "component"
                && (stratum.termination_test == "sum_delta_lengths"
                    || stratum.termination_test == "any_delta")
                && (stratum.loop_form == "while_nonempty"
                    || stratum.loop_form == "loop_until_empty"),
            program,
            "round requires evaluate/admit/publish, before-step observation, and component-local keys",
        )?;
        let members: BTreeSet<_> = stratum.members.iter().copied().collect();
        let termination: BTreeSet<_> = stratum.termination.iter().copied().collect();
        ensure(
            termination == members && termination.len() == stratum.termination.len(),
            program,
            "termination must cover every component member exactly once",
        )?;
        let mut published = BTreeSet::new();
        for publication in &stratum.publication {
            let relation = publication.relation;
            ensure(
                members.contains(&relation) && published.insert(relation),
                program,
                "publication must cover each component member exactly once",
            )?;
            let full: BTreeSet<_> = context
                .layouts
                .iter()
                .enumerate()
                .filter(|(_, layout)| layout.relation == relation)
                .map(|(index, _)| index)
                .collect();
            let delta: BTreeSet<_> = context
                .delta_layouts
                .iter()
                .enumerate()
                .filter(|(_, layout)| layout.relation == relation)
                .map(|(index, _)| index)
                .collect();
            let mut updated = BTreeSet::new();
            let mut rebuilt = BTreeSet::new();
            let mut swapped = false;
            for operation in &publication.operations {
                match symbol_name(&operation.kind).as_str() {
                    "full_layouts" => {
                        ensure(
                            !swapped && !operation.layouts.is_empty(),
                            program,
                            "full-layout maintenance must precede delta replacement",
                        )?;
                        for &layout in &operation.layouts {
                            ensure(
                                full.contains(&layout) && updated.insert(layout),
                                program,
                                "full-layout maintenance must cover each member layout exactly once",
                            )?;
                        }
                    }
                    "delta_swap" => {
                        ensure(
                            !swapped && updated == full && operation.layouts.is_empty(),
                            program,
                            "delta replacement requires complete full-layout maintenance",
                        )?;
                        swapped = true;
                    }
                    "delta_rebuild" => {
                        ensure(
                            swapped && !operation.layouts.is_empty(),
                            program,
                            "delta layouts must rebuild after unconditional delta replacement",
                        )?;
                        for &layout in &operation.layouts {
                            ensure(
                                delta.contains(&layout) && rebuilt.insert(layout),
                                program,
                                "delta rebuild must cover each member layout exactly once, including empty runs",
                            )?;
                        }
                    }
                    _ => {
                        return Err(syn::Error::new_spanned(
                            operation,
                            "unknown publication operation",
                        ));
                    }
                }
            }
            ensure(
                swapped && updated == full && rebuilt == delta,
                program,
                "publication must maintain every layout, including the canonical one, and replace delta",
            )?;
        }
        ensure(
            published == members,
            program,
            "publication omitted a component member",
        )?;
    }
    for output in &context.outputs {
        let relation = context.relations.get(output.relation).ok_or_else(|| {
            syn::Error::new_spanned(output, "output must reference a declared relation")
        })?;
        // Moving a canonical grouped map or set yields its owned all-column
        // keys; bulk providers expose borrowed rows, which are cloned once.
        let canonical = context.layouts.get(relation.providers.canonical);
        ensure(
            canonical.is_some_and(|layout| {
                if layout.protocol == storage::LayoutProtocol::Bulk {
                    output.conversion == "borrowed_cloned_into_iter"
                } else {
                    output.conversion == "owned_into_iter"
                }
            }),
            output,
            "output conversion must match its canonical layout and return owned rows",
        )?;
    }
    Ok(())
}
/// Formal rule DL-AdmitOnce: every producer of `R` discharges `▷R` exactly once.
/// An `in_plan` rule hole reads `R`'s committed canonical layout negatively
/// exactly once, as its last clause (so admission omits the committed probe);
/// an `admit` hole never reads it negatively (admission keeps the probe). Both
/// remain admission-owned by `R`, whose round-local NEW insert-if-absent (the
/// residual δ) runs in the `admit` phase for every candidate.
fn check_admit_once(program: &Program) -> syn::Result<()> {
    let context = &program.context;
    for rule in context
        .strata
        .iter()
        .flat_map(|s| s.seeds.iter().chain(&s.steps))
    {
        let relation = context
            .relations
            .get(rule.head)
            .ok_or_else(|| syn::Error::new_spanned(rule, "rule hole names no head relation"))?;
        let canonical = context
            .layouts
            .get(relation.providers.canonical)
            .ok_or_else(|| syn::Error::new_spanned(relation, "missing canonical layout"))?;
        let hole = program
            .holes
            .get(rule.hole)
            .ok_or_else(|| syn::Error::new_spanned(rule, "rule hole is missing"))?;
        let body = &hole.cq.query.body;
        let reads = body
            .iter()
            .filter(|item| {
                matches!(item, BodyItem::Negation(negative) if negative.atom.relation == canonical.name)
            })
            .count();
        let last = matches!(
            body.iter().last(),
            Some(BodyItem::Negation(negative)) if negative.atom.relation == canonical.name
        );
        ensure(
            relation.admit.contains(&rule.hole)
                && match rule.known {
                    Admission::InPlan => reads == 1 && last,
                    Admission::Admit => reads == 0,
                },
            rule,
            "every producer must discharge the committed anti-join exactly once, in its plan or at admission",
        )?;
    }
    Ok(())
}
/// Formal rule DL-Admit-Owner: every candidate has exactly one matching set consumer.
fn check_facts(program: &Program) -> syn::Result<()> {
    let context = &program.context;
    let certified = |kind: &str, index: usize| {
        program
            .storage
            .certificates
            .iter()
            .any(|c| c.kind == kind && c.index == index)
    };
    let mut owners = vec![0usize; program.holes.len()];
    for (index, relation) in context.relations.iter().enumerate() {
        let expected: BTreeSet<_> = context
            .strata
            .iter()
            .flat_map(|s| s.seeds.iter().chain(&s.steps))
            .filter(|rule| rule.head == index)
            .map(|rule| rule.hole)
            .collect();
        let actual: BTreeSet<_> = relation.admit.iter().copied().collect();
        ensure(
            actual == expected && actual.len() == relation.admit.len(),
            relation,
            "admission owner must list every matching rule variant exactly once",
        )?;
        for &hole in &relation.admit {
            let owner = owners.get_mut(hole).ok_or_else(|| {
                syn::Error::new_spanned(relation, "admission owner references missing hole")
            })?;
            *owner += 1;
        }
    }
    for key in context.strata.iter().flat_map(|s| &s.keys) {
        let owner = owners
            .get_mut(key.hole)
            .ok_or_else(|| syn::Error::new_spanned(key, "key owner references missing hole"))?;
        *owner += 1;
    }
    for (index, hole) in program.holes.iter().enumerate() {
        ensure(
            owners[index] == 1 && hole.facts.consumer_canonical,
            hole,
            "consumer canonical requires exactly one matching admission or key-set owner",
        )?;
        let mut expected = BTreeSet::new();
        for item in &hole.cq.query.body {
            let BodyItem::Positive { atom } = item else {
                continue;
            };
            let name = &atom.relation;
            let relation_host = context
                .relations
                .iter()
                .enumerate()
                .any(|(i, r)| r.delta.as_ref() == Some(name) && certified("delta", i));
            let layout_host = context
                .layouts
                .iter()
                .enumerate()
                .any(|(i, l)| &l.name == name && certified("layout", i))
                || context
                    .delta_layouts
                    .iter()
                    .enumerate()
                    .any(|(i, l)| &l.name == name && certified("delta_layout", i));
            let scan_host = context
                .strata
                .iter()
                .flat_map(|s| s.seeds.iter().chain(&s.steps))
                .filter(|rule| rule.hole == index)
                .filter_map(|rule| rule.scan.as_ref())
                .any(|scan| {
                    &scan.binding == name
                        && match scan.source {
                            InputSource::Delta(i) => certified("delta", i),
                            InputSource::Layout(i) => certified("layout", i),
                        }
                });
            if relation_host || layout_host || scan_host {
                expected.insert(symbol_name(name));
            }
        }
        ensure(
            hole.facts.discharged_host_sets == expected,
            hole,
            "discharged inputs must exactly name the certified hosts actually read",
        )?;
    }
    Ok(())
}

pub mod contract {
    use super::*;
    #[derive(Clone, Debug)]
    pub struct Analysis {
        pub storage: storage::Analysis,
    }
    /// Formal rule WF-DL-Round: checked holes and complete publication protocol.
    pub fn check(program: &Program) -> syn::Result<Analysis> {
        let analysis = storage::contract::check(&program.storage)?;
        let p = &program.policy;
        ensure(
            p.timing == "none" || p.timing == "phases",
            program,
            "unsupported round timing policy",
        )?;
        program.terminals.check()?;
        for hole in &program.holes {
            check_pipeline(hole, &program.terminals)?;
            cq_lang_cq::contract::check(&hole.cq)?;
        }
        let erased = erase(program)?;
        correspondence::check_storage_with_analysis(&program.storage, &erased, &analysis)?;
        check_facts(&erased)?;
        check_admit_once(&erased)?;
        check_execution(program)?;
        schedule::check(program, &analysis)?;
        evidence::check_iteration_scope(program)?;
        Ok(Analysis { storage: analysis })
    }
}
/// Formal rule CK-DL-RoundRewrite: preserve holes and their terminal registry.
pub fn check_rewrite(source: &Program, target: &Program) -> syn::Result<()> {
    contract::check(source)?;
    contract::check(target)?;
    check_rewrite_admitted(source, target)
}
// The stage runner has already checked both language contracts.
fn check_rewrite_admitted(source: &Program, target: &Program) -> syn::Result<()> {
    let mut source = erase(source)?;
    let erased = erase(target)?;
    ensure(
        source.storage == erased.storage,
        target,
        "round rewrite changed storage",
    )?;
    // Both contexts independently establish the same Storage correspondence.
    // Execution decisions and registered terminal interfaces may differ;
    // names, canonical owners, facts, and the terminal registry remain fixed.
    for (before, after) in source.context.strata.iter_mut().zip(&erased.context.strata) {
        before.phases = after.phases.clone();
        before.publication = after.publication.clone();
        before.termination = after.termination.clone();
        before.termination_test = after.termination_test.clone();
        before.observer = after.observer.clone();
        before.key_scope = after.key_scope.clone();
        before.loop_form = after.loop_form.clone();
    }
    source.context.outputs = erased.context.outputs.clone();
    source.policy.timing = erased.policy.timing.clone();
    // The schedule records only admissible evaluation order (CK-DL-Schedule).
    source.schedule = erased.schedule.clone();
    for (before, after) in source.holes.iter_mut().zip(&erased.holes) {
        // Planning facts stay fixed even when invocation ordinals move.
        before.pipeline = after.pipeline.clone();
        before.lower = after.lower;
        before.plan = after.plan;
        before.binding = after.binding;
        before.interface = after.interface;
        // Answer ownership follows the interface and is rechecked by WF-DL-Hole.
        before.answer = after.answer.clone();
    }
    ensure(
        source == erased,
        target,
        "round rewrite changed a hole beyond its input certificate",
    )
}
/// The rewrite callback assumes `cq_stage::run` checked both contracts.
/// Standalone callers must use the public free [`check_rewrite`] function.
impl cq_stage::Language for Program {
    fn check(&self) -> syn::Result<()> {
        contract::check(self).map(|_| ())
    }
    fn check_rewrite(source: &Self, target: &Self) -> syn::Result<()> {
        check_rewrite_admitted(source, target)
    }
}
