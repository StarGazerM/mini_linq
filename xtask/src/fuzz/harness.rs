// Included in an ephemeral consumer crate by xtask. Inputs remain runtime data,
// allowing the very same compiled query to shrink rows after a mismatch.
use std::io::{BufRead, Write};

type Data = Vec<Vec<Vec<i32>>>;

fn integer(value: impl std::borrow::Borrow<i32>) -> i32 {
    *value.borrow()
}

fn main() {
    let stdin = std::io::stdin();
    let stdout = std::io::stdout();
    let mut out = stdout.lock();
    for line in stdin.lock().lines() {
        let line = line.unwrap();
        let mut words = line.split_whitespace();
        let id: usize = words.next().unwrap().parse().unwrap();
        let relations: usize = words.next().unwrap().parse().unwrap();
        let mut data = Vec::new();
        for _ in 0..relations {
            let arity: usize = words.next().unwrap().parse().unwrap();
            let rows: usize = words.next().unwrap().parse().unwrap();
            data.push(
                (0..rows)
                    .map(|_| {
                        (0..arity)
                            .map(|_| words.next().unwrap().parse::<i32>().unwrap())
                            .collect::<Vec<_>>()
                    })
                    .collect::<Vec<_>>(),
            );
        }
        assert!(words.next().is_none());
        match std::panic::catch_unwind(|| dispatch(id, &data)) {
            Ok(mut rows) => {
                rows.sort();
                if rows.windows(2).any(|pair| pair[0] == pair[1]) {
                    writeln!(out, "{id} DUPLICATES").unwrap();
                    out.flush().unwrap();
                    continue;
                }
                write!(out, "{id} OK {}", rows.len()).unwrap();
                for row in rows {
                    for value in row {
                        write!(out, " {value}").unwrap();
                    }
                }
                writeln!(out).unwrap();
            }
            Err(_) => writeln!(out, "{id} PANIC").unwrap(),
        }
        out.flush().unwrap();
    }
}
