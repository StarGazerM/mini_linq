//! Semantic inputs to IX consumers. These are operations, not a reference RA AST:
//! no parser, CQ program, or RA token wrappers cross this boundary.
use crate::{RaEnv, syntax};
use std::collections::BTreeMap;
use syn::{Expr, Ident};

#[derive(Clone, Debug)]
pub enum Operation {
    Unit,
    Rename {
        host: Ident,
        mapping: BTreeMap<Ident, Ident>,
    },
    Join {
        left: Ident,
        right: Ident,
    },
    Semijoin {
        left: Ident,
        right: Ident,
    },
    Select {
        input: Ident,
        condition: Expr,
    },
    Extend {
        input: Ident,
        columns: Vec<syntax::ColumnDecl>,
        value: Expr,
    },
    Project {
        input: Ident,
    },
}

/// The checked producer of an IX section. Headings are sorted; operations are
/// in dependency order. Identifiers retain their Rust macro hygiene. An
/// alternate surface lowers its operators to this vocabulary before IX.
pub trait Env: RaEnv {
    fn inputs(&self) -> Vec<syntax::RelationDecl>;
    fn operations(&self) -> Vec<(Ident, Operation)>;
    fn returned(&self) -> Ident;
    fn head(&self) -> &syntax::Atom;
}

/// Row bindings and computed-column provenance for the semantic operations.
#[derive(Clone, Debug, Default)]
pub struct RowFacts {
    pub bindings: BTreeMap<String, Ident>,
    pub computed: std::collections::BTreeSet<String>,
}
pub fn rows(env: &impl Env) -> BTreeMap<String, RowFacts> {
    use syntax::symbol_name;
    let mut rows: BTreeMap<String, RowFacts> = BTreeMap::new();
    for (name, op) in env.operations() {
        let of = |input: &Ident| rows[&symbol_name(input)].clone();
        let row = match op {
            Operation::Unit => RowFacts::default(),
            Operation::Rename { mapping, .. } => RowFacts {
                bindings: mapping
                    .into_keys()
                    .map(|id| (symbol_name(&id), id))
                    .collect(),
                computed: Default::default(),
            },
            Operation::Join { left, right } => {
                let mut row = of(&right);
                let left = of(&left);
                row.bindings.extend(left.bindings);
                row.computed.extend(left.computed);
                row
            }
            Operation::Semijoin { left: input, .. } | Operation::Select { input, .. } => of(&input),
            Operation::Extend { input, columns, .. } => {
                let mut row = of(&input);
                for column in columns {
                    let key = symbol_name(&column.name);
                    row.computed.insert(key.clone());
                    row.bindings.insert(key, column.name);
                }
                row
            }
            Operation::Project { input } => {
                let mut row = of(&input);
                let heading = env.heading(&symbol_name(&name)).expect("checked heading");
                row.bindings.retain(|c, _| heading.contains(c));
                row.computed.retain(|c| heading.contains(c));
                row
            }
        };
        rows.insert(symbol_name(&name), row);
    }
    rows
}
