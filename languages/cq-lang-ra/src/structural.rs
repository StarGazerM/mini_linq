use std::collections::{HashMap, HashSet};
use std::hash::{DefaultHasher, Hash, Hasher};

use quote::ToTokens;
use syn::Ident;

use crate::{EqualityValue, Operator, Program, Theta, contract::Schedule, symbol_name};

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
enum Column {
    Atom(usize, String),
    Bind(usize, String),
}

type Heading = HashMap<String, Column>;
type Origins = HashMap<String, HashSet<Column>>;

#[derive(Clone, PartialEq, Eq, Hash)]
struct Predicate {
    shape: String,
    columns: Vec<Column>,
}

#[derive(Default, Clone, PartialEq, Eq, Hash)]
enum Kind {
    #[default]
    Unit,
    Atom(usize),
    Project,
    Join(bool),
    Select,
    Rust(String),
}

#[derive(Default)]
pub(super) struct Node {
    kind: Kind,
    heading: Heading,
    origins: Origins,
    operands: Vec<usize>,
    equalities: Vec<(Column, Column)>,
    predicates: Vec<Predicate>,
    owner: Option<usize>,
    reducer: Option<Reduction>,
    views: HashMap<usize, usize>,
    pruned: HashSet<usize>,
    range_base: Option<usize>,
}

#[derive(Clone)]
pub(super) struct Reduction {
    left: usize,
    right: usize,
    left_node: usize,
    equalities: Vec<(Column, Column)>,
    join_node: usize,
    node: usize,
}

impl Reduction {
    fn covered_by(&self, other: &Self) -> bool {
        let mut columns = HashMap::new();
        for (a, b) in self.equalities.iter().chain(&other.equalities) {
            for column in [a, b] {
                let next = columns.len();
                columns.entry(column).or_insert(next);
            }
        }
        let mut closure = UnionFind::new(columns.len());
        for (a, b) in &other.equalities {
            closure.union(columns[a], columns[b]);
        }
        self.equalities
            .iter()
            .all(|(a, b)| closure.root(columns[a]) == closure.root(columns[b]))
    }
}

pub(super) struct Atom {
    relation: String,
    bindings: HashMap<String, String>,
    node: usize,
}

pub(super) struct Rust {
    payload: String,
    input: usize,
    arguments: Heading,
}

pub(super) struct RowView {
    parent: Option<usize>,
    restricted: bool,
}

fn row_view(views: &mut Vec<RowView>, parent: Option<usize>, restricted: bool) -> usize {
    let index = views.len();
    views.push(RowView { parent, restricted });
    index
}

impl Schedule {
    fn push(&mut self, node: Node) -> usize {
        let index = self.nodes.len();
        self.nodes.push(node);
        index
    }

    pub fn unit(&mut self) -> usize {
        self.push(Node::default())
    }

    /// Introduce one complete positional source occurrence.
    pub fn atom(
        &mut self,
        occurrence: usize,
        relation: &Ident,
        mapping: &[(Ident, Ident)],
    ) -> usize {
        let view = row_view(&mut self.row_views, None, false);
        let node = self.push(Node {
            kind: Kind::Atom(occurrence),
            heading: mapping
                .iter()
                .map(|(source, target)| {
                    (
                        symbol_name(target),
                        Column::Atom(occurrence, symbol_name(source)),
                    )
                })
                .collect(),
            origins: mapping
                .iter()
                .map(|(source, target)| {
                    (
                        symbol_name(target),
                        HashSet::from([Column::Atom(occurrence, symbol_name(source))]),
                    )
                })
                .collect(),
            owner: Some(occurrence),
            views: HashMap::from([(occurrence, view)]),
            ..Node::default()
        });
        self.atoms.insert(
            occurrence,
            Atom {
                relation: symbol_name(relation),
                bindings: mapping
                    .iter()
                    .map(|(s, t)| (symbol_name(s), symbol_name(t)))
                    .collect(),
                node,
            },
        );
        node
    }

