//! RoundShared plans are made over checked, versioned Round holes, not generated
//! kernel text. Alpha variable numbers are introduced in physical attribute order.
use cq_lang_datalog_round::{self as round, InputSource, Program, RuleHole};
use crate::storage::Symbols;
use std::collections::{BTreeMap, BTreeSet};
use syn::{Ident, Type};

#[path = "shared_emit.rs"]
mod emit;
pub(crate) use emit::lower;

#[derive(Clone, Debug, Eq, PartialEq)]
enum Access { Scan, Hash, Radix }
#[derive(Clone, Debug, Eq, PartialEq)]
struct Atom {
    host: Ident,
    // The checked physical storage word and column order, not a logical alias.
    word: Type,
    vars: Vec<usize>,
    keys: usize,
    access: Access,
}
#[derive(Clone)]
struct Body {
    hole: usize,
    owner: usize,
    delta: Option<(usize, Ident)>,
    atoms: Vec<Atom>,
    serial_growing_range: bool,
    head: Vec<usize>,
}
#[derive(Debug)]
struct Sink { hole: usize, owner: usize, suffix: Vec<Atom>, head: Vec<usize> }
#[derive(Debug)]
pub(crate) struct RoundShared {
    prefix: Vec<Atom>,
    live: Vec<usize>,
    sinks: Vec<Sink>,
}
impl RoundShared {
    pub(crate) fn holes(&self) -> impl Iterator<Item=usize> + '_ { self.sinks.iter().map(|b| b.hole) }
    pub(crate) fn cut(&self) -> usize { self.prefix.len() }
}
pub(crate) type Phases = BTreeMap<(usize, &'static str, usize), Vec<RoundShared>>;

fn body(program: &Program, component: usize, rule: &RuleHole, symbols: &Symbols) -> Option<Body> {
    use round::cq::BodyItem;
    let cq = &program.holes[rule.hole].cq;
    if !cq.query.captures.items.is_empty() || rule.known != round::Admission::Admit { return None; }
    let ctx = &program.context;
    let device = program.storage.device.as_ref()?;
    let changing = &ctx.strata[component].members;
    let column_ok = |col: &cq_syntax::ColumnDecl| symbols.codec(col.logical_type_id).is_some() || matches!(&col.ty, Type::Path(p) if p.path.is_ident("u32"));
    let output = &ctx.relations[rule.head].declaration.columns;
    if !(1..=2).contains(&output.len()) || !output.iter().all(column_ok) { return None; }
    let mut names = BTreeMap::new();
    let mut domains = Vec::new();
    let mut atoms = Vec::new();
    let mut delta = None;
    let mut serial_growing_range = false;
    for (i,item) in cq.query.body.iter().enumerate() {
        let BodyItem::Positive { atom } = item else { return None };
        let (host, relation, is_delta, word) = if i == 0 && rule.scan.as_ref().is_some_and(|s| s.binding == atom.relation) {
            match rule.scan.as_ref()?.source {
                InputSource::Delta(r) => (ctx.relations[r].delta_host.as_ref()?, r, true, &device.relations[r].word),
                InputSource::Layout(l) => (&ctx.layouts[l].host, ctx.layouts[l].relation, false, &device.layouts[l].word),
            }
        } else if let Some(l) = ctx.delta_layouts.iter().position(|l| l.name == atom.relation) {
            (&ctx.delta_layouts[l].host,ctx.delta_layouts[l].relation,true,&device.delta_layouts[l].word)
        } else if let Some(l) = ctx.layouts.iter().position(|l| l.name == atom.relation) {
            (&ctx.layouts[l].host,ctx.layouts[l].relation,false,&device.layouts[l].word)
        } else {
            let r = ctx.relations.iter().position(|r| r.delta.as_ref() == Some(&atom.relation))?;
            (ctx.relations[r].delta_host.as_ref()?,r,true,&device.relations[r].word)
        };
        let decl = cq.declarations.iter().find(|d| d.name == atom.relation)?;
        if decl.columns.len() != atom.args.len() || !decl.columns.iter().all(column_ok) { return None; }
        if is_delta { delta = Some((program.storage.incremental.variants[rule.variant].delta?, host.clone())); }
        let before = names.len();
        let vars = atom.args.iter().zip(&decl.columns).map(|(a,col)| {
            let var = cq_syntax::symbol_name(cq_syntax::rust::variable(a)?);
            let next = names.len();
            let id = *names.entry(var).or_insert(next);
            if id == domains.len() { domains.push((&col.ty,col.logical_type_id)); }
            (domains[id] == (&col.ty,col.logical_type_id)).then_some(id)
        }).collect::<Option<Vec<_>>>()?;
        let keys = vars.iter().take_while(|&&v| v < before).count();
        // The native directory/radix operators probe a leading key range. They
        // enumerate every remaining column; no FD or uniqueness premise is used.
        if vars.is_empty() || vars.iter().collect::<BTreeSet<_>>().len() != vars.len()
            || vars[keys..].iter().any(|&v| v < before) || (i != 0 && keys == 0) { return None; }
        // A per-root serial loop cannot share a long full-IDB expansion with
        // idle CTAs. Keep the incumbent's load-balanced realization there;
        // never build a full-IDB hash table to disguise that cost.
        serial_growing_range |= i != 0 && !is_delta && changing.contains(&relation) && keys < vars.len();
        let access = if i == 0 { Access::Scan } else if keys > 3 || is_delta || (changing.contains(&relation) && keys < vars.len()) { Access::Radix } else { Access::Hash };
        atoms.push(Atom {host:host.clone(),word:word.clone(),vars,keys,access});
    }
    let head = cq.query.head.args.iter().zip(output).map(|(e,col)| {
        let id = *names.get(&cq_syntax::symbol_name(cq_syntax::rust::variable(e)?))?;
        (domains[id] == (&col.ty,col.logical_type_id)).then_some(id)
    }).collect::<Option<Vec<_>>>()?;
    Some(Body {hole:rule.hole,owner:rule.head,atoms,head,delta,serial_growing_range})
}

/// Formal rule DL-RoundShared: all arms are in a single publication epoch;
/// exact structural equality of alpha-numbered reads proves shared witnesses.
/// Suffixes/projectors retain separate multiplicities and admission owners.
fn checked_group(representative: &Body, depth: usize, branches: &[&Body]) -> Option<RoundShared> {
    let prefix = representative.atoms.get(..depth)?;
    if depth < 2 || branches.len() < 2 || branches.iter().any(|b| b.delta != representative.delta || b.atoms.get(..depth) != Some(prefix)) { return None; }
    let bound = prefix.iter().flat_map(|a| &a.vars).copied().collect::<BTreeSet<_>>();
    let mut live = BTreeSet::new();
    let mut sinks = Vec::new();
    for b in branches {
        let suffix = &b.atoms[depth..];
        live.extend(b.head.iter().chain(suffix.iter().flat_map(|a| &a.vars)).filter(|v| bound.contains(v)).copied());
        sinks.push(Sink { hole:b.hole,owner:b.owner,suffix:suffix.to_vec(),head:b.head.clone() });
    }
    Some(RoundShared {prefix:prefix.to_vec(),live:live.into_iter().collect(),sinks})
}

pub(crate) fn plan(program: &Program, excluded: &BTreeSet<usize>, symbols: &Symbols) -> Phases {
    let mut phases = Phases::new();
    for (s,(stratum,schedule)) in program.context.strata.iter().zip(&program.schedule.strata).enumerate() {
        for (phase,rules) in [(round::schedule::SEED,&stratum.seeds),(round::schedule::STEP,&stratum.steps)] {
            for (point,epoch) in crate::bindings::epochs(stratum,schedule,phase).into_iter().enumerate() {
                let bodies: Vec<_> = epoch.rules.iter().map(|&i| &rules[i]).filter(|r| !excluded.contains(&r.hole))
                    .filter_map(|r| body(program,s,r,symbols)).filter(|b| !b.serial_growing_range).collect();
                let mut used = BTreeSet::new();
                // Deepest common node first. Equality is checked on the whole path,
                // not on hashes, relation spelling, head arity or an observed count.
                for depth in (2..=bodies.iter().map(|b| b.atoms.len()).max().unwrap_or(0)).rev() {
                    for first in &bodies {
                        if used.contains(&first.hole) || first.atoms.len() < depth { continue; }
                        let branches = bodies.iter().filter(|b| !used.contains(&b.hole) && b.delta == first.delta && b.atoms.get(..depth) == first.atoms.get(..depth)).collect::<Vec<_>>();
                        if let Some(group) = checked_group(first,depth,&branches) {
                            used.extend(group.holes());
                            phases.entry((s,phase,point)).or_default().push(group);
                        }
                    }
                }
            }
        }
    }
    phases
}

/// Terminal binding packet: the leader is the first member in the epoch's
/// checked schedule, not a rule selector. The independent checker reconstructs
/// all reads and sinks; a forged reference cannot introduce a new producer.
pub(crate) fn unpack(tokens: &proc_macro2::TokenStream) -> syn::Result<(Option<(usize,usize)>,proc_macro2::TokenStream)> {
    use syn::parse::Parser;
    if !matches!(tokens.clone().into_iter().next(), Some(proc_macro2::TokenTree::Ident(i)) if i == "round_shared") {
        return Ok((None,tokens.clone()));
    }
    (|input: syn::parse::ParseStream<'_>| {
        input.parse::<Ident>()?;
        let args;
        syn::parenthesized!(args in input);
        let leader = args.parse::<syn::LitInt>()?.base10_parse()?;
        args.parse::<syn::Token![,]>()?;
        let depth = args.parse::<syn::LitInt>()?.base10_parse()?;
        if !args.is_empty() { return Err(args.error("unexpected RoundShared fields")); }
        input.parse::<syn::Token![;]>()?;
        Ok((Some((leader,depth)),input.parse()?))
    }).parse2(tokens.clone())
}

