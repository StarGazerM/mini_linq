use crate::{Result, Scale, ScenarioSelection, Selection};

pub(super) fn check_scale(selection: Selection, scale: Scale) -> Result<()> {
    if scale != Scale::Tiny && selection.extensions().next().is_some() {
        return Err("the extended corpus supports tiny fixtures; select classic or retail for larger generated data".into());
    }
    Ok(())
}
pub(super) fn generate(selection: Selection, scenarios: ScenarioSelection) -> Result<()> {
    for case in selection.extensions() {
        for scenario in scenarios.values() {
            let schema = case.schema()?;
            let directory = case.path("data").join(scenario.as_str()).join("tiny");
            case.generate(scenario)?.write_schema(&schema, &directory)?;
            println!(
                "generated {:<24} {:<11} {}",
                case.name,
                scenario.as_str(),
                directory.display()
            );
        }
    }
    Ok(())
}
#[cfg(feature = "oracle")]
pub(super) fn golden(
    selection: Selection,
    scenarios: ScenarioSelection,
    write: bool,
) -> Result<Vec<std::path::PathBuf>> {
    let mut checked = Vec::new();
    for case in selection.extensions() {
        for scenario in scenarios.values() {
            let dataset = case.load(scenario)?;
            let expected = crate::oracle::evaluate_sql(&dataset.relations, case.sql)?;
            let path = case.path("golden").join(scenario.as_str()).join("tiny.csv");
            checked.push(path.clone());
            if write {
                cq_workloads::dataset::write_result_rows(&path, &expected, case.arity)?;
                println!(
                    "wrote {:<28} {:<11} {}",
                    case.name,
                    scenario.as_str(),
                    path.display()
                );
            } else {
                crate::golden::compare(&path, &expected, case.arity)?;
                println!(
                    "golden {:<27} {:<11} {} rows",
                    case.name,
                    scenario.as_str(),
                    expected.len()
                );
            }
        }
    }
    Ok(checked)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn extended_selection_and_scale_have_no_positive_cq_cases() {
        let selection = Selection::parse("extended").unwrap();
        assert_eq!(selection.cases().count(), 0);
        assert_eq!(
            selection.extensions().count(),
            cq_workloads::extended::CASES.len()
        );
        assert!(check_scale(selection, Scale::Tiny).is_ok());
        assert!(check_scale(selection, Scale::Medium).is_err());
        let selection = Selection::parse("recursion_reach").unwrap();
        assert_eq!(selection.extensions().count(), 1);
        assert_eq!(selection.cases().count(), 0);
    }
}
