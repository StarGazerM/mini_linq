use std::collections::{BTreeMap, BTreeSet};

use cq_lang_structures::physical::{Operator, symbol_name};
use syn::Ident;

use crate::{
    Analysis, Dispatch, Guard, Len, LevelNode, Machine, Monoid, Obligation, Program, Term,
};

struct Level<'a> {
    model: &'a LevelNode,
    path: Vec<String>,
    children: Vec<usize>,
}

struct Node<'a> {
    term: &'a Term,
    owner: usize,
    level: usize,
    inputs: Vec<usize>,
    separated_store: bool,
}

struct Checker<'a> {
    program: &'a Program,
    levels: Vec<Level<'a>>,
    grades: Vec<usize>,
    definitions: BTreeMap<String, usize>,
    physical: BTreeMap<String, &'a cq_lang_structures::physical::Definition>,
    roots: Vec<usize>,
    nodes: Vec<Node<'a>>,
    collectives: Vec<(String, Monoid, Vec<String>)>,
    lengths: &'a BTreeMap<String, Len>,
    tilings: BTreeMap<String, Vec<crate::Tiling>>,
    parts: BTreeMap<String, Option<usize>>,
    range_reads: BTreeSet<(String, Vec<String>)>,
    publication: Option<usize>,
    publication_tile: Option<&'a Term>,
}

/// Check placement, then derive dispatch boundaries and runtime failure obligations.
/// A successful check records a required failure channel; it does not certify an
/// emitter's implementation of that channel.
pub fn check_grading(program: &Program, machine: &Machine) -> syn::Result<Analysis> {
    let mut analysis = crate::contract::check(program)?;
    // Formal rule BE-Abort: every below-host dispatch records a channel to the
    // distinguished host root, to be discharged by the target's emitter/runtime.
    if symbol_name(&machine.root.name) != "host" {
        return Err(syn::Error::new(
            machine.root.name.span(),
            "BE-Abort: the machine root must be host",
        ));
    }
    let implicit;
    let graded = if let Some(graded) = &program.graded {
        graded
    } else if machine.root.children.is_empty() {
        implicit = crate::Graded {
            machine: machine.name.clone(),
            grades: program
                .sequence
                .definitions
                .iter()
                .map(|definition| crate::Grade {
                    name: definition.name.clone(),
                    level: vec![machine.root.name.clone()],
                })
                .collect(),
        };
        &implicit
    } else {
        return Err(syn::Error::new(
            machine.name.span(),
            "BE-Scope: grading requires a graded machine block",
        ));
    };
    if symbol_name(&graded.machine) != symbol_name(&machine.name) {
        return Err(syn::Error::new(
            graded.machine.span(),
            "BE-Scope: grading names a different machine",
        ));
    }
    let mut levels = Vec::new();
    add_level(&machine.root, Vec::new(), &mut levels)?;
    let paths: BTreeMap<_, _> = levels
        .iter()
        .enumerate()
        .map(|(index, level)| (level.path.clone(), index))
        .collect();
    let definitions: BTreeMap<_, _> = program
        .sequence
        .definitions
        .iter()
        .enumerate()
        .map(|(index, definition)| (symbol_name(&definition.name), index))
        .collect();
    let mut assigned = BTreeMap::new();
    for grade in &graded.grades {
        let name = symbol_name(&grade.name);
        if !definitions.contains_key(&name) {
            return Err(syn::Error::new(
                grade.name.span(),
                "BE-Scope: grade names an unknown Sequence definition",
            ));
        }
        let mut path: Vec<_> = grade.level.iter().map(symbol_name).collect();
        if path.is_empty() {
            return Err(syn::Error::new(
                grade.name.span(),
                "BE-Scope: a grade needs a machine level",
            ));
        }
        if path[0] != "host" {
            path.insert(0, "host".into());
        }
        let level = *paths.get(&path).ok_or_else(|| {
            syn::Error::new(
                grade.name.span(),
                "BE-Scope: grade is not a path in this machine",
            )
        })?;
        if assigned.insert(name, level).is_some() {
            return Err(syn::Error::new(
                grade.name.span(),
                "BE-Scope: a definition must be graded exactly once",
            ));
        }
    }
    let mut grades = Vec::with_capacity(definitions.len());
    for definition in &program.sequence.definitions {
        let name = symbol_name(&definition.name);
        let level = *assigned.get(&name).ok_or_else(|| {
            syn::Error::new(
                definition.name.span(),
                "BE-Scope: every Sequence definition needs a grade",
            )
        })?;
        analysis.levels.insert(name, levels[level].path.clone());
        grades.push(level);
    }
    let physical = program
        .structures
        .physical
        .program
        .definitions
        .iter()
        .map(|definition| (symbol_name(&definition.name), definition))
        .collect();
    let mut checker = Checker {
        program,
        levels,
        grades,
        definitions,
        physical,
        roots: Vec::with_capacity(program.sequence.definitions.len()),
        nodes: Vec::new(),
        collectives: Vec::new(),
        lengths: &analysis.lengths,
        tilings: BTreeMap::new(),
        parts: BTreeMap::new(),
        range_reads: BTreeSet::new(),
        publication: None,
        publication_tile: None,
    };
    for (owner, definition) in program.sequence.definitions.iter().enumerate() {
        let root = checker.visit(
            &definition.term,
            owner,
            checker.grades[owner],
            false,
            true,
            true,
        )?;
        checker.roots.push(root);
    }
    analysis.dispatches = checker.dispatches()?;
    analysis.collectives = checker.collectives;
    analysis.tilings = checker.tilings;
    for key in checker.range_reads {
        if let Some(capability) = analysis.capabilities.get_mut(&key) {
            if *capability != crate::Capability::Ordered {
                *capability = crate::Capability::Range;
            }
        }
        for obligation in &mut analysis.obligations {
            if let Obligation::Provider {
                provider,
                keys,
                capability,
                ..
            } = obligation
            {
                if provider == &key.0 && keys == &key.1 && *capability != crate::Capability::Ordered
                {
                    *capability = crate::Capability::Range;
                }
            }
        }
    }
    for (dispatch, unit) in analysis.dispatches.iter().enumerate() {
        analysis.obligations.push(Obligation::Failure {
            dispatch,
            channel: unit.failure_channel.clone(),
        });
    }
    Ok(analysis)
}

