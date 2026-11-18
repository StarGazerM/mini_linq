//! Unchanged legacy directory/column reads included in all three configurations.
use super::*;
use gpu_device_columns::{ColumnView,VectorHeader};
#[derive(Deserialize)]
pub(super) struct Legacy { pub launch:usize, pub site:u64, domain:u32, base:i64, rows:u32, columns:Vec<Column> }
#[derive(Deserialize)]
struct Column { name:String, len:usize, codec:u32 }
struct OwnedColumn { headers:Buffer<VectorHeader>, data:Buffer<u64>, host_headers:Vec<VectorHeader>, host_data:Vec<u64>, len:usize, codec:u32 }
impl OwnedColumn {
    fn view(&self)->ColumnView{ColumnView{headers:if self.host_headers.is_empty(){std::ptr::null()}else{self.headers.ptr()},data:self.data.ptr(),len:self.len as u64,count:std::ptr::null(),codec:self.codec}}
    fn value(&self,row:usize)->i64 {unsafe{gpu_device_columns::decode(ColumnView{headers:if self.host_headers.is_empty(){std::ptr::null()}else{self.host_headers.as_ptr()},data:self.host_data.as_ptr(),len:self.len as u64,count:std::ptr::null(),codec:self.codec},row as u64)}}
}
pub(super) fn replay(root:&Path,pool:&Arc<Pool>,metadata:&Legacy,records:Vec<[u64;16]>,site:&Value,launch:&Launch,repeats:usize)->Result<Value>{
    let mut columns=Vec::new();
    for spec in &metadata.columns {
        let raw=fs::read(root.join(format!("{}.headers",spec.name)))?;
        if raw.len()%32!=0{return Err("partial legacy header".into());}
        let mut host_headers=Vec::new();
        for h in raw.chunks_exact(32){host_headers.push(VectorHeader{offset:u64::from_le_bytes(h[0..8].try_into()?),base:i64::from_le_bytes(h[8..16].try_into()?),width:u32::from_le_bytes(h[16..20].try_into()?),codec:u32::from_le_bytes(h[20..24].try_into()?),rows:u32::from_le_bytes(h[24..28].try_into()?),padding:0});}
        let raw=fs::read(root.join(format!("{}.data",spec.name)))?;
        if raw.len()%8!=0{return Err("partial legacy payload".into());}
        let host_data=raw.chunks_exact(8).map(|b|u64::from_le_bytes(b.try_into().unwrap())).collect::<Vec<_>>();
        columns.push(OwnedColumn{headers:Buffer::from_host(pool.clone(),&host_headers)?,data:Buffer::from_host(pool.clone(),&host_data)?,host_headers,host_data,len:spec.len,codec:spec.codec});
    }
    let order=site["order"].as_array().ok_or("legacy order")?;
    let payload=site["index_columns"].as_array().ok_or("legacy payload")?;
    let count=site["keys"].as_array().ok_or("legacy keys")?.len();
    let mut mapping=[0u32;4];
    for i in 1..count{mapping[i]=1+payload.iter().position(|p|p==&order[i]).ok_or("legacy bound column")? as u32;}
    let member=site["membership"]==true;
    let mut rows=records;if site["depth"]==0{rows.sort_by_key(|r|if r[0]>>63!=0{(1,0)}else{(0,r[1])});}
    let mut keys=Vec::new();let mut jobs=Vec::new();let mut expected=Vec::new();let mut counts=[0u64;5];let mut scans=0;
    let mut kinds=Vec::new();
    for r in &rows{
        keys.push([r[8],r[9],r[10],r[11]]);
        let scan=r[0]>>63!=0;jobs.push([scan as u64,r[1]]);
        if scan {
            if r[1]>=metadata.rows as u64{return Err("legacy child outside source".into());}
            let sum=columns.iter().skip(1).enumerate().fold(0u64,|sum,(i,c)|sum.wrapping_add((c.value(r[1] as usize) as u64).wrapping_mul(0x9e3779b97f4a7c15u64.wrapping_add(i as u64*2))));
            expected.push([r[1],r[1]+1,sum]);scans+=1;
            let probe=rows.iter().find(|p|p[0]>>63==0&&p[6]<=r[1]&&r[1]<p[7]).ok_or("legacy child without range")?;
            kinds.push(if probe[7]-probe[6]==1{2}else if probe[7]-probe[6]<=32{3}else{4});
        } else {
            let (mut a,mut b)=(0usize,metadata.rows as usize);
            if count>0 {
                let key=(r[8] as i64).wrapping_sub(metadata.base);
                if key<0||key>=metadata.domain as i64{a=0;b=0;}else{a=columns[0].value(key as usize) as usize;b=columns[0].value(key as usize+1) as usize;}
                for i in 1..count{let values=&columns[mapping[i] as usize];let key=r[8+i] as i64;let end=b;
                    let mut left=a;let mut right=end;
                    while left<right{let mid=left+(right-left)/2;if values.value(mid)<key{left=mid+1;}else{right=mid;}}a=left;
                    right=end;while left<right{let mid=left+(right-left)/2;if values.value(mid)<=key{left=mid+1;}else{right=mid;}}b=left;
                }
            }
            if [a as u64,b as u64]!=[r[6],r[7]]{return Err(format!("legacy host range mismatch launch={} site={}",launch.seq,site["site"]).into());}
            let kind=if a==b{if member{0}else{1}}else if b-a==1{2}else if b-a<=32{3}else{4};counts[kind]+=1;kinds.push(kind);
            expected.push(if member{[0,0,(a!=b) as u64]}else{[a as u64,b as u64,(a!=b) as u64]});
        }
    }
    if scans!=launch.scan_rows[metadata.site as usize]{return Err("legacy scan count mismatch".into());}
    let views=Buffer::from_host(pool.clone(),&columns.iter().map(OwnedColumn::view).collect::<Vec<_>>())?;
    let module=unsafe{device::kernels::load(pool.context())?};let clock=Clock::new(pool)?;
    let execute=|keys:&[[u64;4]],jobs:&[[u64;2]],expected:&[[u64;3]]|->Result<([u64;3],[u64;3])>{
        let len=jobs.len();let keys=Buffer::from_host(pool.clone(),keys)?;let jobs=Buffer::from_host(pool.clone(),jobs)?;
        let output=unsafe{Buffer::<[u64;3]>::uninitialized(pool.clone(),len)?};
        let cfg=module.prepare_legacy(LaunchConfig1D::new(len.div_ceil(256).max(1).min(65535) as u32,256,0))?;
        let call=||->Result<()>{module.legacy(pool.stream(),&cfg,views.ptr(),payload.len() as u32,metadata.domain,metadata.base,metadata.rows,count as u32,mapping,member,keys.ptr(),jobs.ptr(),len as u64,output.ptr())?;Ok(())};
        call()?;if output.to_host()?!=expected{return Err("legacy device oracle mismatch".into());}
        let mut times=[0;3];let mut gpu=[0;3];
        for repeat in 0..repeats{for offset in 0..3{let mode=(launch.seq+repeat+offset)%3;let(w,g)=clock.time(pool,call)?;times[mode]+=w;gpu[mode]+=g;}}
        Ok((times,gpu))
    };
    let(times,gpu)=execute(&keys,&jobs,&expected)?;let mut by_kind=serde_json::Map::new();
    for(kind,name)in ["membership_miss","range_miss","one_row_hit","short_range","long_range"].iter().enumerate(){
        let selected=kinds.iter().filter(|&&k|k==kind).count();if selected==0{continue;}
        let(w,g)=if selected==jobs.len(){(times,gpu)}else{
            let mut kk=Vec::with_capacity(selected);let mut jj=Vec::with_capacity(selected);let mut ee=Vec::with_capacity(selected);
            for(i,&k)in kinds.iter().enumerate(){if k==kind{kk.push(keys[i]);jj.push(jobs[i]);ee.push(expected[i]);}}
            execute(&kk,&jj,&ee)?
        };by_kind.insert((*name).into(),json!({"ns":w,"gpu_ns":g,"probes":counts[kind],"enumerated_rows":selected as u64-counts[kind]}));
    }
    let bytes=columns.iter().map(|c|c.headers.allocated_bytes()+c.data.allocated_bytes()).sum::<usize>();
    Ok(json!({"launch":launch.seq,"kernel":launch.kernel,"rule":launch.rule,"round":launch.round,"site":site,"snapshot":0,"counts":{"membership_miss":counts[0],"range_miss":counts[1],"one_row_hit":counts[2],"short_range":counts[3],"long_range":counts[4]},"probes":counts.iter().sum::<u64>(),"enumerated_rows":scans,"packed_probes":0,"state_bytes":bytes,"representation_bytes":[bytes,bytes,bytes],"order":if site["depth"]==0{"root_legacy"}else{"nested_completion"},"ns":times,"gpu_ns":gpu,"stratified_kind_diagnostics":by_kind,"repeats":repeats,"verified_results":rows.len(),"run_domain_negative_opportunity":0,"block_domain_negative_opportunity":0}))
}

