#[path = "../dedup.rs"]
mod dedup;
use cuda_core::{CudaContext,DeviceBuffer};
use gpu_blocked_run::MemoryResource;
use rccl_memory::{CudaMemoryResource, PoolMemoryResource};
use std::collections::BTreeSet;
fn main()->Result<(),Box<dyn std::error::Error>>{
    let ctx=CudaContext::new(0)?;let stream=ctx.default_stream();
    let memory=MemoryResource::new(PoolMemoryResource::new(CudaMemoryResource::new(ctx.clone()),1<<20,1<<20)?);
    for n in [0usize,1,31,32,33,255,256,257,1023,1024,1025,10007]{
        let mut triples=(0..n).map(|i|(((i*37)%83) as i64,((i*19)%47) as i64,(i%25) as i64)).collect::<Vec<_>>();
        if n>0{triples[0]=((1<<26)-1,(1<<17)-1,31);}
        if n>1{triples[1]=(0,0,0);}
        if n>2{triples[2]=triples[0];}
        let orders=DeviceBuffer::from_host(&stream,&triples.iter().map(|v|v.0).collect::<Vec<_>>())?;
        let suppliers=DeviceBuffer::from_host(&stream,&triples.iter().map(|v|v.1).collect::<Vec<_>>())?;
        let nations=DeviceBuffer::from_host(&stream,&triples.iter().map(|v|v.2).collect::<Vec<_>>())?;
        let capacity=(4*n.max(1)).next_power_of_two();
        let (table,count)=dedup::dedup(&ctx,&stream,&memory,&orders,&suppliers,&nations,n,capacity)?;
        let actual=table.to_host_vec()?.into_iter().filter(|&v|v!=0).map(dedup::decode).collect::<BTreeSet<_>>();
        let expected=triples.into_iter().collect::<BTreeSet<_>>();
        assert_eq!(actual,expected);assert_eq!(count.to_host_vec()?[0] as usize,expected.len());
    }
    println!("dedup oracle: 12 boundary/random cases exact, including zero and maximum encoded triples");
    Ok(())
}
