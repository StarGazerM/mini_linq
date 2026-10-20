//! Checked operational rounds over a storage refinement.
//! Every hole carries its CQ invocation queue and pull interface. Execution
//! certificates may rename one immutable scan input; erasure checks that exact
//! substitution before the ordinary round correspondence is checked.
pub use cq_lang_datalog_storage as storage;
pub use storage::incremental;
pub use cq_lang_cq as cq;
use cq_lang_cq::{BodyItem, Program as CqProgram};
use cq_syntax::{RelationDecl, rust, symbol_name};
use proc_macro2::{TokenStream,TokenTree};
use quote::{ToTokens,quote};
use std::collections::BTreeSet;
use syn::{Expr,Ident};
use storage::{record, text::Text};
mod correspondence;
pub use correspondence::{check_storage, inputs, use_layouts, scan_source, use_scan};
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Relation {
    pub declaration: RelationDecl,
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
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyRepresentation {
    HashSetBorrowed,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KeySet {
    pub relation: usize,
    pub columns: Vec<usize>,
    pub filters: Vec<(usize, Expr)>,
    pub name: Ident,
    pub representation: KeyRepresentation,
    pub hole: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InputSource {
    Delta(usize),
    Layout(usize),
}

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
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Stratum {
    pub members: Vec<usize>,
    pub keys: Vec<KeySet>,
    pub seeds: Vec<RuleHole>,
    pub steps: Vec<RuleHole>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RoundContext {
    pub relations: Vec<Relation>,
    pub layouts: Vec<Layout>,
    pub delta_layouts: Vec<Layout>,
    pub strata: Vec<Stratum>,
    pub outputs: Vec<usize>,
    pub observer: Option<Expr>,
    pub observer_name: Ident,
    pub round_name: Ident,
}


#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Program {
    pub storage: storage::Program,
    pub context: RoundContext,
    pub holes: Vec<Hole>,
    pub execution: Vec<Execution>,
    pub policy: RoundPolicy,
}
#[derive(Clone, Debug)]
pub struct Hole { pub cq: CqProgram, pub pipeline: cq_stage::Pipeline, pub interface: Ident }
impl PartialEq for Hole {
    fn eq(&self, other: &Self) -> bool { self.cq == other.cq && self.interface == other.interface && self.pipeline.to_token_stream().to_string() == other.pipeline.to_token_stream().to_string() }
}
impl Eq for Hole {}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Execution { pub hole: usize, pub occurrence: usize, pub binding: Ident, pub strategy: Ident }
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RoundPolicy { pub phases: Ident, pub admission: Ident, pub termination: Ident, pub observer: Ident, pub outputs: Ident, pub timing: Ident }
record!(Program { storage: storage::Program, context: RoundContext, holes: Vec<Hole>, execution: Vec<Execution>, policy: RoundPolicy });
record!(Hole { cq: CqProgram, pipeline: cq_stage::Pipeline, interface: Ident });
record!(Execution { hole: usize, occurrence: usize, binding: Ident, strategy: Ident });
record!(RoundPolicy { phases: Ident, admission: Ident, termination: Ident, observer: Ident, outputs: Ident, timing: Ident });
record!(Relation { declaration: RelationDecl, all: Ident, seen: Ident, next: Ident, delta: Option<Ident>, delta_host: Option<Ident> });
record!(Layout { relation: usize, columns: Vec<usize>, key: usize, name: Ident, host: Ident });
record!(KeySet { relation: usize, columns: Vec<usize>, filters: Vec<(usize,Expr)>, name: Ident, representation: KeyRepresentation, hole: usize });
record!(ScanBinding { source: InputSource, binding: Ident });
record!(RuleHole { variant: usize, head: usize, hole: usize, scan: Option<ScanBinding> });
record!(Stratum { members: Vec<usize>, keys: Vec<KeySet>, seeds: Vec<RuleHole>, steps: Vec<RuleHole> });
record!(RoundContext { relations: Vec<Relation>, layouts: Vec<Layout>, delta_layouts: Vec<Layout>, strata: Vec<Stratum>, outputs: Vec<usize>, observer: Option<Expr>, observer_name: Ident, round_name: Ident });
impl Text for InputSource {
    fn parse_text(input: syn::parse::ParseStream<'_>)->syn::Result<Self> {
        let kind:Ident=input.parse()?; let index=usize::parse_text(input)?;
        match kind.to_string().as_str() { "delta"=>Ok(Self::Delta(index)), "layout"=>Ok(Self::Layout(index)), _=>Err(syn::Error::new_spanned(kind,"expected delta or layout")) }
    }
    fn text(&self)->TokenStream { match self { Self::Delta(i)=>{let i=i.text();quote!(delta #i)},Self::Layout(i)=>{let i=i.text();quote!(layout #i)} } }
}
impl Text for KeyRepresentation {
    fn parse_text(input: syn::parse::ParseStream<'_>)->syn::Result<Self> { let id:Ident=input.parse()?; if id=="hash_set_borrowed" {Ok(Self::HashSetBorrowed)} else {Err(syn::Error::new_spanned(id,"unsupported key representation"))} }
    fn text(&self)->TokenStream { quote!(hash_set_borrowed) }
}
pub fn ensure(ok:bool,at:impl ToTokens,message:&str)->syn::Result<()> { storage::ensure(ok,at,message) }

/// Erase only certified input substitutions. This never erases arbitrary CQ edits.
pub fn erase(program:&Program)->syn::Result<Program> {
    let mut erased=program.clone();
    let mut used=BTreeSet::new();
    let mut names=BTreeSet::new();
    collect_names(program.storage.to_token_stream(),&mut names);
    collect_names(program.context.to_token_stream(),&mut names);
    for hole in &program.holes {
        names.insert(symbol_name(&hole.cq.name));
        // Certificate bindings are omitted here; all other declaration names
        // and Rust identifiers remain forbidden to a new runtime binding.
        let mut cq=hole.cq.clone();
        for entry in &program.execution {
            if let Some(rule)=program.context.strata.iter().flat_map(|s|s.seeds.iter().chain(&s.steps)).find(|r|r.hole==entry.hole) {
                if program.holes.get(entry.hole).is_some_and(|h| h.cq.name==cq.name) {
                    if let Some(scan)=&rule.scan {
                        for declaration in &mut cq.declarations { if declaration.name==entry.binding { declaration.name=scan.binding.clone(); let binding=&scan.binding; declaration.value=Some(syn::parse_quote!(= (#binding))); } }
                        if let Some(BodyItem::Positive{atom})=cq.query.body.iter_mut().nth(entry.occurrence) { if atom.relation==entry.binding {atom.relation=scan.binding.clone();} }
                    }
                }
            }
        }
        collect_names(cq.to_token_stream(),&mut names);
    }
    for entry in &program.execution {
        ensure(used.insert(entry.hole),program,"duplicate execution certificate")?;
        ensure(entry.occurrence==0,program,"execution input must be the first occurrence")?;
        ensure(names.insert(symbol_name(&entry.binding)),program,"execution binding must be fresh and distinct")?;
        let rule=program.context.strata.iter().flat_map(|s|s.seeds.iter().chain(&s.steps)).find(|r|r.hole==entry.hole)
            .ok_or_else(||syn::Error::new_spanned(program,"execution certificate requires a rule hole"))?;
        let scan=rule.scan.as_ref().ok_or_else(||syn::Error::new_spanned(program,"execution requires an immutable scan"))?;
        let hole=erased.holes.get_mut(entry.hole).ok_or_else(||syn::Error::new_spanned(program,"missing execution hole"))?;
        let count=hole.cq.declarations.iter().filter(|d|d.name==entry.binding).count();
        ensure(count==1,program,"execution binding declaration must occur exactly once")?;
        let declaration=hole.cq.declarations.iter_mut().find(|d|d.name==entry.binding).expect("checked declaration");
        let binding=&entry.binding;
        ensure(declaration.value==Some(syn::parse_quote!(= (#binding))),program,"execution host must bind directly")?;
        declaration.name=scan.binding.clone(); let binding=&scan.binding; declaration.value=Some(syn::parse_quote!(= (#binding)));
        let Some(BodyItem::Positive{atom})=hole.cq.query.body.iter_mut().nth(entry.occurrence) else {return Err(syn::Error::new_spanned(program,"execution input must be positive"));};
        ensure(atom.relation==entry.binding,program,"execution certificate does not name its occurrence")?;
        atom.relation=scan.binding.clone();
    }
    erased.execution.clear();
    Ok(erased)
}
fn collect_names(tokens:TokenStream,names:&mut BTreeSet<String>) {
    for token in tokens { match token {TokenTree::Ident(id)=>{names.insert(symbol_name(&id));},TokenTree::Group(g)=>collect_names(g.stream(),names),_=>{}} }
}
pub fn check_pipeline(hole:&Hole)->syn::Result<()> {
    let terminal=hole.pipeline.passes.last().ok_or_else(||syn::Error::new_spanned(hole,"hole pipeline needs a terminal printer"))?;
    let names:Vec<_>=terminal.path.segments.iter().map(|s|s.ident.to_string()).collect();
    ensure(hole.interface=="pull" && names==["mini_linq","passes","print_pull"] && terminal.path.segments.iter().all(|s|matches!(s.arguments,syn::PathArguments::None)) && terminal.arguments.is_empty(),hole,"pull hole requires terminal mini_linq::passes::print_pull")
}
pub mod contract {
    use super::*;
    #[derive(Clone,Debug)] pub struct Analysis { pub storage: storage::Analysis }
    /// Formal rule WF-DL-Round: checked holes and complete publication protocol.
    pub fn check(program:&Program)->syn::Result<Analysis> {
        let analysis=storage::contract::check(&program.storage)?;
        let p=&program.policy;
        ensure(p.phases=="evaluate_admit_publish" && p.admission=="written_order" && p.termination=="any_delta" && p.observer=="before_step" && p.outputs=="owned_iterators" && (p.timing=="none" || p.timing=="phases"),program,"unsupported round execution policy")?;
        for hole in &program.holes {check_pipeline(hole)?;cq_lang_cq::contract::check(&hole.cq)?;}
        let erased=erase(program)?;
        check_storage(&program.storage,&erased)?;
        Ok(Analysis{storage:analysis})
    }
}
pub fn check_rewrite(source:&Program,target:&Program)->syn::Result<()> {
    contract::check(source)?;contract::check(target)?;
    let mut source=erase(source)?;let erased=erase(target)?;
    ensure(source.storage==erased.storage && source.context==erased.context,target,"round rewrite changed storage or round correspondence")?;
    source.policy.timing=erased.policy.timing.clone();
    for (before,after) in source.holes.iter_mut().zip(&erased.holes) {before.pipeline=after.pipeline.clone();}
    ensure(source==erased,target,"round rewrite changed a hole beyond its input certificate")
}
impl cq_stage::Language for Program {
    fn check(&self)->syn::Result<()> {contract::check(self).map(|_|())}
    fn check_rewrite(source:&Self,target:&Self)->syn::Result<()> {check_rewrite(source,target)}
}
