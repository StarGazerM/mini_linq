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
//! head output. CPU negation is a CQ `!K(args)` atom, not a Rust capture;
//! device-owned keys retain an ordinary opaque membership guard. Correspondence
//! derives these schemas and clauses independently of the fork producer and
//! rejects unused or misbound inputs.
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
pub use correspondence::{check_storage, scan_source, use_layouts, use_scan};
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Relation {
    pub declaration: RelationDecl,
    pub providers: storage::Providers,
    pub all: Ident,
    pub seen: Ident,
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
    pub provider: syn::Type,
    pub maintenance: Ident,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyRepresentation {
    Borrowed,
    DeviceSetOwned,
}

/// Concrete index lifetimes selected by the checked storage policy.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LayoutRealization {
    MaintainedGroupedHost,
    MaintainedFrozenDeviceIndex,
    MaintainedLeveledDeviceIndex,
}
impl KeyRepresentation {
    pub fn layouts(self) -> LayoutRealization {
        match self {
            Self::Borrowed => LayoutRealization::MaintainedGroupedHost,
            Self::DeviceSetOwned => LayoutRealization::MaintainedFrozenDeviceIndex,
        }
    }
    pub fn changing_layouts(self) -> LayoutRealization {
        match self {
            Self::Borrowed => LayoutRealization::MaintainedGroupedHost,
            Self::DeviceSetOwned => LayoutRealization::MaintainedLeveledDeviceIndex,
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KeySet {
    pub relation: usize,
    pub columns: Vec<usize>,
    pub filters: Vec<(usize, Expr)>,
    pub equalities: Vec<(usize, usize)>,
    pub name: Ident,
    pub representation: KeyRepresentation,
    pub provider: syn::Type,
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
    pub insertion: Ident,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Stratum {
    pub members: Vec<usize>,
    pub keys: Vec<KeySet>,
    /// Exact keyed full reads of completed inputs, built before this component.
    pub frozen_layouts: Vec<usize>,
    /// Exact keyed full reads of changing members, maintained each round.
    pub leveled_layouts: Vec<usize>,
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
    pub representation: KeyRepresentation,
    pub layout_realization: LayoutRealization,
    pub changing_layout_realization: LayoutRealization,
    pub leveled_ratio: u32,
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
}
#[derive(Clone, Debug)]
pub struct Hole {
    pub cq: CqProgram,
    pub pipeline: cq_stage::Pipeline,
    pub interface: HoleInterface,
    pub facts: cq_stage::Context,
    /// Invocation that introduces this hole's facts through lower arguments.
    pub lower: usize,
}
impl PartialEq for Hole {
    fn eq(&self, other: &Self) -> bool {
        self.cq == other.cq
            && self.interface == other.interface
            && self.facts == other.facts
            && self.lower == other.lower
            && self.pipeline.to_token_stream().to_string()
                == other.pipeline.to_token_stream().to_string()
    }
}
impl Eq for Hole {}
impl Hole {
    /// Rename a certified scan input and its exact lowering argument packet.
    pub fn rename_host_fact(&mut self, old: &Ident, new: &Ident) {
        if self.facts.discharged_host_sets.remove(&symbol_name(old)) {
            self.facts.discharged_host_sets.insert(symbol_name(new));
            if let Some(lower) = self.pipeline.passes.get_mut(self.lower) {
                lower.arguments = self.facts.to_token_stream();
            }
        }
    }
}

/// The calling convention of an embedded CQ result.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HoleInterface {
    Pull,
    Push,
}

impl syn::parse::Parse for HoleInterface {
    fn parse(input: syn::parse::ParseStream<'_>) -> syn::Result<Self> {
        let name: Ident = input.parse()?;
        match name.to_string().as_str() {
            "pull" => Ok(Self::Pull),
            "push" => Ok(Self::Push),
            _ => Err(syn::Error::new_spanned(
                name,
                "expected pull or push interface",
            )),
        }
    }
}
impl ToTokens for HoleInterface {
    fn to_tokens(&self, tokens: &mut TokenStream) {
        tokens.extend(match self {
            Self::Pull => quote!(pull),
            Self::Push => quote!(push),
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
    pub timing: Ident,
}
record!(Program { storage: storage::Program, context: RoundContext, holes: Vec<Hole>, terminals: TerminalRegistry, execution: Vec<Execution>, policy: RoundPolicy });
record!(Hole {
    cq: CqProgram,
    pipeline: cq_stage::Pipeline,
    interface: HoleInterface,
    facts: cq_stage::Context,
    lower: usize
});
record!(Execution {
    hole: usize,
    occurrence: usize,
    binding: Ident,
    strategy: Ident,
    self_dedup: Option<Ident>
});
record!(RoundPolicy { timing: Ident });
record!(Relation { declaration: RelationDecl, providers: storage::Providers, all: Ident, seen: Ident, next: Ident, delta: Option<Ident>, delta_host: Option<Ident>, admit: Vec<usize> });
record!(Layout { relation: usize, columns: Vec<usize>, key: usize, name: Ident, host: Ident, provider: syn::Type, maintenance: Ident });
record!(KeySet { relation: usize, columns: Vec<usize>, filters: Vec<(usize,Expr)>, equalities: Vec<(usize, usize)>, name: Ident, representation: KeyRepresentation, provider: syn::Type, hole: usize });
record!(ScanBinding {
    source: InputSource,
    binding: Ident
});
record!(RuleHole { variant: usize, head: usize, hole: usize, scan: Option<ScanBinding>, insertion: Ident });
record!(Stratum { members: Vec<usize>, keys: Vec<KeySet>, frozen_layouts: Vec<usize>, leveled_layouts: Vec<usize>, seeds: Vec<RuleHole>, steps: Vec<RuleHole>, phases: Vec<Ident>, publication: Vec<Publication>, termination: Vec<usize>, termination_test: Ident, observer: Ident, key_scope: Ident, loop_form: Ident });
record!(Publication { relation: usize, operations: Vec<PublicationOp> });
record!(PublicationOp { kind: Ident, layouts: Vec<usize> });
record!(Output {
    relation: usize,
    conversion: Ident
});
record!(RoundContext { representation: KeyRepresentation, layout_realization: LayoutRealization, changing_layout_realization: LayoutRealization, leveled_ratio: u32, relations: Vec<Relation>, layouts: Vec<Layout>, delta_layouts: Vec<Layout>, strata: Vec<Stratum>, outputs: Vec<Output>, observer: Option<Expr>, observer_name: Ident, round_name: Ident });
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
impl Text for KeyRepresentation {
    fn parse_text(input: syn::parse::ParseStream<'_>) -> syn::Result<Self> {
        let id: Ident = input.parse()?;
        if id == "borrowed" {
            Ok(Self::Borrowed)
        } else if id == "device_set_owned" {
            Ok(Self::DeviceSetOwned)
        } else {
            Err(syn::Error::new_spanned(
                id,
                "unsupported key representation",
            ))
        }
    }
    fn text(&self) -> TokenStream {
        match self {
            Self::Borrowed => quote!(borrowed),
            Self::DeviceSetOwned => quote!(device_set_owned),
        }
    }
}
impl Text for LayoutRealization {
    fn parse_text(input: syn::parse::ParseStream<'_>) -> syn::Result<Self> {
        let id: Ident = input.parse()?;
        match id.to_string().as_str() {
            "grouped_host" => Ok(Self::MaintainedGroupedHost),
            "frozen_device_index" => Ok(Self::MaintainedFrozenDeviceIndex),
            "leveled_device_index" => Ok(Self::MaintainedLeveledDeviceIndex),
            _ => Err(syn::Error::new_spanned(
                id,
                "unsupported layout realization",
            )),
        }
    }
    fn text(&self) -> TokenStream {
        match self {
            Self::MaintainedGroupedHost => quote!(grouped_host),
            Self::MaintainedFrozenDeviceIndex => quote!(frozen_device_index),
            Self::MaintainedLeveledDeviceIndex => quote!(leveled_device_index),
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
/// Formal rule WF-DL-Hole: the terminal assertion matches the hole interface.
pub fn check_pipeline(hole: &Hole, terminals: &TerminalRegistry) -> syn::Result<()> {
    ensure(
        terminals.interface(&hole.pipeline)? == hole.interface,
        hole,
        "hole interface does not match its registered terminal interface",
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
        for rule in stratum.seeds.iter().chain(&stratum.steps) {
            ensure(
                rule.insertion == "insert",
                program,
                "admission requires the provider's set insertion operation",
            )?;
        }
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
            let mut extended = false;
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
                    "all_extend" => {
                        ensure(
                            !extended && !swapped && operation.layouts.is_empty(),
                            program,
                            "all extension must occur once before delta replacement",
                        )?;
                        extended = true;
                    }
                    "delta_swap" => {
                        ensure(
                            extended && !swapped && updated == full && operation.layouts.is_empty(),
                            program,
                            "delta replacement requires complete full-layout maintenance and all extension",
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
                extended && swapped && updated == full && rebuilt == delta,
                program,
                "publication must extend all, replace delta, and maintain every layout including empty runs",
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
        ensure(
            if relation.declaration.value.is_some() {
                output.conversion == "borrowed_cloned_into_iter"
            } else {
                output.conversion == "owned_into_iter"
            },
            output,
            "output conversion must preserve ownership and return owned rows",
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
            let relation_host = context.relations.iter().enumerate().any(|(i, r)| {
                (&r.declaration.name == name && certified("all", i))
                    || (r.delta.as_ref() == Some(name) && certified("delta", i))
            });
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
        check_execution(program)?;
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
        for (before, after) in before
            .seeds
            .iter_mut()
            .chain(&mut before.steps)
            .zip(after.seeds.iter().chain(&after.steps))
        {
            before.insertion = after.insertion.clone();
        }
        for (before, after) in before.keys.iter_mut().zip(&after.keys) {
            before.provider = after.provider.clone();
        }
    }
    source.context.outputs = erased.context.outputs.clone();
    source.policy.timing = erased.policy.timing.clone();
    for (before, after) in source.holes.iter_mut().zip(&erased.holes) {
        before.pipeline = after.pipeline.clone();
        before.lower = after.lower;
        before.interface = after.interface;
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
