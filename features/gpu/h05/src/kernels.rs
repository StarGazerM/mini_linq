use cuda_device::{device, kernel, launch_bounds, launch_contract, thread, DynamicSharedArray};
use cuda_device::atomic::{AtomicOrdering, DeviceAtomicU32};
use gpu_blocked_run::{point, RankView};

#[derive(Clone, Copy)]
#[repr(C)]
pub struct Dimensions {
    pub orders: RankView, pub customer: RankView, pub supplier: RankView,
    pub order_customer: *const i64, pub customer_nation: *const i64,
    pub supplier_nation: *const i64, pub nation_region: *const i64,
    pub nation_len: u32, pub asia_regions: u64,
    pub order_keys: *const i64, pub order_fences: *const i64,
    pub order_rows: u32, pub order_dates: *const i64,
    pub start: i64, pub end: i64,
}
#[derive(Clone, Copy)]
#[repr(C)]
pub struct Output { pub orders: *mut i64, pub suppliers: *mut i64, pub nations: *mut i64, pub counters: *mut u32 }

#[device]
#[inline(always)]
unsafe fn atomic_add(ptr: *mut u32, value: u32) -> u32 {
    (&*(ptr as *const DeviceAtomicU32)).fetch_add(value, AtomicOrdering::Relaxed)
}
#[device]
#[inline(always)]
unsafe fn asia(d: Dimensions, nation: i64) -> bool {
    if nation < 0 || nation >= d.nation_len as i64 { return false; }
    let region = *d.nation_region.add(nation as usize);
    region >= 0 && region < 64 && d.asia_regions & (1u64 << region as u32) != 0
}

// One reservation for the whole 1024-item tile. Shared row IDs make the
// output stores contiguous even when the predicates leave sparse lanes.
#[device]
#[inline(always)]
unsafe fn store(rows: [u32; 4], nations: [i32; 4], valid: [bool; 4], keys: *const i64,
                suppliers: *const i64, output: Output, scratch: *mut u32) {
    let tid = thread::threadIdx_x() as usize;
    let count = valid[0] as u32 + valid[1] as u32 + valid[2] as u32 + valid[3] as u32;
    let (mut pos, total) = gpu_tiles::scan(count, scratch);
    let packed_rows = scratch.add(256);
    let packed_nations = packed_rows.add(1024);
    let mut j = 0;
    #[unroll]
    while j < 4 {
        if valid[j] {
            *packed_rows.add(pos as usize) = rows[j];
            *packed_nations.add(pos as usize) = nations[j] as u32;
            pos += 1;
        }
        j += 1;
    }
    thread::sync_threads();
    if tid == 0 { *scratch.add(255) = if total > 0 { atomic_add(output.counters, total) } else { 0 }; }
    thread::sync_threads();
    let base = *scratch.add(255) as usize;
    let mut k = tid;
    while k < total as usize {
        let row = *packed_rows.add(k) as usize;
        *output.orders.add(base+k) = *keys.add(row);
        *output.suppliers.add(base+k) = *suppliers.add(row);
        *output.nations.add(base+k) = *packed_nations.add(k) as i64;
        k += 256;
    }
    thread::sync_threads();
}

// At most K blocks are staged per clustered tile; wider spans use rank probes.
pub const MERGE_BLOCK_LIMIT: u32 = 4;

#[device]
unsafe fn fence_block(d: Dimensions, key: u32) -> u32 {
    let mut lo = 0;
    let mut hi = (d.order_rows + 2047) / 2048;
    while lo < hi {
        let mid = (lo + hi) / 2;
        if *d.order_fences.add(mid as usize) <= key as i64 { lo = mid + 1; }
        else { hi = mid; }
    }
    lo.saturating_sub(1)
}