pub(super) fn whole(pool:&Arc<Pool>,mut records:Vec<[u64;16]>,site:&Value,launch:&Launch,repeats:usize)->Result<Value>{
    let row_count=records.iter().filter(|r|r[0]>>63==0).map(|r|r[7]).max().ok_or("whole-source probes absent")?;
    let mut source=vec![None;row_count as usize];let mut columns=None;let mut jobs=Vec::new();let mut expected=Vec::new();let mut scans=0u64;let mut probes=0u64;
    if site["depth"]==0{records.sort_by_key(|r|if r[0]>>63!=0{(1,0)}else{(0,r[1])});}
    for r in &records {
        if r[0]>>63==0 {
            if r[6]!=0||r[7]!=row_count{return Err("inconsistent whole-source bounds".into());}
            jobs.push([0,0]);expected.push([0,row_count,(row_count!=0) as u64]);probes+=1;
        } else {
            if r[15]>4||r[1]>=row_count{return Err("invalid whole-source child record".into());}
            if columns.is_some_and(|c|c!=r[15]){return Err("whole-source column count changed".into());}columns=Some(r[15]);
            let row=[r[8],r[9],r[10],r[11]];
            if source[r[1] as usize].is_some_and(|old|old!=row){return Err("whole-source row changed during launch".into());}
            source[r[1] as usize]=Some(row);
            let sum=row.iter().take(r[15] as usize).enumerate().fold(0u64,|s,(i,&v)|s.wrapping_add(v.wrapping_mul(0x9e3779b97f4a7c15u64.wrapping_add(i as u64*2))));
            jobs.push([1,r[1]]);expected.push([r[1],r[1]+1,sum]);scans+=1;
        }
    }
    if scans!=launch.scan_rows[site["site"].as_u64().ok_or("whole site id")? as usize]{return Err("whole-source scan count mismatch".into());}
    // Unobserved rows are never read: retain their positions, not invented values.
    let source=Buffer::from_host(pool.clone(),&source.into_iter().map(|r|r.unwrap_or([0;4])).collect::<Vec<_>>())?;
    let jobs=Buffer::from_host(pool.clone(),&jobs)?;let output=unsafe{Buffer::<[u64;3]>::uninitialized(pool.clone(),records.len())?};
    let module=unsafe{device::kernels::load(pool.context())?};let cfg=module.prepare_whole(LaunchConfig1D::new(records.len().div_ceil(256).max(1).min(65535) as u32,256,0))?;
    let call=||->Result<()>{module.whole(pool.stream(),&cfg,source.ptr(),columns.unwrap_or(0) as u32,row_count,jobs.ptr(),records.len() as u64,output.ptr())?;Ok(())};
    call()?;if output.to_host()?!=expected{return Err("whole-source device checksum mismatch".into());}
    let clock=Clock::new(pool)?;let mut times=[0u64;3];let mut gpu=[0u64;3];for repeat in 0..repeats{for offset in 0..3{let mode=(launch.seq+repeat+offset)%3;let(w,g)=clock.time(pool,call)?;times[mode]+=w;gpu[mode]+=g;}}
    let mut counts=[0;5];counts[if row_count==0{1}else if row_count==1{2}else if row_count<=32{3}else{4}]=probes;
    let kind=["membership_miss","range_miss","one_row_hit","short_range","long_range"][if row_count==0{1}else if row_count==1{2}else if row_count<=32{3}else{4}];
    let by_kind=json!({kind:{"ns":times,"gpu_ns":gpu,"probes":probes,"enumerated_rows":scans}});
    let bytes=source.allocated_bytes();
    Ok(json!({"launch":launch.seq,"kernel":launch.kernel,"rule":launch.rule,"round":launch.round,"site":site,"snapshot":0,"counts":{"membership_miss":counts[0],"range_miss":counts[1],"one_row_hit":counts[2],"short_range":counts[3],"long_range":counts[4]},"probes":probes,"enumerated_rows":scans,"packed_probes":0,"state_bytes":0,"representation_bytes":[bytes,bytes,bytes],"order":"root_whole_source","ns":times,"gpu_ns":gpu,"stratified_kind_diagnostics":by_kind,"repeats":repeats,"verified_results":records.len(),"run_domain_negative_opportunity":0,"block_domain_negative_opportunity":0}))
}