fn add_level<'a>(
    node: &'a LevelNode,
    mut path: Vec<String>,
    levels: &mut Vec<Level<'a>>,
) -> syn::Result<usize> {
    path.push(symbol_name(&node.name));
    let index = levels.len();
    levels.push(Level {
        model: node,
        path: path.clone(),
        children: Vec::new(),
    });
    let mut names = BTreeSet::new();
    for child in &node.children {
        if !names.insert(symbol_name(&child.name)) {
            return Err(syn::Error::new(
                child.name.span(),
                "machine siblings must have distinct names",
            ));
        }
        let child = add_level(child, path.clone(), levels)?;
        levels[index].children.push(child);
    }
    Ok(index)
}

impl<'a> Checker<'a> {
    fn error(&self, owner: usize, message: &str) -> syn::Error {
        syn::Error::new(
            self.program.sequence.definitions[owner].name.span(),
            message,
        )
    }

    fn name(&self, owner: usize) -> String {
        symbol_name(&self.program.sequence.definitions[owner].name)
    }

    fn reference(&self, name: &Ident, owner: usize, level: usize) -> syn::Result<usize> {
        let source = *self
            .definitions
            .get(&symbol_name(name))
            .ok_or_else(|| syn::Error::new(name.span(), "BE-Scope: unknown Sequence definition"))?;
        // Formal rule BE-Scope: the flat definition list gives a store an extent
        // from its definition to the end; references cannot escape upward or
        // precede that extent. Only materialized values may cross level grades.
        if source >= owner {
            return Err(syn::Error::new(
                name.span(),
                "BE-Scope: reads must name an earlier definition",
            ));
        }
        let source_level = self.grades[source];
        let source_path = &self.levels[source_level].path;
        let read_path = &self.levels[level].path;
        if !read_path.starts_with(source_path) {
            // Formal rule BE-Separate: sibling grades never exchange a local
            // value directly, even if its producer made a sibling-local store.
            // Publishing a store at their common ancestor is the only crossing.
            let message = if !source_path.starts_with(read_path) {
                "BE-Separate: sibling levels must exchange through an ancestor store"
            } else {
                "BE-Scope: a descendant's value is outside this read's extent"
            };
            return Err(syn::Error::new(name.span(), message));
        }
        if source_level != level
            && !self.is_stored(self.roots[source])
            && self.publication != Some(source_level)
        {
            return Err(syn::Error::new(
                name.span(),
                "BE-Scope: crossing into an inner level requires an ancestor store",
            ));
        }
        Ok(self.roots[source])
    }

