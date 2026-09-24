//! Stream → Access: erase types, inline pipes, bind materialized results to
//! runtime structures, and resolve every cursor to a generic trait call.
//!
//! The output is ordinary Rust in two parts: `let` statements that build the
//! structures every `let` definition provides, and one flat Access region for
//! the returned result whose leaves call `cq_storage` traits on host values
//! and on those structures. No data structure is named in the input; each
//! is chosen here from the capabilities alone, and the host's own types are
//! fixed by the host through monomorphization.
//!
//! Every structure is bound by reference for the enclosing block, so pull
//! endpoints capture `&T` and the region is consumed where it is built; a
//! lazy iterator escaping the block must be over host-owned structures.
//!
//! Layout conventions this pass owns:
//! - Host values hold tuples in declared column order: rows `(c0, c1, ..)`,
//!   map keys `(k, ..)` over the key columns in declared order, payloads
//!   `(p, ..)` over the remaining columns in declared order.
//! - Built structures hold tuples of references in sorted heading order.
//! - Every Access leaf yields exactly the tuple of fresh variables in the
//!   order the clause's atom lists them, as the Access contract requires.

use std::collections::BTreeMap;

use cq_ir::{cq, rust_access_plan as access, stream, structures, symbol_name};
use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use syn::visit_mut::VisitMut;
use syn::{Expr, Ident, Path, Stmt};

/// Caller-resolved macro paths the realization emits into.
pub struct Endpoints {
    /// Consumes an Access region and evaluates to a lazy bag iterator whose
    /// items are the region's output tuples.
    pub support_iter: Path,
}

pub struct Realized {
    pub statements: Vec<Stmt>,
    pub access: access::Plan,
}

/// Where a name's rows live once realized.
enum Repr {
    /// A host value, read in declared column order through `layout`.
    Host(HostLayout),
    /// A stream inlined into its single reader.
    Pipe(Fragment),
    /// Structures built here, keyed by the operation each serves.
    Built(Built),
}

#[derive(Clone)]
struct HostLayout {
    /// The host expression (the relation's own name in scope).
    value: Ident,
    /// For each sorted-heading ordinal of the *reading* name, the declared
    /// column position in the host value.
    declared_position: Vec<usize>,
}

#[derive(Default)]
struct Built {
    rows: Option<Ident>,
    seek: BTreeMap<Vec<usize>, Ident>,
    contains: BTreeMap<Vec<usize>, Ident>,
    ordered: BTreeMap<Vec<usize>, Ident>,
}

/// A realized body: Access clauses plus the variables it emits.
#[derive(Clone, Default)]
struct Fragment {
    clauses: Vec<access::Clause>,
    emitted: Vec<Ident>,
    bound: Vec<Ident>,
    supported: bool,
}

