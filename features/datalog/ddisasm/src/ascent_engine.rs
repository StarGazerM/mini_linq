use crate::{Facts, Results};
pub fn run(facts: Facts) -> Results {
    let meta = facts.meta;
    let db = ascent::ascent_run! {
     relation ArchMemoryAccess(i64,i64,i64,i64,i64,i64) = facts.ArchMemoryAccess;
     relation ArchRegRegArithOp(i64,i64,i64,i64,i64,i64) = facts.ArchRegRegArithOp;
     relation ArchReturnReg(i64) = facts.ArchReturnReg;
     relation BlockNext(i64,i64,i64) = facts.BlockNext;
     relation BlockLastInstr(i64,i64) = facts.BlockLastInstr;
     relation CodeInBlock(i64,i64) = facts.CodeInBlock;
     relation DirectCall(i64,i64) = facts.DirectCall;
     relation MayFallthrough(i64,i64) = facts.MayFallthrough;
     relation RegDefUseBlockLastDef(i64,i64,i64) = facts.RegDefUseBlockLastDef;
     relation RegDefUseDefinedInBlock(i64,i64) = facts.RegDefUseDefinedInBlock;
     relation RegDefUseFlowDef(i64,i64,i64,i64) = facts.RegDefUseFlowDef;
     relation RegDefUseLiveVarDef(i64,i64,i64,i64) = facts.RegDefUseLiveVarDef;
     relation RegDefUseRefInBlock(i64,i64) = facts.RegDefUseRefInBlock;
     relation RegDefUseReturnBlockEnd(i64,i64,i64,i64) = facts.RegDefUseReturnBlockEnd;
     relation RegDefUseUsed(i64,i64,i64) = facts.RegDefUseUsed;
     relation RegDefUseUsedInBlock(i64,i64,i64,i64) = facts.RegDefUseUsedInBlock;
     relation RegUsedFor(i64,i64,i64) = facts.RegUsedFor;
     relation RelJumpTableEntryCandidate(i64,i64,i64,i64,i64,i64,i64) = facts.RelJumpTableEntryCandidate;
     relation StackDefUseDef(i64,i64,i64) = facts.StackDefUseDef;
     relation StackDefUseDefinedInBlock(i64,i64,i64) = facts.StackDefUseDefinedInBlock;
     relation StackDefUseLiveVarDef(i64,i64,i64,i64,i64,i64) = facts.StackDefUseLiveVarDef;
     relation StackDefUseRefInBlock(i64,i64,i64) = facts.StackDefUseRefInBlock;
     relation StackDefUseUsedInBlock(i64,i64,i64,i64,i64) = facts.StackDefUseUsedInBlock;
     relation StackDefUseUsed(i64,i64,i64,i64) = facts.StackDefUseUsed;
     relation StackDefUseLiveVarUsedEDB(i64,i64,i64,i64,i64,i64,i64,i64) = facts.StackDefUseLiveVarUsedEDB;
     relation JumpTableStart(i64,i64,i64,i64,i64) = facts.JumpTableStart;
     relation DefUsedForAddressEDB(i64,i64,i64) = facts.DefUsedForAddressEDB;
     relation StackDefUseBlockLastDef(i64,i64,i64,i64) = facts.StackDefUseBlockLastDef;
     relation JumpTableTarget(i64,i64);
     relation RegDefUseDefUsed(i64,i64,i64,i64);
     relation RegDefUseReturnValUsed(i64,i64,i64,i64,i64);
     relation RegDefUseLiveVarUsed(i64,i64,i64,i64);
     relation RegDefUseLiveVarAtPriorUsed(i64,i64,i64);
     relation RegDefUseLiveVarAtBlockEnd(i64,i64,i64);
     relation RegRegArithOpDefs(i64,i64,i64,i64,i64,i64,i64,i64);
     relation DefUsedForAddress(i64,i64,i64);
     relation StackDefUseDefUsed(i64,i64,i64,i64,i64,i64);
     relation StackDefUseLiveVarAtBlockEnd(i64,i64,i64,i64);
     relation StackDefUseLiveVarAtPriorUsed(i64,i64,i64,i64);
    JumpTableTarget(*ea,*dest) <-- JumpTableStart(ea,size,tableStart,_,_), RelJumpTableEntryCandidate(_,tableStart,size,_,dest,_,_);
    RegDefUseDefUsed(*eaDef,*mvar,*eaUsed,*index) <-- RegDefUseUsed(eaUsed,mvar,index), RegDefUseBlockLastDef(eaUsed,eaDef,mvar);
    RegDefUseDefUsed(*eaDef,*varIdentity,*eaUsed,*index) <-- RegDefUseLiveVarAtBlockEnd(blk,blockUsed,mvar), RegDefUseLiveVarDef(blk,varIdentity,mvar,eaDef), RegDefUseLiveVarUsed(blockUsed,mvar,eaUsed,index);
    RegDefUseDefUsed(*eaDef,*mvar,*nextEaUsed,*nextIndex) <-- RegDefUseLiveVarAtPriorUsed(eaUsed,nxtBlk,mvar), RegDefUseDefUsed(eaDef,mvar,eaUsed,_), RegDefUseLiveVarUsed(nxtBlk,mvar,nextEaUsed,nextIndex);
    RegDefUseDefUsed(*eaDef,*reg,*eaUsed,*index) <-- RegDefUseReturnValUsed(_,callee,reg,eaUsed,index), RegDefUseReturnBlockEnd(callee,_,_,blockEnd), RegDefUseBlockLastDef(blockEnd,eaDef,reg);
    RegDefUseReturnValUsed(*eaCall,*callee,*reg,*eaUsed,*indexUsed) <-- ArchReturnReg(reg), RegDefUseDefUsed(eaCall,reg,eaUsed,indexUsed), DirectCall(eaCall,callee);
    RegDefUseLiveVarUsed(*blk,*mvar,*eaUsed,*index) <-- RegDefUseUsedInBlock(blk,eaUsed,mvar,index), !RegDefUseBlockLastDef(eaUsed,_,mvar);
    RegDefUseLiveVarUsed(*retBlock,*reg,*eaUsed,*index) <-- RegDefUseReturnBlockEnd(callee,_,retBlock,retBlockEnd), RegDefUseReturnValUsed(_,callee,reg,eaUsed,index), !RegDefUseBlockLastDef(retBlockEnd,_,reg);
    RegDefUseLiveVarAtPriorUsed(*eaUsed,*blockUsed,*mvar) <-- RegDefUseLiveVarAtBlockEnd(blk,blockUsed,mvar), RegDefUseUsedInBlock(blk,eaUsed,mvar,_), !RegDefUseDefinedInBlock(blk,mvar);
    RegDefUseLiveVarAtBlockEnd(*prevBlock,*blk,*mvar) <-- BlockNext(prevBlock,prevBlockEnd,blk), RegDefUseLiveVarUsed(blk,mvar,_,_), !RegDefUseFlowDef(prevBlockEnd,mvar,blk,_);
    RegDefUseLiveVarAtBlockEnd(*prevBlock,*blockUsed,*mvar) <-- RegDefUseLiveVarAtBlockEnd(blk,blockUsed,mvar), !RegDefUseRefInBlock(blk,mvar), BlockNext(prevBlock,_,blk);
    RegRegArithOpDefs(*ea,*regDef,*eaDef1,*reg1,*eaDef2,*reg2,*mult,*offset) <-- DefUsedForAddress(ea,regDef,_), ArchRegRegArithOp(ea,regDef,reg1,reg2,mult,offset), if reg1 != reg2, RegDefUseDefUsed(eaDef1,reg1,ea,_), if ea != eaDef1, RegDefUseDefUsed(eaDef2,reg2,ea,_), if ea != eaDef2;
    DefUsedForAddress(*ea,*reg,meta.pc_relative) <-- DefUsedForAddressEDB(ea,reg,meta.pc_relative);
    DefUsedForAddress(*eaDef,*reg,*mtype) <-- RegDefUseDefUsed(eaDef,reg,ea,_), RegUsedFor(ea,reg,mtype);
    DefUsedForAddress(*eaDef,*reg,*mtype) <-- DefUsedForAddress(eaUsed,_,mtype), RegDefUseDefUsed(eaDef,reg,eaUsed,_);
    DefUsedForAddress(*eaDef,*reg1,*mtype) <-- DefUsedForAddress(eaLoad,reg2,mtype), ArchMemoryAccess(meta.load,eaLoad,reg2,regBaseLoad,meta.none,stackPosLoad), StackDefUseDefUsed(eaSt,regBaseSt,stackPosSt,eaLoad,regBaseLoad,stackPosLoad), ArchMemoryAccess(meta.store,eaSt,reg1,regBaseSt,meta.none,stackPosSt), RegDefUseDefUsed(eaDef,reg1,eaSt,_);
    StackDefUseDefUsed(*eaDef,*varr,*varp,*eaUsed,*varr,*varp) <-- StackDefUseUsed(eaUsed,varr,varp,_), StackDefUseBlockLastDef(eaUsed,eaDef,varr,varp);
    StackDefUseDefUsed(*eaDef,*defVarr,*defVarp,*eaUsed,*varUsedr,*varUsedp) <-- StackDefUseLiveVarAtBlockEnd(blk,blockUsed,varr,varp), StackDefUseLiveVarDef(blk,defVarr,defVarp,varr,varp,eaDef), StackDefUseLiveVarUsedEDB(blockUsed,varr,varp,varUsedr,varUsedp,eaUsed,_,_);
    StackDefUseDefUsed(*eaDef,*defVarr,*defVarp,*eaUsed,*usedVarr,*usedVarp) <-- StackDefUseLiveVarUsedEDB(ea,defVarr,defVarp,usedVarr,usedVarp,eaUsed,_,_), MayFallthrough(eaDef,ea), CodeInBlock(eaDef,blk), CodeInBlock(ea,blk), StackDefUseDef(eaDef,defVarr,defVarp);
    StackDefUseDefUsed(*eaDef,*varDefr,*varDefp,*nextEaUsed,*varUsedr,*varUsedp) <-- StackDefUseLiveVarAtPriorUsed(eaUsed,nxtBlk,varr,varp), StackDefUseDefUsed(eaDef,varDefr,varDefp,eaUsed,varr,varp), StackDefUseLiveVarUsedEDB(nxtBlk,varr,varp,varUsedr,varUsedp,nextEaUsed,_,_);
    StackDefUseLiveVarAtBlockEnd(*prevBlock,*blockUsed,*varr,*varp) <-- StackDefUseLiveVarAtBlockEnd(blk,blockUsed,varr,varp), !StackDefUseRefInBlock(blk,varr,varp), !RegDefUseDefinedInBlock(blk,varr), BlockNext(prevBlock,_,blk);
    StackDefUseLiveVarAtBlockEnd(*prevBlock,*blk,*varr,*varp) <-- BlockNext(prevBlock,_,blk), StackDefUseLiveVarUsedEDB(blk,varr,varp,_,_,_,_,_);
    StackDefUseLiveVarAtPriorUsed(*eaUsed,*blockUsed,*varr,*varp) <-- StackDefUseLiveVarAtBlockEnd(blk,blockUsed,varr,varp), StackDefUseUsedInBlock(blk,eaUsed,varr,varp,_), !RegDefUseDefinedInBlock(blk,varr), !StackDefUseDefinedInBlock(blk,varr,varp);
     };
    Results {
        JumpTableTarget: db.JumpTableTarget,
        RegDefUseDefUsed: db.RegDefUseDefUsed,
        RegDefUseReturnValUsed: db.RegDefUseReturnValUsed,
        RegDefUseLiveVarUsed: db.RegDefUseLiveVarUsed,
        RegDefUseLiveVarAtPriorUsed: db.RegDefUseLiveVarAtPriorUsed,
        RegDefUseLiveVarAtBlockEnd: db.RegDefUseLiveVarAtBlockEnd,
        RegRegArithOpDefs: db.RegRegArithOpDefs,
        DefUsedForAddress: db.DefUsedForAddress,
        StackDefUseDefUsed: db.StackDefUseDefUsed,
        StackDefUseLiveVarAtBlockEnd: db.StackDefUseLiveVarAtBlockEnd,
        StackDefUseLiveVarAtPriorUsed: db.StackDefUseLiveVarAtPriorUsed,
    }
}
