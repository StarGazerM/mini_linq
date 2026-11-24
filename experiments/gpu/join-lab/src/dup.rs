//! Hole 132 replay: raw expansion, shared witness lists, and exact early admission.
use cuda_device::{device, thread, warp};
use cuda_device::atomic::{AtomicOrdering::Relaxed, DeviceAtomicU32, DeviceAtomicU64};

#[derive(Clone,Copy)] #[repr(C)]
pub struct DupIndex {
    pub delta: *const u32, pub n: u32,
    // Triples (dense target, raw base, method), sorted by (signature,target,base,method).
    pub loads: *const u32, pub sig_off: *const u32, pub sig_min: u32,
    pub reachable: *const u32, pub vp_off: *const u32, pub vp_vars: *const u32,
    pub decode: *const u32, pub targets: u32,
}
#[derive(Clone,Copy)] #[repr(C)]
pub struct DupWork {
    pub groups: *mut u64, pub group_mask: u32, pub slot: *mut u32,
    pub t_range: *mut u64, pub t: *mut u32, pub t_cap: u32,
    pub heaps: *mut u64, pub heap_cap: u32,
    pub seen: *mut u64, pub seen_mask: u32, pub out: *mut u64, pub out_cap: u32,
    // output length, T arena bump, heap descriptor count, overflow flag
    pub meta: *mut u32,
}

#[device] fn dup_thread() -> (u32,u32) {
    (thread::blockIdx_x()*256+thread::threadIdx_x(),thread::gridDim_x()*256)
}
#[device] unsafe fn dup_add(p: *mut u32,n:u32)->u32 { unsafe {DeviceAtomicU32::from_ptr(p)}.fetch_add(n,Relaxed) }
#[device] unsafe fn dup_fail(w:DupWork) {unsafe {DeviceAtomicU32::from_ptr(w.meta.add(3))}.store(1,Relaxed);}
#[device] unsafe fn dup_vp(x:DupIndex,h:u32,v:u32)->bool {unsafe {
    let p=x.vp_off.add(h as usize); let mut lo=*p; let end=*p.add(1); let mut hi=end;
    while lo<hi {let m=lo+(hi-lo)/2; if *x.vp_vars.add(m as usize)<v {lo=m+1;} else {hi=m;}}
    lo<end && *x.vp_vars.add(lo as usize)==v
}}
#[device] unsafe fn dup_hit(x:DupIndex,bh:u32,p:*const u32)->bool {unsafe {
    let m=*p.add(2); (*x.reachable.add((m/32) as usize) & (1<<(m%32)))!=0 && dup_vp(x,bh,*p.add(1))
}}
// Zero is empty; both uses encode the key as pair+1. Never spin on a full table.
#[device] unsafe fn dup_insert(tab:*mut u64,mask:u32,key:u64)->(u32,bool) {unsafe {
    let mut z=key^(key>>30); z=z.wrapping_mul(0xbf58476d1ce4e5b9); z^=z>>27; z=z.wrapping_mul(0x94d049bb133111eb);
    let mut i=(z^(z>>31)) as u32 & mask; let start=i;
    loop { let cell=DeviceAtomicU64::from_ptr(tab.add(i as usize));
        let old=cell.load(Relaxed); if old==key {return (i,false);}
        if old==0 {match cell.compare_exchange(0,key,Relaxed,Relaxed) {Ok(_)=>return(i,true), Err(k) if k==key=>return(i,false), _=>{}}}
        i=(i+1)&mask; if i==start {return (u32::MAX,false);}
    }
}}
#[device] unsafe fn dup_append(w:DupWork,yes:bool,key:u64) {unsafe {
    let bits=warp::ballot_sync(u32::MAX,yes); if bits!=0 {
        let mut dst=0; if warp::lane_id()==0 {dst=dup_add(w.meta,bits.count_ones());}
        dst=warp::shuffle_u64_sync(u32::MAX,dst as u64,0) as u32;
        if yes {let j=dst+(bits&warp::lanemask_lt()).count_ones(); if j<w.out_cap {*w.out.add(j as usize)=key;}else{dup_fail(w);}}
    }
}}