pub fn compile(module: &stream::Module, endpoints: &Endpoints) -> syn::Result<Realized> {
    let analysis = stream::contract::check(module)?;
    let program = &module.structures.index_requirements.relational_plan.program;
    let mut reprs: BTreeMap<String, Repr> = BTreeMap::new();
    for input in program.inputs() {
        let name = symbol_name(&input.name);
        let declared: Vec<_> = input.columns.iter().map(|c| symbol_name(&c.name)).collect();
        let sorted = &analysis.headings[&name];
        let declared_position = sorted
            .iter()
            .map(|c| declared.iter().position(|d| d == c).unwrap())
            .collect();
        reprs.insert(
            name,
            Repr::Host(HostLayout {
                value: input.name.clone(),
                declared_position,
            }),
        );
    }
    let ra_definitions = &module
        .structures
        .index_requirements
        .relational_plan
        .plan
        .definitions;
    // The structures chosen for every name, as the catalogue kinds to build.
    let mut chosen: BTreeMap<String, Vec<(structures::Kind, Vec<usize>)>> = BTreeMap::new();
    for entry in &module.structures.entries {
        if let structures::Structure::Keyed(keyed) = &entry.structure {
            chosen
                .entry(symbol_name(&entry.name))
                .or_default()
                .push((keyed.kind, keyed.ordinals()));
        }
    }
    let mut statements = Vec::new();
    let mut realizer = Realizer {
        analysis: &analysis,
        chosen,
        reprs,
        endpoints,
    };
    for (definition, ra_definition) in module.program.definitions.iter().zip(ra_definitions) {
        let name = symbol_name(&definition.name);
        let repr = match &definition.realization {
            stream::Realization::Alias { host, .. } => {
                let cq_ir::relational_plan::Operator::Rename(rename) = &ra_definition.operator
                else {
                    unreachable!("contract admits alias only for renames");
                };
                let Repr::Host(host_layout) = &realizer.reprs[&symbol_name(host)] else {
                    unreachable!("alias names an opened host");
                };
                let host_sorted = &analysis.headings[&symbol_name(host)];
                let heading = &analysis.headings[&name];
                let declared_position = heading
                    .iter()
                    .map(|target| {
                        let source = rename
                            .mapping
                            .iter()
                            .find(|m| symbol_name(&m.target) == *target)
                            .map(|m| symbol_name(&m.source))
                            .unwrap();
                        let sorted_index = host_sorted.iter().position(|h| *h == source).unwrap();
                        host_layout.declared_position[sorted_index]
                    })
                    .collect();
                Repr::Host(HostLayout {
                    value: host_layout.value.clone(),
                    declared_position,
                })
            }
            stream::Realization::Stream(streamed) => {
                let fragment = realizer.body(&streamed.body)?;
                match &streamed.mode {
                    stream::Mode::Pipe { .. } => Repr::Pipe(fragment),
                    stream::Mode::Let { .. } => {
                        if fragment.supported {
                            return Err(syn::Error::new_spanned(
                                &definition.name,
                                "support inside a materialized definition is not realized; only the returned result is deduplicated",
                            ));
                        }
                        let provided = &analysis.provided[&name];
                        Repr::Built(realizer.build(&name, &fragment, provided, &mut statements)?)
                    }
                }
            }
        };
        realizer.reprs.insert(name, repr);
    }

    let returned = symbol_name(&module.program.return_.name);
    let fragment = match realizer.reprs.remove(&returned) {
        Some(Repr::Pipe(fragment)) => fragment,
        Some(Repr::Built(built)) => {
            // A materialized result is returned by scanning its rows.
            let heading = &analysis.headings[&returned];
            let vars: Vec<Ident> = heading.iter().map(|h| ident(h)).collect();
            let rows = built.rows.expect("returned let provides iter");
            let atom = atom(&ident(&returned), &vars);
            Fragment {
                clauses: vec![syn::parse_quote! {
                    #atom => bind (#(#vars,)*) <- (::core::iter::Iterator::copied(#rows.iter()))
                }],
                emitted: vars,
                bound: Vec::new(),
                supported: false,
            }
        }
        Some(Repr::Host(layout)) => {
            // An aliased host is returned by scanning it in declared order.
            let heading = &analysis.headings[&returned];
            let vars: Vec<Ident> = heading.iter().map(|h| ident(h)).collect();
            let mut fragment = Fragment::default();
            let alias = ident(&returned);
            realizer.reprs.insert(returned.clone(), Repr::Host(layout));
            realizer.scan(&alias, &vars, &mut fragment)?;
            fragment.emitted = vars;
            fragment
        }
        None => unreachable!("return names a definition"),
    };
    let head_relation = &module
        .structures
        .index_requirements
        .relational_plan
        .plan
        .output
        .head
        .relation;
    let emitted = &fragment.emitted;
    let clauses = &fragment.clauses;
    let access: access::Plan = syn::parse2(quote! {
        #head_relation(#(#emitted),*) => ((#(#emitted,)*)) :- #(#clauses),*;
    })?;
    access::contract::check(&access)?;
    Ok(Realized { statements, access })
}

struct Realizer<'a> {
    analysis: &'a stream::Analysis,
    chosen: BTreeMap<String, Vec<(structures::Kind, Vec<usize>)>>,
    reprs: BTreeMap<String, Repr>,
    endpoints: &'a Endpoints,
}

