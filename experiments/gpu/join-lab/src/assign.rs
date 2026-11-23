//! Grouped union for Assign(from,to) ⋈ VPT(heap,from). Either side may be the delta.
use cuda_device::{atomic::{AtomicOrdering::Relaxed,DeviceAtomicU32}, device, thread, warp};

#[derive(Clone,Copy)] #[repr(C)]
pub struct AsIndex {
    pub root:*const u32, pub n:u32, pub max_group:u32, pub kind:u32,
    pub off:*const u32, pub cols:*const u32, pub decode:*const u32, pub domain:u32,
    // Optional hybrid bitmap index of the same sorted resident ranges. Dense
    // ranges have `words` bitmap entries; sparse ranges retain the ordinary CSR.
    pub bit_row:*const u32, pub bits:*mut u32, pub words:u32,
}
#[derive(Clone,Copy)] #[repr(C)]
pub struct AsWork {
    pub groups:*mut u32, pub cursor:*mut u32, pub edges:*mut u32, pub parents:*mut u32,
    pub desc:*mut u64, pub partial:*mut u32, pub tiles:*mut u32, pub tile_cap:u32,
    pub output:*mut u64, pub cap:u32, pub meta:*mut u32,
    pub sorted:*const u64, pub rep:*mut u32, pub sigtable:*mut u32, pub sigmask:u32,
}
#[device] fn as_tid()->(u32,u32){(thread::blockIdx_x()*256+thread::threadIdx_x(),thread::gridDim_x()*256)}
#[device] unsafe fn as_add(p:*mut u32,n:u32)->u32{unsafe{DeviceAtomicU32::from_ptr(p)}.fetch_add(n,Relaxed)}
#[device] unsafe fn as_pack(x:AsIndex,g:u32,h:u32)->u64 {
    let v=unsafe{*x.decode.add(h as usize)};if x.kind==118{(v as u64)<<32|g as u64}else{(g as u64)<<32|v as u64}
}
#[device] unsafe fn as_out(w:AsWork,yes:bool,key:u64){unsafe{
    let mask=warp::ballot_sync(u32::MAX,yes); if mask!=0{
        let mut start=0;if warp::lane_id()==0{start=as_add(w.meta,mask.count_ones());}
        let at=warp::shuffle_u64_sync(u32::MAX,start as u64,0) as u32+(mask&warp::lanemask_lt()).count_ones();
        if yes{if at<w.cap{*w.output.add(at as usize)=key;}else{DeviceAtomicU32::from_ptr(w.meta.add(3)).store(1,Relaxed);}}
    }
}}
#[device] unsafe fn as_compact(x:AsIndex,w:AsWork,g:u32,bits:u32,word:u32){unsafe{
    let lane=warp::lane_id();let mut mask=bits;
    // One reservation per warp for up to 32 nonempty words; each lane owns its word.
    let cnt=bits.count_ones();let mut sum=cnt;let mut k=1;
    while k<32{let v=warp::shuffle_u64_sync(u32::MAX,sum as u64,lane.wrapping_sub(k)&31) as u32;if lane>=k{sum+=v;}k*=2;}
    let total=warp::shuffle_u64_sync(u32::MAX,sum as u64,31) as u32;
    if total!=0{let mut base=0;if lane==0{base=as_add(w.meta,total);}let mut out=warp::shuffle_u64_sync(u32::MAX,base as u64,0) as u32+sum-cnt;
        while mask!=0{let bit=mask.trailing_zeros();if out<w.cap{*w.output.add(out as usize)=as_pack(x,g,word*32+bit);}else{DeviceAtomicU32::from_ptr(w.meta.add(3)).store(1,Relaxed);}out+=1;mask&=mask-1;}
    }
}}
#[cuda_device::cuda_module]
pub mod assign_kernels {
    use super::*;
    use cuda_device::{DynamicSharedArray,kernel,launch_bounds,launch_contract};

    #[kernel] #[launch_bounds(256)] #[launch_contract(domain=1,block=(256,1,1))]
    pub fn as_lengths(x:AsIndex,length:*mut u32){unsafe{let(mut i,stride)=as_tid();while i<x.n{let d=x.root.add(i as usize*2);let f=*d.add(usize::from(x.kind==119));let p=x.off.add(f as usize);*length.add(i as usize)=*p.add(1)-*p;i+=stride;}}}

