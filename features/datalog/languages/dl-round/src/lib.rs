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
//! CQ holes declare exactly their positive inputs and head output, in context
//! order, followed by any scan binding and the output. Negative keys are Rust
//! predicate captures, not CQ inputs. Correspondence derives this schema
//! independently of the fork producer and rejects unused or misbound inputs.
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
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KeySet {
    pub relation: usize,
    pub columns: Vec<usize>,
    pub filters: Vec<(usize, Expr)>,
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
    pub pull_printers: PullPrinters,
    pub execution: Vec<Execution>,
    pub policy: RoundPolicy,
}
#[derive(Clone, Debug)]
pub struct Hole {
    pub cq: CqProgram,
    pub pipeline: cq_stage::Pipeline,
    pub interface: HoleInterface,
}
impl PartialEq for Hole {
    fn eq(&self, other: &Self) -> bool {
        self.cq == other.cq
            && self.interface == other.interface
            && self.pipeline.to_token_stream().to_string()
                == other.pipeline.to_token_stream().to_string()
    }
}
impl Eq for Hole {}

/// The calling convention of an embedded CQ result.
/// Push is represented for diagnostics, but no Round consumer supports it.
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

/// Exact paths explicitly declared to return pull iterators.
/// Declarations are trusted interface assertions, not Rust alias resolution.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PullPrinters(pub Vec<syn::Path>);

impl Default for PullPrinters {
    fn default() -> Self {
        Self(vec![
            syn::parse_quote!(::mini_linq::passes::print_pull),
            syn::parse_quote!(mini_linq::passes::print_pull),
        ])
    }
}
impl syn::parse::Parse for PullPrinters {
    fn parse(input: syn::parse::ParseStream<'_>) -> syn::Result<Self> {
        let paths;
        syn::bracketed!(paths in input);
        Ok(Self(
            paths
                .parse_terminated(syn::Path::parse_mod_style, syn::Token![,])?
                .into_iter()
                .collect(),
        ))
    }
}
impl ToTokens for PullPrinters {
    fn to_tokens(&self, tokens: &mut TokenStream) {
        let paths = &self.0;
        tokens.extend(quote!([#(#paths),*]));
    }
}
impl Text for PullPrinters {
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
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RoundPolicy {
    pub timing: Ident,
}
record!(Program { storage: storage::Program, context: RoundContext, holes: Vec<Hole>, pull_printers: PullPrinters, execution: Vec<Execution>, policy: RoundPolicy });
record!(Hole {
    cq: CqProgram,
    pipeline: cq_stage::Pipeline,
    interface: HoleInterface
});
record!(Execution {
    hole: usize,
    occurrence: usize,
    binding: Ident,
    strategy: Ident
});
record!(RoundPolicy { timing: Ident });
record!(Relation { declaration: RelationDecl, providers: storage::Providers, all: Ident, seen: Ident, next: Ident, delta: Option<Ident>, delta_host: Option<Ident> });
record!(Layout { relation: usize, columns: Vec<usize>, key: usize, name: Ident, host: Ident, provider: syn::Type, maintenance: Ident });
record!(KeySet { relation: usize, columns: Vec<usize>, filters: Vec<(usize,Expr)>, name: Ident, representation: KeyRepresentation, provider: syn::Type, hole: usize });
record!(ScanBinding {
    source: InputSource,
    binding: Ident
});
record!(RuleHole { variant: usize, head: usize, hole: usize, scan: Option<ScanBinding>, insertion: Ident });
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
impl Text for KeyRepresentation {
    fn parse_text(input: syn::parse::ParseStream<'_>) -> syn::Result<Self> {
        let id: Ident = input.parse()?;
        if id == "borrowed" {
            Ok(Self::Borrowed)
        } else {
            Err(syn::Error::new_spanned(
                id,
                "unsupported key representation",
            ))
        }
    }
    fn text(&self) -> TokenStream {
        quote!(borrowed)
    }
}

/// Erase only certified input substitutions. This never erases arbitrary CQ edits.
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
    }
    erased.execution.clear();
    Ok(erased)
}
pub fn check_pipeline(hole: &Hole, printers: &PullPrinters) -> syn::Result<()> {
    ensure(
        hole.interface == HoleInterface::Pull,
        hole,
        "push hole interface is unsupported; Round requires pull iterators",
    )?;
    let terminal =
        hole.pipeline.passes.last().ok_or_else(|| {
            syn::Error::new_spanned(hole, "hole pipeline needs a terminal printer")
        })?;
    check_pull_path(&terminal.path)?;
    ensure(
        printers.0.contains(&terminal.path) && terminal.arguments.is_empty(),
        hole,
        "pull hole requires an argument-free terminal in the recorded pull_printers allowlist",
    )
}

fn check_pull_path(path: &syn::Path) -> syn::Result<()> {
    ensure(
        path.segments
            .iter()
            .all(|s| matches!(s.arguments, syn::PathArguments::None)),
        path,
        "pull printer declarations require macro paths without generic arguments",
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
        for path in &program.pull_printers.0 {
            check_pull_path(path)?;
        }
        for hole in &program.holes {
            check_pipeline(hole, &program.pull_printers)?;
            cq_lang_cq::contract::check(&hole.cq)?;
        }
        let erased = erase(program)?;
        correspondence::check_storage_with_analysis(&program.storage, &erased, &analysis)?;
        check_execution(program)?;
        Ok(Analysis { storage: analysis })
    }
}
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
    // Only explicit execution decisions may differ; names and hole interfaces
    // remain part of the immutable calling convention.
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
