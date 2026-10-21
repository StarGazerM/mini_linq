//! Structural branch boundaries, negative references and the surrounding CQ.
//! Named occurrence paths fix branch membership. A branch is checked as a
//! subprogram with a symbolic context; the surrounding program sees its checked
//! interface. Relational semijoins may restrict positive arms through a named
//! operand of that surrounding program, on interface/context columns only.
//! Negative references are never eligible for that erasure.

use super::*;
use std::collections::{BTreeMap, BTreeSet};

fn frames(schedule: &Schedule) -> Result<BTreeMap<String, usize>, String> {
    let mut result = BTreeMap::new();
    for (index, node) in schedule.nodes.iter().enumerate() {
        if !matches!(node.kind, Kind::Union(..)) {
            continue;
        }
        let origin = node.origin.as_ref().ok_or("union has no block origin")?;
        if result
            .insert(origin.occurrence.to_string(), index)
            .is_some()
        {
            return Err("a block origin is defined more than once".into());
        }
    }
    Ok(result)
}

pub(super) fn check(source: &Schedule, target: &Schedule) -> Result<(), String> {
    if source.paths != target.paths {
        return Err("block occurrence paths differ".into());
    }
    if source
        .nodes
        .iter()
        .chain(&target.nodes)
        .any(|node| matches!(node.kind, Kind::Union(..)) && node.origin.is_none())
    {
        let mut intern = HashMap::new();
        return if wiring(source, &mut intern)? == wiring(target, &mut intern)? {
            Ok(())
        } else {
            Err("unannotated union wiring differs".into())
        };
    }
    let (sf, tf) = (frames(source)?, frames(target)?);
    if sf.keys().collect::<Vec<_>>() != tf.keys().collect::<Vec<_>>() {
        return Err("dropped or added block".into());
    }
    let mut source_root = View::new(source, None, BTreeMap::new(), None);
    let mut target_root = View::new(target, None, BTreeMap::new(), None);
    source_root.finish(
        source.output.ok_or("source has no output")?,
        source.head.clone(),
    )?;
    target_root.finish(
        target.output.ok_or("target has no output")?,
        target.head.clone(),
    )?;
    compare(&source_root.result, &target_root.result)?;
    for (path, si) in sf {
        let ti = tf[&path];
        let (s, t) = (&source.nodes[si], &target.nodes[ti]);
        let (so, to) = (s.origin.as_ref().unwrap(), t.origin.as_ref().unwrap());
        if so.occurrence != to.occurrence
            || so.free != to.free
            || so.interface != to.interface
            || so.fallback.is_some() != to.fallback.is_some()
            || s.operands.len() != t.operands.len()
        {
            return Err(format!(
                "block @{path} changes its operator, heading or branch count"
            ));
        }
        let (sn, tn) = (
            originals(source, s, None)?,
            originals(target, t, Some(&target_root.memo))?,
        );
        let (sc, tc) = (s.correlation.or(s.domain), t.correlation.or(t.domain));
        if so.fallback.is_some() {
            reject_negative_context_cuts(source, &sn, sc, &source_root.memo)?;
            reject_negative_context_cuts(target, &tn, tc, &target_root.memo)?;
        }
        unique_branch_rust(source, &sn, sc, &so.occurrence)?;
        unique_branch_rust(target, &tn, tc, &to.occurrence)?;
        let pinned = has_rust(source, &sn, sc, &so.occurrence);
        if pinned != has_rust(target, &tn, tc, &to.occurrence) {
            return Err(format!("block @{path} changes its Rust tree"));
        }
        if pinned {
            let (sp, tp) = (
                s.correlation.ok_or("Rust block lacks its prefix")?,
                t.correlation.ok_or("Rust block lacks its prefix")?,
            );
            let source_names: BTreeSet<_> = source.nodes[sp].heading.keys().cloned().collect();
            let target_names: BTreeSet<_> = target.nodes[tp].heading.keys().cloned().collect();
            if !source_names.is_subset(&target_names)
                || to
                    .interface
                    .iter()
                    .any(|id| target_names.contains(&symbol_name(id)))
            {
                return Err(format!(
                    "block @{path} changes its prefix scope or binds an interface column outside the block"
                ));
            }
            // The surrounding proof checks every source Before fact at the
            // block's opaque boundary. Extra relational prefix facts may shrink
            // the arm domain; the source lexical scope remains fixed.
        } else if s.domain.is_some() || t.domain.is_some() {
            // A projected finite context domain is an explicit dependency.
            // Preserve its proof independently from branch-local constraints.
            if let (Some(sd), Some(td)) = (s.domain, t.domain) {
                let mut sv = View::new(source, None, BTreeMap::new(), None);
                let mut tv = View::new(target, None, BTreeMap::new(), None);
                sv.finish(sd, format!("domain @{path}"))?;
                tv.finish(td, format!("domain @{path}"))?;
                compare(&sv.result, &tv.result)?;
            }
        }
        let context_names: BTreeSet<_> = if pinned {
            source.nodes[sc.unwrap()].heading.keys().cloned().collect()
        } else {
            so.free.iter().map(symbol_name).collect()
        };
        let mut heading = context_names.clone();
        heading.extend(so.interface.iter().map(symbol_name));
        let signature = format!("branch @{path} {heading:?}");
        let negative_names = |roots: &[usize]| {
            roots
                .iter()
                .enumerate()
                .map(|(j, node)| (*node, format!("@{path}.{j}")))
                .collect::<BTreeMap<_, _>>()
        };
        let mut source_views = Vec::new();
        let mut target_views = Vec::new();
        for operand in &s.operands {
            let mut view = View::new(
                source,
                sc.map(|node| (node, path.clone(), context_names.clone())),
                negative_names(&sn),
                None,
            );
            view.branch(
                *operand,
                &heading,
                &context_names,
                &path,
                sc.is_none(),
                signature.clone(),
            )?;
            source_views.push(view.result);
        }
        for operand in &t.operands {
            let mut view = View::new(
                target,
                tc.map(|node| (node, path.clone(), context_names.clone())),
                negative_names(&tn),
                Some(&target_root.memo),
            );
            view.branch(
                *operand,
                &heading,
                &context_names,
                &path,
                tc.is_none(),
                signature.clone(),
            )?;
            target_views.push(view.result);
        }
        if so.fallback.is_some() {
            for (j, (sv, tv)) in source_views.iter().zip(&target_views).enumerate() {
                compare(sv, tv).map_err(|e| format!("else block @{path} branch {j}: {e}"))?;
            }
        } else {
            // Exact occurrence/fact keys identify unordered siblings. This is
            // lookup of given branch identities, never an atom-mapping search.
            let mut candidates: BTreeMap<Footprint, Vec<&Schedule>> = BTreeMap::new();
            for tv in &target_views {
                candidates.entry(footprint(tv)).or_default().push(tv);
            }
            for (j, sv) in source_views.iter().enumerate() {
                let tv = candidates
                    .get_mut(&footprint(sv))
                    .and_then(Vec::pop)
                    .ok_or_else(|| format!("union block @{path} dropped or changed branch {j}"))?;
                compare(sv, tv).map_err(|e| format!("union block @{path} branch {j}: {e}"))?;
            }
        }
    }
    Ok(())
}

