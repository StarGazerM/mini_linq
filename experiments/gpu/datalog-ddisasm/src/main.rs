//! Faithful 23-rule ddisasm GPU consumer; every i64 is an interned identity.
//! Usage: gpu-datalog-ddisasm DATASET_DIRECTORY [--check] [--meta JSON]
//! Upload includes all 28 EDBs, including the unused BlockLastInstr relation.
//! Dataset constants remain runtime meta expressions, never dataset-specific literals.
//! Timings synchronize the resident stream. Fixedpoint peak includes resident EDBs;
//! allocator figures describe requested live bytes, not CUDA driver reservation.
#![recursion_limit = "4096"]
#![allow(non_snake_case, unused_variables)]
use cq_gpu_runtime::{Codec, Dictionary, Runtime};
use datalog_ddisasm::{loader, mini_engine, Facts, Results};
use std::{error::Error, path::PathBuf, time::Instant};

fn run(
    runtime: &Runtime,
    facts: &Facts,
    check: bool,
) -> Result<(Option<Results>, serde_json::Value), Box<dyn Error>> {
    let meta = facts.meta;
    let mut dictionary = Dictionary::default();
    runtime.pool().resource().push_counters();
    let started = Instant::now();
    let ArchMemoryAccess =
        runtime.upload_rows(&facts.ArchMemoryAccess, &mut dictionary, &[Codec::Plain; 6])?;
    let ArchRegRegArithOp = runtime.upload_rows(
        &facts.ArchRegRegArithOp,
        &mut dictionary,
        &[Codec::Plain; 6],
    )?;
    let ArchReturnReg =
        runtime.upload_rows(&facts.ArchReturnReg, &mut dictionary, &[Codec::Plain; 1])?;
    let BlockNext = runtime.upload_rows(&facts.BlockNext, &mut dictionary, &[Codec::Plain; 3])?;
    let BlockLastInstr =
        runtime.upload_rows(&facts.BlockLastInstr, &mut dictionary, &[Codec::Plain; 2])?;
    let CodeInBlock =
        runtime.upload_rows(&facts.CodeInBlock, &mut dictionary, &[Codec::Plain; 2])?;
    let DirectCall = runtime.upload_rows(&facts.DirectCall, &mut dictionary, &[Codec::Plain; 2])?;
    let MayFallthrough =
        runtime.upload_rows(&facts.MayFallthrough, &mut dictionary, &[Codec::Plain; 2])?;
    let RegDefUseBlockLastDef = runtime.upload_rows(
        &facts.RegDefUseBlockLastDef,
        &mut dictionary,
        &[Codec::Plain; 3],
    )?;
    let RegDefUseDefinedInBlock = runtime.upload_rows(
        &facts.RegDefUseDefinedInBlock,
        &mut dictionary,
        &[Codec::Plain; 2],
    )?;
    let RegDefUseFlowDef =
        runtime.upload_rows(&facts.RegDefUseFlowDef, &mut dictionary, &[Codec::Plain; 4])?;
    let RegDefUseLiveVarDef = runtime.upload_rows(
        &facts.RegDefUseLiveVarDef,
        &mut dictionary,
        &[Codec::Plain; 4],
    )?;
    let RegDefUseRefInBlock = runtime.upload_rows(
        &facts.RegDefUseRefInBlock,
        &mut dictionary,
        &[Codec::Plain; 2],
    )?;
    let RegDefUseReturnBlockEnd = runtime.upload_rows(
        &facts.RegDefUseReturnBlockEnd,
        &mut dictionary,
        &[Codec::Plain; 4],
    )?;
    let RegDefUseUsed =
        runtime.upload_rows(&facts.RegDefUseUsed, &mut dictionary, &[Codec::Plain; 3])?;
    let RegDefUseUsedInBlock = runtime.upload_rows(
        &facts.RegDefUseUsedInBlock,
        &mut dictionary,
        &[Codec::Plain; 4],
    )?;
    let RegUsedFor = runtime.upload_rows(&facts.RegUsedFor, &mut dictionary, &[Codec::Plain; 3])?;
    let RelJumpTableEntryCandidate = runtime.upload_rows(
        &facts.RelJumpTableEntryCandidate,
        &mut dictionary,
        &[Codec::Plain; 7],
    )?;
    let StackDefUseDef =
        runtime.upload_rows(&facts.StackDefUseDef, &mut dictionary, &[Codec::Plain; 3])?;
    let StackDefUseDefinedInBlock = runtime.upload_rows(
        &facts.StackDefUseDefinedInBlock,
        &mut dictionary,
        &[Codec::Plain; 3],
    )?;
    let StackDefUseLiveVarDef = runtime.upload_rows(
        &facts.StackDefUseLiveVarDef,
        &mut dictionary,
        &[Codec::Plain; 6],
    )?;
    let StackDefUseRefInBlock = runtime.upload_rows(
        &facts.StackDefUseRefInBlock,
        &mut dictionary,
        &[Codec::Plain; 3],
    )?;
    let StackDefUseUsedInBlock = runtime.upload_rows(
        &facts.StackDefUseUsedInBlock,
        &mut dictionary,
        &[Codec::Plain; 5],
    )?;
    let StackDefUseUsed =
        runtime.upload_rows(&facts.StackDefUseUsed, &mut dictionary, &[Codec::Plain; 4])?;
    let StackDefUseLiveVarUsedEDB = runtime.upload_rows(
        &facts.StackDefUseLiveVarUsedEDB,
        &mut dictionary,
        &[Codec::Plain; 8],
    )?;
    let JumpTableStart =
        runtime.upload_rows(&facts.JumpTableStart, &mut dictionary, &[Codec::Plain; 5])?;
    let DefUsedForAddressEDB = runtime.upload_rows(
        &facts.DefUsedForAddressEDB,
        &mut dictionary,
        &[Codec::Plain; 3],
    )?;
    let StackDefUseBlockLastDef = runtime.upload_rows(
        &facts.StackDefUseBlockLastDef,
        &mut dictionary,
        &[Codec::Plain; 4],
    )?;
    runtime.pool().stream().synchronize()?;
    let upload_ms = started.elapsed().as_secs_f64() * 1000.0;
    let upload = runtime.pool().resource().pop_counters()?;
    let resident_bytes = runtime.pool().resource().push_counters().bytes.value;
    cq_gpu_runtime::datalog::reset_round_count();
    cq_gpu_runtime::reset_host_wait_count();
    let started = Instant::now();
    let (
        JumpTableTarget,
        RegDefUseDefUsed,
        RegDefUseReturnValUsed,
        RegDefUseLiveVarUsed,
        RegDefUseLiveVarAtPriorUsed,
        RegDefUseLiveVarAtBlockEnd,
        RegRegArithOpDefs,
        DefUsedForAddress,
        StackDefUseDefUsed,
        StackDefUseLiveVarAtBlockEnd,
        StackDefUseLiveVarAtPriorUsed,
    ) = cq_gpu_datalog::datalog! {
     struct Ddisasm;
     relation ArchMemoryAccess(c0:i64,c1:i64,c2:i64,c3:i64,c4:i64,c5:i64) = (&ArchMemoryAccess);
     relation ArchRegRegArithOp(c0:i64,c1:i64,c2:i64,c3:i64,c4:i64,c5:i64) = (&ArchRegRegArithOp);
     relation ArchReturnReg(c0:i64) = (&ArchReturnReg);
     relation BlockNext(c0:i64,c1:i64,c2:i64) = (&BlockNext);
     relation BlockLastInstr(c0:i64,c1:i64) = (&BlockLastInstr);
     relation CodeInBlock(c0:i64,c1:i64) = (&CodeInBlock);
     relation DirectCall(c0:i64,c1:i64) = (&DirectCall);
     relation MayFallthrough(c0:i64,c1:i64) = (&MayFallthrough);
     relation RegDefUseBlockLastDef(c0:i64,c1:i64,c2:i64) = (&RegDefUseBlockLastDef);
     relation RegDefUseDefinedInBlock(c0:i64,c1:i64) = (&RegDefUseDefinedInBlock);
     relation RegDefUseFlowDef(c0:i64,c1:i64,c2:i64,c3:i64) = (&RegDefUseFlowDef);
     relation RegDefUseLiveVarDef(c0:i64,c1:i64,c2:i64,c3:i64) = (&RegDefUseLiveVarDef);
     relation RegDefUseRefInBlock(c0:i64,c1:i64) = (&RegDefUseRefInBlock);
     relation RegDefUseReturnBlockEnd(c0:i64,c1:i64,c2:i64,c3:i64) = (&RegDefUseReturnBlockEnd);
     relation RegDefUseUsed(c0:i64,c1:i64,c2:i64) = (&RegDefUseUsed);
     relation RegDefUseUsedInBlock(c0:i64,c1:i64,c2:i64,c3:i64) = (&RegDefUseUsedInBlock);
     relation RegUsedFor(c0:i64,c1:i64,c2:i64) = (&RegUsedFor);
     relation RelJumpTableEntryCandidate(c0:i64,c1:i64,c2:i64,c3:i64,c4:i64,c5:i64,c6:i64) = (&RelJumpTableEntryCandidate);
     relation StackDefUseDef(c0:i64,c1:i64,c2:i64) = (&StackDefUseDef);
     relation StackDefUseDefinedInBlock(c0:i64,c1:i64,c2:i64) = (&StackDefUseDefinedInBlock);
     relation StackDefUseLiveVarDef(c0:i64,c1:i64,c2:i64,c3:i64,c4:i64,c5:i64) = (&StackDefUseLiveVarDef);
     relation StackDefUseRefInBlock(c0:i64,c1:i64,c2:i64) = (&StackDefUseRefInBlock);
     relation StackDefUseUsedInBlock(c0:i64,c1:i64,c2:i64,c3:i64,c4:i64) = (&StackDefUseUsedInBlock);
     relation StackDefUseUsed(c0:i64,c1:i64,c2:i64,c3:i64) = (&StackDefUseUsed);
     relation StackDefUseLiveVarUsedEDB(c0:i64,c1:i64,c2:i64,c3:i64,c4:i64,c5:i64,c6:i64,c7:i64) = (&StackDefUseLiveVarUsedEDB);
     relation JumpTableStart(c0:i64,c1:i64,c2:i64,c3:i64,c4:i64) = (&JumpTableStart);
     relation DefUsedForAddressEDB(c0:i64,c1:i64,c2:i64) = (&DefUsedForAddressEDB);
     relation StackDefUseBlockLastDef(c0:i64,c1:i64,c2:i64,c3:i64) = (&StackDefUseBlockLastDef);
     relation JumpTableTarget(c0:i64,c1:i64);
     relation RegDefUseDefUsed(c0:i64,c1:i64,c2:i64,c3:i64);
     relation RegDefUseReturnValUsed(c0:i64,c1:i64,c2:i64,c3:i64,c4:i64);
     relation RegDefUseLiveVarUsed(c0:i64,c1:i64,c2:i64,c3:i64);
     relation RegDefUseLiveVarAtPriorUsed(c0:i64,c1:i64,c2:i64);
     relation RegDefUseLiveVarAtBlockEnd(c0:i64,c1:i64,c2:i64);
     relation RegRegArithOpDefs(c0:i64,c1:i64,c2:i64,c3:i64,c4:i64,c5:i64,c6:i64,c7:i64);
     relation DefUsedForAddress(c0:i64,c1:i64,c2:i64);
     relation StackDefUseDefUsed(c0:i64,c1:i64,c2:i64,c3:i64,c4:i64,c5:i64);
     relation StackDefUseLiveVarAtBlockEnd(c0:i64,c1:i64,c2:i64,c3:i64);
     relation StackDefUseLiveVarAtPriorUsed(c0:i64,c1:i64,c2:i64,c3:i64);
     output JumpTableTarget,RegDefUseDefUsed,RegDefUseReturnValUsed,RegDefUseLiveVarUsed,RegDefUseLiveVarAtPriorUsed,RegDefUseLiveVarAtBlockEnd,RegRegArithOpDefs,DefUsedForAddress,StackDefUseDefUsed,StackDefUseLiveVarAtBlockEnd,StackDefUseLiveVarAtPriorUsed;
    JumpTableTarget(*ea,*dest) :- JumpTableStart(ea,size,tableStart,_,_), RelJumpTableEntryCandidate(_,tableStart,size,_,dest,_,_);
    RegDefUseDefUsed(*eaDef,*mvar,*eaUsed,*index) :- RegDefUseUsed(eaUsed,mvar,index), RegDefUseBlockLastDef(eaUsed,eaDef,mvar);
    RegDefUseDefUsed(*eaDef,*varIdentity,*eaUsed,*index) :- RegDefUseLiveVarAtBlockEnd(blk,blockUsed,mvar), RegDefUseLiveVarDef(blk,varIdentity,mvar,eaDef), RegDefUseLiveVarUsed(blockUsed,mvar,eaUsed,index);
    RegDefUseDefUsed(*eaDef,*mvar,*nextEaUsed,*nextIndex) :- RegDefUseLiveVarAtPriorUsed(eaUsed,nxtBlk,mvar), RegDefUseDefUsed(eaDef,mvar,eaUsed,_), RegDefUseLiveVarUsed(nxtBlk,mvar,nextEaUsed,nextIndex);
    RegDefUseDefUsed(*eaDef,*reg,*eaUsed,*index) :- RegDefUseReturnValUsed(_,callee,reg,eaUsed,index), RegDefUseReturnBlockEnd(callee,_,_,blockEnd), RegDefUseBlockLastDef(blockEnd,eaDef,reg);
    RegDefUseReturnValUsed(*eaCall,*callee,*reg,*eaUsed,*indexUsed) :- ArchReturnReg(reg), RegDefUseDefUsed(eaCall,reg,eaUsed,indexUsed), DirectCall(eaCall,callee);
    RegDefUseLiveVarUsed(*blk,*mvar,*eaUsed,*index) :- RegDefUseUsedInBlock(blk,eaUsed,mvar,index), !RegDefUseBlockLastDef(eaUsed,_,mvar);
    RegDefUseLiveVarUsed(*retBlock,*reg,*eaUsed,*index) :- RegDefUseReturnBlockEnd(callee,_,retBlock,retBlockEnd), RegDefUseReturnValUsed(_,callee,reg,eaUsed,index), !RegDefUseBlockLastDef(retBlockEnd,_,reg);
    RegDefUseLiveVarAtPriorUsed(*eaUsed,*blockUsed,*mvar) :- RegDefUseLiveVarAtBlockEnd(blk,blockUsed,mvar), RegDefUseUsedInBlock(blk,eaUsed,mvar,_), !RegDefUseDefinedInBlock(blk,mvar);
    RegDefUseLiveVarAtBlockEnd(*prevBlock,*blk,*mvar) :- BlockNext(prevBlock,prevBlockEnd,blk), RegDefUseLiveVarUsed(blk,mvar,_,_), !RegDefUseFlowDef(prevBlockEnd,mvar,blk,_);
    RegDefUseLiveVarAtBlockEnd(*prevBlock,*blockUsed,*mvar) :- RegDefUseLiveVarAtBlockEnd(blk,blockUsed,mvar), !RegDefUseRefInBlock(blk,mvar), BlockNext(prevBlock,_,blk);
    RegRegArithOpDefs(*ea,*regDef,*eaDef1,*reg1,*eaDef2,*reg2,*mult,*offset) :- DefUsedForAddress(ea,regDef,_), ArchRegRegArithOp(ea,regDef,reg1,reg2,mult,offset), if reg1 != reg2, RegDefUseDefUsed(eaDef1,reg1,ea,_), if ea != eaDef1, RegDefUseDefUsed(eaDef2,reg2,ea,_), if ea != eaDef2;
    DefUsedForAddress(*ea,*reg,meta.pc_relative) :- DefUsedForAddressEDB(ea,reg,meta.pc_relative);
    DefUsedForAddress(*eaDef,*reg,*mtype) :- RegDefUseDefUsed(eaDef,reg,ea,_), RegUsedFor(ea,reg,mtype);
    DefUsedForAddress(*eaDef,*reg,*mtype) :- DefUsedForAddress(eaUsed,_,mtype), RegDefUseDefUsed(eaDef,reg,eaUsed,_);
    DefUsedForAddress(*eaDef,*reg1,*mtype) :- DefUsedForAddress(eaLoad,reg2,mtype), ArchMemoryAccess(meta.load,eaLoad,reg2,regBaseLoad,meta.none,stackPosLoad), StackDefUseDefUsed(eaSt,regBaseSt,stackPosSt,eaLoad,regBaseLoad,stackPosLoad), ArchMemoryAccess(meta.store,eaSt,reg1,regBaseSt,meta.none,stackPosSt), RegDefUseDefUsed(eaDef,reg1,eaSt,_);
    StackDefUseDefUsed(*eaDef,*varr,*varp,*eaUsed,*varr,*varp) :- StackDefUseUsed(eaUsed,varr,varp,_), StackDefUseBlockLastDef(eaUsed,eaDef,varr,varp);
    StackDefUseDefUsed(*eaDef,*defVarr,*defVarp,*eaUsed,*varUsedr,*varUsedp) :- StackDefUseLiveVarAtBlockEnd(blk,blockUsed,varr,varp), StackDefUseLiveVarDef(blk,defVarr,defVarp,varr,varp,eaDef), StackDefUseLiveVarUsedEDB(blockUsed,varr,varp,varUsedr,varUsedp,eaUsed,_,_);
    StackDefUseDefUsed(*eaDef,*defVarr,*defVarp,*eaUsed,*usedVarr,*usedVarp) :- StackDefUseLiveVarUsedEDB(ea,defVarr,defVarp,usedVarr,usedVarp,eaUsed,_,_), MayFallthrough(eaDef,ea), CodeInBlock(eaDef,blk), CodeInBlock(ea,blk), StackDefUseDef(eaDef,defVarr,defVarp);
    StackDefUseDefUsed(*eaDef,*varDefr,*varDefp,*nextEaUsed,*varUsedr,*varUsedp) :- StackDefUseLiveVarAtPriorUsed(eaUsed,nxtBlk,varr,varp), StackDefUseDefUsed(eaDef,varDefr,varDefp,eaUsed,varr,varp), StackDefUseLiveVarUsedEDB(nxtBlk,varr,varp,varUsedr,varUsedp,nextEaUsed,_,_);
    StackDefUseLiveVarAtBlockEnd(*prevBlock,*blockUsed,*varr,*varp) :- StackDefUseLiveVarAtBlockEnd(blk,blockUsed,varr,varp), !StackDefUseRefInBlock(blk,varr,varp), !RegDefUseDefinedInBlock(blk,varr), BlockNext(prevBlock,_,blk);
    StackDefUseLiveVarAtBlockEnd(*prevBlock,*blk,*varr,*varp) :- BlockNext(prevBlock,_,blk), StackDefUseLiveVarUsedEDB(blk,varr,varp,_,_,_,_,_);
    StackDefUseLiveVarAtPriorUsed(*eaUsed,*blockUsed,*varr,*varp) :- StackDefUseLiveVarAtBlockEnd(blk,blockUsed,varr,varp), StackDefUseUsedInBlock(blk,eaUsed,varr,varp,_), !RegDefUseDefinedInBlock(blk,varr), !StackDefUseDefinedInBlock(blk,varr,varp);
     };
    runtime.pool().stream().synchronize()?;
    let fixedpoint_ms = started.elapsed().as_secs_f64() * 1000.0;
    let fixedpoint = runtime.pool().resource().pop_counters()?;
    let metrics = serde_json::json!({
        "upload_ms": upload_ms,
        "upload_allocator_peak_bytes": upload.bytes.peak,
        "upload_allocations": upload.allocations.total,
        "fixedpoint_ms": fixedpoint_ms,
        "fixedpoint_allocator_peak_bytes": resident_bytes + fixedpoint.bytes.peak,
        "fixedpoint_allocations": fixedpoint.allocations.total,
        "nonempty_publications": cq_gpu_runtime::datalog::round_count(),
        "checked_rounds": cq_gpu_runtime::datalog::checked_round_count(),
        "host_waits": cq_gpu_runtime::host_wait_count(),
        "control_readbacks": cq_gpu_runtime::control_readback_count(),
        "resume_readbacks": cq_gpu_runtime::resume_readback_count(),
        "compaction_readbacks": cq_gpu_runtime::compaction_readback_count(),
    });
    // DeviceColumns has no public logical-length accessor. Without an exactness
    // check, keep tuples resident and report unavailable counts rather than capacity.
    let result = if check {
        Some(Results {
            JumpTableTarget: JumpTableTarget.to_host()?,
            RegDefUseDefUsed: RegDefUseDefUsed.to_host()?,
            RegDefUseReturnValUsed: RegDefUseReturnValUsed.to_host()?,
            RegDefUseLiveVarUsed: RegDefUseLiveVarUsed.to_host()?,
            RegDefUseLiveVarAtPriorUsed: RegDefUseLiveVarAtPriorUsed.to_host()?,
            RegDefUseLiveVarAtBlockEnd: RegDefUseLiveVarAtBlockEnd.to_host()?,
            RegRegArithOpDefs: RegRegArithOpDefs.to_host()?,
            DefUsedForAddress: DefUsedForAddress.to_host()?,
            StackDefUseDefUsed: StackDefUseDefUsed.to_host()?,
            StackDefUseLiveVarAtBlockEnd: StackDefUseLiveVarAtBlockEnd.to_host()?,
            StackDefUseLiveVarAtPriorUsed: StackDefUseLiveVarAtPriorUsed.to_host()?,
        })
    } else {
        None
    };
    Ok((result, metrics))
}

