use cq_lang_structures::{self as s, execution as e};
use std::collections::BTreeMap;
struct Source {
    facts: s::PhysicalFacts,
    definitions: Vec<e::Definition>,
    row: e::Row,
    returned: syn::Ident,
    head: s::syntax::Atom,
}
impl Source {
    fn new() -> Self {
        Self {
            facts: s::PhysicalFacts {
                headings: [("u".into(), vec![])].into(),
                modes: [("u".into(), s::ModeKind::Pipe)].into(),
                scanned: ["u".into()].into(),
                ..Default::default()
            },
            definitions: vec![e::Definition {
                name: syn::parse_quote!(u),
                mode: s::ModeKind::Pipe,
                canon: false,
                operator: e::Operator::Unit,
            }],
            row: e::Row::default(),
            returned: syn::parse_quote!(u),
            head: syn::parse_quote!(answer()),
        }
    }
}
impl s::PhysicalEnv for Source {
    fn physical(&self) -> &s::PhysicalFacts {
        &self.facts
    }
    fn host_layouts(&self) -> BTreeMap<String, Vec<String>> {
        BTreeMap::new()
    }
    fn results(&self) -> Vec<(String, s::UniqueOp)> {
        vec![("u".into(), s::UniqueOp::Unit)]
    }
    fn merge_keys(&self) -> BTreeMap<String, Vec<String>> {
        BTreeMap::new()
    }
    fn output(&self) -> String {
        "u".into()
    }
}
impl s::Env for Source {
    fn inputs(&self) -> Vec<s::syntax::RelationDecl> {
        vec![]
    }
    fn definitions(&self) -> &[e::Definition] {
        &self.definitions
    }
    fn row(&self, _: &str) -> &e::Row {
        &self.row
    }
    fn returned(&self) -> &syn::Ident {
        &self.returned
    }
    fn head(&self) -> &s::syntax::Atom {
        &self.head
    }
}

#[test]
fn prepares_bindings_from_a_custom_structures_environment() {
    let env = Source::new();
    let section = syn::parse_quote!(structures {});
    let analysis = s::contract::check(&section, &env).unwrap();
    let mut context = cq_print::Context::new(&section, &env, &analysis);
    context.register("u");
    assert!(context.heading("u").is_empty());
    assert_eq!(context.pattern("u").to_string(), "()");
}