fn reject_negative_context_cuts(
    schedule: &Schedule,
    roots: &[usize],
    boundary: Option<usize>,
    context: &HashMap<usize, usize>,
) -> Result<(), String> {
    for root in roots.iter().take(roots.len().saturating_sub(1)) {
        let mut seen = HashSet::new();
        let mut pending = vec![*root];
        while let Some(index) = pending.pop() {
            if boundary == Some(index) || !seen.insert(index) {
                continue;
            }
            let node = &schedule.nodes[index];
            if matches!(node.kind, Kind::Join(true))
                && (context.contains_key(&node.operands[1]) || boundary == Some(node.operands[1]))
            {
                return Err(
                    "surrounding semijoin step was pushed into a negated original branch".into(),
                );
            }
            let count = if matches!(node.kind, Kind::Anti(_)) {
                1
            } else {
                node.operands.len()
            };
            pending.extend(node.operands.iter().take(count));
        }
    }
    Ok(())
}

fn unique_branch_rust(
    schedule: &Schedule,
    roots: &[usize],
    boundary: Option<usize>,
    block: &cq_syntax::Occurrence,
) -> Result<(), String> {
    let mut owners = HashSet::new();
    for root in roots {
        let mut seen = HashSet::new();
        let mut pending = vec![*root];
        while let Some(node) = pending.pop() {
            if boundary == Some(node) || !seen.insert(node) {
                continue;
            }
            if branch_rust(schedule, node, block) && !owners.insert(node) {
                return Err("a Rust step is shared between sibling branch paths".into());
            }
            let count = if matches!(schedule.nodes[node].kind, Kind::Anti(_)) {
                1
            } else {
                schedule.nodes[node].operands.len()
            };
            pending.extend(schedule.nodes[node].operands.iter().take(count));
        }
    }
    Ok(())
}