    pub fn project(&mut self, input: usize, attributes: &[Ident]) -> usize {
        let input_node = &self.nodes[input];
        let heading: Heading = attributes
            .iter()
            .map(|a| {
                let name = symbol_name(a);
                (name.clone(), input_node.heading[&name].clone())
            })
            .collect();
        let mut owner = input_node.owner;
        let mut restored = None;
        let mut equalities = Vec::new();
        let mut predicates = Vec::new();
        if let Some(reducer) = &input_node.reducer {
            if heading.keys().collect::<HashSet<_>>()
                == self.nodes[reducer.left_node].heading.keys().collect()
            {
                owner = Some(reducer.left);
                let mut reduction = reducer.clone();
                reduction.node = self.nodes.len();
                self.reductions.push(reduction);
                restored = Some(reducer.left_node);
                equalities.extend(
                    reducer
                        .equalities
                        .iter()
                        .filter(|(a, b)| {
                            let binding = |column: &Column| match column {
                                Column::Atom(o, c) => Some(&self.atoms[o].bindings[c]),
                                _ => None,
                            };
                    matches!((a,b),(Column::Atom(o,_),Column::Atom(p,_)) if o == p && *o == reducer.left)
                                || binding(a).is_some() && binding(a) == binding(b)
                        })
                        .cloned(),
                );
                let mut cursor = input;
                loop {
                    predicates.extend(self.nodes[cursor].predicates.iter().filter(|p| {
                        p.columns.iter().all(|column| matches!(column, Column::Atom(o, _) if *o == reducer.left))
                    }).cloned());
                    if cursor == reducer.join_node {
                        break;
                    }
                    cursor = self.nodes[cursor].operands[0];
                }
            }
        }
        let visible: HashSet<_> = heading
            .values()
            .filter_map(|column| match column {
                Column::Atom(occurrence, _) => Some(*occurrence),
                _ => None,
            })
            .collect();
        let retained: HashSet<_> = heading.values().collect();
        let mut pruned = input_node.pruned.clone();
        for column in input_node.heading.values() {
            if let Column::Atom(occurrence, _) = column {
                if owner.is_none_or(|owner| owner == *occurrence)
                    && !retained.contains(column)
                    && input_node
                        .views
                        .get(occurrence)
                        .is_some_and(|view| self.row_views[*view].restricted)
                {
                    pruned.insert(*occurrence);
                }
            }
        }
        let mut views: HashMap<_, _> = input_node
            .views
            .iter()
            .filter(|(occurrence, _)| {
                visible.contains(occurrence)
                    || pruned.contains(occurrence)
                    || owner == Some(**occurrence)
            })
            .map(|(occurrence, view)| (*occurrence, *view))
            .collect();
        if let Some(owner) = owner {
            let full = heading
                .values()
                .filter(|column| matches!(column, Column::Atom(o, _) if *o == owner))
                .count()
                == self.atoms[&owner].bindings.len();
            if full {
                pruned.remove(&owner);
                if let Some(left) = restored {
                    let parent = self.nodes[left].views[&owner];
                    views.insert(owner, row_view(&mut self.row_views, Some(parent), true));
                }
            }
        } else if heading.len() < input_node.heading.len() {
            // A cut of a joined prefix has independent correlated witnesses.
            // It cannot silently identify them with another read of a base row.
            for occurrence in input_node.views.keys() {
                views.insert(*occurrence, row_view(&mut self.row_views, None, true));
                pruned.insert(*occurrence);
            }
        }
        self.push(Node {
            kind: Kind::Project,
            heading,
            origins: attributes
                .iter()
                .map(|a| {
                    let name = symbol_name(a);
                    (name.clone(), input_node.origins[&name].clone())
                })
                .collect(),
            operands: vec![input],
            owner,
            range_base: restored,
            equalities,
            predicates,
            views,
            pruned,
            ..Node::default()
        })
    }