    fn is_stored(&self, mut node: usize) -> bool {
        loop {
            match self.nodes[node].term {
                Term::Store { .. } => return true,
                Term::Ref(_) => node = self.nodes[node].inputs[0],
                _ => return false,
            }
        }
    }

    fn visit(
        &mut self,
        term: &'a Term,
        owner: usize,
        level: usize,
        tiled: bool,
        uniform: bool,
        named: bool,
    ) -> syn::Result<usize> {
        let mut inputs = Vec::new();
        let separated_store = level != 0
            && matches!(term, Term::Store { .. })
            && !self.levels[level].model.collaborative;
        if separated_store && !named {
            return Err(self.error(
                owner,
                "BE-Boundary: name each separated store with its own definition",
            ));
        }
        if separated_store
            && let Term::Store {
                capacity: None,
                inner,
            } = term
            && self.unknown_length(inner)
        {
            return Err(self.error(
                owner,
                "BE-Length: separated unknown store length requires capacity",
            ));
        }
        match term {
            Term::Union(arms) => {
                for arm in arms {
                    inputs.push(self.visit(arm, owner, level, tiled, uniform, false)?);
                }
            }
            Term::Ref(name) => {
                let source = self.reference(name, owner, level)?;
                if level != 0
                    && (tiled || !uniform)
                    && self.requires_collective(source, &mut BTreeSet::new())
                {
                    return Err(self.error(
                        owner,
                        "BE-Collective: a referenced collective lacks whole-level participation",
                    ));
                }
                inputs.push(source);
            }
            Term::Rows(name) => {
                if level != 0 {
                    self.total_physical(name, None)?;
                }
            }
            Term::Map(name, inner) | Term::Compact(name, inner) => {
                if level != 0 {
                    self.total_physical(name, self.physical_result(inner).as_deref())?;
                }
                inputs.push(self.visit(inner, owner, level, tiled, uniform, false)?);
            }
            Term::Lookup {
                provider, inner, ..
            } => {
                if level != 0 {
                    self.total_physical(provider, None)?;
                }
                inputs.push(self.visit(inner, owner, level, tiled, uniform, false)?);
            }
            Term::Tile { n, inner, .. } => {
                let kind = if self
                    .publication_tile
                    .is_some_and(|outer| std::ptr::eq(outer, term))
                {
                    if self
                        .publication
                        .is_none_or(|publication| self.levels[publication].children.len() != 1)
                    {
                        return Err(self
                            .error(owner, "BE-Scope: distributed tile requires one child level"));
                    }
                    self.publication_tile = None;
                    self.parts
                        .entry(self.name(owner))
                        .and_modify(|width| {
                            if *width != Some(*n) {
                                *width = None;
                            }
                        })
                        .or_insert(Some(*n));
                    crate::Tiling::Distribute
                } else {
                    crate::Tiling::Strip
                };
                self.tilings.entry(self.name(owner)).or_default().push(kind);
                inputs.push(self.visit(inner, owner, level, tiled, uniform, false)?);
            }
            Term::Fold(monoid, inner) => {
                self.collective(owner, level, monoid.clone(), tiled, uniform)?;
                if !self.levels[level].children.is_empty() {
                    self.require_ranges(inner);
                }
                inputs.push(self.visit(inner, owner, level, tiled, uniform, false)?);
            }
            Term::Flatten(inner) => {
                if !self.levels[level].children.is_empty() {
                    self.require_ranges(inner);
                }
                // Stored flatten offsets can be read without executing a new
                // scan. Otherwise the offsets require a sum-of-lengths scan;
                // Count denotes its additive counting monoid in Analysis.
                if !self.stored_offsets(inner) {
                    self.collective(owner, level, Monoid::Count, tiled, uniform)?;
                }
                inputs.push(self.visit(inner, owner, level, tiled, uniform, false)?);
            }
            Term::Store { inner, .. } => {
                // A separated store publishes child-produced values, not a
                // collective spanning its mutually separated execution parts.
                let producer = if separated_store && self.levels[level].children.len() == 1 {
                    self.levels[level].children[0]
                } else {
                    level
                };
                let publication = self.publication;
                let publication_tile = self.publication_tile;
                if separated_store {
                    self.publication = Some(level);
                    self.publication_tile = outer_tile(inner);
                }
                let input = self.visit(inner, owner, producer, tiled, uniform, false);
                self.publication = publication;
                self.publication_tile = publication_tile;
                inputs.push(input?);
            }
            Term::Canon(inner) => {
                inputs.push(self.visit(inner, owner, level, tiled, uniform, false)?);
            }
            Term::Choose(alternatives) => {
                // A later else arm also depends on all preceding guards.
                let mut branch_uniform = uniform;
                let publication_tile = self.publication_tile;
                for (guard, alternative) in alternatives {
                    self.publication_tile = publication_tile.and_then(|_| outer_tile(alternative));
                    let guard_uniform = match guard {
                        Guard::Lt(length, _) | Guard::Ge(length, _) => {
                            self.uniform_length(length, owner, level)?
                        }
                        Guard::Else => true,
                    };
                    branch_uniform &= guard_uniform;
                    inputs.push(self.visit(
                        alternative,
                        owner,
                        level,
                        tiled,
                        branch_uniform,
                        false,
                    )?);
                }
            }
        }
        let index = self.nodes.len();
        self.nodes.push(Node {
            term,
            owner,
            level,
            inputs,
            separated_store,
        });
        Ok(index)
    }

