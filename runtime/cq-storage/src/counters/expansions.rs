//! Logical fan-out visits before downstream predicates (SQ-ExpansionCounters).
#[cfg(feature = "counters")]
use std::sync::Arc;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExpansionRecord {
    pub definition: &'static str,
    pub stage: &'static str,
    pub children: u64,
}

/// Snapshot after exhausting/dropping the measured iterators. `output` is the
/// admitted query iterator, not a storage scan or an upstream candidate count.
pub fn expansions() -> Vec<ExpansionRecord> {
    #[cfg(not(feature = "counters"))]
    {
        Vec::new()
    }
    #[cfg(feature = "counters")]
    {
        let context = super::TaskContext::capture();
        context
            .0
            .expansions
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .map(|(&(definition, stage), &children)| ExpansionRecord {
                definition,
                stage,
                children,
            })
            .collect()
    }
}

/// Compatible with tools/expansion_report.py; call after draining the query.
pub fn render_expansions() -> String {
    use std::fmt::Write;
    let mut output = String::new();
    for record in expansions() {
        if record.stage == "output" {
            writeln!(output, "cq_expansion_output\trule={}\tvariant=0\titeration=0\tprogram=cpu\tdefinition={}\toutput={}\tadmitted={}", record.definition, record.definition, record.children, record.children).unwrap();
        } else {
            writeln!(output, "cq_expansion\trule={}\tvariant=0\titeration=0\tprogram=cpu\tdefinition={}\tstage={}\tchildren={}", record.definition, record.definition, record.stage, record.children).unwrap();
        }
    }
    output
}

#[cfg(not(feature = "counters"))]
#[inline]
pub fn expansion<I: Iterator>(source: I, _definition: &'static str, _stage: &'static str) -> I {
    source
}

#[cfg(feature = "counters")]
pub fn expansion<I: Iterator>(
    source: I,
    definition: &'static str,
    stage: &'static str,
) -> Expansion<I> {
    let context = super::TaskContext::capture();
    // Register even empty stages; merging on drop costs no per-child lock.
    context
        .0
        .expansions
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .entry((definition, stage))
        .or_default();
    Expansion {
        source,
        definition,
        stage,
        count: 0,
        context: context.0,
    }
}

#[cfg(feature = "counters")]
pub struct Expansion<I> {
    source: I,
    definition: &'static str,
    stage: &'static str,
    count: u64,
    context: Arc<super::SharedCounts>,
}
#[cfg(feature = "counters")]
impl<I: Iterator> Iterator for Expansion<I> {
    type Item = I::Item;
    fn next(&mut self) -> Option<Self::Item> {
        let row = self.source.next();
        self.count += u64::from(row.is_some());
        if row.is_none() {
            self.flush();
        }
        row
    }
    fn size_hint(&self) -> (usize, Option<usize>) {
        self.source.size_hint()
    }
}
#[cfg(feature = "counters")]
impl<I> Expansion<I> {
    fn flush(&mut self) {
        if self.count != 0 {
            *self
                .context
                .expansions
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .entry((self.definition, self.stage))
                .or_default() += self.count;
            self.count = 0;
        }
    }
}
#[cfg(feature = "counters")]
impl<I> Drop for Expansion<I> {
    fn drop(&mut self) {
        self.flush();
    }
}

#[cfg(all(test, feature = "counters"))]
mod tests {
    use super::*;
    #[test]
    fn counts_children_before_closing_filters_and_resets() {
        crate::counters::reset();
        let rows = expansion(0..8, "rule", "join").filter(|n| n % 4 == 0);
        assert_eq!(expansion(rows, "rule", "output").count(), 2);
        let records = expansions();
        assert_eq!(records[0].children, 8);
        assert_eq!(records[1].children, 2);
        crate::counters::reset();
        assert!(expansions().is_empty());
    }
    #[test]
    fn exhaustion_publishes_without_double_counting_drop() {
        crate::counters::reset();
        let mut rows = expansion(0..3, "rule", "join");
        while rows.next().is_some() {}
        assert_eq!(expansions()[0].children, 3);
        drop(rows);
        assert_eq!(expansions()[0].children, 3);
    }
}