    pub fn join(&mut self, left: usize, right: usize, semijoin: bool) -> usize {
        let l = &self.nodes[left];
        let r = &self.nodes[right];
        let equalities: Vec<_> = l
            .heading
            .iter()
            .filter_map(|(name, column)| {
                r.heading
                    .get(name)
                    .map(|other| (column.clone(), other.clone()))
            })
            .collect();
        let mut heading = l.heading.clone();
        if !semijoin {
            for (name, column) in &r.heading {
                heading
                    .entry(name.clone())
                    .or_insert_with(|| column.clone());
            }
        }
        // Two independent projections of one occurrence do not reconstruct a
        // correlated row. Record that structural split, rather than guessing
        // an existential-witness mapping. Full-row uses subsume partial uses.
        let masks = |origins: &Origins| {
            let mut masks = HashMap::<usize, HashSet<String>>::new();
            for column in origins.values().flatten() {
                if let Column::Atom(occurrence, attribute) = column {
                    masks
                        .entry(*occurrence)
                        .or_default()
                        .insert(attribute.clone());
                }
            }
            masks
        };
        let lm = masks(&l.origins);
        let rm = masks(&r.origins);
        if !(semijoin && l.owner.is_some() && l.owner != r.owner) {
            for (occurrence, a) in &lm {
                if rm
                    .get(occurrence)
                    .is_some_and(|b| !a.is_subset(b) && !b.is_subset(a))
                {
                    self.split_nodes.push((self.nodes.len(), *occurrence));
                }
            }
        }
        if !semijoin {
            for node in [l, r] {
                if let Some(occurrence) = node.owner {
                    if node.heading.len() == self.atoms[&occurrence].bindings.len() {
                        self.full_reads.push((self.nodes.len(), occurrence));
                    }
                }
            }
        }
        for occurrence in &l.pruned {
            if let Some(other) = r.views.get(occurrence) {
                let projected = rm
                    .get(occurrence)
                    .is_none_or(|mask| lm.get(occurrence).is_some_and(|left| mask.is_subset(left)));
                self.view_requests
                    .push((l.views[occurrence], *other, projected));
            }
        }
        for occurrence in &r.pruned {
            if let Some(other) = l.views.get(occurrence) {
                let projected = lm.get(occurrence).is_none_or(|mask| {
                    rm.get(occurrence)
                        .is_some_and(|right| mask.is_subset(right))
                });
                self.view_requests
                    .push((r.views[occurrence], *other, projected));
            }
        }
        let mut views = l.views.clone();
        let mut pruned = l.pruned.clone();
        let mut origins = l.origins.clone();
        for (name, columns) in &r.origins {
            if origins.contains_key(name) || !semijoin {
                origins
                    .entry(name.clone())
                    .or_default()
                    .extend(columns.iter().cloned());
            }
        }
        for (occurrence, view) in &r.views {
            let left_columns = lm.get(occurrence).map_or(0, HashSet::len);
            let right_columns = rm.get(occurrence).map_or(0, HashSet::len);
            if right_columns > left_columns || !views.contains_key(occurrence) {
                views.insert(*occurrence, *view);
            }
        }
        pruned.extend(&r.pruned);
        if semijoin {
            let hidden: HashSet<_> = r
                .origins
                .iter()
                .filter(|(name, _)| !l.heading.contains_key(*name))
                .flat_map(|(_, columns)| columns.iter())
                .filter_map(|column| match column {
                    Column::Atom(occurrence, _) => Some(*occurrence),
                    _ => None,
                })
                .collect();
            for (occurrence, view) in &r.views {
                if hidden.contains(occurrence) && self.row_views[*view].restricted {
                    pruned.insert(*occurrence);
                }
            }
            if let Some(owner) = l.owner {
                if lm.get(&owner).map_or(0, HashSet::len) == self.atoms[&owner].bindings.len() {
                    views.insert(
                        owner,
                        row_view(&mut self.row_views, Some(l.views[&owner]), true),
                    );
                }
            }
        }
        let output_masks = masks(&origins);
        for (occurrence, mask) in &output_masks {
            if mask.len() == self.atoms[occurrence].bindings.len() {
                pruned.remove(occurrence);
            }
        }
        let reducer = l.owner.zip(r.owner).map(|(l, r)| Reduction {
            left: l,
            right: r,
            left_node: left,
            equalities: equalities.clone(),
            join_node: self.nodes.len(),
            node: self.nodes.len(),
        });
        if semijoin {
            if let Some(reducer) = &reducer {
                self.reductions.push(reducer.clone());
            }
        }
        self.push(Node {
            kind: Kind::Join(semijoin),
            heading,
            origins,
            operands: vec![left, right],
            equalities,
            predicates: Vec::new(),
            owner: if semijoin { l.owner } else { None },
            reducer: if semijoin { None } else { reducer },
            views,
            pruned,
            range_base: None,
        })
    }

    pub fn select(&mut self, input: usize, theta: &Theta) -> usize {
        let source = &self.nodes[input];
        let mut node = Node {
            kind: Kind::Select,
            heading: source.heading.clone(),
            origins: source.origins.clone(),
            operands: vec![input],
            owner: source.owner,
            reducer: source.reducer.clone(),
            views: source.views.clone(),
            pruned: source.pruned.clone(),
            ..Node::default()
        };
        for equality in &theta.equalities {
            let left = node.heading[&symbol_name(&equality.left)].clone();
            match &equality.right {
                EqualityValue::Variable(v) => node
                    .equalities
                    .push((left, node.heading[&symbol_name(v)].clone())),
                EqualityValue::Literal(l) => node.predicates.push(Predicate {
                    shape: format!("literal {}", l.to_token_stream()),
                    columns: vec![left],
                }),
            }
        }
        // Formal rule CT-Constraint: Preserve predicate shape and its fixed column arguments.
        for constraint in &theta.constraints {
            let columns = constraint
                .variables()
                .map(|v| node.heading[&symbol_name(v)].clone())
                .collect();
            let mut shape = constraint.clone();
            shape.map_terms(&mut |term| {
                if let crate::syntax::constraint::Term::Variable(v) = term {
                    *v = Ident::new("__column", v.span());
                }
            });
            node.predicates.push(Predicate {
                shape: shape.to_token_stream().to_string(),
                columns,
            });
        }
        if let Some(reducer) = &mut node.reducer {
            reducer.equalities.extend(node.equalities.iter().cloned());
        }
        if !theta.is_empty() {
            if let Some(owner) = node.owner {
                let full = node
                    .heading
                    .values()
                    .filter(|c| matches!(c, Column::Atom(o, _) if *o == owner))
                    .count()
                    == self.atoms[&owner].bindings.len();
                if full {
                    let parent = node.views[&owner];
                    node.views
                        .insert(owner, row_view(&mut self.row_views, Some(parent), true));
                }
            }
        }
        self.push(node)
    }