    fn requires_collective(&self, index: usize, visited: &mut BTreeSet<usize>) -> bool {
        if !visited.insert(index) {
            return false;
        }
        let node = &self.nodes[index];
        if self.levels[node.level].children.is_empty() {
            return false;
        }
        match node.term {
            Term::Store { .. } => false,
            Term::Fold(_, _) => true,
            Term::Flatten(inner) if !self.stored_offsets(inner) => true,
            _ => node
                .inputs
                .iter()
                .any(|&input| self.requires_collective(input, visited)),
        }
    }

    fn require_ranges(&mut self, term: &'a Term) {
        match term {
            Term::Lookup {
                provider,
                keys,
                inner,
            } => {
                let mut keys: Vec<_> = keys.iter().map(symbol_name).collect();
                keys.sort();
                self.range_reads.insert((symbol_name(provider), keys));
                self.require_ranges(inner);
            }
            Term::Ref(name) => {
                if let Some(&owner) = self.definitions.get(&symbol_name(name)) {
                    self.require_ranges(&self.program.sequence.definitions[owner].term);
                }
            }
            Term::Compact(_, inner) if matches!(inner.as_ref(), Term::Lookup { .. }) => {}
            Term::Map(_, inner)
            | Term::Compact(_, inner)
            | Term::Flatten(inner)
            | Term::Fold(_, inner)
            | Term::Canon(inner)
            | Term::Tile { inner, .. } => self.require_ranges(inner),
            Term::Union(arms) => {
                for arm in arms {
                    self.require_ranges(arm);
                }
            }
            Term::Choose(arms) => {
                for (_, arm) in arms {
                    self.require_ranges(arm);
                }
            }
            Term::Store { .. } | Term::Rows(_) => {}
        }
    }
    fn unknown_length(&self, term: &Term) -> bool {
        match term {
            Term::Ref(name) => matches!(
                self.lengths.get(&symbol_name(name)),
                Some(Len::Unknown(None))
            ),
            Term::Lookup { .. } => true,
            Term::Tile { inner, .. }
            | Term::Store {
                capacity: None,
                inner,
            } => self.unknown_length(inner),
            Term::Union(arms) => arms.iter().any(|arm| self.unknown_length(arm)),
            Term::Choose(arms) => {
                // The contract records joins of unequal branch lengths.
                arms.iter().any(|(_, arm)| self.unknown_length(arm))
                    || arms.windows(2).any(|pair| pair[0].1 != pair[1].1)
            }
            _ => false,
        }
    }
    fn collective(
        &mut self,
        owner: usize,
        level: usize,
        monoid: Monoid,
        tiled: bool,
        uniform: bool,
    ) -> syn::Result<()> {
        if self.levels[level].children.is_empty() {
            return Ok(());
        }
        // Formal rule BE-Collective: only a collaborative grade (or the single
        // host executor) supplies a collective. The syntax has no active-mask
        // or group-width certificate, so a tile-nested collective is not a
        // proven whole-level invocation. Local conditional lengths also cannot
        // certify uniform participation. The built-in monoids are associative.
        if level != 0 && !self.levels[level].model.collaborative {
            return Err(self.error(
                owner,
                "BE-Collective: fold/scan requires a collaborative level",
            ));
        }
        if level != 0 && (tiled || !uniform) {
            return Err(self.error(
                owner,
                "BE-Collective: whole-level participation is not established",
            ));
        }
        self.collectives
            .push((self.name(owner), monoid, self.levels[level].path.clone()));
        Ok(())
    }

