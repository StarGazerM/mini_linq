use cq_lang_ix::{Join, RaEnv, Section, contract};
struct Dialect {
    heading: Vec<String>,
}
impl RaEnv for Dialect {
    fn heading(&self, result: &str) -> Option<&[String]> {
        matches!(result, "l" | "r").then_some(self.heading.as_slice())
    }
    fn joins(&self) -> Vec<Join> {
        vec![Join {
            result: "j".into(),
            left: "l".into(),
            right: "r".into(),
            semi: false,
        }]
    }
}
#[test]
fn checks_a_section_against_an_unrelated_producer() {
    let env = Dialect {
        heading: vec!["x".into()],
    };
    let section: Section = syn::parse_quote!(indexes { key = @r[0]; });
    contract::check(&section, &env).unwrap();
    let bad: Section = syn::parse_quote!(indexes { key = @r[1]; });
    assert!(contract::check(&bad, &env).is_err());
}