    fn rust(
        &mut self,
        input: usize,
        payload: String,
        expression: &syn::Expr,
        bindings: &[Ident],
    ) -> usize {
        let source = &self.nodes[input];
        let logical = source.heading.keys().cloned().collect();
        let arguments = crate::syntax::rust::known_uses(expression, &logical)
            .iter()
            .map(|name| (name.clone(), source.heading[name].clone()))
            .collect();
        let mut heading = source.heading.clone();
        let mut origins = source.origins.clone();
        for binding in bindings {
            let name = symbol_name(binding);
            heading.insert(name.clone(), Column::Bind(self.chain.len(), name));
            let name = symbol_name(binding);
            origins.insert(
                name.clone(),
                HashSet::from([Column::Bind(self.chain.len(), name)]),
            );
        }
        self.chain.push(Rust {
            payload: payload.clone(),
            input,
            arguments,
        });
        self.push(Node {
            kind: Kind::Rust(payload),
            heading,
            origins,
            operands: vec![input],
            views: self.nodes[input].views.clone(),
            pruned: self.nodes[input].pruned.clone(),
            ..Node::default()
        })
    }

    pub fn guard(&mut self, input: usize, filter: &crate::syntax::Filter) -> usize {
        self.rust(
            input,
            filter.to_token_stream().to_string(),
            &filter.condition,
            &[],
        )
    }

    pub fn bind(&mut self, input: usize, binding: &crate::syntax::Let) -> usize {
        self.rust(
            input,
            binding.to_token_stream().to_string(),
            &binding.expression,
            &crate::syntax::rust::bindings(&binding.pattern),
        )
    }

    pub fn finish(&mut self, input: usize, head: &crate::syntax::Atom) {
        self.output = Some(input);
        self.head = head.to_token_stream().to_string();
    }

    pub fn from_program(program: &Program) -> syn::Result<Self> {
        crate::contract::check(program)?;
        let mut schedule = Self::default();
        let mut results = HashMap::new();
        for definition in &program.plan.definitions {
            let node = match &definition.operator {
                Operator::Unit(_) => schedule.unit(),
                Operator::Rename(r) => schedule.atom(
                    r.occurrence.base10_parse().unwrap(),
                    &r.relation,
                    &r.mapping
                        .iter()
                        .map(|m| (m.source.clone(), m.target.clone()))
                        .collect::<Vec<_>>(),
                ),
                Operator::Project(p) => schedule.project(
                    results[&symbol_name(&p.input)],
                    &p.attributes.iter().cloned().collect::<Vec<_>>(),
                ),
                Operator::NaturalJoin(j) => schedule.join(
                    results[&symbol_name(&j.left)],
                    results[&symbol_name(&j.right)],
                    false,
                ),
                Operator::Semijoin(j) => schedule.join(
                    results[&symbol_name(&j.left)],
                    results[&symbol_name(&j.right)],
                    true,
                ),
                Operator::Select(s) => {
                    schedule.select(results[&symbol_name(&s.input)], &s.condition)
                }
                Operator::Guard(g) => schedule.guard(
                    results[&symbol_name(&g.input)],
                    &crate::syntax::Filter {
                        if_token: g.if_token,
                        condition: g.condition.clone(),
                    },
                ),
                Operator::Bind(b) => schedule.bind(results[&symbol_name(&b.input)], &b.binding()),
            };
            results.insert(symbol_name(&definition.result), node);
        }
        schedule.finish(
            results[&symbol_name(&program.plan.output.input)],
            &program.plan.output.head,
        );
        Ok(schedule)
    }

    // Reverse DAG propagation assigns each fact its first observing Rust node,
    // or the final output. Each edge is visited once; shared operands stay shared.
    fn regions(&self) -> Vec<usize> {
        let mut regions = vec![self.chain.len(); self.nodes.len()];
        for (ordinal, rust) in self.chain.iter().enumerate() {
            regions[rust.input] = regions[rust.input].min(ordinal);
        }
        for (index, node) in self.nodes.iter().enumerate().rev() {
            for operand in &node.operands {
                regions[*operand] = regions[*operand].min(regions[index]);
            }
        }
        regions
    }