pub(crate) fn check(program: &Program) -> syn::Result<Phases> {
    let mut marked = BTreeMap::new();
    for (i,h) in program.holes.iter().enumerate() {
        if let Some(pass) = h.pipeline.passes.last() && let (Some(tag),_) = unpack(&pass.arguments)? { marked.insert(i,tag); }
    }
    let mut phases = Phases::new();
    if marked.is_empty() { return Ok(phases); }
    let symbols = Symbols::from_program(&program.storage)?;
    for (s,(stratum,schedule)) in program.context.strata.iter().zip(&program.schedule.strata).enumerate() {
        for (phase,rules) in [(round::schedule::SEED,&stratum.seeds),(round::schedule::STEP,&stratum.steps)] {
            for (point,epoch) in crate::bindings::epochs(stratum,schedule,phase).into_iter().enumerate() {
                let mut groups = BTreeMap::<_,Vec<Body>>::new();
                for i in epoch.rules {
                    let rule = &rules[i];
                    if let Some(tag) = marked.remove(&rule.hole) {
                        let branch = body(program,s,rule,&symbols).ok_or_else(||syn::Error::new_spanned(rule,"ineligible RoundShared branch"))?;
                        groups.entry(tag).or_default().push(branch);
                    }
                }
                for ((leader,depth),branches) in groups {
                    let first = &branches[0];
                    let group = checked_group(first,depth,&branches.iter().collect::<Vec<_>>())
                        .filter(|_| first.hole == leader).ok_or_else(||syn::Error::new_spanned(&program.holes[first.hole],"RoundShared prefix, epoch, or leader mismatch"))?;
                    phases.entry((s,phase,point)).or_default().push(group);
                }
            }
        }
    }
    if !marked.is_empty() { return Err(syn::Error::new_spanned(program,"RoundShared member is not a scheduled rule")); }
    Ok(phases)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn atom(host: &str, keys: usize, vars: &[usize]) -> Atom {
        Atom { host:syn::parse_str(host).unwrap(), word:syn::parse_quote!(u64), vars:vars.to_vec(), keys,
            access:if keys == 0 { Access::Scan } else { Access::Hash } }
    }
    #[test]
    fn physical_roles_not_variable_spellings_identify_a_prefix() {
        let mut p = crate::print_tests::program_with_symbols(syn::parse_quote!(u32),quote::quote!());
        let (s,rule) = p.context.strata.iter().enumerate()
            .find_map(|(s,stratum)| stratum.steps.first().map(|r|(s,r.clone()))).unwrap();
        let symbols = Symbols::from_program(&p.storage).unwrap();
        let mut renamed = p.clone();
        let set_args = |program: &mut Program, names: [&str;3]| {
            let [x,y,z] = names.map(|s| syn::parse_str::<syn::Expr>(s).unwrap());
            let q = &mut program.holes[rule.hole].cq.query;
            for (item,args) in q.body.iter_mut().zip([[x.clone(),y.clone()],[y,z.clone()]]) {
                let round::cq::BodyItem::Positive {atom} = item else { panic!("expected a positive path") };
                for (out,arg) in atom.args.iter_mut().zip(args) { *out = arg; }
            }
            for (out,arg) in q.head.args.iter_mut().zip([x,z]) { *out = arg; }
        };
        set_args(&mut p,["x","y","z"]);
        set_args(&mut renamed,["left","separator","right"]);
        let a = body(&p,s,&rule,&symbols).unwrap();
        let b = body(&renamed,s,&rule,&symbols).unwrap();
        assert_eq!(a.atoms,b.atoms);
        assert_eq!(a.head,b.head);
        assert_eq!(a.delta,b.delta);
    }
    #[test]
    fn differing_suffixes_keep_independent_bags_and_owners() {
        let a = Body { hole:7,owner:4,delta:Some((2,syn::parse_quote!(d))), serial_growing_range:false,
            atoms:vec![atom("d",0,&[0,1]),atom("x",1,&[1,2])],head:vec![2] };
        let mut b = a.clone();
        b.hole = 10;
        b.owner = 9;
        b.atoms.push(atom("y",1,&[2,3]));
        b.head = vec![0,3];
        let g = checked_group(&a,2,&[&a,&b]).unwrap();
        assert_eq!(g.live,[0,2]); // root join key is dead after the cut
        assert_eq!(g.sinks[0].suffix.len(),0);
        assert_eq!(g.sinks[1].suffix.len(),1);
        assert_eq!((g.sinks[0].owner,g.sinks[1].owner),(4,9));
        assert!(checked_group(&a,3,&[&a,&b]).is_none());
        b.delta.as_mut().unwrap().0 = 1;
        assert!(checked_group(&a,2,&[&a,&b]).is_none());
        b.delta = a.delta.clone();
        b.atoms[1].host = syn::parse_quote!(other_layout);
        assert!(checked_group(&a,2,&[&a,&b]).is_none());
        b.atoms[1].host = a.atoms[1].host.clone();
        b.atoms[1].vars[0] = 0;
        assert!(checked_group(&a,2,&[&a,&b]).is_none());
    }
}
