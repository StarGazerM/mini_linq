//! D-ATOM / D-GUARD / D-BIND derive clause premises; D-MOVE and D-GRAPH
//! establish the current optimizer domain. See doc/RUST-EXPRESSIONS.md.
//! These are judgments over CQ, not a language or an IR-to-IR pass.
use super::{Heading, Rule};
use cq_ir::{cq, symbol_name};
use std::collections::BTreeMap;

pub(crate) struct ClauseFacts {
    pub required: Heading,
    // All bare atom variables can provide a join column, regardless of which
    // occurrence introduced the name in the written order.
    pub provided: Heading,
    pub reorderable: bool,
    pub atom_arity: Option<usize>,
    pub literal_arguments: Vec<usize>,
}

pub(crate) struct ClauseDependencies {
    pub clauses: Vec<ClauseFacts>,
    /// D-GRAPH is the domain of the existing graph kernel, not an acyclicity test.
    pub supports_graph_rules: bool,
}

impl ClauseDependencies {
    /// D-ATOM / D-GUARD / D-BIND, followed by D-MOVE and D-GRAPH.
    /// `bound` is the written prefix; `primitive` accumulates *all* occurrences.
    pub fn analyze(source: &cq::Module) -> syn::Result<Self> {
        let schema: BTreeMap<_, _> = source
            .program
            .inputs()
            .map(|d| (symbol_name(&d.name), d))
            .collect();
        let mut clauses = Vec::new();
        let mut bound = Heading::new();
        let mut let_bound = Heading::new();
        let mut primitive = BTreeMap::new();
        let mut comparisons = Vec::new();
        let mut supports_graph_rules = source.program.query.head.is_normalized();
        for (clause, item) in source.program.query.body.iter().enumerate() {
            let facts = match item {
                cq::BodyItem::Positive { atom } => {
                    let declaration = schema
                        .get(&symbol_name(&atom.relation))
                        .ok_or_else(|| syn::Error::new_spanned(atom, "undeclared atom"))?;
                    let mut facts = ClauseFacts::atom(atom, declaration, &bound, &mut primitive);
                    // A preceding let supplies a value, not an interchangeable
                    // atom occurrence: matching its name must wait for that let.
                    facts
                        .required
                        .extend(facts.provided.intersection(&let_bound).cloned());
                    supports_graph_rules &= facts.provided.len() == atom.args.len();
                    facts
                }
                cq::BodyItem::Filter(filter) => {
                    // D-GUARD. D-MOVE resolves the type premise after all atoms.
                    if let Some(predicate) = cq::Predicate::from_expr(&filter.condition) {
                        comparisons.push((clause, predicate.variables()));
                    }
                    ClauseFacts {
                        required: cq::rust::free_variables(&filter.condition).dependencies(&bound),
                        provided: Heading::new(),
                        reorderable: false,
                        atom_arity: None,
                        literal_arguments: Vec::new(),
                    }
                }
                cq::BodyItem::Let(binding) => {
                    // D-BIND. Pattern bindings are fresh by the CQ contract.
                    let mut provided = Heading::new();
                    for (id, ty) in binding.binding_types() {
                        let name = symbol_name(&id);
                        provided.insert(name.clone());
                        let_bound.insert(name.clone());
                        primitive.insert(name, cq::rust::primitive_i32(&ty));
                    }
                    supports_graph_rules = false;
                    let reorderable = matches!(&binding.pattern, syn::Pat::Ident(p)
                        if p.by_ref.is_none() && p.subpat.is_none())
                        && cq::rust::literal(&binding.expression);
                    ClauseFacts {
                        required: cq::rust::free_variables(&binding.expression)
                            .dependencies(&bound),
                        provided,
                        reorderable,
                        atom_arity: None,
                        literal_arguments: Vec::new(),
                    }
                }
                cq::BodyItem::Negation(_) | cq::BodyItem::Aggregate(_) => {
                    return Err(syn::Error::new_spanned(
                        item,
                        "negation and aggregation require the extended CQ semantic contract",
                    ));
                }
            };
            bound.extend(facts.provided.iter().cloned());
            clauses.push(facts);
        }
        // D-MOVE: free variables establish availability, never purity.
        for (clause, variables) in comparisons {
            clauses[clause].reorderable = variables.iter().all(|v| primitive.get(v) == Some(&true));
        }
        // D-GRAPH: this restricted domain supports P-* and Y-* rules directly.
        supports_graph_rules &= clauses.iter().all(|c| c.reorderable)
            && source
                .program
                .query
                .head
                .variables()
                .all(|v| bound.contains(&symbol_name(v)));
        Ok(Self {
            clauses,
            supports_graph_rules,
        })
    }