fn main() -> Result<(), Box<dyn Error>> {
    let mut args = std::env::args_os().skip(1);
    let directory = PathBuf::from(
        args.next()
            .ok_or("usage: gpu-datalog-ddisasm DATASET_DIRECTORY [--check] [--meta JSON]")?,
    );
    let mut check = false;
    let mut metadata = None;
    while let Some(arg) = args.next() {
        if arg == "--check" {
            check = true;
        } else if arg == "--meta" {
            metadata = Some(PathBuf::from(
                args.next().ok_or("--meta requires a JSON path")?,
            ));
        } else {
            return Err("expected --check or --meta JSON".into());
        }
    }
    let started = Instant::now();
    let facts = match metadata.as_deref() {
        Some(path) => loader::load_with_meta(&directory, path)?,
        None => loader::load(&directory)?,
    };
    let load_ms = started.elapsed().as_secs_f64() * 1000.0;
    let runtime = Runtime::new()?;
    let (actual, mut report) = run(&runtime, &facts, check)?;
    report["dataset"] = serde_json::json!(directory);
    report["metadata"] = serde_json::json!(metadata);
    report["load_ms"] = serde_json::json!(load_ms);
    report["counts"] = actual
        .as_ref()
        .map(|result| serde_json::json!(result.counts()))
        .unwrap_or(serde_json::Value::Null);
    report["exact_cpu_check"] = serde_json::Value::Null;
    if let Some(mut actual) = actual {
        let started = Instant::now();
        let mut expected = mini_engine::run(facts);
        report["cpu_ms"] = serde_json::json!(started.elapsed().as_secs_f64() * 1000.0);
        actual.check(&mut expected);
        report["exact_cpu_check"] = serde_json::json!(true);
    }
    println!("{}", serde_json::to_string_pretty(&report)?);
    Ok(())
}