    fn uniform_length(&self, length: &Len, owner: usize, level: usize) -> syn::Result<bool> {
        match length {
            Len::Lit(_) => Ok(true),
            Len::CeilDiv(inner, _) => self.uniform_length(inner, owner, level),
            Len::Add(left, right) => {
                let left = self.uniform_length(left, owner, level)?;
                let right = self.uniform_length(right, owner, level)?;
                Ok(left && right)
            }
            Len::Input(name) | Len::Scan(name) => {
                let Some(&source) = self.definitions.get(name) else {
                    // Physical and host input lengths are global size values.
                    return Ok(true);
                };
                self.reference(
                    &self.program.sequence.definitions[source].name,
                    owner,
                    level,
                )?;
                Ok(self.grades[source] == 0
                    || self.levels[self.grades[source]].path.len() < self.levels[level].path.len())
            }
            Len::Unknown(_) => Ok(false),
        }
    }

    fn stored_offsets(&self, term: &Term) -> bool {
        match term {
            Term::Ref(name) => self
                .definitions
                .get(&symbol_name(name))
                .and_then(|&owner| self.roots.get(owner))
                .is_some_and(|&root| self.stored_offsets(self.nodes[root].term)),
            Term::Store { inner, .. } => matches!(inner.as_ref(), Term::Flatten(_)),
            _ => false,
        }
    }

    fn physical_result(&self, term: &Term) -> Option<String> {
        match term {
            Term::Rows(name) | Term::Map(name, _) | Term::Compact(name, _) => {
                Some(symbol_name(name))
            }
            Term::Ref(name) => self
                .definitions
                .get(&symbol_name(name))
                .and_then(|&owner| self.roots.get(owner))
                .and_then(|&root| self.physical_result(self.nodes[root].term)),
            Term::Flatten(inner)
            | Term::Fold(_, inner)
            | Term::Canon(inner)
            | Term::Tile { inner, .. }
            | Term::Store { inner, .. } => self.physical_result(inner),
            Term::Lookup { .. } | Term::Choose(_) | Term::Union(_) => None,
        }
    }