    fn primary_regions(&self) -> Vec<usize> {
        let mut regions = vec![self.chain.len() + 1; self.nodes.len()];
        if let Some(output) = self.output {
            regions[output] = self.chain.len();
        }
        for (ordinal, rust) in self.chain.iter().enumerate() {
            regions[rust.input] = regions[rust.input].min(ordinal);
        }
        for (index, node) in self.nodes.iter().enumerate().rev() {
            if let Some(left) = node.range_base {
                regions[left] = regions[left].min(regions[index]);
            } else {
                let count = if matches!(node.kind, Kind::Join(true)) {
                    1
                } else {
                    node.operands.len()
                };
                for operand in node.operands.iter().take(count) {
                    regions[*operand] = regions[*operand].min(regions[index]);
                }
            }
        }
        regions
    }

    fn split_views(&self) -> bool {
        let primary = self.primary_regions();
        let regions = self.regions();
        let mut full = HashMap::<usize, usize>::new();
        for &(node, occurrence) in &self.full_reads {
            if primary[node] <= self.chain.len() {
                let region = full.entry(occurrence).or_insert(regions[node]);
                *region = (*region).min(regions[node]);
            }
        }
        if self.split_nodes.iter().any(|&(node, occurrence)| {
            primary[node] <= self.chain.len()
                && full
                    .get(&occurrence)
                    .is_none_or(|region| *region > regions[node])
        }) {
            return true;
        }
        // Clause restrictions form a forest along each named relation's left
        // spine. A later restricted row may certify an earlier projected read
        // only when it descends from that same view. Euler intervals answer
        // every fixed ancestry obligation in constant time, without search.
        let mut children = vec![Vec::new(); self.row_views.len()];
        let mut roots = Vec::new();
        for (index, view) in self.row_views.iter().enumerate() {
            if let Some(parent) = view.parent {
                children[parent].push(index);
            } else {
                roots.push(index);
            }
        }
        let mut intervals = vec![(0, 0); children.len()];
        let mut ordinal = 0;
        let mut pending: Vec<_> = roots.into_iter().map(|i| (i, false)).collect();
        while let Some((index, finish)) = pending.pop() {
            if finish {
                intervals[index].1 = ordinal;
            } else {
                intervals[index].0 = ordinal;
                ordinal += 1;
                pending.push((index, true));
                pending.extend(children[index].iter().map(|i| (*i, false)));
            }
        }
        self.view_requests
            .iter()
            .any(|&(earlier, later, projected)| {
                let (a, b) = (intervals[earlier], intervals[later]);
                !(a.0 <= b.0 && b.1 <= a.1 || projected && b.0 <= a.0 && a.1 <= b.1)
            })
    }

    /// Check the supplied schedule's structural obligations against checked RA.
    /// Required reductions are named occurrence edges, including the equivalent
    /// join/select/project form used when opaque Rust protects both aliases.
    pub fn check(
        &self,
        target: &Self,
        required_reductions: &[(usize, usize)],
    ) -> Result<(), String> {
        let required: HashSet<_> = required_reductions.iter().copied().collect();
        let (source_regions, target_regions) = (self.regions(), target.regions());
        let mut edges = HashMap::new();
        for reduction in &target.reductions {
            edges
                .entry((reduction.left, reduction.right))
                .or_insert(reduction);
        }
        for reduction in &self.reductions {
            let edge = (reduction.left, reduction.right);
            if !required.contains(&edge) {
                continue;
            }
            let Some(other) = edges.get(&edge) else {
                return Err("missing semijoin reduction between named atom occurrences".into());
            };
            if target_regions[other.node] > source_regions[reduction.node] {
                return Err(format!(
                    "reduction @{} with @{} lost a source-before occurrence edge",
                    edge.0, edge.1
                ));
            }
            // A reducer's support row is an existential read of that named
            // occurrence. Equalities from the opposite direction or from the
            // final prefix cannot repair missing column matches at this edge.
            let mut columns = HashMap::new();
            for (a, b) in reduction.equalities.iter().chain(&other.equalities) {
                for column in [a, b] {
                    let next = columns.len();
                    columns.entry(column).or_insert(next);
                }
            }
            let mut closure = UnionFind::new(columns.len());
            for (a, b) in &other.equalities {
                closure.union(columns[a], columns[b]);
            }
            if reduction
                .equalities
                .iter()
                .any(|(a, b)| closure.root(columns[a]) != closure.root(columns[b]))
            {
                return Err(format!(
                    "reduction @{} with @{} lost source-before support column matches",
                    edge.0, edge.1
                ));
            }
        }
        compare(self, target)
    }
}

