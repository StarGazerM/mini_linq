//! Optional checked continuation diagnostics. These records never select code.
//!
//! The facade observes every checked stage and records its language, selected
//! printed program, and the base programs needed to check an annotation.
//! The calibrator reads the versioned token descriptor with
//! `Trace::parse_descriptor`, independently re-parses and checks each recorded
//! program with its bases, and supplies its dependency graph to `Trace::coverage`.
//! Version 4 combines annotation bases with the sparse lowering `Steps`
//! correspondence, not a replay of all VO operations. Step identities are
//! slash-separated VO step/branch/step paths; `$output` denotes unanchored
//! lowering work, not a fabricated VO index. Inherited dependency coverage
//! is diagnostic, not a stream-work certificate.
use std::collections::{BTreeMap, BTreeSet};
use proc_macro2::{Span, TokenStream};
use quote::{ToTokens, quote};
use syn::parse::{Parse, ParseStream};
use syn::{Token, bracketed, parenthesized};

pub const OUTPUT: &str = "$output";

/// A checked stage as observed by the facade: the Rust type of its language
/// and its printed program, plus separately printed bases for annotations.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Stage {
    pub language: String,
    pub program: String,
    /// Programs needed to parse/check this layer, in stack order.
    /// These are continuation premises, not the selected program's identity.
    pub bases: Vec<String>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Node {
    pub name: String,
    /// Logical operator payload with named operands replaced by ordered slots.
    /// Physical modes/choices remain in Snapshot::program, not in this identity.
    pub operator: String,
    pub inputs: Vec<String>,
}
/// A recorded stage with the dependency graph derived by the calibrator.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Snapshot {
    pub language: String,
    pub program: String,
    pub nodes: Vec<Node>,
}
/// `origins` names VO steps in coverage and preceding definitions in a Transition.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Coverage {
    pub node: String,
    pub origins: Vec<String>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Seed {
    pub valuation: Stage,
    pub lowering: Stage,
    /// Sparse definition-name correspondence to VO step/branch/step positions.
    pub steps: BTreeMap<String, Vec<usize>>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Transition {
    pub target: Snapshot,
    pub parents: Vec<Coverage>,
    /// Removed definitions with no surviving consumer. They are not fused work.
    pub elided: Vec<String>,
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Trace {
    pub seed: Option<Seed>,
    /// Checked stages after the seed's lowering, in pipeline order.
    pub stages: Vec<Stage>,
}
fn fail(message: &str) -> syn::Error {
    syn::Error::new(Span::call_site(), format!("ST-Calibration: {message}"))
}
impl Stage {
    /// Record a flat language program, with no separately supplied bases.
    pub fn of<L: ToTokens>(program: &L) -> Self {
        Self::annotation(program, Vec::new())
    }
    /// Record the selected annotation's identity and its checking premises.
    /// Bases are printed in stack order: `[Physical]` for Structures and
    /// `[Physical, Structures]` for Sequence. They participate in continuation
    /// agreement, but not in `Snapshot`'s selected-program identity.
    pub fn annotation<L: ToTokens>(program: &L, bases: Vec<String>) -> Self {
        Self { language: std::any::type_name::<L>().into(), program: program.to_token_stream().to_string(), bases }
    }
    pub fn is<L>(&self) -> bool {
        self.language == std::any::type_name::<L>()
    }
}
impl Snapshot {
    pub fn new(stage: &Stage, nodes: Vec<Node>) -> Self {
        Self { language: stage.language.clone(), program: stage.program.clone(), nodes }
    }
    pub fn check(&self) -> syn::Result<()> {
        let mut seen = BTreeSet::new();
        for node in &self.nodes {
            if node.name.is_empty() || node.inputs.iter().any(|name| !seen.contains(name))
                || !seen.insert(node.name.clone()) {
                return Err(fail("snapshot definitions must be unique and topological"));
            }
        }
        Ok(())
    }
}
impl Transition {
    /// Local dependency-cone witness: retain identities, transfer deleted nodes
    /// to their first surviving consumers, and anchor inserted nodes in their
    /// actual downstream retained consumers (or upstream retained operands).
    /// No unrelated definition is ever included merely because it shares a pass.
    pub fn between(source: &Snapshot, target: Snapshot) -> syn::Result<Self> {
        source.check()?;
        target.check()?;
        let mut parents: BTreeMap<String, BTreeSet<String>> = target.nodes.iter()
            .map(|node| {
                let Some(previous) = source.nodes.iter().find(|n| n.name == node.name) else {
                    return (node.name.clone(), BTreeSet::new());
                };
                let mut identity = previous.name.clone();
                if previous.operator != node.operator {
                    let candidates: Vec<_> = source.nodes.iter().filter(|candidate| {
                        if candidate.operator != node.operator { return false; }
                        let stop = BTreeSet::from([candidate.name.clone()]);
                        !frontier(&source.nodes, &previous.name, &stop, true).is_empty()
                            || !frontier(&source.nodes, &previous.name, &stop, false).is_empty()
                    }).collect();
                    if let [candidate] = candidates.as_slice() {
                        identity = candidate.name.clone();
                    }
                }
                (node.name.clone(), BTreeSet::from([identity]))
            }).collect();
        // Alpha-renamed definitions are exact when the logical payload and
        // already established operand identities select one source definition.
        for node in &target.nodes {
            if !parents[&node.name].is_empty() { continue; }
            let candidates: Vec<_> = source.nodes.iter().filter(|candidate|
                candidate.operator == node.operator
                    && candidate.inputs.len() == node.inputs.len()
                    && candidate.inputs.iter().zip(&node.inputs).all(|(old, new)|
                        parents[new].len() == 1 && parents[new].contains(old))
            ).collect();
            if let [candidate] = candidates.as_slice() {
                parents.get_mut(&node.name).unwrap().insert(candidate.name.clone());
            }
        }
        let retained: BTreeSet<_> = parents.iter().filter(|(_, origins)| !origins.is_empty())
            .map(|(name, _)| name.clone()).collect();
        let mut survivors: BTreeMap<String, Vec<String>> = BTreeMap::new();
        for (target, sources) in &parents {
            for source in sources {
                survivors.entry(source.clone()).or_default().push(target.clone());
            }
        }
        let surviving_sources = survivors.keys().cloned().collect();
        let mut elided = Vec::new();
        for removed in source.nodes.iter().filter(|n| !survivors.contains_key(&n.name)) {
            let consumers = frontier(&source.nodes, &removed.name, &surviving_sources, false);
            if consumers.is_empty() { elided.push(removed.name.clone()); }
            for consumer in consumers {
                for target in &survivors[&consumer] {
                    parents.get_mut(target).unwrap().insert(removed.name.clone());
                }
            }
        }
        for added in &target.nodes {
            if retained.contains(&added.name) { continue; }
            let consumers = frontier(&target.nodes, &added.name, &retained, false);
            let mut anchors = BTreeSet::new();
            for consumer in consumers {
                anchors.extend(parents[&consumer].iter().cloned());
            }
            if anchors.is_empty() {
                for operand in frontier(&target.nodes, &added.name, &retained, true) {
                    anchors.extend(parents[&operand].iter().cloned());
                }
            }
            if anchors.is_empty() {
                return Err(fail("introduced definition requires an explicit producer origin"));
            }
            parents.insert(added.name.clone(), anchors);
        }
        Ok(Self { parents: target.nodes.iter().map(|n| Coverage {
            node: n.name.clone(), origins: parents.remove(&n.name).unwrap().into_iter().collect(),
        }).collect(), target, elided })
    }
}
fn frontier(nodes: &[Node], start: &str, retained: &BTreeSet<String>, upstream: bool) -> BTreeSet<String> {
    let mut pending = vec![start.to_owned()];
    let mut visited = BTreeSet::new();
    let mut found = BTreeSet::new();
    while let Some(name) = pending.pop() {
        if !visited.insert(name.clone()) { continue; }
        for node in nodes {
            let adjacent: Vec<&str> = if upstream {
                if node.name == name { node.inputs.iter().map(String::as_str).collect() } else { Vec::new() }
            } else if node.inputs.contains(&name) { vec![node.name.as_str()] } else { Vec::new() };
            for next in adjacent {
                if retained.contains(next) { found.insert(next.to_owned()); }
                else { pending.push(next.to_owned()); }
            }
        }
    }
    found
}
impl Trace {
    pub fn parse_descriptor(text: &str) -> syn::Result<Self> {
        let trace: Self = syn::parse_str(text)?;
        trace.check()?;
        Ok(trace)
    }
    pub fn descriptor(&self) -> syn::Result<String> {
        self.check()?;
        if self.seed.is_none() { return Err(fail("terminal has no lowering origin producer")); }
        Ok(self.to_token_stream().to_string())
    }
    pub fn current(&self) -> Option<&Stage> {
        self.stages.last().or_else(|| self.seed.as_ref().map(|s| &s.lowering))
    }
    pub fn initialize(&mut self, seed: Seed) -> syn::Result<()> {
        if self.seed.is_some() { return Err(fail("lowering origin already initialized")); }
        self.seed = Some(seed);
        self.check()
    }
    pub fn check_source(&self, source: &Stage) -> syn::Result<()> {
        if let Some(current) = self.current() {
            if source != current { return Err(fail("continuation does not match checked source")); }
        }
        Ok(())
    }
    pub fn advance(&mut self, target: Stage) {
        if self.current().is_some_and(|source| *source != target) {
            self.stages.push(target);
        }
    }
    pub fn check(&self) -> syn::Result<()> {
        if self.seed.is_none() && !self.stages.is_empty() { return Err(fail("unseeded transitions")); }
        Ok(())
    }
    /// Seed lowering snapshot and dependency-cone witnesses between consecutive
    /// recorded stages, with graphs supplied by the calibrator.
    pub fn transitions(&self, mut nodes: impl FnMut(&Stage) -> syn::Result<Vec<Node>>)
        -> syn::Result<(Snapshot, Vec<Transition>)> {
        self.check()?;
        let seed = self.seed.as_ref().ok_or_else(|| fail("missing origin seed"))?;
        let valuation = Snapshot::new(&seed.valuation, nodes(&seed.valuation)?);
        let lowering = Snapshot::new(&seed.lowering, nodes(&seed.lowering)?);
        valuation.check()?;
        lowering.check()?;
        let allowed: BTreeSet<_> = valuation.nodes.iter().map(|n| n.name.as_str())
            .chain(std::iter::once(OUTPUT)).collect();
        for (name, path) in &seed.steps {
            let origin = path.iter().map(usize::to_string).collect::<Vec<_>>().join("/");
            if path.is_empty() || path.len() % 2 == 0 || !allowed.contains(origin.as_str())
                || !lowering.nodes.iter().any(|node| node.name == *name) {
                return Err(fail("invalid lowering step correspondence"));
            }
        }
        let mut transitions: Vec<Transition> = Vec::new();
        for stage in &self.stages {
            let target = Snapshot::new(stage, nodes(stage)?);
            let source = transitions.last().map_or(&lowering, |t| &t.target);
            transitions.push(Transition::between(source, target)?);
        }
        Ok((lowering, transitions))
    }
    /// Final dependency coverage, in actual final definition order. Sparse
    /// step ancestry does not certify the work of unmapped stream operators.
    pub fn coverage(&self, nodes: impl FnMut(&Stage) -> syn::Result<Vec<Node>>) -> syn::Result<Vec<Coverage>> {
        let (lowering, transitions) = self.transitions(nodes)?;
        let seed = self.seed.as_ref().expect("checked seed");
        let mut inherited: BTreeMap<String, Vec<String>> = BTreeMap::new();
        let mut coverage = Vec::with_capacity(lowering.nodes.len());
        for node in &lowering.nodes {
            let origins = if let Some(path) = seed.steps.get(&node.name) {
                vec![path.iter().map(usize::to_string).collect::<Vec<_>>().join("/")]
            } else {
                let mut origins: BTreeSet<_> = node.inputs.iter()
                    .flat_map(|input| inherited[input].iter().cloned()).collect();
                if origins.is_empty() { origins.insert(OUTPUT.into()); }
                origins.into_iter().collect()
            };
            inherited.insert(node.name.clone(), origins.clone());
            coverage.push(Coverage { node: node.name.clone(), origins });
        }
        for transition in &transitions {
            let previous: BTreeMap<_, _> = coverage.iter().map(|c| (c.node.as_str(), &c.origins)).collect();
            coverage = transition.parents.iter().map(|c| Coverage {
                node: c.node.clone(), origins: c.origins.iter().flat_map(|p| previous[p.as_str()].iter().cloned())
                    .collect::<BTreeSet<_>>().into_iter().collect(),
            }).collect();
        }
        Ok(coverage)
    }
}

fn text(input: ParseStream<'_>) -> syn::Result<String> { Ok(input.parse::<syn::LitStr>()?.value()) }
fn strings(input: ParseStream<'_>) -> syn::Result<Vec<String>> {
    let body; bracketed!(body in input);
    Ok(body.parse_terminated(text, Token![,])?.into_iter().collect())
}
fn list<T: Parse>(input: ParseStream<'_>) -> syn::Result<Vec<T>> {
    let body; bracketed!(body in input);
    Ok(body.parse_terminated(T::parse, Token![,])?.into_iter().collect())
}
impl Parse for Stage {
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        let b; parenthesized!(b in input);
        let language = text(&b)?; b.parse::<Token![,]>()?;
        let program = text(&b)?; b.parse::<Token![,]>()?;
        let bases = strings(&b)?;
        if !b.is_empty() { return Err(b.error("unexpected stage data")); }
        Ok(Self {language, program, bases})
    }
}
impl ToTokens for Stage {
    fn to_tokens(&self, out: &mut TokenStream) {
        let Self {language, program, bases} = self;
        out.extend(quote!((#language, #program, [#(#bases),*])));
    }
}
impl Parse for Coverage {
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        let b; parenthesized!(b in input);
        let node = text(&b)?; b.parse::<Token![,]>()?;
        let origins = strings(&b)?;
        if !b.is_empty() { return Err(b.error("unexpected coverage data")); }
        Ok(Self {node, origins})
    }
}
impl ToTokens for Coverage {
    fn to_tokens(&self, out: &mut TokenStream) {
        let Self {node, origins} = self;
        out.extend(quote!((#node, [#(#origins),*])));
    }
}
impl Parse for Seed {
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        let b; parenthesized!(b in input);
        let valuation = b.parse()?; b.parse::<Token![,]>()?;
        let lowering = b.parse()?; b.parse::<Token![,]>()?;
        let entries; bracketed!(entries in b);
        let mut steps = BTreeMap::new();
        while !entries.is_empty() {
            let entry; parenthesized!(entry in entries);
            let name = text(&entry)?; entry.parse::<Token![,]>()?;
            let position; bracketed!(position in entry);
            let path = position.parse_terminated(syn::LitInt::parse, Token![,])?
                .iter().map(syn::LitInt::base10_parse).collect::<syn::Result<Vec<usize>>>()?;
            if !entry.is_empty() { return Err(entry.error("unexpected step correspondence data")); }
            if steps.insert(name, path).is_some() { return Err(entries.error("duplicate lowering definition")); }
            if !entries.is_empty() { entries.parse::<Token![,]>()?; }
        }
        if !b.is_empty() { return Err(b.error("unexpected seed data")); }
        Ok(Self { valuation, lowering, steps })
    }
}
impl ToTokens for Seed {
    fn to_tokens(&self, out: &mut TokenStream) {
        let Self {valuation, lowering, steps} = self;
        let entries = steps.iter().map(|(name, path)| quote!((#name, [#(#path),*])));
        out.extend(quote!((#valuation, #lowering, [#(#entries),*])));
    }
}
impl Parse for Trace {
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        input.parse::<crate::kw::calibration_trace>()?;
        let b; syn::braced!(b in input);
        let version: syn::LitInt = b.parse()?;
        if version.base10_parse::<u32>()? != 4 { return Err(b.error("unsupported calibration descriptor version")); }
        b.parse::<Token![;]>()?;
        let mut seeds: Vec<Seed> = list(&b)?;
        if seeds.len() > 1 { return Err(b.error("duplicate calibration seed")); }
        b.parse::<Token![;]>()?;
        let stages = list(&b)?;
        if !b.is_empty() { return Err(b.error("unexpected trace data")); }
        let trace = Self { seed: seeds.pop(), stages };
        trace.check()?;
        Ok(trace)
    }
}
impl ToTokens for Trace {
    fn to_tokens(&self, out: &mut TokenStream) {
        let seed = self.seed.iter();
        let stages = &self.stages;
        out.extend(quote!(calibration_trace { 4; [#(#seed),*]; [#(#stages),*] }));
    }
}