fn ident(name: &str) -> Ident {
    syn::parse_str::<Ident>(name).unwrap_or_else(|_| format_ident!("r#{}", name))
}

fn atom(relation: &Ident, args: &[Ident]) -> TokenStream {
    quote!(#relation(#(#args),*))
}

fn pattern_variables(pattern: &syn::Pat) -> Vec<Ident> {
    cq::rust::bindings(pattern)
}

/// A seek binds its payload; with no payload columns it is an existence
/// test, which the Access contract requires to be a guard.
fn seek_clause(atom: TokenStream, payload: &[Ident], source: TokenStream) -> access::Clause {
    if payload.is_empty() {
        syn::parse_quote! {
            #atom => if (::core::iter::Iterator::next(&mut (#source)).is_some())
        }
    } else {
        syn::parse_quote! { #atom => bind (#(#payload,)*) <- (#source) }
    }
}

fn clone_of(variable: &Ident) -> TokenStream {
    quote!(::core::clone::Clone::clone(#variable))
}

impl Realizer<'_> {
    fn build(
        &mut self,
        name: &str,
        fragment: &Fragment,
        provided: &[stream::Operation],
        statements: &mut Vec<Stmt>,
    ) -> syn::Result<Built> {
        let heading: Vec<Ident> = self.analysis.headings[name]
            .iter()
            .map(|h| ident(h))
            .collect();
        let emitted = &fragment.emitted;
        let clauses = &fragment.clauses;
        let label = ident(name);
        let plan: access::Plan = syn::parse2(quote! {
            #label(#(#emitted),*) => ((#(#emitted,)*)) :- #(#clauses),*;
        })?;
        access::contract::check(&plan)?;
        let endpoint = &self.endpoints.support_iter;
        let iterator = quote! { #endpoint! { #plan } };

        let mut built = Built::default();
        let entries = self.chosen.get(name).cloned().unwrap_or_default();
        let needs_rows = provided.contains(&stream::Operation::Iter) || entries.len() > 1;
        let source: TokenStream = if needs_rows {
            let rows = format_ident!("__cq_{}_rows", name);
            statements.push(syn::parse_quote! {
                let #rows: &::std::vec::Vec<_> = &::core::iter::Iterator::collect(#iterator);
            });
            built.rows = Some(rows.clone());
            quote!(::core::iter::Iterator::copied(#rows.iter()))
        } else {
            iterator
        };
        for (kind, columns) in &entries {
            let key: Vec<&Ident> = columns.iter().map(|&i| &heading[i]).collect();
            let payload: Vec<&Ident> = heading
                .iter()
                .enumerate()
                .filter(|(i, _)| !columns.contains(i))
                .map(|(_, v)| v)
                .collect();
            let suffix = columns
                .iter()
                .map(usize::to_string)
                .collect::<Vec<_>>()
                .join("_");
            let row = quote!((#(#heading,)*));
            match kind {
                structures::Kind::Hash(_) => {
                    let structure = format_ident!("__cq_{}_hash_{}", name, suffix);
                    statements.push(syn::parse_quote! {
                        let #structure = &<::cq_storage::HashIndex<_, _> as ::cq_storage::BuildIndex<_>>::build(
                            ::core::iter::Iterator::map(#source, |#row| ((#(#key,)*), (#(#payload,)*)))
                        );
                    });
                    built.seek.insert(columns.clone(), structure);
                }
                structures::Kind::KeySet(_) => {
                    let structure = format_ident!("__cq_{}_key_set_{}", name, suffix);
                    statements.push(syn::parse_quote! {
                        let #structure: &::std::collections::HashSet<_> = &::core::iter::Iterator::collect(
                            ::core::iter::Iterator::map(#source, |#row| (#(#key,)*))
                        );
                    });
                    built.contains.insert(columns.clone(), structure);
                }
                structures::Kind::Ordered(_) => {
                    let structure = format_ident!("__cq_{}_ordered_{}", name, suffix);
                    statements.push(syn::parse_quote! {
                        let #structure = &{
                            let mut __cq_map = ::std::collections::BTreeMap::<_, ::std::vec::Vec<_>>::new();
                            for #row in #source {
                                __cq_map.entry((#(#key,)*)).or_default().push((#(#payload,)*));
                            }
                            __cq_map
                        };
                    });
                    built.ordered.insert(columns.clone(), structure);
                }
            }
        }
        Ok(built)
    }

    fn body(&mut self, body: &stream::Body) -> syn::Result<Fragment> {
        let mut fragment = Fragment::default();
        self.walk(body, &mut fragment)?;
        Ok(fragment)
    }

    fn walk(&mut self, body: &stream::Body, fragment: &mut Fragment) -> syn::Result<()> {
        match body {
            stream::Body::For(f) => {
                let vars = pattern_variables(&f.pattern);
                match &f.cursor {
                    stream::Cursor::Scan {
                        relation,
                        within: None,
                        ..
                    } => {
                        self.scan(relation, &vars, fragment)?;
                    }
                    stream::Cursor::Scan {
                        relation,
                        within: Some(within),
                        ..
                    } => {
                        self.bounded_scan(relation, within, &vars, fragment)?;
                    }
                    stream::Cursor::Seek {
                        relation,
                        columns,
                        keys,
                        ..
                    } => {
                        let keys: Vec<Ident> = keys.variables.iter().cloned().collect();
                        self.seek(relation, &columns.ordinals(), &keys, &vars, fragment)?;
                    }
                    stream::Cursor::Order { relation, .. } => {
                        return Err(syn::Error::new_spanned(relation, "order only inside co"));
                    }
                }
                fragment.bound.extend(vars);
                self.walk(&f.body, fragment)
            }
            stream::Body::Co(c) => {
                let key = pattern_variables(&c.key);
                let mut sides = Vec::new();
                for bound in &c.cursors {
                    let stream::Cursor::Order {
                        relation, columns, ..
                    } = &bound.cursor
                    else {
                        unreachable!()
                    };
                    sides.push((
                        relation.clone(),
                        columns.ordinals(),
                        pattern_variables(&bound.pattern),
                    ));
                }
                self.co(&key, &sides, fragment)?;
                fragment.bound.extend(key);
                for (_, _, payload) in sides {
                    fragment.bound.extend(payload);
                }
                self.walk(&c.body, fragment)
            }
            stream::Body::If(i) => {
                match &i.guard {
                    stream::Guard::Expr(p) => {
                        let condition = &p.expr;
                        let realized = deref_predicates(condition);
                        fragment
                            .clauses
                            .push(syn::parse_quote! { if #condition => if (#realized) });
                    }
                    stream::Guard::Contains {
                        relation,
                        columns,
                        keys,
                        ..
                    } => {
                        let keys: Vec<Ident> = keys.variables.iter().cloned().collect();
                        let test = self.contains(relation, &columns.ordinals(), &keys)?;
                        fragment
                            .clauses
                            .push(syn::parse_quote! { if #test => if (#test) });
                    }
                }
                self.walk(&i.body, fragment)
            }
            stream::Body::Let(l) => {
                let pattern = &l.pattern;
                let annotation = &l.annotation;
                let value = &l.value;
                fragment.clauses.push(syn::parse_quote! {
                    let #pattern #annotation = #value => bind #pattern <- (::core::iter::once(#value))
                });
                fragment.bound.extend(pattern_variables(pattern));
                self.walk(&l.body, fragment)
            }
            stream::Body::Support(s) => {
                fragment.supported = true;
                self.walk(&s.body, fragment)
            }
            stream::Body::Unit(u) => self.walk(&u.body, fragment),
            stream::Body::Emit(e) => {
                fragment.emitted = e.variables.iter().cloned().collect();
                Ok(())
            }
        }
    }

    fn scan(
        &mut self,
        relation: &Ident,
        vars: &[Ident],
        fragment: &mut Fragment,
    ) -> syn::Result<()> {
        let name = symbol_name(relation);
        match &self.reprs[&name] {
            Repr::Host(layout) => {
                // Pattern in declared order; the host iterates its own tuples.
                let mut declared: Vec<Option<&Ident>> = vec![None; vars.len()];
                for (sorted, var) in vars.iter().enumerate() {
                    declared[layout.declared_position[sorted]] = Some(var);
                }
                let declared: Vec<&Ident> = declared.into_iter().map(Option::unwrap).collect();
                let value = &layout.value;
                let atom = atom(
                    &layout.value,
                    &declared.iter().map(|v| (*v).clone()).collect::<Vec<_>>(),
                );
                fragment.clauses.push(syn::parse_quote! {
                    #atom => bind (#(#declared,)*) <- (#value.iter())
                });
                Ok(())
            }
            Repr::Built(built) => {
                let rows = built
                    .rows
                    .as_ref()
                    .expect("scan requires iter, which built rows");
                let atom = atom(relation, vars);
                fragment.clauses.push(syn::parse_quote! {
                    #atom => bind (#(#vars,)*) <- (::core::iter::Iterator::copied(#rows.iter()))
                });
                Ok(())
            }
            Repr::Pipe(_) => {
                let Some(Repr::Pipe(producer)) = self.reprs.remove(&name) else {
                    unreachable!()
                };
                // Inline: the producer's emitted names become the consumer's
                // pattern names; its other bindings are made private to it.
                let mut renames: BTreeMap<String, Ident> = BTreeMap::new();
                for (from, to) in producer.emitted.iter().zip(vars) {
                    renames.insert(symbol_name(from), to.clone());
                }
                for bound in &producer.bound {
                    let key = symbol_name(bound);
                    if !renames.contains_key(&key) {
                        renames.insert(key.clone(), format_ident!("{}__{}", bound, name));
                    }
                }
                for mut clause in producer.clauses {
                    Rename(&renames).visit_clause(&mut clause);
                    fragment.clauses.push(clause);
                }
                fragment.supported |= producer.supported;
                Ok(())
            }
        }
    }

    /// `seek(lo)` then every pair while `key <= hi`, over an ordered host.
    fn bounded_scan(
        &mut self,
        relation: &Ident,
        within: &stream::Within,
        vars: &[Ident],
        fragment: &mut Fragment,
    ) -> syn::Result<()> {
        let name = symbol_name(relation);
        let column = within.column.index as usize;
        let arity = self.analysis.headings[&name].len();
        let Repr::Host(layout) = &self.reprs[&name] else {
            return Err(syn::Error::new_spanned(
                relation,
                "bounded scans are realized over ordered host relations",
            ));
        };
        let value = &layout.value;
        let payload_sorted: Vec<usize> = (0..arity).filter(|&i| i != column).collect();
        // Payload tuple fields are in the host's declared order.
        let mut declared: Vec<usize> = payload_sorted
            .iter()
            .map(|&s| layout.declared_position[s])
            .collect();
        declared.sort_unstable();
        let mut fields: Vec<Option<TokenStream>> = vec![None; arity];
        fields[layout.declared_position[column]] = Some(quote!(&__cq_k.0));
        for &sorted in &payload_sorted {
            let rank = declared
                .iter()
                .position(|&d| d == layout.declared_position[sorted])
                .unwrap();
            let index = syn::Index::from(rank);
            fields[layout.declared_position[sorted]] = Some(quote!(&__cq_v.#index));
        }
        let fields: Vec<TokenStream> = fields.into_iter().map(Option::unwrap).collect();
        let mut all: Vec<Option<Ident>> = vec![None; arity];
        for (sorted, var) in vars.iter().enumerate() {
            all[layout.declared_position[sorted]] = Some(var.clone());
        }
        let all: Vec<Ident> = all.into_iter().map(Option::unwrap).collect();
        let seek = within.low.value().map(|low| {
            quote! { ::cq_storage::OrderedCursor::seek(&mut __cq_cursor, &&(#low,)); }
        });
        let stop = match within.high.value() {
            Some(high) => quote! { |(__cq_k, _)| **__cq_k <= (#high,) },
            None => quote! { |_| true },
        };
        let atom = atom(&layout.value, &all);
        fragment.clauses.push(syn::parse_quote! {
            #atom => bind (#(#all,)*) <- ({
                let mut __cq_cursor = ::cq_storage::OrderedRead::ordered(&#value);
                #seek
                ::core::iter::Iterator::map(
                    ::core::iter::Iterator::take_while(__cq_cursor, #stop),
                    |(__cq_k, __cq_v)| (#(#fields,)*)
                )
            })
        });
        Ok(())
    }

    fn seek(
        &mut self,
        relation: &Ident,
        columns: &[usize],
        keys: &[Ident],
        payload: &[Ident],
        fragment: &mut Fragment,
    ) -> syn::Result<()> {
        let name = symbol_name(relation);
        let arity = self.analysis.headings[&name].len();
        match &self.reprs[&name] {
            Repr::Host(layout) => {
                // Sorted ordinals of the key and payload columns, then the
                // host's declared order for both tuples.
                let payload_sorted: Vec<usize> =
                    (0..arity).filter(|i| !columns.contains(i)).collect();
                let by_declared = |sorted: &[usize], vars: &[Ident]| -> Vec<Ident> {
                    let mut pairs: Vec<(usize, &Ident)> = sorted
                        .iter()
                        .zip(vars)
                        .map(|(&s, v)| (layout.declared_position[s], v))
                        .collect();
                    pairs.sort_by_key(|(d, _)| *d);
                    pairs.into_iter().map(|(_, v)| v.clone()).collect()
                };
                let key_declared = by_declared(columns, keys);
                let payload_declared = by_declared(&payload_sorted, payload);
                let mut all: Vec<Option<Ident>> = vec![None; arity];
                for (s, v) in columns.iter().zip(keys) {
                    all[layout.declared_position[*s]] = Some(v.clone());
                }
                for (s, v) in payload_sorted.iter().zip(payload) {
                    all[layout.declared_position[*s]] = Some(v.clone());
                }
                let all: Vec<Ident> = all.into_iter().map(Option::unwrap).collect();
                let value = &layout.value;
                let key_clones = key_declared.iter().map(clone_of);
                let atom = atom(&layout.value, &all);
                let source = quote!(::cq_storage::IndexRead::lookup(&&#value, &(#(#key_clones,)*)));
                fragment
                    .clauses
                    .push(seek_clause(atom, &payload_declared, source));
                Ok(())
            }
            Repr::Built(built) => {
                let structure = built
                    .seek
                    .get(columns)
                    .or_else(|| built.ordered.get(columns));
                let Some(structure) = structure else {
                    return Err(syn::Error::new_spanned(
                        relation,
                        "no structure serves this seek",
                    ));
                };
                let mut all: Vec<Option<Ident>> = vec![None; arity];
                for (s, v) in columns.iter().zip(keys) {
                    all[*s] = Some(v.clone());
                }
                let payload_sorted: Vec<usize> =
                    (0..arity).filter(|i| !columns.contains(i)).collect();
                for (s, v) in payload_sorted.iter().zip(payload) {
                    all[*s] = Some(v.clone());
                }
                let all: Vec<Ident> = all.into_iter().map(Option::unwrap).collect();
                let atom = atom(relation, &all);
                let source = if built.seek.contains_key(columns) {
                    quote!(::cq_storage::IndexRead::lookup(&#structure, &(#(#keys,)*)))
                } else {
                    // An ordered map answers seek by exact key too.
                    quote!(::core::iter::Iterator::copied(::core::iter::IntoIterator::into_iter(
                        ::core::option::Option::map_or(#structure.get(&(#(#keys,)*)), &[][..], |v| v.as_slice())
                    )))
                };
                fragment.clauses.push(seek_clause(atom, payload, source));
                Ok(())
            }
            Repr::Pipe(_) => Err(syn::Error::new_spanned(relation, "a pipe cannot be seeked")),
        }
    }

    fn contains(&self, relation: &Ident, columns: &[usize], keys: &[Ident]) -> syn::Result<Expr> {
        let name = symbol_name(relation);
        match &self.reprs[&name] {
            Repr::Host(layout) => {
                let mut pairs: Vec<(usize, &Ident)> = columns
                    .iter()
                    .zip(keys)
                    .map(|(&s, v)| (layout.declared_position[s], v))
                    .collect();
                pairs.sort_by_key(|(d, _)| *d);
                let key_clones = pairs.iter().map(|(_, v)| clone_of(v));
                let value = &layout.value;
                Ok(syn::parse_quote! {
                    ::cq_storage::ExactMembership::contains(&#value, &(#(#key_clones,)*))
                })
            }
            Repr::Built(built) => {
                if let Some(structure) = built.contains.get(columns) {
                    return Ok(syn::parse_quote! {
                        ::cq_storage::ExactMembership::contains(&#structure, &(#(#keys,)*))
                    });
                }
                if let Some(structure) = built.seek.get(columns) {
                    return Ok(syn::parse_quote! {
                        ::core::iter::Iterator::next(&mut ::cq_storage::IndexRead::lookup(&#structure, &(#(#keys,)*))).is_some()
                    });
                }
                if let Some(structure) = built.ordered.get(columns) {
                    return Ok(syn::parse_quote! { #structure.contains_key(&(#(#keys,)*)) });
                }
                Err(syn::Error::new_spanned(
                    relation,
                    "no structure serves this membership test",
                ))
            }
            Repr::Pipe(_) => Err(syn::Error::new_spanned(relation, "a pipe cannot be tested")),
        }
    }

    fn co(
        &mut self,
        key: &[Ident],
        sides: &[(Ident, Vec<usize>, Vec<Ident>)],
        fragment: &mut Fragment,
    ) -> syn::Result<()> {
        if sides.len() != 2 {
            return Err(syn::Error::new_spanned(
                &sides[0].0,
                "co-iteration is realized for exactly two cursors",
            ));
        }
        let mut cursors = Vec::new();
        // For each side: the ordered cursor expression, and how to read its
        // key and payload tuples into sorted-order variables.
        let mut key_from_left = TokenStream::new();
        let mut payload_reads: Vec<TokenStream> = Vec::new();
        for (index, (relation, columns, payload)) in sides.iter().enumerate() {
            let name = symbol_name(relation);
            let arity = self.analysis.headings[&name].len();
            let payload_sorted: Vec<usize> = (0..arity).filter(|i| !columns.contains(i)).collect();
            let side = if index == 0 {
                format_ident!("__cq_l")
            } else {
                format_ident!("__cq_r")
            };
            match &self.reprs[&name] {
                Repr::Host(layout) => {
                    let value = &layout.value;
                    cursors.push(quote!(::cq_storage::OrderedRead::ordered(&#value)));
                    // Host tuples are in declared order of their columns.
                    let declared_rank = |sorted: &[usize]| -> Vec<usize> {
                        let mut declared: Vec<usize> = sorted
                            .iter()
                            .map(|&s| layout.declared_position[s])
                            .collect();
                        declared.sort_unstable();
                        sorted
                            .iter()
                            .map(|&s| {
                                declared
                                    .iter()
                                    .position(|&d| d == layout.declared_position[s])
                                    .unwrap()
                            })
                            .collect()
                    };
                    if index == 0 {
                        let ranks = declared_rank(columns);
                        let fields = ranks.iter().map(|&r| syn::Index::from(r));
                        key_from_left = quote!(#(&__cq_k.#fields,)*);
                    }
                    let ranks = declared_rank(&payload_sorted);
                    let fields = ranks.iter().map(|&r| syn::Index::from(r));
                    payload_reads.push(quote!(#(&#side.#fields,)*));
                }
                Repr::Built(built) => {
                    let Some(structure) = built.ordered.get(columns) else {
                        return Err(syn::Error::new_spanned(
                            relation,
                            "no ordered structure serves this co-iteration",
                        ));
                    };
                    cursors.push(quote!(::cq_storage::OrderedRead::ordered(&#structure)));
                    if index == 0 {
                        let fields = (0..columns.len()).map(syn::Index::from);
                        key_from_left = quote!(#(__cq_k.#fields,)*);
                    }
                    let fields = (0..payload_sorted.len()).map(syn::Index::from);
                    payload_reads.push(quote!(#(#side.#fields,)*));
                }
                Repr::Pipe(_) => {
                    return Err(syn::Error::new_spanned(
                        relation,
                        "a pipe cannot be co-iterated",
                    ));
                }
            }
            let _ = payload;
        }
        let (left, right) = (&cursors[0], &cursors[1]);
        let (left_read, right_read) = (&payload_reads[0], &payload_reads[1]);
        let vars: Vec<Ident> = key
            .iter()
            .chain(&sides[0].2)
            .chain(&sides[1].2)
            .cloned()
            .collect();
        let label = format_ident!("__cq_co_{}_{}", sides[0].0, sides[1].0);
        let atom = atom(&label, &vars);
        fragment.clauses.push(syn::parse_quote! {
            #atom => bind (#(#vars,)*) <- (
                ::core::iter::Iterator::map(
                    ::cq_storage::merge(#left, #right),
                    |(__cq_k, __cq_l, __cq_r)| (#key_from_left #left_read #right_read)
                )
            )
        });
        Ok(())
    }
}

/// Bound variables are references; the total comparison fragment is written
/// over values, so its variables are dereferenced here. Other Rust is the
/// author's and is emitted verbatim.
fn deref_predicates(expr: &Expr) -> Expr {
    use cq::{Predicate, Term};
    if let Expr::Binary(syn::ExprBinary {
        op: syn::BinOp::And(and),
        left,
        right,
        ..
    }) = cq::rust::unparen(expr)
    {
        let (left, right) = (deref_predicates(left), deref_predicates(right));
        return syn::parse_quote!((#left) #and (#right));
    }
    let Some(predicate) = Predicate::from_expr(expr) else {
        return expr.clone();
    };
    let term = |term: &Term| -> Expr {
        match term {
            Term::Variable(v) => syn::parse_quote!(*#v),
            Term::Integer(i) => syn::parse_quote!(#i),
        }
    };
    let (left, right, comparison) = (
        term(&predicate.left),
        term(&predicate.right),
        predicate.comparison,
    );
    syn::parse_quote!(#left #comparison #right)
}

/// Rename variables inside an inlined clause: pattern bindings, atom
/// arguments, and single-segment paths in Rust expressions.
struct Rename<'a>(&'a BTreeMap<String, Ident>);

impl Rename<'_> {
    fn visit_clause(&mut self, clause: &mut access::Clause) {
        match &mut clause.item {
            cq::BodyItem::Positive { atom } => {
                for arg in &mut atom.args {
                    self.visit_expr_mut(arg);
                }
            }
            cq::BodyItem::Filter(filter) => self.visit_expr_mut(&mut filter.condition),
            cq::BodyItem::Let(binding) => {
                self.visit_pat_mut(&mut binding.pattern);
                self.visit_expr_mut(&mut binding.expression);
            }
            cq::BodyItem::Negation(_) | cq::BodyItem::Aggregate(_) => {}
        }
        match &mut clause.access {
            access::RustAccess::Bind {
                pattern, source, ..
            } => {
                self.visit_pat_mut(pattern);
                self.visit_expr_mut(source);
            }
            access::RustAccess::If { condition, .. } => self.visit_expr_mut(condition),
        }
    }
}

impl VisitMut for Rename<'_> {
    fn visit_expr_path_mut(&mut self, path: &mut syn::ExprPath) {
        if path.qself.is_none() && path.path.segments.len() == 1 {
            let ident = &mut path.path.segments[0].ident;
            if let Some(renamed) = self.0.get(&symbol_name(ident)) {
                *ident = renamed.clone();
            }
        }
    }

    fn visit_pat_ident_mut(&mut self, pattern: &mut syn::PatIdent) {
        if let Some(renamed) = self.0.get(&symbol_name(&pattern.ident)) {
            pattern.ident = renamed.clone();
        }
        if let Some((_, sub)) = &mut pattern.subpat {
            self.visit_pat_mut(sub);
        }
    }
}
