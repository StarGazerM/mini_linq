//! Optional checked continuation diagnostics. These records never select code.
//!
//! The versioned token descriptor implements `Parse`/`ToTokens`; consumers should
//! use `Trace::parse_descriptor` and independently check the recorded language
//! programs. Step identities are slash-separated VO step/branch/step paths;
//! `$output` denotes lowering's return/unit work, not a fabricated VO index.
use std::collections::{BTreeMap, BTreeSet};
use proc_macro2::{Span, TokenStream};
use quote::{ToTokens, quote};
use syn::parse::{Parse, ParseStream};
use syn::{Token, bracketed, parenthesized};

pub const OUTPUT: &str = "$output";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Node {
    pub name: String,
    /// Logical operator payload with named operands replaced by ordered slots.
    /// Physical modes/choices remain in Snapshot::program, not in this identity.
    pub operator: String,
    pub inputs: Vec<String>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Snapshot {
    pub language: String,
    pub program: String,
    pub nodes: Vec<Node>,
}
/// `origins` names VO steps in a Seed and preceding definitions in a Transition.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Coverage {
    pub node: String,
    pub origins: Vec<String>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Seed {
    pub valuation: Snapshot,
    pub lowering: Snapshot,
    /// VO steps explicitly erased by mechanical lowering (for example x = x).
    pub elided: Vec<String>,
    pub origins: Vec<Coverage>,
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
    pub transitions: Vec<Transition>,
}
fn fail(message: &str) -> syn::Error {
    syn::Error::new(Span::call_site(), format!("ST-Calibration: {message}"))
}
impl Snapshot {
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
        if !self.current().is_some_and(|snapshot|
            matches!(snapshot.language.as_str(), "physical" | "structures" | "sequence")) {
            return Err(fail("terminal requires actual final Physical definitions"));
        }
        Ok(self.to_token_stream().to_string())
    }
    pub fn current(&self) -> Option<&Snapshot> {
        self.transitions.last().map(|t| &t.target)
            .or_else(|| self.seed.as_ref().map(|s| &s.lowering))
    }
    pub fn initialize(&mut self, seed: Seed) -> syn::Result<()> {
        if self.seed.is_some() { return Err(fail("lowering origin already initialized")); }
        self.seed = Some(seed);
        self.check()
    }
    pub fn check_source(&self, source: Option<&Snapshot>) -> syn::Result<()> {
        if let Some(current) = self.current() {
            if source != Some(current) { return Err(fail("continuation does not match checked source")); }
        }
        Ok(())
    }
    pub fn advance(&mut self, target: Snapshot) -> syn::Result<()> {
        if let Some(source) = self.current() {
            if source != &target {
                let transition = Transition::between(source, target)?;
                self.transitions.push(transition);
            }
        }
        Ok(())
    }
    pub fn check(&self) -> syn::Result<()> {
        let Some(seed) = &self.seed else {
            return if self.transitions.is_empty() { Ok(()) } else { Err(fail("unseeded transitions")) };
        };
        seed.valuation.check()?;
        seed.lowering.check()?;
        if seed.valuation.language != "vo" || seed.lowering.language != "ra" {
            return Err(fail("origin seed must be actual VO to RA"));
        }
        let allowed: BTreeSet<_> = seed.valuation.nodes.iter().map(|n| n.name.as_str())
            .chain(std::iter::once(OUTPUT)).collect();
        if seed.origins.len() != seed.lowering.nodes.len() { return Err(fail("incomplete lowering origins")); }
        for (node, coverage) in seed.lowering.nodes.iter().zip(&seed.origins) {
            let unique: BTreeSet<_> = coverage.origins.iter().collect();
            if node.name != coverage.node || coverage.origins.is_empty()
                || unique.len() != coverage.origins.len()
                || coverage.origins.iter().any(|o| !allowed.contains(o.as_str())) {
                return Err(fail("invalid lowering origin"));
            }
        }
        let covered: BTreeSet<_> = seed.origins.iter().flat_map(|c| c.origins.iter().map(String::as_str)).collect();
        let elided: BTreeSet<_> = seed.elided.iter().map(String::as_str).collect();
        if elided.len() != seed.elided.len() || !elided.is_disjoint(&covered)
            || elided.iter().any(|name| *name == OUTPUT || !allowed.contains(name))
            || allowed.iter().any(|name| *name != OUTPUT && !covered.contains(name) && !elided.contains(name)) {
            return Err(fail("incomplete or conflicting VO coverage"));
        }
        let mut source = &seed.lowering;
        for transition in &self.transitions {
            if *transition != Transition::between(source, transition.target.clone())? {
                return Err(fail("invalid dependency-cone witness"));
            }
            source = &transition.target;
        }
        Ok(())
    }
    /// Final local VO coverage, in actual final definition order. Consumers must
    /// union these sets for runtime row aliases before summing VO bounds.
    pub fn coverage(&self) -> syn::Result<Vec<Coverage>> {
        self.check()?;
        let seed = self.seed.as_ref().ok_or_else(|| fail("missing origin seed"))?;
        let mut coverage = seed.origins.clone();
        for transition in &self.transitions {
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
impl Parse for Node {
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        let b; parenthesized!(b in input);
        let name = text(&b)?; b.parse::<Token![,]>()?;
        let operator = text(&b)?; b.parse::<Token![,]>()?;
        let inputs = strings(&b)?;
        if !b.is_empty() { return Err(b.error("unexpected node data")); }
        Ok(Self { name, operator, inputs })
    }
}
impl ToTokens for Node {
    fn to_tokens(&self, out: &mut TokenStream) {
        let Self {name, operator, inputs} = self;
        out.extend(quote!((#name, #operator, [#(#inputs),*])));
    }
}
impl Parse for Snapshot {
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        let b; parenthesized!(b in input);
        let language = text(&b)?; b.parse::<Token![,]>()?;
        let program = text(&b)?; b.parse::<Token![,]>()?;
        let nodes = list(&b)?;
        if !b.is_empty() { return Err(b.error("unexpected snapshot data")); }
        Ok(Self {language, program, nodes})
    }
}
impl ToTokens for Snapshot {
    fn to_tokens(&self, out: &mut TokenStream) {
        let Self {language, program, nodes} = self;
        out.extend(quote!((#language, #program, [#(#nodes),*])));
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
        let origins = list(&b)?; b.parse::<Token![,]>()?;
        let elided = strings(&b)?;
        if !b.is_empty() { return Err(b.error("unexpected seed data")); }
        Ok(Self { valuation, lowering, origins, elided })
    }
}
impl ToTokens for Seed {
    fn to_tokens(&self, out: &mut TokenStream) {
        let Self {valuation, lowering, origins, elided} = self;
        out.extend(quote!((#valuation, #lowering, [#(#origins),*], [#(#elided),*])));
    }
}
impl Parse for Transition {
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        let b; parenthesized!(b in input);
        let target = b.parse()?; b.parse::<Token![,]>()?;
        let parents = list(&b)?; b.parse::<Token![,]>()?;
        let elided = strings(&b)?;
        if !b.is_empty() { return Err(b.error("unexpected transition data")); }
        Ok(Self {target, parents, elided})
    }
}
impl ToTokens for Transition {
    fn to_tokens(&self, out: &mut TokenStream) {
        let Self {target, parents, elided} = self;
        out.extend(quote!((#target, [#(#parents),*], [#(#elided),*])));
    }
}
impl Parse for Trace {
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        input.parse::<crate::kw::calibration_trace>()?;
        let b; syn::braced!(b in input);
        let version: syn::LitInt = b.parse()?;
        if version.base10_parse::<u32>()? != 1 { return Err(b.error("unsupported calibration descriptor version")); }
        b.parse::<Token![;]>()?;
        let mut seeds: Vec<Seed> = list(&b)?;
        if seeds.len() > 1 { return Err(b.error("duplicate calibration seed")); }
        b.parse::<Token![;]>()?;
        let transitions = list(&b)?;
        if !b.is_empty() { return Err(b.error("unexpected trace data")); }
        let trace = Self { seed: seeds.pop(), transitions };
        trace.check()?;
        Ok(trace)
    }
}
impl ToTokens for Trace {
    fn to_tokens(&self, out: &mut TokenStream) {
        let seed = self.seed.iter();
        let transitions = &self.transitions;
        out.extend(quote!(calibration_trace { 1; [#(#seed),*]; [#(#transitions),*] }));
    }
}