fn branch_rust(schedule: &Schedule, node: usize, block: &cq_syntax::Occurrence) -> bool {
    schedule.nodes[node].rust_index.is_some_and(|index| {
        schedule.chain[index]
            .environment
            .as_ref()
            .and_then(|env| env.path.as_ref())
            .is_none_or(|path| path.0.starts_with(&block.0))
    })
}

fn has_rust(
    schedule: &Schedule,
    roots: &[usize],
    boundary: Option<usize>,
    block: &cq_syntax::Occurrence,
) -> bool {
    let mut seen = HashSet::new();
    let mut pending = roots.to_vec();
    while let Some(node) = pending.pop() {
        if boundary == Some(node) || !seen.insert(node) {
            continue;
        }
        if branch_rust(schedule, node, block) {
            return true;
        }
        pending.extend(&schedule.nodes[node].operands);
    }
    false
}

fn foreign_semi(
    schedule: &Schedule,
    node: usize,
    allowed: &HashMap<usize, usize>,
    heading: &Heading,
) -> bool {
    let n = &schedule.nodes[node];
    if !matches!(n.kind, Kind::Join(true)) || !allowed.contains_key(&n.operands[1]) {
        return false;
    }
    let (left, right) = (
        &schedule.nodes[n.operands[0]],
        &schedule.nodes[n.operands[1]],
    );
    left.heading
        .keys()
        .filter(|name| right.heading.contains_key(*name))
        .all(|name| heading.contains_key(name))
}
fn originals(
    schedule: &Schedule,
    frame: &Node,
    allowed: Option<&HashMap<usize, usize>>,
) -> Result<Vec<usize>, String> {
    let fallback = frame.origin.as_ref().unwrap().fallback.is_some();
    let mut roots = Vec::new();
    for operand in &frame.operands {
        let mut node = *operand;
        loop {
            let n = &schedule.nodes[node];
            if (fallback && frame.correlation.is_none() && matches!(n.kind, Kind::Anti(_)))
                || allowed.is_some_and(|a| foreign_semi(schedule, node, a, &frame.heading))
            {
                node = n.operands[0];
            } else {
                break;
            }
        }
        roots.push(node);
    }
    Ok(roots)
}

#[derive(PartialEq, Eq, PartialOrd, Ord)]
struct Footprint {
    atoms: Vec<usize>,
    predicates: Vec<(String, Vec<Column>)>,
    rust: Vec<String>,
}
fn footprint(schedule: &Schedule) -> Footprint {
    let mut atoms: Vec<_> = schedule.atoms.keys().copied().collect();
    atoms.sort_unstable();
    let columns: BTreeSet<_> = schedule
        .nodes
        .iter()
        .flat_map(|n| n.heading.values().cloned())
        .collect();
    let columns: BTreeMap<_, _> = columns
        .into_iter()
        .enumerate()
        .map(|(i, c)| (c, i))
        .collect();
    let mut equality = UnionFind::new(columns.len());
    for node in &schedule.nodes {
        for (a, b) in &node.equalities {
            equality.union(columns[a], columns[b]);
        }
    }
    let mut representatives = BTreeMap::new();
    for (column, id) in &columns {
        let label = equality.root(*id);
        representatives
            .entry(label)
            .or_insert_with(|| column.clone());
    }
    let mut predicates: Vec<_> = schedule
        .nodes
        .iter()
        .flat_map(|n| n.predicates.iter())
        .map(|p| {
            (
                p.shape.clone(),
                p.columns
                    .iter()
                    .map(|c| representatives[&equality.root(columns[c])].clone())
                    .collect(),
            )
        })
        .collect();
    predicates.sort();
    predicates.dedup();
    let mut rust: Vec<_> = schedule.chain.iter().map(|r| r.payload.clone()).collect();
    rust.sort();
    Footprint {
        atoms,
        predicates,
        rust,
    }
}