    // Every lane emits one position from the oracle-sized (possibly >2B) virtual bag.
    // One binary search per warp finds the owner of its first position; remaining lanes
    // walk through any short roots intersecting this 32-item window.
    #[kernel] #[launch_bounds(256)] #[launch_contract(domain=1,block=(256,1,1))]
    pub fn as_raw(x:AsIndex,start:*const u32,first:u32,n:u32,out:*mut u64){unsafe{
        let(id,stride)=as_tid();let lane=warp::lane_id();let mut base=id-lane;
        while base<n{let p=first+base;let mut i=0;
            if lane==0{let(mut lo,mut hi)=(0,x.n);while lo<hi{let m=lo+(hi-lo)/2;if *start.add((m+1) as usize)<=p{lo=m+1;}else{hi=m;}}i=lo;}
            i=warp::shuffle_u64_sync(u32::MAX,i as u64,0) as u32;
            if base+lane<n{let pos=p+lane;let original=i; if *start.add((i+1) as usize)<=pos{
                let(mut lo,mut hi)=(original+1,x.n);while lo<hi{let m=lo+(hi-lo)/2;if *start.add((m+1) as usize)<=pos{lo=m+1;}else{hi=m;}}i=lo;}
                let d=x.root.add(i as usize*2);let inv=usize::from(x.kind==119);let f=*d.add(inv);let g=*d.add(1-inv);
                let child=*x.cols.add((*x.off.add(f as usize)+(pos-*start.add(i as usize))) as usize);
                *out.add((base+lane) as usize)=as_pack(x,g,child);
            }base+=stride;
        }
    }}
    #[kernel] #[launch_bounds(256)] #[launch_contract(domain=1,block=(256,1,1))]
    pub fn as_bit_index(x:AsIndex,froms:*const u32,n:u32){unsafe{
        let mut j=thread::blockIdx_x();let tid=thread::threadIdx_x();while j<n{let f=*froms.add(j as usize);let p=x.off.add(f as usize);let at=*x.bit_row.add(f as usize)*x.words;
            let mut i=*p+tid;let end=*p.add(1);while i<end{let h=*x.cols.add(i as usize);DeviceAtomicU32::from_ptr(x.bits.add((at+h/32) as usize)).fetch_or(1<<(h%32),Relaxed);i+=256;} j+=thread::gridDim_x();}
    }}
    // Skip dead edges before grouping. Descriptor generation is entirely device-side.
    #[kernel] #[launch_bounds(256)] #[launch_contract(domain=1,block=(256,1,1))]
    pub fn as_group(x:AsIndex,w:AsWork,scatter:bool){unsafe{
        let(mut i,stride)=as_tid();let inv=usize::from(x.kind==119);while i<x.n{let p=x.root.add(i as usize*2);let f=*p.add(inv);let g=*p.add(1-inv);
            let o=x.off.add(f as usize);if *o!=*o.add(1){if scatter{let j=*w.groups.add(g as usize)+as_add(w.cursor.add(g as usize),1);*w.edges.add(j as usize)=f;}
                else{as_add(w.groups.add(g as usize),1);}}i+=stride;
        }
    }}
    #[kernel] #[launch_bounds(256)] #[launch_contract(domain=1,block=(256,1,1))]
    pub fn as_keys(x:AsIndex,w:AsWork,keys:*mut u64){unsafe{
        let(mut i,stride)=as_tid();let inv=usize::from(x.kind==119);while i<x.n{let p=x.root.add(i as usize*2);let f=*p.add(inv);let g=*p.add(1-inv);
            let o=x.off.add(f as usize);*keys.add(i as usize)=if *o!=*o.add(1){as_add(w.groups.add(g as usize),1);(g as u64)<<32|f as u64}else{u64::MAX};i+=stride;}
    }}
    // Intern full sorted from-lists. Hash collision != equality: all members
    // must match before two head groups are allowed to reuse one union.
    #[kernel] #[launch_bounds(256)] #[launch_contract(domain=1,block=(256,1,1))]
    pub fn as_intern(x:AsIndex,w:AsWork){unsafe{
        let(mut g,stride)=as_tid();while g<=x.max_group{let a=*w.groups.add(g as usize);let b=*w.groups.add(g as usize+1);
            if b>a+1{let mut h=0x811c9dc5u32;let mut i=a;while i<b{h=(h^(*w.sorted.add(i as usize) as u32)).wrapping_mul(0x01000193);i+=1;}
                let mut slot=(h^(h>>16))&w.sigmask;let initial=slot;
                loop{let cell=DeviceAtomicU32::from_ptr(w.sigtable.add(slot as usize));let mut old=cell.load(Relaxed);
                    if old==0{match cell.compare_exchange(0,g+1,Relaxed,Relaxed){Ok(_)=>{*w.rep.add(g as usize)=g;break;},Err(k)=>old=k,}}
                    let other=old-1;let oa=*w.groups.add(other as usize);let ob=*w.groups.add(other as usize+1);
                    if ob-oa==b-a{let mut n=0;while n<b-a&&(*w.sorted.add((oa+n) as usize) as u32)==(*w.sorted.add((a+n) as usize) as u32){n+=1;}
                        if n==b-a{*w.rep.add(g as usize)=other;break;}}
                    slot=(slot+1)&w.sigmask;if slot==initial{*w.rep.add(g as usize)=g;DeviceAtomicU32::from_ptr(w.meta.add(3)).store(1,Relaxed);break;}
                }
            }g+=stride;}
    }}
    #[kernel] #[launch_bounds(256)] #[launch_contract(domain=1,block=(256,1,1))]
    pub fn as_tasks(x:AsIndex,w:AsWork,step:u32,reuse:bool){unsafe{
        let(mut g,stride)=as_tid();while g<=x.max_group{let a=*w.groups.add(g as usize);let b=*w.groups.add(g as usize+1);
            let k=b-a;if k!=0 {let a_group=as_add(w.meta.add(1),1);*w.parents.add(a_group as usize)=g;
                if k>1 && (!reuse||*w.rep.add(g as usize)==g) {let n=k.div_ceil(step);let j=as_add(w.meta.add(2),n);*w.desc.add(g as usize)=(j as u64)<<32 | n as u64;
                    let mut l=0;while l<n {if j+l<w.tile_cap{*w.tiles.add((j+l) as usize)=g;}else{DeviceAtomicU32::from_ptr(w.meta.add(3)).store(1,Relaxed);}l+=1;}
                }
            }g+=stride;
        }
    }}
    // Each CTA forms a partial union for <=step ranges. Non-skewed groups are
    // emitted directly; only a split parent's partials are written to global memory.
    #[kernel] #[launch_bounds(256)] #[launch_contract(domain=1,block=(256,1,1),dynamic_shared_range=(4,98304))]
    pub fn as_tile(x:AsIndex,w:AsWork,step:u32,hybrid:bool,reuse:bool){unsafe{
        let s=DynamicSharedArray::<u32,4>::get_raw().cast::<u32>();let tid=thread::threadIdx_x();let lane=warp::lane_id();
        let nt=(*w.meta.add(2)).min(w.tile_cap);let mut task=thread::blockIdx_x();
        while task<nt{let g=*w.tiles.add(task as usize);let desc=*w.desc.add(g as usize);let begin=*w.groups.add(g as usize)+(task-(desc>>32) as u32)*step;
            let end=(begin+step).min(*w.groups.add(g as usize+1));let mut k=tid;while k<x.words{*s.add(k as usize)=0;k+=256;}thread::sync_threads();
            let mut e=begin+tid/32;while e<end{let f=if reuse{*w.sorted.add(e as usize) as u32}else{*w.edges.add(e as usize)};let o=x.off.add(f as usize);let br=if hybrid{*x.bit_row.add(f as usize)}else{u32::MAX};
                if br!=u32::MAX{let mut j=lane;while j<x.words{let v=*x.bits.add((br*x.words+j) as usize);if v!=0{DeviceAtomicU32::from_ptr(s.add(j as usize)).fetch_or(v,Relaxed);}j+=32;}}
                else{let mut i=*o+lane;let high=*o.add(1);while i<high{let h=*x.cols.add(i as usize);DeviceAtomicU32::from_ptr(s.add((h/32) as usize)).fetch_or(1<<(h%32),Relaxed);i+=32;}}
                e+=8;
            }thread::sync_threads();
            let mut j=tid-lane;while j<x.words{let word=j+lane;let v=if word<x.words{*s.add(word as usize)}else{0};
                if desc as u32==1&&!reuse{as_compact(x,w,g,v,word);}else if word<x.words{*w.partial.add(task as usize*x.words as usize+word as usize)=v;}j+=256;
            }thread::sync_threads();task+=thread::gridDim_x();
        }
    }}
    // One warp/group handles the many single-range groups, also merging split CTA outputs.
    #[kernel] #[launch_bounds(256)] #[launch_contract(domain=1,block=(256,1,1))]
    pub fn as_finish(x:AsIndex,w:AsWork,reuse:bool){unsafe{
        let(id,stride)=as_tid();let lane=warp::lane_id();let mut group=id/32;let n=*w.meta.add(1);
        while group<n{let g=*w.parents.add(group as usize);let a=*w.groups.add(g as usize);let b=*w.groups.add(g as usize+1);
            if b-a==1{let f=if reuse{*w.sorted.add(a as usize) as u32}else{*w.edges.add(a as usize)};let p=x.off.add(f as usize);let mut j=*p;let end=*p.add(1);
                while j<end{let ok=j+lane<end;let key=if ok{as_pack(x,g,*x.cols.add((j+lane) as usize))}else{0};as_out(w,ok,key);j+=32;}}
            else{let owner=if reuse{*w.rep.add(g as usize)}else{g};let desc=*w.desc.add(owner as usize);if reuse||desc as u32>1{let mut j=0;while j<x.words{let word=j+lane;let mut v=0;
                if word<x.words{let mut t=0;while t<desc as u32{v|=*w.partial.add(((desc>>32) as usize+t as usize)*x.words as usize+word as usize);t+=1;}}
                as_compact(x,w,g,v,word);j+=32;}}}
            group+=stride/32;
        }
    }}
}
