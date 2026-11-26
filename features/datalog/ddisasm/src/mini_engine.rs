/// Instantiate the same facts, rules and outputs with the selected compiler pipeline.
#[macro_export]
macro_rules! ddisasm_engine {
    ($name:ident, $id:ident, [$($params:tt)*], $decode:expr, pipeline { $($pipeline:tt)* }) => {
pub fn $name(facts: $crate::Facts<$id> $($params)*) -> $crate::Results {
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
     pipeline { $($pipeline)* }
     struct Ddisasm;
     relation ArchMemoryAccess(c0:$id,c1:$id,c2:$id,c3:$id,c4:$id,c5:$id) = (facts.ArchMemoryAccess);
     relation ArchRegRegArithOp(c0:$id,c1:$id,c2:$id,c3:$id,c4:$id,c5:$id) = (facts.ArchRegRegArithOp);
     relation ArchReturnReg(c0:$id) = (facts.ArchReturnReg);
     relation BlockNext(c0:$id,c1:$id,c2:$id) = (facts.BlockNext);
     relation BlockLastInstr(c0:$id,c1:$id) = (facts.BlockLastInstr);
     relation CodeInBlock(c0:$id,c1:$id) = (facts.CodeInBlock);
     relation DirectCall(c0:$id,c1:$id) = (facts.DirectCall);
     relation MayFallthrough(c0:$id,c1:$id) = (facts.MayFallthrough);
     relation RegDefUseBlockLastDef(c0:$id,c1:$id,c2:$id) = (facts.RegDefUseBlockLastDef);
     relation RegDefUseDefinedInBlock(c0:$id,c1:$id) = (facts.RegDefUseDefinedInBlock);
     relation RegDefUseFlowDef(c0:$id,c1:$id,c2:$id,c3:$id) = (facts.RegDefUseFlowDef);
     relation RegDefUseLiveVarDef(c0:$id,c1:$id,c2:$id,c3:$id) = (facts.RegDefUseLiveVarDef);
     relation RegDefUseRefInBlock(c0:$id,c1:$id) = (facts.RegDefUseRefInBlock);
     relation RegDefUseReturnBlockEnd(c0:$id,c1:$id,c2:$id,c3:$id) = (facts.RegDefUseReturnBlockEnd);
     relation RegDefUseUsed(c0:$id,c1:$id,c2:$id) = (facts.RegDefUseUsed);
     relation RegDefUseUsedInBlock(c0:$id,c1:$id,c2:$id,c3:$id) = (facts.RegDefUseUsedInBlock);
     relation RegUsedFor(c0:$id,c1:$id,c2:$id) = (facts.RegUsedFor);
     relation RelJumpTableEntryCandidate(c0:$id,c1:$id,c2:$id,c3:$id,c4:$id,c5:$id,c6:$id) = (facts.RelJumpTableEntryCandidate);
     relation StackDefUseDef(c0:$id,c1:$id,c2:$id) = (facts.StackDefUseDef);
     relation StackDefUseDefinedInBlock(c0:$id,c1:$id,c2:$id) = (facts.StackDefUseDefinedInBlock);
     relation StackDefUseLiveVarDef(c0:$id,c1:$id,c2:$id,c3:$id,c4:$id,c5:$id) = (facts.StackDefUseLiveVarDef);
     relation StackDefUseRefInBlock(c0:$id,c1:$id,c2:$id) = (facts.StackDefUseRefInBlock);
     relation StackDefUseUsedInBlock(c0:$id,c1:$id,c2:$id,c3:$id,c4:$id) = (facts.StackDefUseUsedInBlock);
     relation StackDefUseUsed(c0:$id,c1:$id,c2:$id,c3:$id) = (facts.StackDefUseUsed);
     relation StackDefUseLiveVarUsedEDB(c0:$id,c1:$id,c2:$id,c3:$id,c4:$id,c5:$id,c6:$id,c7:$id) = (facts.StackDefUseLiveVarUsedEDB);
     relation JumpTableStart(c0:$id,c1:$id,c2:$id,c3:$id,c4:$id) = (facts.JumpTableStart);
     relation DefUsedForAddressEDB(c0:$id,c1:$id,c2:$id) = (facts.DefUsedForAddressEDB);
     relation StackDefUseBlockLastDef(c0:$id,c1:$id,c2:$id,c3:$id) = (facts.StackDefUseBlockLastDef);
     relation JumpTableTarget(c0:$id,c1:$id);
     relation RegDefUseDefUsed(c0:$id,c1:$id,c2:$id,c3:$id);
     relation RegDefUseReturnValUsed(c0:$id,c1:$id,c2:$id,c3:$id,c4:$id);
     relation RegDefUseLiveVarUsed(c0:$id,c1:$id,c2:$id,c3:$id);
     relation RegDefUseLiveVarAtPriorUsed(c0:$id,c1:$id,c2:$id);
     relation RegDefUseLiveVarAtBlockEnd(c0:$id,c1:$id,c2:$id);
     relation RegRegArithOpDefs(c0:$id,c1:$id,c2:$id,c3:$id,c4:$id,c5:$id,c6:$id,c7:$id);
     relation DefUsedForAddress(c0:$id,c1:$id,c2:$id);
     relation StackDefUseDefUsed(c0:$id,c1:$id,c2:$id,c3:$id,c4:$id,c5:$id);
     relation StackDefUseLiveVarAtBlockEnd(c0:$id,c1:$id,c2:$id,c3:$id);
     relation StackDefUseLiveVarAtPriorUsed(c0:$id,c1:$id,c2:$id,c3:$id);
     output JumpTableTarget,RegDefUseDefUsed,RegDefUseReturnValUsed,RegDefUseLiveVarUsed,RegDefUseLiveVarAtPriorUsed,RegDefUseLiveVarAtBlockEnd,RegRegArithOpDefs,DefUsedForAddress,StackDefUseDefUsed,StackDefUseLiveVarAtBlockEnd,StackDefUseLiveVarAtPriorUsed;
    JumpTableTarget(ea,dest) :- JumpTableStart(ea,size,tableStart,_,_), RelJumpTableEntryCandidate(_,tableStart,size,_,dest,_,_);
    RegDefUseDefUsed(eaDef,mvar,eaUsed,index) :- RegDefUseUsed(eaUsed,mvar,index), RegDefUseBlockLastDef(eaUsed,eaDef,mvar);
    RegDefUseDefUsed(eaDef,varIdentity,eaUsed,index) :- RegDefUseLiveVarAtBlockEnd(blk,blockUsed,mvar), RegDefUseLiveVarDef(blk,varIdentity,mvar,eaDef), RegDefUseLiveVarUsed(blockUsed,mvar,eaUsed,index);
    RegDefUseDefUsed(eaDef,mvar,nextEaUsed,nextIndex) :- RegDefUseLiveVarAtPriorUsed(eaUsed,nxtBlk,mvar), RegDefUseDefUsed(eaDef,mvar,eaUsed,_), RegDefUseLiveVarUsed(nxtBlk,mvar,nextEaUsed,nextIndex);
    RegDefUseDefUsed(eaDef,reg,eaUsed,index) :- RegDefUseReturnValUsed(_,callee,reg,eaUsed,index), RegDefUseReturnBlockEnd(callee,_,_,blockEnd), RegDefUseBlockLastDef(blockEnd,eaDef,reg);
    RegDefUseReturnValUsed(eaCall,callee,reg,eaUsed,indexUsed) :- ArchReturnReg(reg), RegDefUseDefUsed(eaCall,reg,eaUsed,indexUsed), DirectCall(eaCall,callee);
    RegDefUseLiveVarUsed(blk,mvar,eaUsed,index) :- RegDefUseUsedInBlock(blk,eaUsed,mvar,index), !RegDefUseBlockLastDef(eaUsed,_,mvar);
    RegDefUseLiveVarUsed(retBlock,reg,eaUsed,index) :- RegDefUseReturnBlockEnd(callee,_,retBlock,retBlockEnd), RegDefUseReturnValUsed(_,callee,reg,eaUsed,index), !RegDefUseBlockLastDef(retBlockEnd,_,reg);
    RegDefUseLiveVarAtPriorUsed(eaUsed,blockUsed,mvar) :- RegDefUseLiveVarAtBlockEnd(blk,blockUsed,mvar), RegDefUseUsedInBlock(blk,eaUsed,mvar,_), !RegDefUseDefinedInBlock(blk,mvar);
    RegDefUseLiveVarAtBlockEnd(prevBlock,blk,mvar) :- BlockNext(prevBlock,prevBlockEnd,blk), RegDefUseLiveVarUsed(blk,mvar,_,_), !RegDefUseFlowDef(prevBlockEnd,mvar,blk,_);
    RegDefUseLiveVarAtBlockEnd(prevBlock,blockUsed,mvar) :- RegDefUseLiveVarAtBlockEnd(blk,blockUsed,mvar), !RegDefUseRefInBlock(blk,mvar), BlockNext(prevBlock,_,blk);
    RegRegArithOpDefs(ea,regDef,eaDef1,reg1,eaDef2,reg2,mult,offset) :- DefUsedForAddress(ea,regDef,_), ArchRegRegArithOp(ea,regDef,reg1,reg2,mult,offset), where reg1 != reg2, RegDefUseDefUsed(eaDef1,reg1,ea,_), where ea != eaDef1, RegDefUseDefUsed(eaDef2,reg2,ea,_), where ea != eaDef2;
    DefUsedForAddress(ea,reg,meta.pc_relative) :- DefUsedForAddressEDB(ea,reg,meta.pc_relative);
    DefUsedForAddress(eaDef,reg,mtype) :- RegDefUseDefUsed(eaDef,reg,ea,_), RegUsedFor(ea,reg,mtype);
    DefUsedForAddress(eaDef,reg,mtype) :- DefUsedForAddress(eaUsed,_,mtype), RegDefUseDefUsed(eaDef,reg,eaUsed,_);
    DefUsedForAddress(eaDef,reg1,mtype) :- DefUsedForAddress(eaLoad,reg2,mtype), ArchMemoryAccess(meta.load,eaLoad,reg2,regBaseLoad,meta.none,stackPosLoad), StackDefUseDefUsed(eaSt,regBaseSt,stackPosSt,eaLoad,regBaseLoad,stackPosLoad), ArchMemoryAccess(meta.store,eaSt,reg1,regBaseSt,meta.none,stackPosSt), RegDefUseDefUsed(eaDef,reg1,eaSt,_);
    StackDefUseDefUsed(eaDef,varr,varp,eaUsed,varr,varp) :- StackDefUseUsed(eaUsed,varr,varp,_), StackDefUseBlockLastDef(eaUsed,eaDef,varr,varp);
    StackDefUseDefUsed(eaDef,defVarr,defVarp,eaUsed,varUsedr,varUsedp) :- StackDefUseLiveVarAtBlockEnd(blk,blockUsed,varr,varp), StackDefUseLiveVarDef(blk,defVarr,defVarp,varr,varp,eaDef), StackDefUseLiveVarUsedEDB(blockUsed,varr,varp,varUsedr,varUsedp,eaUsed,_,_);
    StackDefUseDefUsed(eaDef,defVarr,defVarp,eaUsed,usedVarr,usedVarp) :- StackDefUseLiveVarUsedEDB(ea,defVarr,defVarp,usedVarr,usedVarp,eaUsed,_,_), MayFallthrough(eaDef,ea), CodeInBlock(eaDef,blk), CodeInBlock(ea,blk), StackDefUseDef(eaDef,defVarr,defVarp);
    StackDefUseDefUsed(eaDef,varDefr,varDefp,nextEaUsed,varUsedr,varUsedp) :- StackDefUseLiveVarAtPriorUsed(eaUsed,nxtBlk,varr,varp), StackDefUseDefUsed(eaDef,varDefr,varDefp,eaUsed,varr,varp), StackDefUseLiveVarUsedEDB(nxtBlk,varr,varp,varUsedr,varUsedp,nextEaUsed,_,_);
    StackDefUseLiveVarAtBlockEnd(prevBlock,blockUsed,varr,varp) :- StackDefUseLiveVarAtBlockEnd(blk,blockUsed,varr,varp), !StackDefUseRefInBlock(blk,varr,varp), !RegDefUseDefinedInBlock(blk,varr), BlockNext(prevBlock,_,blk);
    StackDefUseLiveVarAtBlockEnd(prevBlock,blk,varr,varp) :- BlockNext(prevBlock,_,blk), StackDefUseLiveVarUsedEDB(blk,varr,varp,_,_,_,_,_);
    StackDefUseLiveVarAtPriorUsed(eaUsed,blockUsed,varr,varp) :- StackDefUseLiveVarAtBlockEnd(blk,blockUsed,varr,varp), StackDefUseUsedInBlock(blk,eaUsed,varr,varp,_), !RegDefUseDefinedInBlock(blk,varr), !StackDefUseDefinedInBlock(blk,varr,varp);
     };
    $crate::Results {
        JumpTableTarget: JumpTableTarget.map($decode).collect(),
        RegDefUseDefUsed: RegDefUseDefUsed.map($decode).collect(),
        RegDefUseReturnValUsed: RegDefUseReturnValUsed.map($decode).collect(),
        RegDefUseLiveVarUsed: RegDefUseLiveVarUsed.map($decode).collect(),
        RegDefUseLiveVarAtPriorUsed: RegDefUseLiveVarAtPriorUsed.map($decode).collect(),
        RegDefUseLiveVarAtBlockEnd: RegDefUseLiveVarAtBlockEnd.map($decode).collect(),
        RegRegArithOpDefs: RegRegArithOpDefs.map($decode).collect(),
        DefUsedForAddress: DefUsedForAddress.map($decode).collect(),
        StackDefUseDefUsed: StackDefUseDefUsed.map($decode).collect(),
        StackDefUseLiveVarAtBlockEnd: StackDefUseLiveVarAtBlockEnd.map($decode).collect(),
        StackDefUseLiveVarAtPriorUsed: StackDefUseLiveVarAtPriorUsed.map($decode).collect(),
    }
}
    };
}

ddisasm_engine!(run, i64, [], ::core::convert::identity, pipeline {});