struct View<'a> {
    original: &'a Schedule,
    result: Schedule,
    memo: HashMap<usize, usize>,
    columns: HashMap<Column, Column>,
    boundary: Option<(usize, String, BTreeSet<String>)>,
    negatives: BTreeMap<usize, String>,
    allowed: Option<&'a HashMap<usize, usize>>,
    common: Heading,
}
impl<'a> View<'a> {
    fn new(
        original: &'a Schedule,
        boundary: Option<(usize, String, BTreeSet<String>)>,
        negatives: BTreeMap<usize, String>,
        allowed: Option<&'a HashMap<usize, usize>>,
    ) -> Self {
        Self {
            original,
            result: Schedule {
                paths: original.paths.clone(),
                row_views: original.row_views.clone(),
                ..Schedule::default()
            },
            memo: HashMap::new(),
            columns: HashMap::new(),
            boundary,
            negatives,
            allowed,
            common: Heading::new(),
        }
    }
    fn column(&self, column: &Column) -> Column {
        self.columns.get(column).unwrap_or(column).clone()
    }
    fn context(&mut self, label: &str, names: &BTreeSet<String>) -> usize {
        self.result.push(Node {
            heading: names
                .iter()
                .map(|name| {
                    (
                        name.clone(),
                        Column::Context(label.to_owned(), name.clone()),
                    )
                })
                .collect(),
            origins: names
                .iter()
                .map(|name| {
                    (
                        name.clone(),
                        HashSet::from([Column::Context(label.to_owned(), name.clone())]),
                    )
                })
                .collect(),
            ..Node::default()
        })
    }
    fn branch(
        &mut self,
        output: usize,
        heading: &BTreeSet<String>,
        context: &BTreeSet<String>,
        label: &str,
        seed: bool,
        signature: String,
    ) -> Result<(), String> {
        self.common = heading
            .iter()
            .map(|name| {
                (
                    name.clone(),
                    Column::Context(label.to_owned(), name.clone()),
                )
            })
            .collect();
        let mut input = self.node(output)?;
        if seed && !context.is_empty() {
            let ctx = self.context(label, context);
            input = self.result.join(ctx, input, false);
        }
        let attributes = heading
            .iter()
            .map(|name| Ident::new(name, proc_macro2::Span::call_site()))
            .collect::<Vec<_>>();
        input = self.result.project(input, &attributes);
        self.result.output = Some(input);
        self.result.head = signature;
        self.finish_views();
        Ok(())
    }
    fn finish(&mut self, output: usize, head: String) -> Result<(), String> {
        self.result.output = Some(self.node(output)?);
        self.result.head = head;
        self.finish_views();
        Ok(())
    }
    fn finish_views(&mut self) {
        let ids: HashSet<_> = self.result.atoms.keys().copied().collect();
        let views: HashSet<_> = self
            .result
            .nodes
            .iter()
            .flat_map(|n| n.views.values().copied())
            .collect();
        self.result.view_requests.extend(
            self.original
                .view_requests
                .iter()
                .filter(|(a, b, _)| views.contains(a) && views.contains(b))
                .copied(),
        );
        self.result
            .split_nodes
            .extend(self.original.split_nodes.iter().filter_map(|(n, a)| {
                self.memo
                    .get(n)
                    .filter(|_| ids.contains(a))
                    .map(|m| (*m, *a))
            }));
        self.result
            .full_reads
            .extend(self.original.full_reads.iter().filter_map(|(n, a)| {
                self.memo
                    .get(n)
                    .filter(|_| ids.contains(a))
                    .map(|m| (*m, *a))
            }));
    }
    fn node(&mut self, index: usize) -> Result<usize, String> {
        if let Some(node) = self.memo.get(&index) {
            return Ok(*node);
        }
        if let Some((boundary, label, names)) =
            self.boundary.clone().filter(|(node, _, _)| *node == index)
        {
            let _ = boundary;
            let node = self.context(&label, &names);
            for (name, column) in &self.original.nodes[index].heading {
                self.columns
                    .insert(column.clone(), Column::Context(label.clone(), name.clone()));
            }
            self.memo.insert(index, node);
            return Ok(node);
        }
        let original = &self.original.nodes[index];
        if let Kind::Atom(id) = original.kind {
            // A collapsed block may expose this occurrence's free columns
            // before its actual atom is visited. That symbolic interface must
            // not replace the atom's own provenance: the subsequent join
            // records their equality explicitly.
            for column in self.original.atoms[&id].bindings.keys() {
                self.columns.remove(&Column::Atom(id, column.clone()));
            }
        }
        if self
            .allowed
            .is_some_and(|allowed| foreign_semi(self.original, index, allowed, &self.common))
        {
            let node = self.node(original.operands[0])?;
            self.memo.insert(index, node);
            return Ok(node);
        }
        if matches!(original.kind, Kind::Union(..)) {
            return self.block(index);
        }
        let mut node = original.clone();
        let mut operands = Vec::new();
        let mut negative = None;
        for (j, operand) in original.operands.iter().enumerate() {
            if j == 1 && matches!(original.kind, Kind::Anti(_)) {
                let label = self
                    .negatives
                    .get(operand)
                    .ok_or("negative operand must reference the original named branch result")?
                    .clone();
                negative = Some(label.clone());
                let heading = self.original.nodes[*operand]
                    .heading
                    .keys()
                    .map(|name| (name.clone(), Column::Negative(label.clone(), name.clone())))
                    .collect();
                operands.push(self.result.push(Node {
                    kind: Kind::Negative(label),
                    heading,
                    ..Node::default()
                }));
            } else {
                operands.push(self.node(*operand)?);
            }
        }
        node.operands = operands;
        if let Some(rust_index) = original.rust_index {
            let rust = &self.original.chain[rust_index];
            let ordinal = self.result.chain.len();
            for column in original.heading.values() {
                if let Column::Bind(id, name) = column {
                    if *id == rust_index {
                        self.columns
                            .insert(column.clone(), Column::Bind(ordinal, name.clone()));
                    }
                }
            }
            self.result.chain.push(Rust {
                payload: rust.payload.clone(),
                input: node.operands[0],
                arguments: rust
                    .arguments
                    .iter()
                    .map(|(name, column)| (name.clone(), self.column(column)))
                    .collect(),
                expression: rust.expression.clone(),
                bindings: rust.bindings.clone(),
                environment: rust.environment.clone(),
            });
            node.rust_index = Some(ordinal);
        }
        node.heading = original
            .heading
            .iter()
            .map(|(name, column)| (name.clone(), self.column(column)))
            .collect();
        node.origins = original
            .origins
            .iter()
            .map(|(name, columns)| {
                (
                    name.clone(),
                    columns.iter().map(|c| self.column(c)).collect(),
                )
            })
            .collect();
        node.equalities = original
            .equalities
            .iter()
            .map(|(a, b)| (self.column(a), self.column(b)))
            .collect();
        node.predicates = original
            .predicates
            .iter()
            .map(|p| Predicate {
                shape: p.shape.clone(),
                columns: p.columns.iter().map(|c| self.column(c)).collect(),
            })
            .collect();
        node.reducer = original.reducer.as_ref().and_then(|reducer| {
            Some(Reduction {
                left: reducer.left,
                right: reducer.right,
                left_node: *self.memo.get(&reducer.left_node)?,
                join_node: *self.memo.get(&reducer.join_node)?,
                node: self
                    .memo
                    .get(&reducer.node)
                    .copied()
                    .unwrap_or(self.result.nodes.len()),
                equalities: reducer
                    .equalities
                    .iter()
                    .map(|(a, b)| (self.column(a), self.column(b)))
                    .collect(),
            })
        });
        if matches!(node.kind, Kind::Select) {
            let input = &self.result.nodes[node.operands[0]];
            node.views = input.views.clone();
            node.pruned = input.pruned.clone();
            node.reducer = input.reducer.clone();
            node.owner = input
                .owner
                .filter(|owner| self.result.atoms.contains_key(owner));
            if let Some(reducer) = &mut node.reducer {
                reducer.equalities.extend(node.equalities.iter().cloned());
            }
            if !node.equalities.is_empty() || !node.predicates.is_empty() {
                if let Some(owner) = node.owner {
                    let full = node
                        .heading
                        .values()
                        .filter(|c| matches!(c,Column::Atom(id,_) if *id==owner))
                        .count()
                        == self.result.atoms[&owner].bindings.len();
                    if full {
                        let parent = node.views[&owner];
                        node.views.insert(
                            owner,
                            row_view(&mut self.result.row_views, Some(parent), true),
                        );
                    }
                }
            }
        }
        node.range_base = original.range_base.and_then(|n| self.memo.get(&n).copied());
        node.owner = original.owner.filter(|id| {
            self.result.atoms.contains_key(id)
                || matches!(original.kind,Kind::Atom(own) if own == *id)
        });
        node.views.retain(|id, _| {
            self.result.atoms.contains_key(id)
                || matches!(original.kind,Kind::Atom(own) if own == *id)
        });
        node.pruned.retain(|id| self.result.atoms.contains_key(id));
        if let Some(label) = negative {
            let Kind::Anti(columns) = &node.kind else {
                unreachable!("anti reference")
            };
            node.predicates.push(Predicate {
                shape: format!("anti {columns:?} {label}"),
                columns: columns
                    .iter()
                    .map(|name| self.result.nodes[node.operands[0]].heading[name].clone())
                    .collect(),
            });
        }
        let result = match &node.kind {
            Kind::Project => {
                let attributes = node
                    .heading
                    .keys()
                    .filter(|name| {
                        self.result.nodes[node.operands[0]]
                            .heading
                            .contains_key(*name)
                    })
                    .map(|name| Ident::new(name, proc_macro2::Span::call_site()))
                    .collect::<Vec<_>>();
                self.result.project(node.operands[0], &attributes)
            }
            Kind::Join(semi) => self.result.join(node.operands[0], node.operands[1], *semi),
            _ => self.result.push(node),
        };
        self.memo.insert(index, result);
        if let Kind::Atom(id) = original.kind {
            let atom = &self.original.atoms[&id];
            self.result.atoms.insert(
                id,
                Atom {
                    relation: atom.relation.clone(),
                    bindings: atom.bindings.clone(),
                    constants: atom.constants.clone(),
                    node: result,
                },
            );
        }
        Ok(result)
    }
    fn block(&mut self, index: usize) -> Result<usize, String> {
        let original = &self.original.nodes[index];
        let origin = original.origin.as_ref().ok_or("block origin missing")?;
        let path = origin.occurrence.to_string();
        let roots = originals(self.original, original, self.allowed)?;
        let rust = has_rust(
            self.original,
            &roots,
            original.correlation.or(original.domain),
            &origin.occurrence,
        );
        let common: BTreeSet<_> = origin
            .free
            .iter()
            .chain(&origin.interface)
            .map(symbol_name)
            .collect();
        let result = if rust {
            let prefix = original
                .correlation
                .ok_or("a Rust block must keep its prefix")?;
            let input = self.node(prefix)?;
            let ordinal = self.result.chain.len();
            let mut node = Node {
                kind: Kind::Rust(format!("block @{path} {}", origin.fallback.is_some())),
                heading: self.result.nodes[input].heading.clone(),
                origins: self.result.nodes[input].origins.clone(),
                operands: vec![input],
                views: self.result.nodes[input].views.clone(),
                pruned: self.result.nodes[input].pruned.clone(),
                rust_index: Some(ordinal),
                ..Node::default()
            };
            let arguments = origin
                .free
                .iter()
                .map(|id| {
                    let name = symbol_name(id);
                    (name.clone(), node.heading[&name].clone())
                })
                .collect();
            for id in &origin.interface {
                let name = symbol_name(id);
                let column = Column::Bind(ordinal, name.clone());
                node.heading.insert(name.clone(), column.clone());
                node.origins.insert(name, HashSet::from([column]));
            }
            self.result.chain.push(Rust {
                payload: format!("block @{path} {}", origin.fallback.is_some()),
                input,
                arguments,
                expression: syn::parse_quote!(()),
                bindings: origin.interface.iter().cloned().collect(),
                environment: None,
            });
            self.result.push(node)
        } else {
            let id = self.original.paths[&origin.occurrence];
            let mapping = common
                .iter()
                .map(|name| {
                    let id = Ident::new(name, proc_macro2::Span::call_site());
                    (id.clone(), id)
                })
                .collect::<Vec<_>>();
            let relation = Ident::new("block", proc_macro2::Span::call_site());
            let atom = self.result.atom(id, &relation, &mapping, None);
            if let Some(prefix) = original.correlation {
                let prefix = self.node(prefix)?;
                self.result.join(prefix, atom, false)
            } else {
                atom
            }
        };
        for (name, columns) in &original.origins {
            if let Some(replacement) = self.result.nodes[result].heading.get(name) {
                for column in columns {
                    self.columns.insert(column.clone(), replacement.clone());
                }
            }
        }
        self.memo.insert(index, result);
        Ok(result)
    }
}
