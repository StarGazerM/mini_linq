use crate::{Facts, Meta};
use std::{
    fs::File,
    io::{BufRead, BufReader},
    path::Path,
};
type Error = Box<dyn std::error::Error>;
fn rows<const N: usize>(path: &Path) -> Result<Vec<[i64; N]>, Error> {
    let mut result = Vec::new();
    let mut reader = BufReader::new(File::open(path)?);
    let mut line = String::new();
    let mut line_no = 0;
    while reader.read_line(&mut line)? != 0 {
        line_no += 1;
        if line.trim().is_empty() {
            line.clear();
            continue;
        }
        let mut cells = line.trim_end().split(|c: char| c == ',' || c == '\t');
        let mut row = [0; N];
        for (i, value) in row.iter_mut().enumerate() {
            let cell = cells
                .next()
                .ok_or_else(|| format!("{}:{line_no}: expected {N} columns", path.display()))?;
            *value = cell
                .trim()
                .parse()
                .map_err(|e| format!("{}:{line_no} column {}: {e}", path.display(), i + 1))?;
        }
        if cells.next().is_some() {
            return Err(format!("{}:{line_no}: expected {N} columns", path.display()).into());
        }
        result.push(row);
        line.clear();
    }
    Ok(result)
}
pub fn load(path: &Path) -> Result<Facts, Error> {
    load_with_meta(path, &path.join("str2num.json"))
}
pub fn load_with_meta(path: &Path, metadata: &Path) -> Result<Facts, Error> {
    let dict: std::collections::BTreeMap<String, i64> = serde_json::from_reader(
        File::open(metadata).map_err(|e| format!("{}: {e}", metadata.display()))?,
    )?;
    let get = |name: &str| {
        dict.get(name)
            .copied()
            .ok_or_else(|| format!("missing dataset constant {name}"))
    };
    Ok(Facts {
        meta: Meta {
            load: get("LOAD")?,
            none: get("NONE")?,
            pc_relative: get("PCRelative")?,
            store: get("STORE")?,
        },
        ArchMemoryAccess: rows::<6>(&path.join("Arch_memory_access_truncate.csv"))?
            .into_iter()
            .map(|r| (r[0], r[1], r[2], r[3], r[4], r[5]))
            .collect(),
        ArchRegRegArithOp: rows::<6>(&path.join("Arch_reg_reg_arithmetic_operation.csv"))?
            .into_iter()
            .map(|r| (r[0], r[1], r[2], r[3], r[4], r[5]))
            .collect(),
        ArchReturnReg: rows::<1>(&path.join("Arch_return_reg.csv"))?
            .into_iter()
            .map(|r| (r[0],))
            .collect(),
        BlockNext: rows::<3>(&path.join("Block_next.csv"))?
            .into_iter()
            .map(|r| (r[0], r[1], r[2]))
            .collect(),
        BlockLastInstr: rows::<2>(&path.join("Block_last_instruction.csv"))?
            .into_iter()
            .map(|r| (r[0], r[1]))
            .collect(),
        CodeInBlock: rows::<2>(&path.join("Code_in_block.csv"))?
            .into_iter()
            .map(|r| (r[0], r[1]))
            .collect(),
        DirectCall: rows::<2>(&path.join("Direct_call.csv"))?
            .into_iter()
            .map(|r| (r[0], r[1]))
            .collect(),
        MayFallthrough: rows::<2>(&path.join("May_fallthrough.csv"))?
            .into_iter()
            .map(|r| (r[0], r[1]))
            .collect(),
        RegDefUseBlockLastDef: rows::<3>(&path.join("Reg_def_use_block_last_def.csv"))?
            .into_iter()
            .map(|r| (r[0], r[1], r[2]))
            .collect(),
        RegDefUseDefinedInBlock: rows::<2>(&path.join("Reg_def_use_defined_in_block.csv"))?
            .into_iter()
            .map(|r| (r[0], r[1]))
            .collect(),
        RegDefUseFlowDef: rows::<4>(&path.join("Reg_def_use_flow_def.csv"))?
            .into_iter()
            .map(|r| (r[0], r[1], r[2], r[3]))
            .collect(),
        RegDefUseLiveVarDef: rows::<4>(&path.join("Reg_def_use_live_var_def.csv"))?
            .into_iter()
            .map(|r| (r[0], r[1], r[2], r[3]))
            .collect(),
        RegDefUseRefInBlock: rows::<2>(&path.join("Reg_def_use_ref_in_block.csv"))?
            .into_iter()
            .map(|r| (r[0], r[1]))
            .collect(),
        RegDefUseReturnBlockEnd: rows::<4>(&path.join("Reg_def_use_return_block_end.csv"))?
            .into_iter()
            .map(|r| (r[0], r[1], r[2], r[3]))
            .collect(),
        RegDefUseUsed: rows::<3>(&path.join("Reg_def_use_used.csv"))?
            .into_iter()
            .map(|r| (r[0], r[1], r[2]))
            .collect(),
        RegDefUseUsedInBlock: rows::<4>(&path.join("Reg_def_use_used_in_block.csv"))?
            .into_iter()
            .map(|r| (r[0], r[1], r[2], r[3]))
            .collect(),
        RegUsedFor: rows::<3>(&path.join("Reg_used_for.csv"))?
            .into_iter()
            .map(|r| (r[0], r[1], r[2]))
            .collect(),
        RelJumpTableEntryCandidate: rows::<7>(
            &path.join("Relative_jump_table_entry_candidate.csv"),
        )?
        .into_iter()
        .map(|r| (r[0], r[1], r[2], r[3], r[4], r[5], r[6]))
        .collect(),
        StackDefUseDef: rows::<3>(&path.join("Stack_def_use_def.csv"))?
            .into_iter()
            .map(|r| (r[0], r[1], r[2]))
            .collect(),
        StackDefUseDefinedInBlock: rows::<3>(&path.join("Stack_def_use_defined_in_block.csv"))?
            .into_iter()
            .map(|r| (r[0], r[1], r[2]))
            .collect(),
        StackDefUseLiveVarDef: rows::<6>(&path.join("Stack_def_use_live_var_def.csv"))?
            .into_iter()
            .map(|r| (r[0], r[1], r[2], r[3], r[4], r[5]))
            .collect(),
        StackDefUseRefInBlock: rows::<3>(&path.join("Stack_def_use_ref_in_block.csv"))?
            .into_iter()
            .map(|r| (r[0], r[1], r[2]))
            .collect(),
        StackDefUseUsedInBlock: rows::<5>(&path.join("Stack_def_use_used_in_block.csv"))?
            .into_iter()
            .map(|r| (r[0], r[1], r[2], r[3], r[4]))
            .collect(),
        StackDefUseUsed: rows::<4>(&path.join("Stack_def_use_used.csv"))?
            .into_iter()
            .map(|r| (r[0], r[1], r[2], r[3]))
            .collect(),
        StackDefUseLiveVarUsedEDB: rows::<8>(&path.join("Stack_def_use_live_var_used.csv"))?
            .into_iter()
            .map(|r| (r[0], r[1], r[2], r[3], r[4], r[5], r[6], r[7]))
            .collect(),
        JumpTableStart: rows::<5>(&path.join("Jump_table_start.csv"))?
            .into_iter()
            .map(|r| (r[0], r[1], r[2], r[3], r[4]))
            .collect(),
        DefUsedForAddressEDB: rows::<3>(&path.join("Def_used_for_address.csv"))?
            .into_iter()
            .map(|r| (r[0], r[1], r[2]))
            .collect(),
        StackDefUseBlockLastDef: rows::<4>(&path.join("Stack_def_use_block_last_def.csv"))?
            .into_iter()
            .map(|r| (r[0], r[1], r[2], r[3]))
            .collect(),
    })
}
