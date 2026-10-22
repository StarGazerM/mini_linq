use crate::{Facts, Results};
pub fn run(facts: Facts) -> Results {
    let meta = facts.meta;
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
    ) = cq_datalog::datalog! {
     struct Ddisasm;
     relation ArchMemoryAccess(c0:i64,c1:i64,c2:i64,c3:i64,c4:i64,c5:i64) = (facts.ArchMemoryAccess);
     relation ArchRegRegArithOp(c0:i64,c1:i64,c2:i64,c3:i64,c4:i64,c5:i64) = (facts.ArchRegRegArithOp);
     relation ArchReturnReg(c0:i64) = (facts.ArchReturnReg);
     relation BlockNext(c0:i64,c1:i64,c2:i64) = (facts.BlockNext);
     relation BlockLastInstr(c0:i64,c1:i64) = (facts.BlockLastInstr);
     relation CodeInBlock(c0:i64,c1:i64) = (facts.CodeInBlock);
     relation DirectCall(c0:i64,c1:i64) = (facts.DirectCall);
     relation MayFallthrough(c0:i64,c1:i64) = (facts.MayFallthrough);
     relation RegDefUseBlockLastDef(c0:i64,c1:i64,c2:i64) = (facts.RegDefUseBlockLastDef);
     relation RegDefUseDefinedInBlock(c0:i64,c1:i64) = (facts.RegDefUseDefinedInBlock);
     relation RegDefUseFlowDef(c0:i64,c1:i64,c2:i64,c3:i64) = (facts.RegDefUseFlowDef);
     relation RegDefUseLiveVarDef(c0:i64,c1:i64,c2:i64,c3:i64) = (facts.RegDefUseLiveVarDef);
     relation RegDefUseRefInBlock(c0:i64,c1:i64) = (facts.RegDefUseRefInBlock);
     relation RegDefUseReturnBlockEnd(c0:i64,c1:i64,c2:i64,c3:i64) = (facts.RegDefUseReturnBlockEnd);
     relation RegDefUseUsed(c0:i64,c1:i64,c2:i64) = (facts.RegDefUseUsed);
     relation RegDefUseUsedInBlock(c0:i64,c1:i64,c2:i64,c3:i64) = (facts.RegDefUseUsedInBlock);
     relation RegUsedFor(c0:i64,c1:i64,c2:i64) = (facts.RegUsedFor);
     relation RelJumpTableEntryCandidate(c0:i64,c1:i64,c2:i64,c3:i64,c4:i64,c5:i64,c6:i64) = (facts.RelJumpTableEntryCandidate);
     relation StackDefUseDef(c0:i64,c1:i64,c2:i64) = (facts.StackDefUseDef);
     relation StackDefUseDefinedInBlock(c0:i64,c1:i64,c2:i64) = (facts.StackDefUseDefinedInBlock);
     relation StackDefUseLiveVarDef(c0:i64,c1:i64,c2:i64,c3:i64,c4:i64,c5:i64) = (facts.StackDefUseLiveVarDef);
     relation StackDefUseRefInBlock(c0:i64,c1:i64,c2:i64) = (facts.StackDefUseRefInBlock);
     relation StackDefUseUsedInBlock(c0:i64,c1:i64,c2:i64,c3:i64,c4:i64) = (facts.StackDefUseUsedInBlock);
     relation StackDefUseUsed(c0:i64,c1:i64,c2:i64,c3:i64) = (facts.StackDefUseUsed);
     relation StackDefUseLiveVarUsedEDB(c0:i64,c1:i64,c2:i64,c3:i64,c4:i64,c5:i64,c6:i64,c7:i64) = (facts.StackDefUseLiveVarUsedEDB);
     relation JumpTableStart(c0:i64,c1:i64,c2:i64,c3:i64,c4:i64) = (facts.JumpTableStart);
     relation DefUsedForAddressEDB(c0:i64,c1:i64,c2:i64) = (facts.DefUsedForAddressEDB);
     relation StackDefUseBlockLastDef(c0:i64,c1:i64,c2:i64,c3:i64) = (facts.StackDefUseBlockLastDef);
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
    Results {
        JumpTableTarget: JumpTableTarget.collect(),
        RegDefUseDefUsed: RegDefUseDefUsed.collect(),
        RegDefUseReturnValUsed: RegDefUseReturnValUsed.collect(),
        RegDefUseLiveVarUsed: RegDefUseLiveVarUsed.collect(),
        RegDefUseLiveVarAtPriorUsed: RegDefUseLiveVarAtPriorUsed.collect(),
        RegDefUseLiveVarAtBlockEnd: RegDefUseLiveVarAtBlockEnd.collect(),
        RegRegArithOpDefs: RegRegArithOpDefs.collect(),
        DefUsedForAddress: DefUsedForAddress.collect(),
        StackDefUseDefUsed: StackDefUseDefUsed.collect(),
        StackDefUseLiveVarAtBlockEnd: StackDefUseLiveVarAtBlockEnd.collect(),
        StackDefUseLiveVarAtPriorUsed: StackDefUseLiveVarAtPriorUsed.collect(),
    }
}
