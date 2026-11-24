//! Replay adapters only: production readers and layouts are deliberately unchanged.
use gpu_datalog_runtime::{Word, radix_run::RadixRunView, packed_run::PackedView};

#[cuda_host::cuda_module]
pub mod kernels {
    use super::*;
    use gpu_datalog_runtime::{radix_run::device as engine, packed_run::{device as packed, read_support::checksum}};
    use cuda_device::{kernel, launch_bounds, launch_contract, thread};

    // Each job is one recorded run probe or one actually enumerated child.
    // Probe/child order in the input is defined by the host, never called hardware issue order.
    #[kernel]
    #[launch_bounds(256)]
    #[launch_contract(domain = 1, block = (256, 1, 1))]
    pub fn replay<W: Word, const MODE: u32>(plain: *const RadixRunView<W,u64>, packed_views: *const PackedView<W>,
        low: *const W, high: *const W, jobs: *const [u64;3], count: u64, output: *mut [u64;3]) {
        let mut index = thread::index_1d().get() as u64;
        while index < count {
            unsafe {
                let job = *jobs.add(index as usize);
                let mut result = [0u64;3];
                if job[0] != u64::MAX {
                    if job[1] == 0 {
                        let lo = *low.add(index as usize);
                        let hi = *high.add(index as usize);
                        let (a,b) = if MODE == 0 { engine::interval(*plain.add(job[0] as usize),lo,hi) }
                            else if MODE == 1 { packed::read_range::<W,true>(*packed_views.add(job[0] as usize),lo,hi) }
                            else { packed::read_range::<W,false>(*packed_views.add(job[0] as usize),lo,hi) };
                        result = [a,b,(a != b) as u64];
                    } else {
                        let sum = if MODE == 0 { checksum(*(*plain.add(job[0] as usize)).keys.add(job[2] as usize), W::BITS) }
                            else if MODE == 1 { checksum(*(*packed_views.add(job[0] as usize)).plain.add(job[2] as usize), W::BITS) }
                            else { gpu_datalog_runtime::packed_run::read_support::checksum_row(*packed_views.add(job[0] as usize), job[2], W::BITS) };
                        result = [job[2],job[2]+1,sum];
                    }
                }
                *output.add(index as usize) = result;
            }
            index += thread::gridDim_x() as u64 * 256;
        }
    }

    #[kernel]
    #[launch_bounds(256)]
    #[launch_contract(domain = 1, block = (256, 1, 1))]
    pub fn legacy(columns: *const gpu_device_columns::ColumnView, payload_columns:u32,
        domain:u32, base:i64, rows:u32, fields:u32, mapping:[u32;4], membership:bool,
        keys:*const [u64;4], jobs:*const [u64;2], count:u64, output:*mut [u64;3]) {
        let mut index=thread::index_1d().get() as u64;
        while index<count {
            unsafe {
                let job=*jobs.add(index as usize);
                let mut result=[0u64;3];
                if job[0]!=0 {
                    let mut sum=0u64;let mut col=0;
                    while col<payload_columns {
                        let value=gpu_device_columns::decode(*columns.add(1+col as usize),job[1]);
                        sum=sum.wrapping_add((value as u64).wrapping_mul(0x9e3779b97f4a7c15u64.wrapping_add(col as u64*2)));
                        col+=1;
                    }
                    result=[job[1],job[1]+1,sum];
                } else {
                    let values=*keys.add(index as usize);
                    let (mut a,mut b)=(0u64,rows as u64);
                    let mut found=false;
                    if fields!=0 {
                        let key=(values[0] as i64).wrapping_sub(base);
                        if key<0||key>=domain as i64 {a=0;b=0;}
                        else {a=gpu_device_columns::decode(*columns,key as u64) as u64;b=gpu_device_columns::decode(*columns,key as u64+1) as u64;}
                        let mut field=1;
                        while field<fields {
                            let column=*columns.add(mapping[field as usize] as usize);
                            let key=values[field as usize] as i64;
                            let mut left=a;let mut right=b;
                            while left<right {let mid=left+(right-left)/2;if gpu_device_columns::decode(column,mid)<key{left=mid+1;}else{right=mid;}}
                            a=left;
                            if membership && field+1==fields {
                                found=a<b && gpu_device_columns::decode(column,a)==key;
                            } else {
                                right=b;
                                while left<right{let mid=left+(right-left)/2;if gpu_device_columns::decode(column,mid)<=key{left=mid+1;}else{right=mid;}}
                                b=left;
                            }
                            field+=1;
                        }
                    }
                    if !membership||fields<=1{found=a<b;}
                    result=if membership{[0,0,found as u64]}else{[a,b,found as u64]};
                }
                *output.add(index as usize)=result;
            }
            index+=thread::gridDim_x() as u64*256;
        }
    }

    #[kernel]
    #[launch_bounds(256)]
    #[launch_contract(domain = 1, block = (256, 1, 1))]
    pub fn whole(source:*const [u64;4], columns:u32, rows:u64, jobs:*const [u64;2], count:u64, output:*mut [u64;3]) {
        let mut index=thread::index_1d().get() as u64;
        while index<count {
            unsafe {
                let job=*jobs.add(index as usize);
                let result=if job[0]==0{[0,rows,(rows!=0) as u64]}else{
                    let row=*source.add(job[1] as usize);let mut sum=0u64;let mut col=0;
                    while col<columns{sum=sum.wrapping_add(row[col as usize].wrapping_mul(0x9e3779b97f4a7c15u64.wrapping_add(col as u64*2)));col+=1;}
                    [job[1],job[1]+1,sum]
                };
                *output.add(index as usize)=result;
            }
            index+=thread::gridDim_x() as u64*256;
        }
    }
}
