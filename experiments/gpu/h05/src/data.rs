use std::{collections::{BTreeMap, BTreeSet, HashMap}, fs, path::Path};
use serde_json::{json, Value};
pub type Error = Box<dyn std::error::Error>;
pub struct Data { pub sf: Value, pub columns: BTreeMap<String,Vec<i64>>, pub start:i64, pub end:i64, pub asia:i64, pub answer:Vec<(i64,i64,i64)> }
impl Data {
    pub fn col(&self,name:&str)->&[i64] { &self.columns[name] }
    pub fn load(path:&Path)->Result<Self,Error> {
        let m:Value=serde_json::from_slice(&fs::read(path.join("manifest.json"))?)?;
        let mut columns=BTreeMap::new();
        for (relation,meta) in m["relations"].as_object().ok_or("relations missing")? {
            let rows=meta["rows"].as_u64().ok_or("row count missing")? as usize;
            if rows>u32::MAX as usize {return Err("u32 row domain exceeded".into());}
            for column in meta["columns"].as_array().ok_or("columns missing")? {
                let name=column["name"].as_str().ok_or("name missing")?;
                let bytes=fs::read(path.join(column["file"].as_str().ok_or("file missing")?))?;
                let ty=column["type"].as_str().ok_or("type missing")?;
                let width=match ty {"i64"=>8,"i32"=>4,_=>return Err("unsupported input type".into())};
                if bytes.len()!=rows*width {return Err(format!("{relation}.{name}: byte length mismatch").into());}
                let values=bytes.chunks_exact(width).map(|b|if width==8 {i64::from_le_bytes(b.try_into().unwrap())} else {i32::from_le_bytes(b.try_into().unwrap()) as i64}).collect();
                columns.insert(format!("{relation}.{name}"),values);
            }
        }
        let mut result=Self{sf:m["sf"].clone(),columns,start:m["constants"]["date_start_days"].as_i64().ok_or("start missing")?,end:m["constants"]["date_end_days"].as_i64().ok_or("end missing")?,asia:m["constants"]["asia_id"].as_i64().ok_or("asia missing")?,answer:vec![]};
        if path.join("answer.tsv").exists() {
            for line in fs::read_to_string(path.join("answer.tsv"))?.lines() {
                let fields=line.split('\t').map(str::parse::<i64>).collect::<Result<Vec<_>,_>>()?;
                if fields.len()!=3 {return Err("answer must have three columns".into());}
                result.answer.push((fields[0],fields[1],fields[2]));
            }
            result.answer.sort_unstable();result.answer.dedup();
        } else {result.answer=result.reference();}
        // Lossless u64 result encoding relies on the validated projected domains.
        for (name,bits) in [("orders.o_orderkey",26),("lineitem.l_orderkey",26),("supplier.s_suppkey",17),("lineitem.l_suppkey",17),("customer.c_nationkey",5),("supplier.s_nationkey",5),("nation.n_nationkey",5)] {
            if result.col(name).iter().any(|&k|k<0 || k>=(1i64<<bits)) {return Err(format!("{name} exceeds packed {bits}-bit domain").into());}
        }
        Ok(result)
    }
    pub fn reference(&self)->Vec<(i64,i64,i64)> {
        let zip=|a:&str,b:&str|self.col(a).iter().copied().zip(self.col(b).iter().copied()).collect::<HashMap<_,_>>();
        let c=zip("customer.c_custkey","customer.c_nationkey"); let s=zip("supplier.s_suppkey","supplier.s_nationkey");
        let n=zip("nation.n_nationkey","nation.n_regionkey"); let r=zip("region.r_regionkey","region.r_name");
        let mut o=HashMap::new();
        for ((&key,&customer),&date) in self.col("orders.o_orderkey").iter().zip(self.col("orders.o_custkey")).zip(self.col("orders.o_orderdate")) {
            if date>=self.start && date<self.end {o.insert(key,customer);}
        }
        let mut out=BTreeSet::new();
        for (&order,&supplier) in self.col("lineitem.l_orderkey").iter().zip(self.col("lineitem.l_suppkey")) {
            if let Some(customer)=o.get(&order) {if let (Some(&cn),Some(&sn))=(c.get(customer),s.get(&supplier)) {
                if cn==sn && n.get(&cn).and_then(|region|r.get(region))==Some(&self.asia) {out.insert((order,supplier,cn));}
            }}
        }
        out.into_iter().collect()
    }
}
pub fn synthetic(path:&Path)->Result<(),Error> {
    fs::create_dir_all(path)?;
    let mut relations=serde_json::Map::new();
    let mut emit=|relation:&str,cols:Vec<(&str,&str,Vec<i64>)>|->Result<(),Error>{
        let rows=cols[0].2.len();let mut columns=vec![];
        for (name,ty,values) in cols {let file=format!("{relation}.{name}.bin");let mut bytes=vec![];
            for value in values {if ty=="i64"{bytes.extend(value.to_le_bytes())}else{bytes.extend((value as i32).to_le_bytes())}}
            fs::write(path.join(&file),bytes)?;columns.push(json!({"name":name,"type":ty,"file":file}));
        }
        relations.insert(relation.into(),json!({"rows":rows,"columns":columns}));Ok(())
    };
    emit("region",vec![("r_regionkey","i32",(0..5).collect()),("r_name","i32",(0..5).collect())])?;
    emit("nation",vec![("n_nationkey","i32",(0..25).collect()),("n_regionkey","i32",(0..25).map(|n|n%5).collect())])?;
    emit("customer",vec![("c_custkey","i64",(1..=2051).collect()),("c_nationkey","i32",(1..=2051).map(|n|n%25).collect())])?;
    emit("supplier",vec![("s_suppkey","i64",(1..=257).collect()),("s_nationkey","i32",(1..=257).map(|n|n%25).collect())])?;
    emit("orders",vec![("o_orderkey","i64",(1..=12001).collect()),("o_custkey","i64",(1..=12001).map(|n|1+(n*37)%2051).collect()),("o_orderdate","i32",(1..=12001).map(|n|8765+n%368).collect())])?;
    emit("lineitem",vec![("l_orderkey","i64",(0..72019).map(|n|1+(n*17)%12001).collect()),("l_suppkey","i64",(0..72019).map(|n|1+(n*31)%257).collect())])?;
    fs::write(path.join("region.r_name.dict.txt"),"0\tAFRICA\n1\tAMERICA\n2\tASIA\n3\tEUROPE\n4\tMIDDLE EAST\n")?;
    fs::write(path.join("manifest.json"),serde_json::to_vec_pretty(&json!({"sf":"synthetic","relations":relations,"constants":{"date_start_days":8766,"date_end_days":9131,"asia_id":2}}))?)?;
    let data=Data::load(path)?;let answer=data.reference();
    let text=answer.iter().map(|(o,s,n)|format!("{o}\t{s}\t{n}\n")).collect::<String>();
    fs::write(path.join("answer.tsv"),text)?;fs::write(path.join("answer.count"),format!("{}\n",answer.len()))?;
    Ok(())
}
