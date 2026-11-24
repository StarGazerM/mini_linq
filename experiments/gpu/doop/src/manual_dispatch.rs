//! One traversal of the virtual-dispatch delta produces three independent candidate bags.
//! The first manual hook owns the traversal; the other two move the completed batches, after
//! checking every common input's immutable snapshot ID. No result is admitted by a hook.
use std::{cell::RefCell, sync::Arc};
use cuda_device::{device, thread, atomic::{AtomicOrdering::Relaxed, DeviceAtomicU32, DeviceAtomicU64}};
use gpu_datalog_runtime::{Buffer, CandidateArena, CandidateBatch, Packing, Pool, RadixLayout, RadixLayoutView, Scalar, Word, Result};
use gpu_datalog_runtime::radix_run::device::{decode_key,run};
use cuda_core::LaunchConfig1D;

type Layout<K,W> = RadixLayout<K, (), W, u64, ()>;
const READS: [&str; 8] = ["delta varpointsto", "instruction_method[0, 1]", "heapallocation_type[0, 1]", "reachable[0]", "virtualmethodinvocation_simplename[0, 1]", "virtualmethodinvocation_descriptor[0, 1]", "methodlookup[0, 1, 2, 3]", "virtualmethodinvocation_base[1, 0]"];

// Separate chaining keeps *all* witnesses, including key collisions and non-functional keys.
// A row in a bucket is (next, raw physical columns...). 0 terminates; stored indices are 1-based.
#[repr(C)]
#[derive(Copy, Clone)]
pub struct Dir { heads: *mut u32, rows: *mut u32, mask: u32 }
unsafe impl cuda_core::DeviceCopy for Dir {}
#[repr(C)]
#[derive(Copy, Clone)]
pub struct Tables { base:Dir, method:Dir, reach:Dir, ty:Dir, name:Dir, desc:Dir, lookup:Dir, this:Dir }
unsafe impl cuda_core::DeviceCopy for Tables {}

#[device]
fn md_hash3(a:u32,b:u32,c:u32)->u32 {
    (a.wrapping_mul(0x9e3779b1) ^ b.wrapping_mul(0x85ebca77) ^ c.wrapping_mul(0xc2b2ae3d)).wrapping_mul(0x27d4eb2d)
}
#[device]
fn md_tid()->usize { (thread::blockIdx_x()*256+thread::threadIdx_x()) as usize }
#[device]
unsafe fn md_start(d:Dir,k:u32)->u32 { unsafe { *d.heads.add((k&d.mask) as usize) } }
#[device]
unsafe fn md_next(d:Dir,k:u32,slot:&mut u32)->Option<u32> {unsafe {
    while *slot!=0 { let p=d.rows.add((*slot as usize-1)*3); *slot=*p;
        if *p.add(1)==k {return Some(*p.add(2));}
    } None
}}
#[device]
unsafe fn md_has(d:Dir,k:u32)->bool {unsafe {
    let mut pos=md_start(d,k); while pos!=0 {let p=d.rows.add((pos as usize-1)*2);if *p.add(1)==k{return true;}pos=*p;} false
}}

#[cuda_device::cuda_module]
pub mod dispatch_kernels {
    use super::*;
    use cuda_device::{kernel, launch_bounds, launch_contract};
    #[kernel]
    #[launch_bounds(256)]
    #[launch_contract(domain=1,block=(256,1,1))]
    pub fn md_index<W:Word+Copy>(v:RadixLayoutView<W,u64>, cols:u32, d:Dir) {unsafe {
        let mut offset=0; let mut r=0;
        while r<v.keys.run_count {let n=run(v.keys,r).keys_len as usize;let mut i=md_tid();
            while i<n {let p=d.rows.add((offset+i)*(cols as usize+1));let mut j=0;
                while j<cols {*p.add(1+j as usize)=decode_key(v,r,i as u64,j) as u32;j+=1;}
                let k=if cols==4 {md_hash3(*p.add(1),*p.add(2),*p.add(3))}else{*p.add(1)};
                *p=DeviceAtomicU32::from_ptr(d.heads.add((k & d.mask) as usize)).swap((offset+i+1) as u32,Relaxed);
                i+=thread::gridDim_x() as usize*256;
            } offset+=n;r+=1;
        }
    }}
    #[kernel]
    #[launch_bounds(256)]
    #[launch_contract(domain=1,block=(256,1,1))]
    pub fn md_prefix<W:Word+Copy>(v:RadixLayoutView<W,u64>, t:Tables, counts:*mut u64,
      bindings:*mut u32, pair_cap:u64) {unsafe {
        let mut r=0;while r<v.keys.run_count {let n=run(v.keys,r).keys_len as usize; let mut i=md_tid();
        while i<n {let base=decode_key(v,r,i as u64,1) as u32;let mut a=md_start(t.base,base);
        while let Some(inv)=md_next(t.base,base,&mut a){let mut b=md_start(t.method,inv);
        while let Some(m)=md_next(t.method,inv,&mut b){if !md_has(t.reach,m){continue;}
        let heap=decode_key(v,r,i as u64,0) as u32;let mut c=md_start(t.ty,heap);
        while let Some(ty)=md_next(t.ty,heap,&mut c){let mut d=md_start(t.name,inv);
        while let Some(name)=md_next(t.name,inv,&mut d){let mut e=md_start(t.desc,inv);
        while let Some(desc)=md_next(t.desc,inv,&mut e){let mut f=md_start(t.lookup,md_hash3(name,desc,ty));
        while f!=0 {let p=t.lookup.rows.add((f as usize-1)*5);f=*p;
            if *p.add(1)!=name||*p.add(2)!=desc||*p.add(3)!=ty {continue;}
            let target=*p.add(4);let slot=DeviceAtomicU64::from_ptr(counts).fetch_add(1,Relaxed);
            if slot<pair_cap {let b=bindings.add(3*slot as usize);*b=heap;*b.add(1)=inv;*b.add(2)=target;}
        }}}}}} i+=thread::gridDim_x() as usize*256;}r+=1;}
    }}
    #[kernel]
    #[launch_bounds(256)]
    #[launch_contract(domain=1,block=(256,1,1))]
    pub fn md_project(th:Dir, head:u32, bindings:*const u32, n:usize, out:*mut u64, cap:u64, count:*mut u64) {unsafe {
        let mut i=md_tid();while i<n {let b=bindings.add(3*i);let target=*b.add(2);
            if head==0 {let mut it=md_start(th,target);while let Some(this)=md_next(th,target,&mut it){
                let pos=DeviceAtomicU64::from_ptr(count).fetch_add(1,Relaxed);if pos<cap {*out.add(pos as usize)=u64::from(*b)<<32|u64::from(this);}
            }}else if head==1 {*out.add(i)=u64::from(target);}else{*out.add(i)=u64::from(*b.add(1))<<32|u64::from(target);}
            i+=thread::gridDim_x() as usize*256;
        }
    }}
}

