//! Feature x route coverage and the shared failure report format.

use super::Case;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

/// Cases per (feature, route), split into passed and failed.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RouteCoverage {
    cells: BTreeMap<(String, String), (usize, usize)>,
    routes: BTreeSet<String>,
}

impl RouteCoverage {
    /// Count every feature of `case` once for `route`.
    pub fn record(&mut self, route: &str, case: &Case, passed: bool) {
        self.routes.insert(route.to_owned());
        for feature in case.features() {
            let cell = self.cells.entry((feature, route.to_owned())).or_default();
            if passed {
                cell.0 += 1;
            } else {
                cell.1 += 1;
            }
        }
    }

    /// (passed, failed) cases with `feature` on `route`.
    pub fn count(&self, feature: &str, route: &str) -> (usize, usize) {
        self.cells
            .get(&(feature.to_owned(), route.to_owned()))
            .copied()
            .unwrap_or_default()
    }

    /// Required features that some recorded route never exercised.
    pub fn gaps<'a>(&self, required: &[&'a str]) -> Vec<(&'a str, String)> {
        let mut gaps = Vec::new();
        for feature in required {
            for route in &self.routes {
                if self.count(feature, route) == (0, 0) {
                    gaps.push((*feature, route.clone()));
                }
            }
        }
        gaps
    }
}

impl fmt::Display for RouteCoverage {
    /// A tab-separated table: one feature per row, `passed/total` per route.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "feature")?;
        for route in &self.routes {
            write!(f, "\t{route}")?;
        }
        writeln!(f)?;
        let features = self
            .cells
            .keys()
            .map(|(feature, _)| feature)
            .collect::<BTreeSet<_>>();
        for feature in features {
            write!(f, "{feature}")?;
            for route in &self.routes {
                let (passed, failed) = self.count(feature, route);
                write!(f, "\t{passed}/{}", passed + failed)?;
            }
            writeln!(f)?;
        }
        Ok(())
    }
}

/// The shared failure report: `FUZZ FAILURE seed=… route=… stage=…`, the
/// minimized case's CQ, data, and reference answer.
pub fn failure_report(case: &Case, route: &str, stage: &str, minimized: &Case) -> String {
    let data = minimized
        .query
        .schema
        .iter()
        .zip(&minimized.data)
        .map(|(relation, rows)| {
            format!(
                "{}: [{}]",
                relation.name,
                rows.iter()
                    .map(|row| format!(
                        "({})",
                        row.iter()
                            .map(ToString::to_string)
                            .collect::<Vec<_>>()
                            .join(", ")
                    ))
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    format!(
        "FUZZ FAILURE seed={} route={route} stage={stage}\nCQ:\n{}\nminimized dataset:\n{data}\nreference: {:?}\nreproduce: cq_testkit::fuzz::generate({})",
        case.seed,
        minimized.cq(),
        minimized.reference(),
        case.seed
    )
}