#[device]
unsafe fn plan_a_impl<const MERGE: bool>(orderkeys: &[i64], suppkeys: &[i64], d: Dimensions, output: Output) {
    let tid = thread::threadIdx_x() as usize;
    let scratch = DynamicSharedArray::<u8>::get_raw() as *mut u32;
    let tile_keys = scratch.add(256);
    let matches = tile_keys.add(1024);
    let index = matches.add(1024);
    loop {
        if tid == 0 { *scratch.add(254) = atomic_add(output.counters.add(1), 1); }
        thread::sync_threads();
        let tile = *scratch.add(254) as usize * 1024;
        if tile >= orderkeys.len() { break; }
        let n = (orderkeys.len() - tile).min(1024) as u32;
        let mut merged = false;
        if MERGE {
            let mut p = tid;
            while p < 1024 {
                *tile_keys.add(p) = if p < n as usize { orderkeys[tile+p] as u32 } else { u32::MAX };
                *matches.add(p) = u32::MAX;
                p += 256;
            }
            thread::sync_threads();
            gpu_tiles::sort(tile_keys,1024);
            let first = fence_block(d,*tile_keys);
            let last = fence_block(d,*tile_keys.add(n as usize-1));
            merged = d.order_rows > 0 && last-first+1 <= MERGE_BLOCK_LIMIT;
            if merged {
                let mut block = first;
                while block <= last {
                    let base = block*2048;
                    let len = (d.order_rows-base).min(2048);
                    p = tid;
                    while p < len as usize {
                        *index.add(p) = *d.order_keys.add(base as usize+p) as u32;
                        p += 256;
                    }
                    thread::sync_threads();
                    // Stable A-before-B merge preserves every repeated probe.
                    let work = (n+len+255)/256;
                    let begin = (tid as u32*work).min(n+len);
                    let end = (begin+work).min(n+len);
                    let (mut a,mut b) = gpu_tiles::merge_path(tile_keys,n,index,len,begin);
                    let mut diagonal = begin;
                    while diagonal < end {
                        if a < n && (b == len || *tile_keys.add(a as usize) <= *index.add(b as usize)) {
                            if b < len && *tile_keys.add(a as usize) == *index.add(b as usize) {
                                *matches.add(a as usize) = base+b;
                            }
                            a += 1;
                        } else { b += 1; }
                        diagonal += 1;
                    }
                    thread::sync_threads();
                    block += 1;
                }
            }
        }
        let mut rows = [0u32;4]; let mut nations = [0i32;4]; let mut valid = [false;4];
        let mut j=0;
        #[unroll]
        while j<4 {
            let row = tile+tid+j*256;
            rows[j] = row as u32;
            if row < orderkeys.len() {
                let key = orderkeys[row];
                let o = if MERGE && merged {
                    let mut lo=0; let mut hi=n;
                    while lo<hi {
                        let mid=(lo+hi)/2;
                        if *tile_keys.add(mid as usize) < key as u32 {lo=mid+1;} else {hi=mid;}
                    }
                    *matches.add(lo as usize)
                } else {
                    match point(d.orders,key) {Some((o,_))=>o,None=>u32::MAX}
                };
                if o != u32::MAX && (d.order_dates.is_null() ||
                    (*d.order_dates.add(o as usize)>=d.start && *d.order_dates.add(o as usize)<d.end)) {
                    let customer=*d.order_customer.add(o as usize);
                    if let Some((c,_))=point(d.customer,customer) {
                        let nation=*d.customer_nation.add(c as usize);
                        if asia(d,nation) {
                            if let Some((s,_))=point(d.supplier,suppkeys[row]) {
                                valid[j]=*d.supplier_nation.add(s as usize)==nation;
                                nations[j]=nation as i32;
                            }
                        }
                    }
                }
            }
            j+=1;
        }
        thread::sync_threads();
        store(rows,nations,valid,orderkeys.as_ptr(),suppkeys.as_ptr(),output,scratch);
    }
}

#[cuda_device::cuda_module]
pub mod kernels {
    use super::*;
    #[kernel]
    #[launch_bounds(256)]
    #[launch_contract(domain = 1, block = (256, 1, 1), dynamic_shared_range = (64, 64))]
    pub fn filter_orders(keys: &[i64], customers: &[i64], dates: &[i64], start: i64, end: i64,
                         filtered_keys: *mut i64, filtered_customers: *mut i64, blocks: *mut u32, scatter: bool) {
        let i = thread::index_1d().get();
        let tid = thread::threadIdx_x();
        let valid = i < keys.len() && dates[i] >= start && dates[i] < end;
        let scratch = DynamicSharedArray::<u8>::get_raw() as *mut u32;
        let (offset,total) = gpu_tiles::scan(valid as u32,scratch);
        unsafe {
            if !scatter {
                if tid == 0 { *blocks.add(thread::blockIdx_x() as usize) = total; }
                return;
            }
            if valid {
                let out = (*blocks.add(thread::blockIdx_x() as usize)+offset) as usize;
                *filtered_keys.add(out) = keys[i];
                *filtered_customers.add(out) = customers[i];
            }
        }
    }
    #[kernel]
    #[launch_bounds(256)]
    #[launch_contract(domain = 1, block = (256, 1, 1), dynamic_shared_range = (9216, 9216))]
    pub fn plan_a(orderkeys: &[i64], suppkeys: &[i64], d: Dimensions, output: Output) {
        unsafe { plan_a_impl::<false>(orderkeys, suppkeys, d, output); }
    }

