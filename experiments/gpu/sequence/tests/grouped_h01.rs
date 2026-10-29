//! Exact grouped H01 over the exported SF1/SF10 databases, with an independent
//! DuckDB DISTINCT-valuation oracle computed in the test (never a saved answer).
use cq_gpu_runtime::{Codec, DeviceColumns, Dictionary, Pool};
use cq_gpu_sequence_runtime::Result;
use std::{
    io::{BufRead, BufReader, Write},
    path::Path,
    process::{Command, Stdio},
    time::Instant,
};

type Line = (u32, u32, u32, u32, i64, i64, i32);
type Answer = (u32, u32, i64, i64, usize);

fn duckdb(database: &Path, sql: &str) -> Result<std::process::Child> {
    let mut child = Command::new(std::env::var("DUCKDB").unwrap_or_else(|_| "duckdb".into()))
        .args(["-batch", "-bail", "-csv", "-noheader", "-readonly"])
        .arg(database)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()?;
    child.stdin.take().unwrap().write_all(sql.as_bytes())?;
    Ok(child)
}

#[test]
fn h01_gpu_matches_duckdb_sf1() -> Result<()> {
    h01_gpu_matches_duckdb(1)
}

#[test]
fn h01_gpu_matches_duckdb_sf10() -> Result<()> {
    h01_gpu_matches_duckdb(10)
}

fn h01_gpu_matches_duckdb(scale: u32) -> Result<()> {
    let source =
        include_str!("../../../../features/workloads/src/queries/retail/tpch/retail_h01.rs");
    let sql = source
        .split("pub const SQL: &str = r#\"")
        .nth(1)
        .unwrap()
        .split("\"#;")
        .next()
        .unwrap();
    {
        let directory = std::path::PathBuf::from(format!("/data/tpch-gpu/sf{scale}"));
        let database = directory.join("dbgen.duckdb");
        if !database.is_file() {
            return Err(format!("H01 acceptance requires {}", database.display()).into());
        }
        let oracle_sql = format!(
            "SET threads=24; SELECT unicode(l_returnflag),unicode(l_linestatus),sum_quantity,sum_price,count_order FROM ({sql}) ORDER BY ALL;\n"
        );
        let start = Instant::now();
        let mut oracle = duckdb(&database, &oracle_sql)?;
        let mut expected = Vec::<Answer>::new();
        for line in BufReader::new(oracle.stdout.take().unwrap()).lines() {
            let line = line?;
            let fields = line.split(',').collect::<Vec<_>>();
            expected.push((
                fields[0].parse()?,
                fields[1].parse()?,
                fields[2].parse()?,
                fields[3].parse()?,
                fields[4].parse()?,
            ));
        }
        if !oracle.wait()?.success() {
            return Err("DuckDB H01 oracle failed".into());
        }
        let measured_duckdb_ms = start.elapsed().as_secs_f64() * 1000.0;
        // duckdb_timing.json normally records H05: reuse only a matching query.
        let recorded = std::fs::read_to_string(directory.join("duckdb_timing.json"))
            .ok()
            .and_then(|text| serde_json::from_str::<serde_json::Value>(&text).ok())
            .filter(|value| value["query"].as_str() == Some(sql))
            .and_then(|value| value["threads"]["24"]["median_seconds"].as_f64());
        let duckdb_ms = recorded.map_or(measured_duckdb_ms, |seconds| seconds * 1000.0);
        let mut loader = duckdb(
            &database,
            "SET threads=24; SELECT l_orderkey,l_linenumber,unicode(l_returnflag),unicode(l_linestatus),CAST(l_quantity*100 AS BIGINT),CAST(l_extendedprice*100 AS BIGINT),date_diff('day',DATE '1970-01-01',l_shipdate) FROM lineitem;\n",
        )?;
        let mut raw = Vec::<Line>::new();
        let mut reader = BufReader::new(loader.stdout.take().unwrap());
        let mut line = String::new();
        while reader.read_line(&mut line)? != 0 {
            let mut fields = line.trim_end().split(',');
            raw.push((
                fields.next().unwrap().parse()?,
                fields.next().unwrap().parse()?,
                fields.next().unwrap().parse()?,
                fields.next().unwrap().parse()?,
                fields.next().unwrap().parse()?,
                fields.next().unwrap().parse()?,
                fields.next().unwrap().parse()?,
            ));
            line.clear();
        }
        if !loader.wait()?.success() {
            return Err("DuckDB H01 source export failed".into());
        }
        let pool = Pool::new(0, 0, 0)?;
        let data =
            DeviceColumns::from_rows(pool, &raw, &mut Dictionary::default(), &[Codec::For; 7])?;
        drop(raw);
        let start = Instant::now();
        let result = cq_gpu_sequence_test_macros::sequence! {
            physical {
                relation L(orderkey:u32,linenumber:u32,returnflag:u32,linestatus:u32,quantity:i64,price:i64,shipdate:i32)=(&data);
                l=pipe rename L@0 {orderkey -> orderkey,linenumber -> linenumber,returnflag -> returnflag,linestatus -> linestatus,quantity -> quantity,price -> price,shipdate -> shipdate};
                filtered=pipe canon select l if (shipdate <= 10471);
                grouped=let group filtered by [returnflag,linestatus] {sum_quantity:i64=sum(quantity);sum_price:i64=sum(price);count_order:usize=count();};
                return grouped as answer(returnflag,linestatus,sum_quantity,sum_price,count_order);
            }
            structures {
                l host scan=::cq_gpu_runtime::DeviceColumns<'static,(u32,u32,u32,u32,i64,i64,i32)> [];
                filtered canon=::cq_gpu_sequence_runtime::canon::PackedCanon;
                grouped rows=::cq_gpu_sequence_runtime::group::GroupedRows<_>;
                grouped group=::std::collections::HashMap<_, (i64, i64, usize)>;
            }
            sequence {
                source=store rows l;
                feed=canon map filtered flatten tile 256 source;
                valuations=store feed;
                result=store group grouped valuations;
                output count;
            }
            graded gpu {source at host;feed at device.block;valuations at host;result at host;}
        };
        let gpu_ms = start.elapsed().as_secs_f64() * 1000.0;
        // DECODE P: publish final H01 groups for exact host comparison with DuckDB.
        let mut actual: Vec<Answer> = result.to_host()?;
        actual.sort();
        expected.sort();
        assert_eq!(actual, expected, "H01 SF{scale}");
        eprintln!(
            "H01 SF{scale}: {} exact groups, GPU wall {gpu_ms:.3} ms, DuckDB t24 {duckdb_ms:.3} ms, ratio {:.3}",
            actual.len(),
            gpu_ms / duckdb_ms
        );
    }
    Ok(())
}