struct Index { snapshot:u64, heads:Buffer<u32>, rows:Buffer<u32> }
fn config(n:usize)->LaunchConfig1D {LaunchConfig1D::new(n.div_ceil(256).clamp(1,65535) as u32,256,0)}
fn index<K,W:Word+Copy+'static>(pool:&Arc<Pool>,module:&dispatch_kernels::LoadedModule, slot:&mut Option<Index>, src:&Layout<K,W>, width:usize)->Result<Dir>{
    if slot.as_ref().is_none_or(|x|x.snapshot!=src.snapshot_id()) {
        let n=src.len(); if n>u32::MAX as usize{return Err("dispatch index exceeds u32 positions".into());}
        let buckets=n.max(1).next_power_of_two();
        let heads=Buffer::zeroed(pool.clone(),buckets)?;
        let rows=unsafe{Buffer::uninitialized(pool.clone(),n*(width+1))?};
        let dir=Dir{heads:heads.ptr(),rows:rows.ptr(),mask:(buckets-1) as u32};
        if n!=0 {module.md_index(pool.stream(),&module.prepare_md_index::<W>(config(n))?,src.view_on(pool)?,width as u32,dir)?;}
        *slot=Some(Index{snapshot:src.snapshot_id(),heads,rows});
    }
    let x=slot.as_ref().unwrap();Ok(Dir{heads:x.heads.ptr(),rows:x.rows.ptr(),mask:(x.heads.len()-1) as u32})
}
struct Pending { snapshots:[u64;8], reachable:Option<CandidateBatch>, edge:Option<CandidateBatch> }
struct State { module:dispatch_kernels::LoadedModule, pool:Arc<Pool>, indexes:[Option<Index>;8], arenas:[CandidateArena;2], pending:Option<Pending> }
thread_local! {static STATE:RefCell<Option<State>>=const{RefCell::new(None)};}