struct UnionFind {
    parent: Vec<usize>,
    rank: Vec<u8>,
    canonical: Vec<usize>,
}

impl UnionFind {
    fn new(size: usize) -> Self {
        Self {
            parent: (0..size).collect(),
            rank: vec![0; size],
            canonical: (0..size).collect(),
        }
    }
    fn root(&mut self, x: usize) -> usize {
        if self.parent[x] != x {
            self.parent[x] = self.root(self.parent[x]);
        }
        self.parent[x]
    }
    fn union(&mut self, a: usize, b: usize) {
        let (mut a, mut b) = (self.root(a), self.root(b));
        if a == b {
            return;
        }
        if self.rank[a] < self.rank[b] {
            std::mem::swap(&mut a, &mut b);
        }
        self.parent[b] = a;
        self.canonical[a] = self.canonical[a].min(self.canonical[b]);
        if self.rank[a] == self.rank[b] {
            self.rank[a] += 1;
        }
    }
    fn label(&mut self, x: usize) -> usize {
        let root = self.root(x);
        self.canonical[root]
    }
}

type PredicateKey = (String, Vec<usize>);

// Equality-indexed facts at a Rust input. Only predicates incident to the
// smaller merged class are re-keyed; a fact never scans unrelated aliases.
struct Prefix {
    equality: UnionFind,
    members: Vec<Vec<usize>>,
    incidence: Vec<Vec<usize>>,
    predicates: Vec<PredicateKey>,
    keys: Vec<Option<PredicateKey>>,
    available: HashMap<PredicateKey, usize>,
    seen: Vec<usize>,
    epoch: usize,
}

impl Prefix {
    fn new(columns: usize, predicates: Vec<PredicateKey>) -> Self {
        let mut incidence = vec![Vec::new(); columns];
        for (index, (_, arguments)) in predicates.iter().enumerate() {
            for column in arguments {
                incidence[*column].push(index);
            }
        }
        let count = predicates.len();
        Self {
            equality: UnionFind::new(columns),
            members: (0..columns).map(|c| vec![c]).collect(),
            incidence,
            predicates,
            keys: vec![None; count],
            available: HashMap::new(),
            seen: vec![0; count],
            epoch: 0,
        }
    }
    fn key(&mut self, predicate: &PredicateKey) -> PredicateKey {
        (
            predicate.0.clone(),
            predicate.1.iter().map(|c| self.equality.root(*c)).collect(),
        )
    }
    fn insert(&mut self, index: usize) {
        let predicate = self.predicates[index].clone();
        let key = self.key(&predicate);
        *self.available.entry(key.clone()).or_default() += 1;
        self.keys[index] = Some(key);
    }
    fn contains(&mut self, predicate: &PredicateKey) -> bool {
        let key = self.key(predicate);
        self.available.contains_key(&key)
    }
    fn union(&mut self, a: usize, b: usize) {
        let (mut a, mut b) = (self.equality.root(a), self.equality.root(b));
        if a == b {
            return;
        }
        if self.members[a].len() < self.members[b].len() {
            std::mem::swap(&mut a, &mut b);
        }
        self.epoch += 1;
        let mut affected = Vec::new();
        for column in &self.members[b] {
            for &predicate in &self.incidence[*column] {
                if self.keys[predicate].is_some() && self.seen[predicate] != self.epoch {
                    self.seen[predicate] = self.epoch;
                    affected.push(predicate);
                }
            }
        }
        for &predicate in &affected {
            let key = self.keys[predicate].take().unwrap();
            let count = self.available.get_mut(&key).unwrap();
            *count -= 1;
            if *count == 0 {
                self.available.remove(&key);
            }
        }
        self.equality.parent[b] = a;
        self.equality.canonical[a] = self.equality.canonical[a].min(self.equality.canonical[b]);
        let members = std::mem::take(&mut self.members[b]);
        self.members[a].extend(members);
        for predicate in affected {
            self.insert(predicate);
        }
    }
}

// Unordered local facts have an order-independent hash and exact set equality;
// hash collisions never establish equality. This avoids sorting wide headings.
#[derive(PartialEq, Eq)]
struct Facts<T: Eq + Hash>(HashSet<T>);

impl<T: Eq + Hash> Hash for Facts<T> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.0.len().hash(state);
        let mut sum = 0u64;
        for item in &self.0 {
            let mut hash = DefaultHasher::new();
            item.hash(&mut hash);
            sum = sum.wrapping_add(hash.finish());
        }
        sum.hash(state);
    }
}