#[cuda_device::cuda_module]
pub mod dup_kernels {
    use super::*;
    use cuda_device::{kernel,launch_bounds,launch_contract};
    #[kernel] #[launch_bounds(256)] #[launch_contract(domain=1,block=(256,1,1))]
    pub fn dup_raw(x:DupIndex,w:DupWork) { unsafe {
        let (id,stride)=dup_thread(); let lane=warp::lane_id(); let mut i=id/32;
        while i<x.n {let d=x.delta.add(i as usize*3); let h=*d; let bh=*d.add(2);
            let dir=x.sig_off.add((*d.add(1)-x.sig_min) as usize); let mut j=*dir; let end=*dir.add(1);
            while j<end {let mut yes=false; let mut key=0;
                if j+lane<end {let p=x.loads.add((j+lane) as usize*3);yes=dup_hit(x,bh,p);if yes {key=(h as u64)<<32 | *x.decode.add(*p as usize) as u64;}}
                dup_append(w,yes,key); j+=32;
            } i+=stride/32;
        }
    }}
    #[kernel] #[launch_bounds(256)] #[launch_contract(domain=1,block=(256,1,1))]
    pub fn dup_index_groups(x:DupIndex,w:DupWork,segment:u32) {unsafe {
        let (mut i,stride)=dup_thread(); while i<x.n {let d=x.delta.add(i as usize*3);
            let key=(((*d.add(1) as u64)<<32)| *d.add(2) as u64)+1;
            let (s,_)=dup_insert(w.groups,w.group_mask,key); if s==u32::MAX {dup_fail(w);} *w.slot.add(i as usize)=s;
            if segment!=0 && (i==0 || *d!=*d.sub(3) || (segment==2 && *d.add(1)!=*d.sub(2))) {
                let h=*d;let sig=*d.add(1);let(mut lo,mut hi)=(i+1,x.n);
                while lo<hi {let m=lo+(hi-lo)/2;let k=x.delta.add(m as usize*3);
                    if *k<h || (*k==h && (segment==1 || *k.add(1)<=sig)) {lo=m+1;}else{hi=m;}}
                let j=dup_add(w.meta.add(2),1); if j<w.heap_cap {*w.heaps.add(j as usize)=((i as u64)<<32)|lo as u64;}else{dup_fail(w);}
            } i+=stride;
        }
    }}
    // One warp for each (sig,baseheap). Reserve the fanout upper bound, but keep only distinct successful targets.
    #[kernel] #[launch_bounds(256)] #[launch_contract(domain=1,block=(256,1,1))]
    pub fn dup_targets(x:DupIndex,w:DupWork,as_bits:bool) {unsafe {
        let (id,stride)=dup_thread(); let lane=warp::lane_id(); let mut s=id/32;
        while s<=w.group_mask {let encoded=*w.groups.add(s as usize); if encoded!=0 {
            let key=encoded-1;let dir=x.sig_off.add(((key>>32) as u32-x.sig_min) as usize);let start=*dir;let end=*dir.add(1);
            let mut at=0; if lane==0 && end>start {at=dup_add(w.meta.add(1),if as_bits {(end-start).div_ceil(32)}else{end-start});} at=warp::shuffle_u64_sync(u32::MAX,at as u64,0) as u32;
            let mut count=0; let mut j=start;
            while j<end {let k=j+lane;let mut target=0;let mut yes=false;
                if k<end {let mut p=x.loads.add(k as usize*3);target=*p;
                    if k==start || *p.sub(3)!=target {let mut z=k;
                        while z<end && *p==target {if dup_hit(x,key as u32,p) {yes=true;break;} z+=1;p=p.add(3);}
                    }
                }
                let mask=warp::ballot_sync(u32::MAX,yes);
                if as_bits {if lane==0 {let dest=at+(j-start)/32;if dest<w.t_cap {*w.t.add(dest as usize)=mask;}else{dup_fail(w);}}}
                else if yes {let dest=at+count+(mask&warp::lanemask_lt()).count_ones();if dest<w.t_cap {*w.t.add(dest as usize)=target;}else{dup_fail(w);}}
                count+=mask.count_ones(); j+=32;
            }
            if lane==0 {*w.t_range.add(s as usize)=((at as u64)<<32)|count as u64;}
        } s+=stride/32;}
    }}
    // Union the signature-local T bitsets across all baseheaps for a single (heap,sig).
    // Only cross-signature duplicates remain for the round hash.
    #[kernel] #[launch_bounds(256)] #[launch_contract(domain=1,block=(256,1,1))]
    pub fn dup_union(x:DupIndex,w:DupWork) {unsafe {
        let (id,stride)=dup_thread();let lane=warp::lane_id();let mut desc=id/32;
        let n=(*w.meta.add(2)).min(w.heap_cap);
        while desc<n {let desc_range=*w.heaps.add(desc as usize);let start=(desc_range>>32) as u32;let end=desc_range as u32;
            let d=x.delta.add(start as usize*3);let h=*d;let off=x.sig_off.add((*d.add(1)-x.sig_min) as usize);
            let l=*off;let rows=*off.add(1)-l;
            let mut word=0;while word<rows.div_ceil(32) {let mut i=start+lane;let mut mask=0;
                while i<end {let slot=*w.slot.add(i as usize);
                    if slot!=u32::MAX {let at=(*w.t_range.add(slot as usize)>>32) as u32;mask|=*w.t.add((at+word) as usize);}
                    i+=32;
                }
                let mut step=16;while step!=0 {mask|=warp::shuffle_u64_sync(u32::MAX,mask as u64,lane^step) as u32;step/=2;}
                let mut yes=(mask&(1<<lane))!=0;let mut key=0;
                if yes {let t=*x.loads.add((l+word*32+lane) as usize*3);let v=*x.decode.add(t as usize);
                    key=((h as u64)<<32)|v as u64;let(slot,won)=dup_insert(w.seen,w.seen_mask,key+1);
                    if slot==u32::MAX {dup_fail(w);}yes=won && !dup_vp(x,h,v);
                }
                dup_append(w,yes,key);word+=1;
            }
            desc+=stride/32;
        }
    }}
    // mode 0 = bag, 1 = unique, 2 = anti-membership only, 3 = unique then anti-membership.
    #[kernel] #[launch_bounds(256)] #[launch_contract(domain=1,block=(256,1,1))]
    pub fn dup_emit(x:DupIndex,w:DupWork,mode:u32) {unsafe {
        let(id,stride)=dup_thread();let mut i=id/32;let lane=warp::lane_id();
        while i<x.n {let s=*w.slot.add(i as usize); if s!=u32::MAX {let r=*w.t_range.add(s as usize);let h=*x.delta.add(i as usize*3);
            let mut j=0;while j<r as u32 {let mut key=0;let mut yes=false;
                if j+lane<r as u32 {let t=*w.t.add(((r>>32) as u32+j+lane) as usize);let v=*x.decode.add(t as usize);
                    key=((h as u64)<<32)|v as u64;yes=true;
                    if mode&1!=0 {let(slot,won)=dup_insert(w.seen,w.seen_mask,key+1);if slot==u32::MAX{dup_fail(w);}yes=won;}
                    if mode&2!=0 && yes {yes=!dup_vp(x,h,v);}
                }
                dup_append(w,yes,key);j+=32;
            }
        } i+=stride/32;}
    }}
    // Delta is heap-major: one CTA owns all candidates of a heap. The entire output row of the
    // incidence matrix fits in shared memory after static load-target dictionary compression.
    #[kernel] #[launch_bounds(256)] #[launch_contract(domain=1,block=(256,1,1),dynamic_shared_range=(4,32768))]
    pub fn dup_heap_tile(x:DupIndex,w:DupWork) {unsafe {
        use cuda_device::DynamicSharedArray;
        let bits=DynamicSharedArray::<u32,4>::get_raw().cast::<u32>();let tid=thread::threadIdx_x();let lane=warp::lane_id();
        let mut desc=thread::blockIdx_x(); let nh=(*w.meta.add(2)).min(w.heap_cap);
        while desc<nh {let mut word=tid;while word<x.targets.div_ceil(32) {*bits.add(word as usize)=0;word+=256;}thread::sync_threads();
            let range=*w.heaps.add(desc as usize);let mut i=(range>>32) as u32+tid/32;
            let h=*x.delta.add((range>>32) as usize*3);
            while i<range as u32 {let s=*w.slot.add(i as usize);if s!=u32::MAX {let r=*w.t_range.add(s as usize);let mut j=0;
                while j<r as u32 {let mut yes=false;let mut v=0;
                    if j+lane<r as u32 {let t=*w.t.add(((r>>32) as u32+j+lane) as usize);let mask=1<<(t%32);
                        let old=DeviceAtomicU32::from_ptr(bits.add((t/32) as usize)).fetch_or(mask,Relaxed);
                        if old&mask==0 {v=*x.decode.add(t as usize);yes=!dup_vp(x,h,v);}
                    } dup_append(w,yes,((h as u64)<<32)|v as u64); j+=32;
                }
            }i+=8;}
            thread::sync_threads();desc+=thread::gridDim_x();
        }
    }}
}