    fn total_physical(&self, result: &Ident, stop: Option<&str>) -> syn::Result<()> {
        // Formal rule BE-Partial: opaque Bind/Guard code cannot be evaluated
        // below host, including through an indirect Physical dependency. A map
        // stops at its explicit input: a host Sequence store can supply an
        // already-evaluated opaque result without re-executing that Rust.
        let mut pending = vec![symbol_name(result)];
        let mut visited = BTreeSet::new();
        while let Some(name) = pending.pop() {
            if Some(name.as_str()) == stop || !visited.insert(name.clone()) {
                continue;
            }
            let Some(definition) = self.physical.get(&name) else {
                continue;
            };
            if matches!(definition.operator, Operator::Bind(_) | Operator::Guard(_)) {
                return Err(syn::Error::new(
                    result.span(),
                    "BE-Partial: opaque Physical Rust must be evaluated at host before dispatch",
                ));
            }
            pending.extend(definition.operator.operands().into_iter().map(symbol_name));
        }
        Ok(())
    }

    fn dispatches(&self) -> syn::Result<Vec<Dispatch>> {
        // Formal rule BE-Boundary: connected below-host work is maximal until
        // a separated store cuts a producer/consumer edge. The outer stored
        // space is traced from the incoming driver edges, never guessed from
        // a launch width. Provider reads are not driver edges. Collaborative
        // stores remain local to their enclosing dispatch.
        // A reconverging edge must not fuse across a store through a second,
        // unstored path. Nodes are in dependency order, so barrier depths are
        // available before connected components are formed.
        let mut depths = vec![0usize; self.nodes.len()];
        for (index, node) in self.nodes.iter().enumerate() {
            if node.level != 0 {
                depths[index] = node
                    .inputs
                    .iter()
                    .filter(|&&input| self.nodes[input].level != 0)
                    .map(|&input| depths[input] + usize::from(self.nodes[input].separated_store))
                    .max()
                    .unwrap_or(0);
            }
        }
        let mut parents: Vec<_> = (0..self.nodes.len()).collect();
        for (index, node) in self.nodes.iter().enumerate() {
            if node.level == 0 {
                continue;
            }
            for &input in &node.inputs {
                let source = &self.nodes[input];
                if source.level != 0
                    && !source.separated_store
                    && self.region(node.level) == self.region(source.level)
                    && depths[index] == depths[input]
                {
                    let left = representative(&mut parents, index);
                    let right = representative(&mut parents, input);
                    parents[left] = right;
                }
            }
        }
        let mut groups: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
        for (index, node) in self.nodes.iter().enumerate() {
            if node.level != 0 {
                let group = representative(&mut parents, index);
                groups.entry(group).or_default().push(index);
            }
        }
        // Stable program/dependency order, rather than union-find root order.
        let mut groups: Vec<_> = groups.into_values().collect();
        groups.sort_by_key(|members| members[0]);
        let mut result = Vec::with_capacity(groups.len());
        for members in groups {
            let member_set: BTreeSet<_> = members.iter().copied().collect();
            let mut domains = BTreeSet::new();
            let mut owners = BTreeSet::new();
            let mut stores = BTreeSet::new();
            let mut visited = BTreeSet::new();
            for &index in &members {
                let node = &self.nodes[index];
                owners.insert(node.owner);
                if matches!(node.term, Term::Store { .. }) {
                    stores.insert(node.owner);
                }
                if let Term::Rows(name) = node.term {
                    domains.insert(symbol_name(name));
                }
                for &input in &node.inputs {
                    if !member_set.contains(&input) {
                        if self.nodes[input].level != 0 && !self.nodes[input].separated_store {
                            return Err(self.error(
                                node.owner,
                                "BE-Boundary: a value crossing a dispatch boundary must be stored",
                            ));
                        }
                        self.domains(input, &mut domains, &mut visited);
                    }
                }
            }
            // A host store can enclose a tile whose work is in this dispatch;
            // publication belongs to that dispatch even though the store's
            // allocation/extent itself is at host.
            for (owner, &root) in self.roots.iter().enumerate() {
                if self.nodes[root].level == 0
                    && matches!(self.nodes[root].term, Term::Store { .. })
                    && self.nodes[root]
                        .inputs
                        .iter()
                        .any(|&input| self.reaches(input, &member_set, &mut BTreeSet::new()))
                {
                    stores.insert(owner);
                }
            }
            let first = &self.nodes[members[0]];
            if domains.len() != 1 {
                return Err(self.error(
                    first.owner,
                    "BE-Boundary: a dispatch must have one unambiguous outer stored space",
                ));
            }
            let level = self.levels[first.level].path[..2].to_vec();
            result.push(Dispatch {
                outer_store: domains.into_iter().next().expect("one domain was checked"),
                members: owners.into_iter().map(|owner| self.name(owner)).collect(),
                inner_stores: stores.into_iter().map(|owner| self.name(owner)).collect(),
                failure_channel: level.iter().rev().cloned().collect(),
                level,
                producer_level: members
                    .iter()
                    .filter_map(|&index| {
                        let node = &self.nodes[index];
                        node.separated_store
                            .then(|| node.inputs.first().copied())
                            .flatten()
                    })
                    .map(|input| self.levels[self.nodes[input].level].path.clone())
                    .next()
                    .unwrap_or_else(|| self.levels[first.level].path.clone()),
                parts: members.iter().find_map(|&index| {
                    let node = &self.nodes[index];
                    if !node.separated_store {
                        return None;
                    }
                    self.parts.get(&self.name(node.owner)).copied().flatten()
                }),
                concatenation: if members.iter().any(|&index| {
                    self.nodes[index].owner + 1 == self.program.sequence.definitions.len()
                }) {
                    self.program
                        .sequence
                        .output
                        .clone()
                        .unwrap_or(crate::OutputProtocol::Count)
                } else {
                    crate::OutputProtocol::Count
                },
            });
        }
        Ok(result)
    }

