//! The device radix provider accepts up to 32 bits per column; the host's
//! interned identities (including offsets) occupy the full i64 domain.
use datalog_ddisasm::{Facts, Meta};
use std::collections::HashSet;

pub(super) struct Dictionary(Vec<i64>);

impl Dictionary {
    pub(super) fn encode(facts: Facts) -> Result<(Facts<u32>, Self), &'static str> {
        // Exhaustive destructuring makes a newly added relation or metadata
        // constant a compile error until included in the dictionary domain.
        macro_rules! encode {
            ($($relation:ident ($($column:tt),+)),+ $(,)?) => {{
                let Facts { meta: Meta { load, none, pc_relative, store }, $($relation),+ } = facts;
                let mut ids = HashSet::from([load, none, pc_relative, store]);
                $(for row in &$relation { $(ids.insert(row.$column);)+ })+
                u32::try_from(ids.len() - 1).map_err(|_| "ddisasm exceeds 2^32 distinct identities")?;
                let mut sorted: Vec<_> = ids.into_iter().collect();
                sorted.sort_unstable();
                let code = |id| sorted.binary_search(&id).expect("scanned ddisasm identity") as u32;
                Ok((Facts {
                    meta: Meta { load: code(load), none: code(none), pc_relative: code(pc_relative), store: code(store) },
                    $($relation: $relation.into_iter().map(|row| ($(code(row.$column),)+)).collect()),+
                }, Dictionary(sorted)))
            }};
        }
        encode! {
            ArchMemoryAccess(0,1,2,3,4,5), ArchRegRegArithOp(0,1,2,3,4,5),
            ArchReturnReg(0), BlockNext(0,1,2), BlockLastInstr(0,1),
            CodeInBlock(0,1), DirectCall(0,1), MayFallthrough(0,1),
            RegDefUseBlockLastDef(0,1,2), RegDefUseDefinedInBlock(0,1),
            RegDefUseFlowDef(0,1,2,3), RegDefUseLiveVarDef(0,1,2,3),
            RegDefUseRefInBlock(0,1), RegDefUseReturnBlockEnd(0,1,2,3),
            RegDefUseUsed(0,1,2), RegDefUseUsedInBlock(0,1,2,3), RegUsedFor(0,1,2),
            RelJumpTableEntryCandidate(0,1,2,3,4,5,6), StackDefUseDef(0,1,2),
            StackDefUseDefinedInBlock(0,1,2), StackDefUseLiveVarDef(0,1,2,3,4,5),
            StackDefUseRefInBlock(0,1,2), StackDefUseUsedInBlock(0,1,2,3,4),
            StackDefUseUsed(0,1,2,3), StackDefUseLiveVarUsedEDB(0,1,2,3,4,5,6,7),
            JumpTableStart(0,1,2,3,4), DefUsedForAddressEDB(0,1,2), StackDefUseBlockLastDef(0,1,2,3),
        }
    }

    pub(super) fn decode<const N: usize, R: From<[i64; N]>>(&self, row: impl Into<[u32; N]>) -> R {
        row.into().map(|code| self.0[code as usize]).into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_include_constants_and_preserve_signed_order() {
        let facts = Facts {
            meta: Meta { load: i64::MIN, none: 7, store: 7, pc_relative: i64::MAX },
            ArchReturnReg: vec![(-3,), (1_i64 << 40,), (-3,)],
            ..Facts::default()
        };
        let (coded, dictionary) = Dictionary::encode(facts).unwrap();
        assert_eq!(coded.ArchReturnReg, vec![(1,), (3,), (1,)]);
        let meta = coded.meta;
        let decoded: (i64, i64, i64, i64) = dictionary.decode((meta.load, meta.none, meta.store, meta.pc_relative));
        assert_eq!(decoded, (i64::MIN, 7, 7, i64::MAX));
    }
}