    pub fn normalization_rules(&self) -> impl Iterator<Item = Rule> + '_ {
        self.clauses.iter().enumerate().flat_map(|(clause, info)| {
            info.literal_arguments
                .iter()
                .map(move |&argument| Rule::Constant { clause, argument })
        })
    }
}

impl ClauseFacts {
    /// D-ATOM: earlier arguments extend L; expression inputs are uses(B ∪ L) \ L.
    fn atom(
        atom: &cq::Atom,
        declaration: &cq::RelationDecl,
        prefix: &Heading,
        primitive: &mut BTreeMap<String, bool>,
    ) -> Self {
        let mut facts = Self {
            required: Heading::new(),
            provided: Heading::new(),
            reorderable: true,
            atom_arity: Some(atom.args.len()),
            literal_arguments: Vec::new(),
        };
        for (position, (arg, column)) in atom.args.iter().zip(&declaration.columns).enumerate() {
            if let Some(id) = cq::rust::variable(arg) {
                let name = symbol_name(id);
                facts.provided.insert(name.clone());
                *primitive.entry(name).or_insert(true) &= cq::rust::primitive_i32(&column.ty);
            } else if cq::rust::literal(arg) {
                facts.literal_arguments.push(position);
            } else if !matches!(arg, syn::Expr::Infer(_)) {
                let available = prefix.union(&facts.provided).cloned().collect();
                facts.required.extend(
                    cq::rust::free_variables(arg)
                        .dependencies(&available)
                        .difference(&facts.provided)
                        .cloned(),
                );
                facts.reorderable = false;
            }
        }
        facts
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clause_facts_distinguish_outer_inputs_local_bindings_and_opaque_uses() {
        let source: cq::Module = syn::parse_quote! {
            struct P;
            relation R(a:i32,b:i32); relation S(a:i32,b:i32); relation T(a:i32,b:i32);
            answer(x) :- R(x,witness),
                let y:i32 = { let x = 1; *witness + x },
                if accept(y), if accept!(y),
                S(y,z), T(z,transform(x)), S(k,next(k)), R(7,extra);
        };
        cq::contract::check(&source).unwrap();
        let facts = ClauseDependencies::analyze(&source).unwrap();
        assert!(!facts.supports_graph_rules);
        let expected = [
            ("", "witness x", true, Some(2)),
            ("witness", "y", false, None),
            ("y", "", false, None),
            ("witness x y", "", false, None),
            ("y", "y z", true, Some(2)),
            ("x", "z", false, Some(2)),
            ("", "k", false, Some(2)),
            ("", "extra", true, Some(2)),
        ];
        assert_eq!(facts.clauses.len(), expected.len());
        for (fact, (inputs, outputs, reorderable, arity)) in facts.clauses.iter().zip(expected) {
            assert_eq!(
                fact.required
                    .iter()
                    .map(String::as_str)
                    .collect::<Vec<_>>()
                    .join(" "),
                inputs
            );
            assert_eq!(
                fact.provided
                    .iter()
                    .map(String::as_str)
                    .collect::<Vec<_>>()
                    .join(" "),
                outputs
            );
            assert_eq!(fact.reorderable, reorderable);
            assert_eq!(fact.atom_arity, arity);
        }
        assert_eq!(
            facts.normalization_rules().collect::<Vec<_>>(),
            vec![Rule::Constant {
                clause: 7,
                argument: 0
            }]
        );
    }

    #[test]
    fn comparison_movement_requires_primitive_types_at_every_occurrence() {
        for body in ["R(x), S(x), if x > 0", "S(x), R(x), if x > 0"] {
            for (ty, expected) in [
                ("i32", true),
                ("::core::primitive::i32", true),
                ("Alias", false),
            ] {
                let source: cq::Module = syn::parse_str(&format!(
                    "struct P; relation R(a:{ty}); relation S(a:i32); answer(x) :- {body};"
                ))
                .unwrap();
                let facts = ClauseDependencies::analyze(&source).unwrap();
                assert_eq!(facts.clauses[2].reorderable, expected, "{body}, {ty}");
                assert_eq!(facts.supports_graph_rules, expected, "{body}, {ty}");
            }
        }
    }
}