    // One entry branch below host owns its collaborative and per-part levels.
    // A store at any separated descendant still cuts the dataflow above. This
    // avoids incorrectly turning every lane of a collaborative group into an
    // independent dispatch just because lanes themselves are not collaborative.
    fn region(&self, level: usize) -> &str {
        &self.levels[level].path[1]
    }

    fn domains(&self, index: usize, domains: &mut BTreeSet<String>, visited: &mut BTreeSet<usize>) {
        if !visited.insert(index) {
            return;
        }
        let node = &self.nodes[index];
        match node.term {
            Term::Store { .. } => {
                domains.insert(self.name(node.owner));
            }
            Term::Rows(name) => {
                domains.insert(symbol_name(name));
            }
            _ => {
                for &input in &node.inputs {
                    self.domains(input, domains, visited);
                }
            }
        }
    }

    fn reaches(
        &self,
        index: usize,
        members: &BTreeSet<usize>,
        visited: &mut BTreeSet<usize>,
    ) -> bool {
        if members.contains(&index) {
            return true;
        }
        let node = &self.nodes[index];
        if node.level != 0 || matches!(node.term, Term::Store { .. }) || !visited.insert(index) {
            return false;
        }
        node.inputs
            .iter()
            .any(|&input| self.reaches(input, members, visited))
    }
}

fn representative(parents: &mut [usize], mut index: usize) -> usize {
    while parents[index] != index {
        let parent = parents[index];
        parents[index] = parents[parent];
        index = parents[index];
    }
    index
}

/// The first tile applied to input data is the outer partition of the shape,
/// even though it is the innermost tile constructor in the syntax tree.
fn outer_tile(term: &Term) -> Option<&Term> {
    match term {
        Term::Tile { inner, .. } => outer_tile(inner).or(Some(term)),
        Term::Map(_, inner)
        | Term::Compact(_, inner)
        | Term::Flatten(inner)
        | Term::Fold(_, inner)
        | Term::Canon(inner)
        | Term::Store { inner, .. }
        | Term::Lookup { inner, .. } => outer_tile(inner),
        Term::Choose(arms) => arms.iter().find_map(|(_, arm)| outer_tile(arm)),
        Term::Union(arms) => arms.iter().find_map(outer_tile),
        Term::Rows(_) | Term::Ref(_) => None,
    }
}