#[derive(PartialEq, Eq, Hash)]
struct Wiring {
    kind: Kind,
    operands: Vec<usize>,
    heading: Facts<(String, Column)>,
    equalities: Facts<(Column, Column)>,
    predicates: Facts<Predicate>,
}

fn wiring(schedule: &Schedule, intern: &mut HashMap<Wiring, usize>) -> Result<usize, String> {
    let mut nodes = Vec::with_capacity(schedule.nodes.len());
    for node in &schedule.nodes {
        let mut operands: Vec<usize> = node.operands.iter().map(|i| nodes[*i]).collect();
        if matches!(node.kind, Kind::Join(false)) {
            operands.sort_unstable();
        }
        let key = Wiring {
            kind: node.kind.clone(),
            operands,
            heading: Facts(
                node.heading
                    .iter()
                    .map(|(n, c)| (n.clone(), c.clone()))
                    .collect(),
            ),
            equalities: Facts(
                node.equalities
                    .iter()
                    .map(|(a, b)| {
                        if a <= b {
                            (a.clone(), b.clone())
                        } else {
                            (b.clone(), a.clone())
                        }
                    })
                    .collect(),
            ),
            predicates: Facts(node.predicates.iter().cloned().collect()),
        };
        let next = intern.len();
        nodes.push(*intern.entry(key).or_insert(next));
    }
    Ok(nodes[schedule.output.ok_or("schedule has no output")?])
}

