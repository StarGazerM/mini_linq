//! Literal-only disjointness, independently derived from Physical operands.
//! A fallback's negative references prove arm disjointness by construction.

use super::*;
use cq_syntax::algebra::{LiteralBindings, constant_disjoint, literal_value};

pub(super) fn disjoint(program: &Program) -> BTreeSet<String> {
    let mut constants: BTreeMap<String, LiteralBindings> = BTreeMap::new();
    let mut disjoint = BTreeSet::new();
    for definition in &program.program.definitions {
        let of = |id: &Ident| constants[&symbol_name(id)].clone();
        let values = match &definition.operator {
            Operator::Unit(_) | Operator::Rename(_) => LiteralBindings::new(),
            Operator::Probe(_, join) | Operator::Merge(_, join) => {
                let mut values = of(&join.left);
                values.extend(of(&join.right));
                values
            }
            Operator::Semi(_, join) | Operator::Anti(_, join) => of(&join.left),
            Operator::Select(select) | Operator::Point(_, select) | Operator::Range(_, select) => {
                let mut values = of(&select.input);
                values.extend(select.condition.literal_bindings());
                values
            }
            Operator::Guard(guard) => of(&guard.input),
            Operator::Bind(bind) => {
                let mut values = of(&bind.input);
                if let syn::Pat::Ident(pattern) = &bind.pattern {
                    if let Some(value) = literal_value(&bind.expression) {
                        values.insert(symbol_name(&pattern.ident), value);
                    }
                }
                values
            }
            Operator::Project(project) => {
                let mut values = of(&project.input);
                values
                    .retain(|name, _| project.attributes.iter().any(|id| symbol_name(id) == *name));
                values
            }
            Operator::Union(union) => {
                let interface = union
                    .origin
                    .as_ref()
                    .map(|o| o.interface.iter().map(symbol_name).collect())
                    .unwrap_or_else(|| {
                        constants[&symbol_name(&union.operands[0])]
                            .keys()
                            .cloned()
                            .collect()
                    });
                if union.origin.as_ref().is_some_and(|o| o.fallback.is_some())
                    || constant_disjoint(
                        union.operands.iter().map(|id| &constants[&symbol_name(id)]),
                        &interface,
                    )
                {
                    disjoint.insert(symbol_name(&definition.name));
                }
                let mut values = of(&union.operands[0]);
                values.retain(|name, value| {
                    union
                        .operands
                        .iter()
                        .all(|id| constants[&symbol_name(id)].get(name) == Some(value))
                });
                values
            }
        };
        constants.insert(symbol_name(&definition.name), values);
    }
    disjoint
}
