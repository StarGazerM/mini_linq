use cq_lang_structures::{PhysicalEnv, PhysicalFacts, Section, UniqueOp, contract};
use std::collections::BTreeMap;
struct Dialect(PhysicalFacts);
impl PhysicalEnv for Dialect {
    fn physical(&self) -> &PhysicalFacts {
        &self.0
    }
    fn host_layouts(&self) -> BTreeMap<String, Vec<String>> {
        BTreeMap::new()
    }
    fn results(&self) -> Vec<(String, UniqueOp)> {
        vec![("u".into(), UniqueOp::Unit)]
    }
    fn merge_keys(&self) -> BTreeMap<String, Vec<String>> {
        BTreeMap::new()
    }
    fn output(&self) -> String {
        "u".into()
    }
}
#[test]
fn infers_uniqueness_without_the_reference_physical_language() {
    let env = Dialect(PhysicalFacts {
        headings: BTreeMap::from([("u".into(), vec![])]),
        ..PhysicalFacts::default()
    });
    let section: Section = syn::parse_quote!(structures {});
    let analysis = contract::check(&section, &env).unwrap();
    assert_eq!(analysis.unique["u"], Vec::<String>::new());
}