fn compare(source: &Schedule, target: &Schedule) -> Result<(), String> {
    if source.head != target.head {
        return Err("output head differs from source".into());
    }
    if source.chain.len() != target.chain.len()
        || source
            .chain
            .iter()
            .zip(&target.chain)
            .any(|(s, t)| s.payload != t.payload)
    {
        return Err("Rust chain differs in order, payload or bindings".into());
    }
    if source.atoms.len() != target.atoms.len()
        || source.atoms.iter().any(|(occurrence, atom)| {
            target.atoms.get(occurrence).is_none_or(|t| {
                atom.relation != t.relation
                    || atom.bindings.keys().collect::<HashSet<_>>() != t.bindings.keys().collect()
            })
        })
    {
        return Err("atom occurrence multiset differs from source".into());
    }
    if source.split_views() || target.split_views() {
        // Independent projected reads of a single occurrence are outside the
        // ordinary one-row contract. Preserve their exact structural wiring,
        // including which witness owns a predicate. There is no equivalence
        // search; shared DAG nodes are interned once in topological order.
        let mut intern = HashMap::new();
        if wiring(source, &mut intern)? != wiring(target, &mut intern)? {
            return Err("projection correlations differ from source".into());
        }
    }
    let mut columns = HashMap::new();
    for node in source.nodes.iter().chain(&target.nodes) {
        for column in node.heading.values() {
            let next = columns.len();
            columns.entry(column.clone()).or_insert(next);
        }
    }
    let (sr, tr) = (source.regions(), target.regions());
    let (sp, tp) = (source.primary_regions(), target.primary_regions());
    let closure = |schedule: &Schedule| {
        let mut uf = UnionFind::new(columns.len());
        let primary = schedule.primary_regions();
        for (index, node) in schedule.nodes.iter().enumerate() {
            if primary[index] > schedule.chain.len() {
                continue;
            }
            for (a, b) in &node.equalities {
                uf.union(columns[a], columns[b]);
            }
        }
        uf
    };
    let (mut su, mut tu) = (closure(source), closure(target));
    if (0..columns.len()).any(|c| su.label(c) != tu.label(c)) {
        return Err("relational equality closure differs from source".into());
    }
    let mut names = HashMap::<String, HashSet<usize>>::new();
    for (occurrence, atom) in &source.atoms {
        for (column, name) in &atom.bindings {
            names
                .entry(name.clone())
                .or_default()
                .insert(su.label(columns[&Column::Atom(*occurrence, column.clone())]));
        }
    }
    for (occurrence, atom) in &source.atoms {
        let other = &target.atoms[occurrence];
        for (column, name) in &atom.bindings {
            let target_name = &other.bindings[column];
            if name != target_name
                && !names.get(target_name).is_some_and(|classes| {
                    classes.contains(&su.label(columns[&Column::Atom(*occurrence, column.clone())]))
                })
            {
                return Err(format!(
                    "atom occurrence @{occurrence} changes variable binding of `{column}`"
                ));
            }
        }
        if tr[other.node] > sr[atom.node] {
            return Err(format!(
                "Rust step {} lost source-before atom occurrence @{occurrence}",
                sr[atom.node]
            ));
        }
    }
    let key = |p: &Predicate, uf: &mut UnionFind| -> PredicateKey {
        (
            p.shape.clone(),
            p.columns.iter().map(|c| uf.label(columns[c])).collect(),
        )
    };
    let predicates = |schedule: &Schedule, uf: &mut UnionFind| {
        let mut facts = HashSet::new();
        for node in &schedule.nodes {
            for predicate in &node.predicates {
                facts.insert(key(predicate, uf));
            }
        }
        facts
    };
    let (sf, tf) = (predicates(source, &mut su), predicates(target, &mut tu));
    if sf != tf {
        return Err("relational constraints or projection correlations differ from source".into());
    }
    let mut buckets = vec![Vec::new(); source.chain.len() + 1];
    let mut requests = vec![Vec::new(); buckets.len()];
    for (index, node) in target.nodes.iter().enumerate() {
        if tp[index] >= buckets.len() {
            continue;
        }
        buckets[tp[index]].extend(
            node.equalities
                .iter()
                .map(|(a, b)| (columns[a], columns[b])),
        );
    }
    for (index, node) in source.nodes.iter().enumerate() {
        if sp[index] >= requests.len() {
            continue;
        }
        requests[sp[index]].extend(
            node.equalities
                .iter()
                .map(|(a, b)| (columns[a], columns[b])),
        );
    }
    let mut predicate_requests = vec![Vec::new(); buckets.len()];
    for (node, region) in source.nodes.iter().zip(&sr) {
        for predicate in &node.predicates {
            let arguments: Vec<_> = predicate.columns.iter().map(|c| columns[c]).collect();
            // Never deduplicate Before obligations using the final closure:
            // two arguments may only become equal after this Rust region.
            predicate_requests[*region].push((predicate.shape.clone(), arguments));
        }
    }
    for (ordinal, (s, t)) in source.chain.iter().zip(&target.chain).enumerate() {
        if s.arguments.keys().collect::<HashSet<_>>() != t.arguments.keys().collect() {
            return Err("Rust chain input bindings differ".into());
        }
        requests[ordinal].extend(
            s.arguments
                .iter()
                .map(|(name, c)| (columns[c], columns[&t.arguments[name]])),
        );
    }
    let so = &source.nodes[source.output.ok_or("source schedule has no output")?].heading;
    let to = &target.nodes[target.output.ok_or("target schedule has no output")?].heading;
    if so.keys().collect::<HashSet<_>>() != to.keys().collect() {
        return Err("output heading differs from source".into());
    }
    requests[source.chain.len()]
        .extend(so.iter().map(|(name, c)| (columns[c], columns[&to[name]])));
    let mut additions = vec![Vec::new(); buckets.len()];
    let mut target_predicates = Vec::new();
    for (node, region) in target.nodes.iter().zip(&tr) {
        for predicate in &node.predicates {
            additions[*region].push(target_predicates.len());
            target_predicates.push((
                predicate.shape.clone(),
                predicate.columns.iter().map(|c| columns[c]).collect(),
            ));
        }
    }
    let mut before = Prefix::new(columns.len(), target_predicates);
    let mut reductions = vec![Vec::new(); buckets.len()];
    for reduction in &source.reductions {
        let region = sr[reduction.node];
        if region < source.chain.len() {
            reductions[region].push(reduction);
        }
    }
    let mut target_reductions = HashMap::new();
    for reduction in &target.reductions {
        let edge = (reduction.left, reduction.right);
        let entry = target_reductions.entry(edge).or_insert(reduction);
        if tr[reduction.node] < tr[entry.node] {
            *entry = reduction;
        }
    }
    for region in 0..buckets.len() {
        for &(a, b) in &buckets[region] {
            before.union(a, b);
        }
        for &predicate in &additions[region] {
            before.insert(predicate);
        }
        for reduction in &reductions[region] {
            let edge = (reduction.left, reduction.right);
            let direct = target_reductions
                .get(&edge)
                .is_some_and(|other| tr[other.node] <= region && reduction.covered_by(other));
            let joined = tr[target.atoms[&reduction.right].node] <= region
                && reduction.equalities.iter().all(|(a, b)| {
                    before.equality.root(columns[a]) == before.equality.root(columns[b])
                });
            if !direct && !joined {
                return Err(format!(
                    "Rust step {region} lost source-before reduction @{} with @{} column matches",
                    edge.0, edge.1
                ));
            }
        }
        if requests[region]
            .iter()
            .any(|&(a, b)| before.equality.root(a) != before.equality.root(b))
            || predicate_requests[region]
                .iter()
                .any(|predicate| !before.contains(predicate))
        {
            return Err(if region < source.chain.len() {
                format!("Rust step {region} lost a source-before relational constraint or binding")
            } else {
                "output relational bindings differ from source".into()
            });
        }
    }
    Ok(())
}