    #[kernel]
    #[launch_bounds(256)]
    #[launch_contract(domain = 1, block = (256, 1, 1), dynamic_shared_range = (17408, 17408))]
    pub fn plan_a_merge(orderkeys: &[i64], suppkeys: &[i64], d: Dimensions, output: Output) {
        unsafe { plan_a_impl::<true>(orderkeys, suppkeys, d, output); }
    }

    // Depth-first virtual expansion. Each stage retains only 1024 descriptors,
    // irrespective of fanout. There is deliberately no cross-block work stealing.
    #[kernel]
    #[launch_bounds(256)]
    #[launch_contract(domain = 1, block = (256, 1, 1), dynamic_shared_range = (33808, 33808))]
    pub fn plan_b(customer_keys: &[i64], customer_nations: &[i64], d: Dimensions,
                  orders_by_customer: RankView, order_keys: *const i64,
                  lines_by_order: RankView, line_keys: *const i64, line_suppliers: *const i64,
                  output: Output) {
        let tid = thread::threadIdx_x() as usize;
        let scratch = DynamicSharedArray::<u8>::get_raw() as *mut u32;
        unsafe {
            let begins1 = scratch.add(2304);
            let offsets1 = begins1.add(1024);
            let nations1 = offsets1.add(1025);
            let begins2 = nations1.add(1024);
            let offsets2 = begins2.add(1024);
            let nations2 = offsets2.add(1025);
            loop {
                if tid == 0 { *scratch.add(254) = atomic_add(output.counters.add(1),1); }
                thread::sync_threads();
                let tile = *scratch.add(254) as usize * 1024;
                if tile >= customer_keys.len() { break; }
                let mut sum1=0u32;
                let mut stripe=0;
                while stripe<4 {
                    let p=tid+stripe*256; let row=tile+p;
                    let mut begin=0; let mut count=0; let mut nation=0;
                    if row<customer_keys.len() {
                        nation=customer_nations[row] as u32;
                        if asia(d,nation as i64) { if let Some((a,b))=point(orders_by_customer,customer_keys[row]) { begin=a;count=b-a; } }
                    }
                    *begins1.add(p)=begin; *nations1.add(p)=nation;
                    let (offset,total)=gpu_tiles::scan(count,scratch);
                    *offsets1.add(p)=sum1+offset;
                    sum1+=total; stripe+=1;
                }
                if tid==0 { *offsets1.add(1024)=sum1; (&*(output.counters.add(2) as *const DeviceAtomicU32)).fetch_max(sum1,AtomicOrdering::Relaxed); }
                thread::sync_threads();
                let mut order_tile=0u32;
                while order_tile<sum1 {
                    let mut sum2=0u32; stripe=0;
                    while stripe<4 {
                        let p=tid+stripe*256; let pos=order_tile+p as u32;
                        let mut begin=0;let mut count=0;let mut nation=0;
                        if pos<sum1 {
                            let parent=gpu_tiles::owner(offsets1,1024,pos) as usize;
                            let order_row=*begins1.add(parent)+pos-*offsets1.add(parent);
                            nation=*nations1.add(parent);
                            if let Some((a,b))=point(lines_by_order,*order_keys.add(order_row as usize)) { begin=a;count=b-a; }
                        }
                        *begins2.add(p)=begin; *nations2.add(p)=nation;
                        let (offset,total)=gpu_tiles::scan(count,scratch);
                        *offsets2.add(p)=sum2+offset;sum2+=total;stripe+=1;
                    }
                    if tid==0 { *offsets2.add(1024)=sum2; (&*(output.counters.add(3) as *const DeviceAtomicU32)).fetch_max(sum2,AtomicOrdering::Relaxed); }
                    thread::sync_threads();
                    let mut line_tile=0u32;
                    while line_tile<sum2 {
                        let mut rows=[0u32;4];let mut nations=[0i32;4];let mut valid=[false;4];
                        let mut j=0;
                        #[unroll]
                        while j<4 {
                            let pos=line_tile+tid as u32+j as u32*256;
                            if pos<sum2 {
                                let parent=gpu_tiles::owner(offsets2,1024,pos) as usize;
                                let row=*begins2.add(parent)+pos-*offsets2.add(parent);
                                let nation=*nations2.add(parent);
                                rows[j]=row;nations[j]=nation as i32;
                                if let Some((s,_))=point(d.supplier,*line_suppliers.add(row as usize)) {valid[j]=*d.supplier_nation.add(s as usize)==nation as i64;}
                            }
                            j+=1;
                        }
                        store(rows,nations,valid,line_keys,line_suppliers,output,scratch);
                        line_tile+=1024;
                    }
                    thread::sync_threads();
                    order_tile+=1024;
                }
                thread::sync_threads();
            }
        }
    }
}
