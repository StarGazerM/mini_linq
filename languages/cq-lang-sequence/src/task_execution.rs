//! Printable producer identities and carrier publications for host/task translation.
use crate::structures::symbol_name;
use crate::{Analysis, Guard, Program, Term, Tiling};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct TermId {
    pub owner: String,
    pub ordinal: usize,
}

/// Formal rule BE-CarrierPublication: every carrier buffer has a stable publication identity and symbolic row volume.
#[derive(Clone, Debug)]
pub enum Staging {
    /// A non-borrowable distributing driver's complete input is retained once.
    TileInput,
    /// Only the selected arm is retained, recursively preserving its dimensions.
    Choice { pull_runtime: bool },
}

#[derive(Clone, Debug)]
pub struct Node {
    pub id: TermId,
    pub producers: BTreeSet<TermId>,
    pub staging: Option<Staging>,
    pub borrowed_host: Option<String>,
}

#[derive(Clone, Debug, Default)]
pub struct Execution {
    pub definitions: BTreeMap<String, Vec<Node>>,
}

/// Formal rule BE-TaskProducer: term ancestry, never emitted identifiers, determines task ownership.
pub fn analyze(program: &Program, analysis: &Analysis) -> Execution {
    fn input_result(term: &Term, analysis: &Analysis) -> Option<String> {
        match term {
            Term::Rows(name) => Some(symbol_name(name)),
            Term::Ref(name) => analysis.results.get(&symbol_name(name)).cloned(),
            Term::Store { inner, .. } => input_result(inner, analysis),
            _ => None,
        }
    }
    fn visit(
        term: &Term,
        owner: &str,
        ordinal: &mut usize,
        tile: &mut usize,
        program: &Program,
        analysis: &Analysis,
        named: &BTreeMap<String, BTreeSet<TermId>>,
        nodes: &mut Vec<Node>,
    ) -> BTreeSet<TermId> {
        let index = *ordinal;
        *ordinal += 1;
        let id = TermId {
            owner: owner.into(),
            ordinal: index,
        };
        nodes.push(Node {
            id: id.clone(),
            producers: BTreeSet::new(),
            staging: None,
            borrowed_host: None,
        });
        let distributing = if let Term::Tile { inner, .. } = term {
            let yes =
                analysis.tilings.get(owner).and_then(|v| v.get(*tile)) == Some(&Tiling::Distribute);
            nodes[index].borrowed_host = analysis
                .host_tiles
                .get(owner)
                .and_then(|tiles| tiles.get(*tile))
                .as_ref()
                .filter(|source| source.is_some())
                .and_then(|_| input_result(inner, analysis))
                .filter(|result| {
                    analysis.structures.physical.hosts.contains_key(result)
                        && !analysis.structures.selections.contains_key(result)
                        && program.structures.physical.program.definitions.iter().any(
                            |definition| {
                                symbol_name(&definition.name) == *result
                                    && matches!(&definition.operator,
                                    crate::structures::physical::Operator::Rename(rename)
                                    if !matches!(analysis.structures.repr.get(result),
                                        Some(crate::structures::Repr::Positions { .. }))
                                        || rename.constants.is_none())
                            },
                        )
                });
            *tile += 1;
            yes
        } else {
            false
        };
        let mut producers = BTreeSet::new();
        match term {
            Term::Ref(name) => {
                producers.extend(named.get(&symbol_name(name)).into_iter().flatten().cloned())
            }
            Term::Rows(_) => {}
            Term::Choose(arms) => {
                for (_, arm) in arms {
                    producers.extend(visit(
                        arm, owner, ordinal, tile, program, analysis, named, nodes,
                    ));
                }
            }
            Term::Union(arms) => {
                for arm in arms {
                    producers.extend(visit(
                        arm, owner, ordinal, tile, program, analysis, named, nodes,
                    ));
                }
            }
            Term::Map(_, inner)
            | Term::Compact(_, inner)
            | Term::Flatten(inner)
            | Term::Tile { inner, .. }
            | Term::PartitionK { inner, .. }
            | Term::Lookup { inner, .. }
            | Term::Fold(_, inner)
            | Term::Group(_, inner)
            | Term::Canon(inner)
            | Term::Store { inner, .. } => {
                producers = visit(inner, owner, ordinal, tile, program, analysis, named, nodes);
            }
        }
        if distributing {
            producers.insert(id);
        }
        nodes[index].producers = producers.clone();
        nodes[index].staging = if distributing && nodes[index].borrowed_host.is_none() {
            Some(Staging::TileInput)
        } else if let Term::Choose(arms) = term {
            Some(Staging::Choice {
                pull_runtime: arms
                    .iter()
                    .any(|(guard, _)| matches!(guard, Guard::WorkersLt(_))),
            })
        } else {
            None
        };
        producers
    }
    let mut execution = Execution::default();
    let mut named = BTreeMap::new();
    for definition in &program.sequence.definitions {
        let owner = symbol_name(&definition.name);
        let mut nodes = Vec::new();
        let producers = visit(
            &definition.term,
            &owner,
            &mut 0,
            &mut 0,
            program,
            analysis,
            &named,
            &mut nodes,
        );
        named.insert(owner.clone(), producers);
        execution.definitions.insert(owner, nodes);
    }
    execution
}

impl Execution {
    /// Index stable records against this program's terms; addresses are only
    /// lookup handles, never producer identities or printable analysis keys.
    pub fn terms<'a>(&'a self, program: &Program) -> BTreeMap<*const Term, &'a Node> {
        fn walk<'a>(
            term: &Term,
            nodes: &mut impl Iterator<Item = &'a Node>,
            out: &mut BTreeMap<*const Term, &'a Node>,
        ) {
            out.insert(term, nodes.next().expect("checked term preorder"));
            match term {
                Term::Rows(_) | Term::Ref(_) => {}
                Term::Choose(arms) => {
                    for (_, arm) in arms {
                        walk(arm, nodes, out);
                    }
                }
                Term::Union(arms) => {
                    for arm in arms {
                        walk(arm, nodes, out);
                    }
                }
                Term::Map(_, inner)
                | Term::Compact(_, inner)
                | Term::Flatten(inner)
                | Term::Tile { inner, .. }
                | Term::PartitionK { inner, .. }
                | Term::Lookup { inner, .. }
                | Term::Fold(_, inner)
                | Term::Group(_, inner)
                | Term::Canon(inner)
                | Term::Store { inner, .. } => walk(inner, nodes, out),
            }
        }
        let mut out = BTreeMap::new();
        for definition in &program.sequence.definitions {
            walk(
                &definition.term,
                &mut self.definitions[&symbol_name(&definition.name)].iter(),
                &mut out,
            );
        }
        out
    }

    /// Conservative retained-row traffic: at most one complete driver input,
    /// or exactly the selected arm's rows. Borrowed drivers incur zero traffic.
    pub fn staging(&self, pull: bool) -> impl Iterator<Item = (&Node, &Staging)> {
        self.definitions.values().flatten().filter_map(move |node| {
            let staging = node.staging.as_ref()?;
            if pull
                && matches!(
                    staging,
                    Staging::TileInput
                        | Staging::Choice {
                            pull_runtime: false
                        }
                )
            {
                None
            } else {
                Some((node, staging))
            }
        })
    }
}
