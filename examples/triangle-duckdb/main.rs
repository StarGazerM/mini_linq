//! Run one small Triangle query through the complete MiniLinq compiler and
//! compare its result with equivalent SQL over the same in-memory rows.

use duckdb::{Connection, params};
use mini_linq::mini_linq;

mini_linq! {
    pub struct TriangleProgram;
    relation R(c0: i32, c1: i32);
    relation S(c0: i32, c1: i32);
    relation T(c0: i32, c1: i32);
    triangle(x, y, z) :- R(x, y), S(y, z), T(z, x).
}

const R: &[(i32, i32)] = &[(1, 2), (1, 2), (2, 3), (8, 9)];
const S: &[(i32, i32)] = &[(2, 3), (3, 4), (9, 10)];
const T: &[(i32, i32)] = &[(3, 1), (4, 2), (10, 0)];

const TRIANGLE_SQL: &str = r#"
WITH
    R_set AS (SELECT DISTINCT x, y FROM R),
    S_set AS (SELECT DISTINCT y, z FROM S),
    T_set AS (SELECT DISTINCT z, x FROM T)
SELECT DISTINCT R_set.x, R_set.y, S_set.z
FROM R_set
JOIN S_set ON R_set.y = S_set.y
JOIN T_set ON S_set.z = T_set.z AND R_set.x = T_set.x
ORDER BY 1, 2, 3
"#;

fn main() -> duckdb::Result<()> {
    println!("Input relations:");
    print_relation("R(x, y)", R);
    print_relation("S(y, z)", S);
    print_relation("T(z, x)", T);

    let mini_linq_rows =
        TriangleProgram::run(R.iter().copied(), S.iter().copied(), T.iter().copied());
    let duckdb_rows = run_duckdb()?;

    println!("\nMiniLinq CQ:");
    println!("  triangle(x, y, z) :- R(x, y), S(y, z), T(z, x).");
    println!("MiniLinq result: {mini_linq_rows:?}");
    println!("\nDuckDB SQL:{TRIANGLE_SQL}");
    println!("DuckDB result:  {duckdb_rows:?}");

    assert_eq!(mini_linq_rows, duckdb_rows);
    println!("\nResults match exactly.");
    Ok(())
}

fn print_relation(name: &str, rows: &[(i32, i32)]) {
    println!("  {name} = {rows:?}");
}

fn run_duckdb() -> duckdb::Result<Vec<(i32, i32, i32)>> {
    let connection = Connection::open_in_memory()?;
    connection.execute_batch(
        "CREATE TABLE R (x INTEGER NOT NULL, y INTEGER NOT NULL);\
         CREATE TABLE S (y INTEGER NOT NULL, z INTEGER NOT NULL);\
         CREATE TABLE T (z INTEGER NOT NULL, x INTEGER NOT NULL);",
    )?;

    for &(x, y) in R {
        connection.execute("INSERT INTO R VALUES (?, ?)", params![x, y])?;
    }
    for &(y, z) in S {
        connection.execute("INSERT INTO S VALUES (?, ?)", params![y, z])?;
    }
    for &(z, x) in T {
        connection.execute("INSERT INTO T VALUES (?, ?)", params![z, x])?;
    }

    let mut statement = connection.prepare(TRIANGLE_SQL)?;
    statement
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?
        .collect()
}
