//! Opt-in SQ-ExpansionCounters observations; never used to choose execution.
use std::cell::RefCell;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Record {
    pub program: &'static str,
    pub definition: &'static str,
    pub children: Vec<u64>,
    pub output: u64,
}

thread_local! {
    static CAPTURE: RefCell<Option<Vec<Record>>> = const { RefCell::new(None) };
}

/// Capture one rule invocation, restoring any enclosing diagnostic scope on drop.
pub struct Capture(Option<Vec<Record>>);
impl Capture {
    pub fn begin() -> Self {
        Self(CAPTURE.with(|slot| slot.replace(Some(Vec::new()))))
    }
    /// `admitted` is unique output absent from the pre-round accepted state.
    /// Different rules may admit the same row; their counts are not additive.
    pub fn report(&self, rule: &str, variant: usize, iteration: usize, admitted: usize) {
        CAPTURE.with(|slot| {
            for record in slot.borrow().as_ref().into_iter().flatten() {
                report(record, rule, variant, iteration, admitted as u64);
            }
        });
    }
}
impl Drop for Capture {
    fn drop(&mut self) {
        CAPTURE.with(|slot| slot.replace(self.0.take()));
    }
}

/// Called only after a successful launch attempt, so capacity retries do not
/// multiply logical work. Outside Datalog this reports directly (including TPC-H).
pub fn publish(program: &'static str, definition: &'static str, children: &[u64], output: u64) {
    let record = Record { program, definition, children: children.to_vec(), output };
    CAPTURE.with(|slot| {
        if let Some(records) = slot.borrow_mut().as_mut() {
            records.push(record);
        } else {
            report(&record, program, 0, 0, output);
        }
    });
}

fn report(record: &Record, rule: &str, variant: usize, iteration: usize, admitted: u64) {
    // Identifiers originate in checked programs (no tabs/newlines).
    eprintln!("cq_expansion_output\trule={rule}\tvariant={variant}\titeration={iteration}\tprogram={}\tdefinition={}\toutput={}\tadmitted={admitted}", record.program, record.definition, record.output);
    for (stage, children) in record.children.iter().enumerate() {
        eprintln!("cq_expansion\trule={rule}\tvariant={variant}\titeration={iteration}\tprogram={}\tdefinition={}\tstage={stage}\tchildren={children}", record.program, record.definition);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn capture_restores_outer_scope() {
        let outer = Capture::begin();
        publish("p", "a", &[7, 11], 2);
        {
            let _inner = Capture::begin();
            publish("p", "b", &[3], 1);
            CAPTURE.with(|slot| assert_eq!(slot.borrow().as_ref().unwrap()[0].definition, "b"));
        }
        CAPTURE.with(|slot| assert_eq!(slot.borrow().as_ref().unwrap()[0].children, [7, 11]));
        drop(outer);
        CAPTURE.with(|slot| assert!(slot.borrow().is_none()));
    }
}