#[allow(clippy::too_many_arguments)]
pub fn vpt<K0,W0:Word+Copy+'static,K1,W1:Word+Copy+'static,K2,W2:Word+Copy+'static,K3,W3:Word+Copy+'static,K4,W4:Word+Copy+'static,K5,W5:Word+Copy+'static,K6,W6:Word+Copy+'static,K7,W7:Word+Copy+'static,K8,W8:Word+Copy+'static>(arena:&mut CandidateArena,reads:&[&str], delta:&Layout<K0,W0>,method:&Layout<K1,W1>,ty:&Layout<K2,W2>,reach:&Layout<K3,W3>,name:&Layout<K4,W4>,desc:&Layout<K5,W5>,lookup:&Layout<K6,W6>,this:&Layout<K7,W7>,base:&Layout<K8,W8>)->CandidateBatch {
    assert_eq!(&reads[..7],&READS[..7]);assert_eq!(&reads[7..], &["thisvar[0, 1]",READS[7]]);
    STATE.with(|state|->Result<_>{let mut slot=state.borrow_mut();let pool=arena.pool().clone();
        if slot.as_ref().is_none_or(|s| !Arc::ptr_eq(&s.pool,&pool)){*slot=Some(State{module:unsafe{dispatch_kernels::load(pool.context())?},pool:pool.clone(),indexes:Default::default(),arenas:std::array::from_fn(|_|CandidateArena::new(pool.clone())),pending:None});}
        let s=slot.as_mut().unwrap();assert!(s.pending.is_none(),"previous dispatch consumers did not run");
        let snapshots=[delta.snapshot_id(),method.snapshot_id(),ty.snapshot_id(),reach.snapshot_id(),name.snapshot_id(),desc.snapshot_id(),lookup.snapshot_id(),base.snapshot_id()];
        let t=Tables{method:index(&pool,&s.module,&mut s.indexes[0],method,2)?,ty:index(&pool,&s.module,&mut s.indexes[1],ty,2)?,reach:index(&pool,&s.module,&mut s.indexes[2],reach,1)?,name:index(&pool,&s.module,&mut s.indexes[3],name,2)?,desc:index(&pool,&s.module,&mut s.indexes[4],desc,2)?,lookup:index(&pool,&s.module,&mut s.indexes[5],lookup,4)?,this:index(&pool,&s.module,&mut s.indexes[6],this,2)?,base:index(&pool,&s.module,&mut s.indexes[7],base,2)?};
        let pair=Packing::with_columns(&[<u32 as Scalar>::COLUMN;2])?; let one=Packing::with_columns(&[<u32 as Scalar>::COLUMN])?;
        let mut cv=arena.capacity_hint(&pair);let mut cp=s.arenas[0].capacity_hint(&one);
        let (bindings,cp)=loop {
            let b=unsafe{Buffer::<u32>::uninitialized(pool.clone(),cp*3)?};let cnt=Buffer::<u64>::zeroed(pool.clone(),1)?;
            if !delta.is_empty(){s.module.md_prefix(pool.stream(),&s.module.prepare_md_prefix::<W0>(config(delta.len()))?,delta.view_on(&pool)?,t,cnt.ptr(),b.ptr(),cp as u64)?;}
            let n=cnt.to_host()?[0];gpu_datalog_runtime::record_readback();if n>u64::from(u32::MAX){return Err("prefix exceeds candidate position limit".into());}
            if n<=cp as u64{break (b,n as usize);}cp=n as usize;
        };
        let r=s.arenas[0].reserve(one.clone(),cp)?;let e=s.arenas[1].reserve(pair.clone(),cp)?;
        let proj=s.module.prepare_md_project(config(cp))?;
        if cp>0{for (h,dst) in [(1,r.buffer().ptr()),(2,e.buffer().ptr())]{
            s.module.md_project(pool.stream(),&proj,t.this,h,bindings.ptr(),cp,dst,cp as u64,std::ptr::null_mut())?;
        }}
        loop {let v=arena.reserve(pair.clone(),cv)?;let cnt=Buffer::<u64>::zeroed(pool.clone(),1)?;
            if cp>0{s.module.md_project(pool.stream(),&proj,t.this,0,bindings.ptr(),cp,v.buffer().ptr(),v.capacity() as u64,cnt.ptr())?;}
            let n=cnt.to_host()?[0];gpu_datalog_runtime::record_readback();if n>u64::from(u32::MAX){return Err("VPT exceeds candidate position limit".into());}
            cv=n as usize;
            if cv<=v.capacity(){
                arena.record_count(cv);s.arenas[0].record_count(cp);s.arenas[1].record_count(cp);
                // Counter read fenced the stream: no retained read-side owner is needed.
                s.pending=Some(Pending {snapshots,reachable:Some(unsafe{r.finish(cp,vec![])?}),edge:Some(unsafe{e.finish(cp,vec![])?})});
                return unsafe{v.finish(cv,vec![])};
            }
        }
    }).expect("virtual dispatch traversal")
}
macro_rules! consumer {($fn:ident,$field:ident)=>{
    pub fn $fn<K0,W0:Word+Copy+'static,K1,W1:Word+Copy+'static,K2,W2:Word+Copy+'static,K3,W3:Word+Copy+'static,K4,W4:Word+Copy+'static,K5,W5:Word+Copy+'static,K6,W6:Word+Copy+'static,K7,W7:Word+Copy+'static>(arena:&mut CandidateArena,reads:&[&str],d:&Layout<K0,W0>,m:&Layout<K1,W1>,t:&Layout<K2,W2>,r:&Layout<K3,W3>,n:&Layout<K4,W4>,de:&Layout<K5,W5>,l:&Layout<K6,W6>,b:&Layout<K7,W7>)->CandidateBatch {
        assert_eq!(reads,READS); STATE.with(|state|{let mut slot=state.borrow_mut();let s=slot.as_mut().expect("dispatch producer must precede consumers");assert!(Arc::ptr_eq(&s.pool,arena.pool()));let p=s.pending.as_mut().expect("missing dispatch prefix");
            assert_eq!(p.snapshots,[d.snapshot_id(),m.snapshot_id(),t.snapshot_id(),r.snapshot_id(),n.snapshot_id(),de.snapshot_id(),l.snapshot_id(),b.snapshot_id()]);
            let out=p.$field.take().expect("dispatch consumer already ran");if p.reachable.is_none()&&p.edge.is_none(){s.pending=None;}arena.record_count(out.len());out})
    }
};}
consumer!(reachable,reachable);
consumer!(edge,edge);
