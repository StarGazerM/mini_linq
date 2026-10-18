use crate::interner::{IStr, sym};
use std::{
    fs::File,
    io::{self, BufRead, BufReader},
    path::Path,
};

trait Column: Sized {
    fn parse(value: &str) -> io::Result<Self>;
}

impl Column for IStr {
    fn parse(value: &str) -> io::Result<Self> {
        Ok(sym(value))
    }
}

impl Column for i32 {
    fn parse(value: &str) -> io::Result<Self> {
        value
            .parse()
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
    }
}

pub(crate) trait Row: Sized {
    fn parse(line: &str) -> io::Result<Self>;
}

macro_rules! tuple_row {
    ($($column:ident),+) => {
        impl<$($column: Column),+> Row for ($($column,)+) {
            fn parse(line: &str) -> io::Result<Self> {
                let mut fields = line.split('\t');
                let row = ($($column::parse(fields.next().ok_or_else(||
                    io::Error::new(io::ErrorKind::InvalidData, "too few columns"))?)?,)+);
                if fields.next().is_some() {
                    return Err(io::Error::new(io::ErrorKind::InvalidData, "too many columns"));
                }
                Ok(row)
            }
        }
    };
}
tuple_row!(A);
tuple_row!(A, B);
tuple_row!(A, B, C);
tuple_row!(A, B, C, D);
tuple_row!(A, B, C, D, E);
tuple_row!(A, B, C, D, E, F);
tuple_row!(A, B, C, D, E, F, G);

pub(crate) fn load<T: Row>(path: &Path) -> io::Result<Vec<T>> {
    // The upstream harness treats omitted input relations as empty.
    let file = match File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error),
    };
    let mut reader = BufReader::new(file);
    let mut line = String::new();
    let mut rows = Vec::new();
    let mut number = 0;
    while reader.read_line(&mut line)? != 0 {
        number += 1;
        let text = line.strip_suffix('\n').unwrap_or(&line);
        let text = text.strip_suffix('\r').unwrap_or(text);
        rows.push(T::parse(text).map_err(|error| {
            io::Error::new(
                error.kind(),
                format!("{}:{number}: {error}", path.display()),
            )
        })?);
        line.clear();
    }
    Ok(rows)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::interner::res;

    #[test]
    fn exact_columns_and_empty_strings() {
        let (index, name, empty) = <(i32, IStr, IStr)>::parse("-3\tname\t").unwrap();
        assert_eq!(index, -3);
        assert_eq!(&*res(name), "name");
        assert_eq!(&*res(empty), "");
        assert!(<(IStr, IStr)>::parse("one").is_err());
        assert!(<(IStr,)>::parse("one\ttwo").is_err());
        assert!(<(i32,)>::parse("2147483648").is_err());
    }
}
