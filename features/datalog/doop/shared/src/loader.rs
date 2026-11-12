use crate::domains::{Domain, Domains};
use crate::interner::{IStr, res, sym};
use crate::manifest::{CertifiedReader, Manifest};
use std::{
    fs::File,
    io::{self, BufRead, BufReader},
    path::Path,
};

trait Column: Sized {
    const TYPE: &'static str;
    fn parse(value: &str) -> io::Result<Self>;
    fn record(&self, domain: &mut Domain);
}

impl Column for IStr {
    const TYPE: &'static str = "string";
    fn parse(value: &str) -> io::Result<Self> {
        Ok(sym(value))
    }
    fn record(&self, domain: &mut Domain) {
        let Domain::Symbols(values) = domain else { unreachable!("symbol domain type") };
        values.insert(self.code());
    }
}

impl Column for i32 {
    const TYPE: &'static str = "i32";
    fn parse(value: &str) -> io::Result<Self> {
        value
            .parse()
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
    }
    fn record(&self, domain: &mut Domain) {
        let Domain::Integers(bounds) = domain else { unreachable!("integer domain type") };
        *bounds = Some(match *bounds {
            Some((min, max)) => (min.min(*self), max.max(*self)),
            None => (*self, *self),
        });
    }
}

pub(crate) trait Row: Sized {
    const TYPES: &'static [&'static str];
    fn parse(line: &str, domains: Option<(&[u32], &mut Domains<'_>)>) -> io::Result<Self>;
}

macro_rules! tuple_row {
    ($($column:ident),+) => {
        impl<$($column: Column),+> Row for ($($column,)+) {
            const TYPES: &'static [&'static str] = &[$($column::TYPE),+];
            fn parse(line: &str, mut domains: Option<(&[u32], &mut Domains<'_>)>) -> io::Result<Self> {
                let mut fields = line.split('\t');
                let mut column = 0;
                let row = ($({
                    let value = $column::parse(fields.next().ok_or_else(||
                        io::Error::new(io::ErrorKind::InvalidData, "too few columns"))?)?;
                    if let Some((ids, values)) = domains.as_mut() {
                        value.record(values.get_mut(ids[column]));
                    }
                    column += 1;
                    value
                },)+);
                let _ = column;
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

pub(crate) fn load<T: Row + Ord>(
    path: &Path,
    manifest: &Manifest,
    relation: &str,
    mut domains: Option<&mut Domains<'_>>,
) -> io::Result<Vec<T>> {
    // Initialize from tuple types even if the relation's file is absent.
    let columns = match domains.as_mut() {
        Some(domains) => domains.columns(relation, T::TYPES)?,
        None => None,
    };
    // The upstream harness treats omitted input relations as empty.
    let file = match File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error),
    };
    let projection = [0, 1, 2, 3, 4, 5, 6];
    let hash = manifest.expected_hash(path, T::TYPES, &projection[..T::TYPES.len()]);
    let mut reader = BufReader::new(CertifiedReader::new(file, hash));
    let mut line = String::new();
    let mut rows = Vec::new();
    let mut number = 0;
    while reader.read_line(&mut line)? != 0 {
        number += 1;
        let text = line.strip_suffix('\n').unwrap_or(&line);
        let text = text.strip_suffix('\r').unwrap_or(text);
        let membership = columns.zip(domains.as_deref_mut());
        rows.push(T::parse(text, membership).map_err(|error| {
            io::Error::new(
                error.kind(),
                format!("{}:{number}: {error}", path.display()),
            )
        })?);
        line.clear();
    }
    if !reader.into_inner().verified() {
        rows.sort_unstable();
        rows.dedup();
    }
    Ok(rows)
}

/// Materialize the join of Method_ReturnType and Method_ParamTypes, not just
/// each raw row: repeated method IDs can contribute multiple values to either.
pub(crate) fn method_descriptors(
    methods: &[(IStr, IStr, IStr, IStr, IStr, IStr, i32)],
    mut domains: Option<&mut Domains<'_>>,
) -> io::Result<Vec<(IStr, IStr)>> {
    let columns = match domains.as_mut() {
        Some(domains) => domains.columns("method_descriptor", &["string", "string"])?,
        None => None,
    };
    use std::collections::{BTreeMap, BTreeSet};

    let mut projected = BTreeMap::<IStr, (BTreeSet<IStr>, BTreeSet<IStr>)>::new();
    for &(method, _, params, _, returntype, _, _) in methods {
        let (returns, parameters) = projected.entry(method).or_default();
        returns.insert(returntype);
        parameters.insert(params);
    }
    let mut descriptors = Vec::new();
    for (method, (returns, parameters)) in projected {
        for returntype in returns {
            // DECODE R: method_descriptors' authored format! below needs return-type text to compute a new descriptor.
            let returntype = res(returntype);
            for &params in &parameters {
                // DECODE R: method_descriptors reads parameter text in opaque format! before interning the computed descriptor.
                let descriptor = sym(&format!("{}({})", returntype, res(params)));
                if let Some((columns, values)) = columns.zip(domains.as_deref_mut()) {
                    method.record(values.get_mut(columns[0]));
                    descriptor.record(values.get_mut(columns[1]));
                }
                descriptors.push((method, descriptor));
            }
        }
    }
    descriptors.sort_unstable();
    descriptors.dedup();
    Ok(descriptors)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::interner::res;

    #[test]
    fn shared_components_collect_membership_across_rows_and_columns() {
        let schema: &[(&str, &[u32])] = &[
            ("directsuperclass", &[0, 0]),
            ("mainclass", &[0]),
        ];
        let mut domains = Domains::new(schema, &[(None, true)]).unwrap();
        let columns = domains.columns("directsuperclass", <(IStr, IStr)>::TYPES).unwrap().unwrap();
        <(IStr, IStr)>::parse("domain-left\tdomain-right", Some((columns, &mut domains))).unwrap();
        <(IStr, IStr)>::parse("domain-left\tdomain-left", Some((columns, &mut domains))).unwrap();
        let columns = domains.columns("mainclass", <(IStr,)>::TYPES).unwrap().unwrap();
        <(IStr,)>::parse("domain-third", Some((columns, &mut domains))).unwrap();
        let mut expected = ["domain-left", "domain-right", "domain-third"]
            .map(|value| sym(value).code()).to_vec();
        expected.sort_unstable();
        assert_eq!(domains.finish(), [crate::DomainValues::Symbols(expected)]);
    }

    #[test]
    fn missing_relations_initialize_typed_empty_domains() {
        let directory = std::env::temp_dir()
            .join(format!("doop-domain-missing-{}", std::process::id()))
            .join("absent");
        let (facts, domains) = crate::Facts::load_with_domains(&directory, &[
            ("formalparam", &[0, 1, 1]),
            ("method_descriptor", &[1, 2]),
        ], &[
            (Some((i32::MIN as i128, i32::MAX as i128)), true),
            (None, true), (None, true),
        ]).unwrap();
        assert!(facts.formalparam.is_empty());
        assert!(facts.method_descriptor.is_empty());
        assert_eq!(domains, [
            crate::DomainValues::Integers(None),
            crate::DomainValues::Symbols(Vec::new()),
            crate::DomainValues::Symbols(Vec::new()),
        ]);
    }

    #[test]
    fn numeric_components_accumulate_inclusive_bounds() {
        let mut domains = Domains::new(&[("numbers", &[0, 0])], &[
            (Some((i32::MIN as i128, i32::MAX as i128)), true),
        ]).unwrap();
        let columns = domains.columns("numbers", <(i32, i32)>::TYPES).unwrap().unwrap();
        <(i32, i32)>::parse("-7\t3", Some((columns, &mut domains))).unwrap();
        <(i32, i32)>::parse("-2147483648\t2147483647", Some((columns, &mut domains))).unwrap();
        <(i32, i32)>::parse("0\t0", Some((columns, &mut domains))).unwrap();
        assert_eq!(domains.finish(), [
            crate::DomainValues::Integers(Some((i32::MIN, i32::MAX))),
        ]);
    }

    #[test]
    fn loading_retains_observed_widths_and_all_derived_component_bounds() {
        let directory = std::env::temp_dir()
            .join(format!("doop-domain-derived-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        std::fs::write(directory.join("FormalParam.facts"), "-2\tx\tx\n3\tx\tx\n").unwrap();
        let (_, values) = crate::Facts::load_with_domains(&directory, &[
            ("derived_first", &[0]), ("formalparam", &[1, 2, 2]),
            ("derived_last", &[3]),
        ], &[
            (Some((-100, 100)), false),
            (Some((i32::MIN as i128, i32::MAX as i128)), true),
            (None, true),
            (Some((i32::MIN as i128, i32::MAX as i128)), false),
        ]).unwrap();
        std::fs::remove_dir_all(directory).unwrap();
        assert_eq!(values, [
            crate::DomainValues::Integers(Some((-100, 100))),
            crate::DomainValues::Integers(Some((-2, 3))),
            crate::DomainValues::Symbols(vec![sym("x").code()]),
            crate::DomainValues::Integers(Some((i32::MIN, i32::MAX))),
        ]);
    }

    #[test]
    fn computed_descriptors_contribute_domain_membership() {
        let method = sym("domain-method");
        let unused = sym("");
        let methods = [(method, unused, sym("int"), unused, sym("void"), unused, 0)];
        let mut domains = Domains::new(&[("method_descriptor", &[0, 1])], &[
            (None, true), (None, true),
        ]).unwrap();
        let descriptors = method_descriptors(&methods, Some(&mut domains)).unwrap();
        assert_eq!(descriptors, [(method, sym("void(int)"))]);
        assert_eq!(domains.finish(), [
            crate::DomainValues::Symbols(vec![method.code()]),
            crate::DomainValues::Symbols(vec![sym("void(int)").code()]),
        ]);
    }

    #[test]
    fn file_loading_deduplicates_complete_rows_not_leading_keys() {
        let path = std::env::temp_dir().join(format!("doop-set-input-{}.facts", std::process::id()));
        std::fs::write(&path, "2\t7\n1\t9\n2\t7\n1\t8\n").unwrap();
        let rows = load::<(i32, i32)>(&path, &Manifest::default(), "", None).unwrap();
        std::fs::remove_file(&path).unwrap();
        assert_eq!(rows, [(1, 8), (1, 9), (2, 7)]);
    }

    #[test]
    fn manifest_override_skips_only_hash_bound_verified_sets() {
        use sha2::{Digest, Sha256};
        let root = std::env::temp_dir().join(format!("doop-manifest-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let path = root.join("FormalParam.facts");
        let manifest_path = root.join("override.json");
        let bytes = b"2\tx\ty\r\n1\tx\ty\r\n";
        std::fs::write(&path, bytes).unwrap();
        let certificate = serde_json::json!({
            "version": 1, "relations": {"FormalParam.facts": {
                "types": ["i32", "string", "string"],
                "verified_keys": [{"columns": [0, 1, 2], "set": "verified",
                    "sha256": format!("{:x}", Sha256::digest(bytes))}]
            }}
        });
        let save = |value: &serde_json::Value| {
            std::fs::write(&manifest_path, serde_json::to_vec(value).unwrap()).unwrap();
        };
        let load = |manifest: Option<&Path>| {
            crate::Facts::load_with_manifest(&root, manifest).unwrap().formalparam
                .into_iter().map(|row| row.0).collect::<Vec<_>>()
        };
        save(&certificate);
        assert_eq!(load(None), [1, 2]);
        assert_eq!(load(Some(&manifest_path)), [2, 1]);
        std::fs::copy(&manifest_path, root.join(crate::manifest::DEFAULT_MANIFEST)).unwrap();
        assert_eq!(load(None), [2, 1]);
        assert_eq!(load(Some(&root.join("missing.json"))), [1, 2]);
        for change in ["unverified", "wrong-decoder", "wrong-version"] {
            let mut invalid = certificate.clone();
            match change {
                "unverified" => invalid["relations"]["FormalParam.facts"]["verified_keys"][0]["set"] = "assumed".into(),
                "wrong-decoder" => invalid["relations"]["FormalParam.facts"]["types"][0] = "string".into(),
                _ => invalid["version"] = 2.into(),
            }
            save(&invalid);
            assert_eq!(load(Some(&manifest_path)), [1, 2]);
        }
        save(&certificate);
        std::fs::write(&path, b"2\tx\ty\n1\tx\ty\n2\tx\ty\n").unwrap();
        assert_eq!(load(Some(&manifest_path)), [1, 2]);
        std::fs::write(&manifest_path, b"not json").unwrap();
        assert_eq!(load(Some(&manifest_path)), [1, 2]);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn descriptors_join_projections_and_deduplicate() {
        let method = sym("method");
        let unused = sym("");
        let row = |params, returns| (method, unused, sym(params), unused, sym(returns), unused, 0);
        let methods = [row("int", "void"), row("int", "void"), row("long", "int")];
        let mut expected = ["void(int)", "void(long)", "int(int)", "int(long)"]
            .map(|descriptor| (method, sym(descriptor)))
            .to_vec();
        expected.sort_unstable();
        assert_eq!(method_descriptors(&methods, None).unwrap(), expected);
        assert!(method_descriptors(&[], None).unwrap().is_empty());
    }

    #[test]
    fn exact_columns_and_empty_strings() {
        let (index, name, empty) = <(i32, IStr, IStr)>::parse("-3\tname\t", None).unwrap();
        assert_eq!(index, -3);
        // DECODE P: exact_columns_and_empty_strings exposes the parsed name for a diagnostic answer assertion.
        assert_eq!(&*res(name), "name");
        // DECODE P: exact_columns_and_empty_strings exposes the empty symbol for a diagnostic answer assertion.
        assert_eq!(&*res(empty), "");
        assert!(<(IStr, IStr)>::parse("one", None).is_err());
        assert!(<(IStr,)>::parse("one\ttwo", None).is_err());
        assert!(<(i32,)>::parse("2147483648", None).is_err());
    }
}
