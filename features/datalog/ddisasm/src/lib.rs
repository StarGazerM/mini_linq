//! SRDatalog's 23-rule ddisasm slice: 28 declared inputs and 11 outputs.
//!
//! Rules follow `source/examples/ddisasm.py` in the next-round snapshot,
//! cross-checked with upstream `ddisasm.nim` and `ddisasm.hir.json`.
//! Only LiveVarUsed2 moves its negative atom after the positive register binder.
//! SPLIT, variable-order hints and fingerprint dedup are physical hints, not
//! logical clauses. Both engines use exact tuple-set semantics instead.
//! Prepared CSV columns are interned identities (including offsets), represented
//! losslessly as i64, not arithmetic values. Dataset constants come from str2num.
//! BlockLastInstr is loaded to honor the 28-input contract although no rule uses it.
//! Workspace tests compare all outputs with sequential Ascent on empty inputs
//! and a recursive register-liveness fixture with blocked and unblocked negation.
#![recursion_limit = "2048"]
#![allow(non_snake_case, unused_variables)]
pub mod ascent_engine;
pub mod loader;
pub mod mini_engine;
#[derive(Clone, Copy, Debug, Default)]
pub struct Meta {
    pub load: i64,
    pub none: i64,
    pub pc_relative: i64,
    pub store: i64,
}
pub type StackLiveVarUse = (i64, i64, i64, i64, i64, i64, i64, i64);
pub type RegisterArithmeticDefs = (i64, i64, i64, i64, i64, i64, i64, i64);
#[derive(Clone, Default)]
pub struct Facts {
    pub meta: Meta,
    pub ArchMemoryAccess: Vec<(i64, i64, i64, i64, i64, i64)>,
    pub ArchRegRegArithOp: Vec<(i64, i64, i64, i64, i64, i64)>,
    pub ArchReturnReg: Vec<(i64,)>,
    pub BlockNext: Vec<(i64, i64, i64)>,
    pub BlockLastInstr: Vec<(i64, i64)>,
    pub CodeInBlock: Vec<(i64, i64)>,
    pub DirectCall: Vec<(i64, i64)>,
    pub MayFallthrough: Vec<(i64, i64)>,
    pub RegDefUseBlockLastDef: Vec<(i64, i64, i64)>,
    pub RegDefUseDefinedInBlock: Vec<(i64, i64)>,
    pub RegDefUseFlowDef: Vec<(i64, i64, i64, i64)>,
    pub RegDefUseLiveVarDef: Vec<(i64, i64, i64, i64)>,
    pub RegDefUseRefInBlock: Vec<(i64, i64)>,
    pub RegDefUseReturnBlockEnd: Vec<(i64, i64, i64, i64)>,
    pub RegDefUseUsed: Vec<(i64, i64, i64)>,
    pub RegDefUseUsedInBlock: Vec<(i64, i64, i64, i64)>,
    pub RegUsedFor: Vec<(i64, i64, i64)>,
    pub RelJumpTableEntryCandidate: Vec<(i64, i64, i64, i64, i64, i64, i64)>,
    pub StackDefUseDef: Vec<(i64, i64, i64)>,
    pub StackDefUseDefinedInBlock: Vec<(i64, i64, i64)>,
    pub StackDefUseLiveVarDef: Vec<(i64, i64, i64, i64, i64, i64)>,
    pub StackDefUseRefInBlock: Vec<(i64, i64, i64)>,
    pub StackDefUseUsedInBlock: Vec<(i64, i64, i64, i64, i64)>,
    pub StackDefUseUsed: Vec<(i64, i64, i64, i64)>,
    pub StackDefUseLiveVarUsedEDB: Vec<StackLiveVarUse>,
    pub JumpTableStart: Vec<(i64, i64, i64, i64, i64)>,
    pub DefUsedForAddressEDB: Vec<(i64, i64, i64)>,
    pub StackDefUseBlockLastDef: Vec<(i64, i64, i64, i64)>,
}
#[derive(Default)]
pub struct Results {
    pub JumpTableTarget: Vec<(i64, i64)>,
    pub RegDefUseDefUsed: Vec<(i64, i64, i64, i64)>,
    pub RegDefUseReturnValUsed: Vec<(i64, i64, i64, i64, i64)>,
    pub RegDefUseLiveVarUsed: Vec<(i64, i64, i64, i64)>,
    pub RegDefUseLiveVarAtPriorUsed: Vec<(i64, i64, i64)>,
    pub RegDefUseLiveVarAtBlockEnd: Vec<(i64, i64, i64)>,
    pub RegRegArithOpDefs: Vec<RegisterArithmeticDefs>,
    pub DefUsedForAddress: Vec<(i64, i64, i64)>,
    pub StackDefUseDefUsed: Vec<(i64, i64, i64, i64, i64, i64)>,
    pub StackDefUseLiveVarAtBlockEnd: Vec<(i64, i64, i64, i64)>,
    pub StackDefUseLiveVarAtPriorUsed: Vec<(i64, i64, i64, i64)>,
}
impl Results {
    pub fn counts(&self) -> std::collections::BTreeMap<String, usize> {
        [
            ("JumpTableTarget".into(), self.JumpTableTarget.len()),
            ("RegDefUseDefUsed".into(), self.RegDefUseDefUsed.len()),
            (
                "RegDefUseReturnValUsed".into(),
                self.RegDefUseReturnValUsed.len(),
            ),
            (
                "RegDefUseLiveVarUsed".into(),
                self.RegDefUseLiveVarUsed.len(),
            ),
            (
                "RegDefUseLiveVarAtPriorUsed".into(),
                self.RegDefUseLiveVarAtPriorUsed.len(),
            ),
            (
                "RegDefUseLiveVarAtBlockEnd".into(),
                self.RegDefUseLiveVarAtBlockEnd.len(),
            ),
            ("RegRegArithOpDefs".into(), self.RegRegArithOpDefs.len()),
            ("DefUsedForAddress".into(), self.DefUsedForAddress.len()),
            ("StackDefUseDefUsed".into(), self.StackDefUseDefUsed.len()),
            (
                "StackDefUseLiveVarAtBlockEnd".into(),
                self.StackDefUseLiveVarAtBlockEnd.len(),
            ),
            (
                "StackDefUseLiveVarAtPriorUsed".into(),
                self.StackDefUseLiveVarAtPriorUsed.len(),
            ),
        ]
        .into()
    }
    pub fn check(&mut self, other: &mut Self) {
        self.JumpTableTarget.sort_unstable();
        self.JumpTableTarget.dedup();
        other.JumpTableTarget.sort_unstable();
        other.JumpTableTarget.dedup();
        assert_eq!(
            self.JumpTableTarget, other.JumpTableTarget,
            "JumpTableTarget"
        );
        self.RegDefUseDefUsed.sort_unstable();
        self.RegDefUseDefUsed.dedup();
        other.RegDefUseDefUsed.sort_unstable();
        other.RegDefUseDefUsed.dedup();
        assert_eq!(
            self.RegDefUseDefUsed, other.RegDefUseDefUsed,
            "RegDefUseDefUsed"
        );
        self.RegDefUseReturnValUsed.sort_unstable();
        self.RegDefUseReturnValUsed.dedup();
        other.RegDefUseReturnValUsed.sort_unstable();
        other.RegDefUseReturnValUsed.dedup();
        assert_eq!(
            self.RegDefUseReturnValUsed, other.RegDefUseReturnValUsed,
            "RegDefUseReturnValUsed"
        );
        self.RegDefUseLiveVarUsed.sort_unstable();
        self.RegDefUseLiveVarUsed.dedup();
        other.RegDefUseLiveVarUsed.sort_unstable();
        other.RegDefUseLiveVarUsed.dedup();
        assert_eq!(
            self.RegDefUseLiveVarUsed, other.RegDefUseLiveVarUsed,
            "RegDefUseLiveVarUsed"
        );
        self.RegDefUseLiveVarAtPriorUsed.sort_unstable();
        self.RegDefUseLiveVarAtPriorUsed.dedup();
        other.RegDefUseLiveVarAtPriorUsed.sort_unstable();
        other.RegDefUseLiveVarAtPriorUsed.dedup();
        assert_eq!(
            self.RegDefUseLiveVarAtPriorUsed, other.RegDefUseLiveVarAtPriorUsed,
            "RegDefUseLiveVarAtPriorUsed"
        );
        self.RegDefUseLiveVarAtBlockEnd.sort_unstable();
        self.RegDefUseLiveVarAtBlockEnd.dedup();
        other.RegDefUseLiveVarAtBlockEnd.sort_unstable();
        other.RegDefUseLiveVarAtBlockEnd.dedup();
        assert_eq!(
            self.RegDefUseLiveVarAtBlockEnd, other.RegDefUseLiveVarAtBlockEnd,
            "RegDefUseLiveVarAtBlockEnd"
        );
        self.RegRegArithOpDefs.sort_unstable();
        self.RegRegArithOpDefs.dedup();
        other.RegRegArithOpDefs.sort_unstable();
        other.RegRegArithOpDefs.dedup();
        assert_eq!(
            self.RegRegArithOpDefs, other.RegRegArithOpDefs,
            "RegRegArithOpDefs"
        );
        self.DefUsedForAddress.sort_unstable();
        self.DefUsedForAddress.dedup();
        other.DefUsedForAddress.sort_unstable();
        other.DefUsedForAddress.dedup();
        assert_eq!(
            self.DefUsedForAddress, other.DefUsedForAddress,
            "DefUsedForAddress"
        );
        self.StackDefUseDefUsed.sort_unstable();
        self.StackDefUseDefUsed.dedup();
        other.StackDefUseDefUsed.sort_unstable();
        other.StackDefUseDefUsed.dedup();
        assert_eq!(
            self.StackDefUseDefUsed, other.StackDefUseDefUsed,
            "StackDefUseDefUsed"
        );
        self.StackDefUseLiveVarAtBlockEnd.sort_unstable();
        self.StackDefUseLiveVarAtBlockEnd.dedup();
        other.StackDefUseLiveVarAtBlockEnd.sort_unstable();
        other.StackDefUseLiveVarAtBlockEnd.dedup();
        assert_eq!(
            self.StackDefUseLiveVarAtBlockEnd, other.StackDefUseLiveVarAtBlockEnd,
            "StackDefUseLiveVarAtBlockEnd"
        );
        self.StackDefUseLiveVarAtPriorUsed.sort_unstable();
        self.StackDefUseLiveVarAtPriorUsed.dedup();
        other.StackDefUseLiveVarAtPriorUsed.sort_unstable();
        other.StackDefUseLiveVarAtPriorUsed.dedup();
        assert_eq!(
            self.StackDefUseLiveVarAtPriorUsed, other.StackDefUseLiveVarAtPriorUsed,
            "StackDefUseLiveVarAtPriorUsed"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_inputs_match_ascent() {
        let mut actual = mini_engine::run(Facts::default());
        actual.check(&mut ascent_engine::run(Facts::default()));
        assert!(actual.counts().values().all(|&count| count == 0));
    }

    #[test]
    fn recursive_liveness_and_negation_match_ascent() {
        let facts = Facts {
            BlockNext: vec![(1, 10, 2), (2, 20, 3)],
            RegDefUseUsedInBlock: vec![(3, 30, 7, 0), (3, 31, 8, 1)],
            RegDefUseBlockLastDef: vec![(31, 25, 8)],
            RegDefUseUsed: vec![(31, 8, 1), (31, 8, 1)],
            JumpTableStart: vec![(40, 4, 50, 0, 0)],
            RelJumpTableEntryCandidate: vec![(0, 50, 4, 0, 60, 0, 0)],
            ..Facts::default()
        };
        let mut actual = mini_engine::run(facts.clone());
        actual.check(&mut ascent_engine::run(facts));
        assert_eq!(actual.RegDefUseLiveVarUsed, vec![(3, 7, 30, 0)]);
        assert_eq!(actual.RegDefUseLiveVarAtBlockEnd, vec![(1, 3, 7), (2, 3, 7)]);
        assert_eq!(actual.RegDefUseDefUsed, vec![(25, 8, 31, 1)]);
        assert_eq!(actual.JumpTableTarget, vec![(40, 60)]);
    }
}
