//! A loader is derived from authored tokens, including custom owned columns.
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

use cq_load::CsvOptions;

struct File(PathBuf);
impl File {
    fn new(contents: &str) -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let path = std::env::temp_dir().join(format!(
            "mini-linq-load-{}-{}.csv",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::write(&path, contents).unwrap();
        Self(path)
    }
    fn path(&self) -> &Path {
        &self.0
    }
}
impl Drop for File {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

#[allow(unused_imports)]
mod authored {
    ::mini_linq::workload_query! {
        struct Loading;
        relation Reading(id: i64, label: String, amount: i32);
        relation answer(id: i64, label: String, amount: i32);
        answer(id, label, amount) :- Reading(id, label, amount);
    }
}

mod loaders {
    crate::authored::with_query!(mini_linq::relation_loaders);
}

#[test]
fn one_authored_callback_loads_every_prefix_without_losing_duplicates() {
    let file = File::new("1,a,10\n1,a,10\n1,b,20\n2,c,30\n");
    let options = CsvOptions {
        batch_size: 2,
        ..CsvOptions::default()
    };
    let expected = vec![
        (1, "a".into(), 10),
        (1, "a".into(), 10),
        (1, "b".into(), 20),
        (2, "c".into(), 30),
    ];
    assert_eq!(
        loaders::ReadingLoader::csv(file.path(), options).unwrap(),
        expected
    );
    assert_eq!(
        loaders::ReadingLoader::csv_batches(file.path(), options)
            .unwrap()
            .map(|batch| batch.unwrap().len())
            .collect::<Vec<_>>(),
        [2, 2]
    );
    let zero = loaders::ReadingLoader::csv_hash::<0>(file.path(), options).unwrap();
    assert_eq!(zero[&()], expected);
    let one = loaders::ReadingLoader::csv_hash::<1>(file.path(), options).unwrap();
    assert_eq!(
        one[&(1,)],
        [("a".into(), 10), ("a".into(), 10), ("b".into(), 20)]
    );
    let two = loaders::ReadingLoader::csv_hash::<2>(file.path(), options).unwrap();
    assert_eq!(two[&(1, "a".into())], [(10,), (10,)]);
    let full = loaders::ReadingLoader::csv_hash::<3>(file.path(), options).unwrap();
    assert_eq!(full[&(1, "a".into(), 10)], [(), ()]);
    assert_eq!(
        loaders::ReadingLoader::csv_btree::<0>(file.path(), options)
            .unwrap()
            .into_iter()
            .collect::<Vec<_>>(),
        zero.into_iter().collect::<Vec<_>>()
    );
    assert_eq!(
        loaders::ReadingLoader::csv_btree::<1>(file.path(), options).unwrap(),
        one.into_iter().collect()
    );
    assert_eq!(
        loaders::ReadingLoader::csv_btree::<2>(file.path(), options).unwrap(),
        two.into_iter().collect()
    );
    assert_eq!(
        loaders::ReadingLoader::csv_btree::<3>(file.path(), options).unwrap(),
        full.into_iter().collect()
    );
    assert!(!authored::SOURCE.is_empty());
    assert!(authored::RUST_PATH.ends_with("relation_loaders.rs"));
    assert_eq!(authored::module().unwrap().inputs().count(), 1);
}

#[derive(Debug, PartialEq)]
struct OwnedUserType(String);
impl cq_load::Load for OwnedUserType {
    fn load(value: &str) -> Result<Self, String> {
        Ok(Self(
            value.strip_prefix('@').ok_or("expected @ prefix")?.into(),
        ))
    }
}

mod payload {
    use super::OwnedUserType;
    ::mini_linq::relation_loaders! {
        relation Sample(key: i64, reading: f64, annotation: OwnedUserType);
    }
}

#[test]
fn unordered_owned_payload_does_not_require_key_traits() {
    let file = File::new("annotation|reading|key\n@first|NaN|42\n@second|1.5|42\n");
    let options = CsvOptions {
        headers: true,
        delimiter: b'|',
        batch_size: 1,
        ..CsvOptions::default()
    };
    let groups = payload::SampleLoader::csv_hash::<1>(file.path(), options).unwrap();
    assert!(groups[&(42,)][0].0.is_nan());
    assert_eq!(groups[&(42,)][1], (1.5, OwnedUserType("second".into())));
    let ordered = payload::SampleLoader::csv_btree::<1>(file.path(), options).unwrap();
    assert_eq!(ordered[&(42,)][0].1, OwnedUserType("first".into()));
}

mod interned {
    use super::OwnedUserType;
    use cq_load::StringCode;

    // Alias spelling deliberately differs from StringCode: dispatch must use
    // trait implementations, not inspect authored Rust type names.
    type Text = cq_load::StringCode;

    ::mini_linq::interned_relation_loaders! {
        relation Encoded(id: i64, label: Text, other: Text, annotation: OwnedUserType);
    }

    #[test]
    fn generated_interning_preserves_rows_and_custom_scalar_fields() {
        let mut dictionaries = cq_load::Dictionaries::default();
        let rows = <relation_loader!(Encoded)>::intern(
            vec![
                (2, "z".into(), "a".into(), OwnedUserType("first".into())),
                (1, "a".into(), "z".into(), OwnedUserType("second".into())),
                (2, "z".into(), "a".into(), OwnedUserType("first".into())),
            ],
            &mut dictionaries,
        );
        assert_eq!(
            rows,
            [
                (
                    2,
                    StringCode(1),
                    StringCode(0),
                    OwnedUserType("first".into())
                ),
                (
                    1,
                    StringCode(0),
                    StringCode(1),
                    OwnedUserType("second".into())
                ),
                (
                    2,
                    StringCode(1),
                    StringCode(0),
                    OwnedUserType("first".into())
                ),
            ]
        );
        assert_eq!(
            ::cq_storage::domain::exact(dictionaries.domain("label"), "missing"),
            Text::MAX
        );
        assert_eq!(
            dictionaries.select("label", |value| value == "a"),
            [(cq_load::StringCode(0),)]
        );
        assert!(dictionaries.get("id").is_none());
        assert!(dictionaries.get("annotation").is_none());
    }

    #[test]
    fn generated_interning_registers_empty_string_columns() {
        let mut dictionaries = cq_load::Dictionaries::default();
        assert!(<relation_loader!(Encoded)>::intern(Vec::new(), &mut dictionaries).is_empty());
        assert!(dictionaries.get("label").is_some());
        assert!(dictionaries.get("other").is_some());
        assert_eq!(
            ::cq_storage::domain::exact(dictionaries.domain("label"), ""),
            Text::MAX
        );
    }
}

#[test]
fn schema_column_names_survive_parse_errors_and_header_reordering() {
    let file = File::new("amount,label,id\n10,a,1\nwrong,b,2\n");
    let error = loaders::ReadingLoader::csv(
        file.path(),
        CsvOptions {
            headers: true,
            batch_size: 1,
            ..CsvOptions::default()
        },
    )
    .unwrap_err();
    let message = error.to_string();
    assert!(message.contains(file.path().to_str().unwrap()), "{message}");
    assert!(message.contains('3'), "{message}");
    assert!(message.contains("amount"), "{message}");
    assert!(message.contains("wrong"), "{message}");
}

mod names {
    use super::{CsvOptions, File};

    #[derive(Debug, PartialEq)]
    struct R(String);
    impl cq_load::Load for R {
        fn load(value: &str) -> Result<Self, String> {
            Ok(Self(value.into()))
        }
    }

    ::mini_linq::relation_loaders! {
        relation String(value: String);
        relation R(value: R);
        relation RLoader(id: i64);
        relation r#type(r#match: i32);
    }

    #[test]
    fn relation_names_preserve_owned_and_custom_type_names() {
        let text = File::new("hello\n");
        assert_eq!(
            <relation_loader!(String)>::csv(text.path(), CsvOptions::default()).unwrap(),
            [("hello".to_owned(),)]
        );
        assert_eq!(
            <relation_loader!(R)>::csv(text.path(), CsvOptions::default()).unwrap(),
            [(R("hello".into()),)]
        );
        let number = File::new("42\n");
        assert_eq!(
            <relation_loader!(RLoader)>::csv(number.path(), CsvOptions::default()).unwrap(),
            [(42_i64,)]
        );
        let raw = File::new("match\n7\n");
        assert_eq!(
            <relation_loader!(r#type)>::csv(
                raw.path(),
                CsvOptions {
                    headers: true,
                    ..CsvOptions::default()
                },
            )
            .unwrap(),
            [(7,)]
        );
    }
}

mod dispatch {
    use super::{CsvOptions, File};
    crate::authored::with_query!(::mini_linq::relation_loaders);

    #[test]
    #[allow(non_snake_case)]
    fn loaded_prefix_maps_execute_the_authored_query() {
        let file = File::new("1,a,10\n1,a,10\n1,b,20\n2,c,30\n");
        let expected = vec![
            (1, "a".to_owned(), 10),
            (1, "b".to_owned(), 20),
            (2, "c".to_owned(), 30),
        ];
        macro_rules! run {
            (struct $query:ident; relation $relation:ident($($columns:tt)*); $($rest:tt)*) => {{
                macro_rules! check {
                    ($method:ident, $prefix:literal) => {{
                        let $relation = <relation_loader!($relation)>::$method::<$prefix>(
                            file.path(), CsvOptions::default(),
                        ).unwrap();
                        let mut actual = ::mini_linq::compile! { pipeline { }
                            struct $query; relation $relation($($columns)*); $($rest)*
                        }
                            .map(|(id, label, amount)| (*id, label.clone(), *amount))
                            .collect::<Vec<_>>();
                        actual.sort();
                        assert_eq!(actual, expected);
                    }};
                }
                check!(csv_hash, 0);
                check!(csv_hash, 1);
                check!(csv_hash, 2);
                check!(csv_hash, 3);
                check!(csv_btree, 0);
                check!(csv_btree, 1);
                check!(csv_btree, 2);
                check!(csv_btree, 3);
            }};
        }
        crate::authored::with_query!(run);
    }
}

mod columnar {
    use super::{CsvOptions, File};
    use cq_storage::scan::{BlockRows, Bytes, Family, OrdinalDecode, Plain, ScanColumn, VECTOR_ROWS};

    type Integer = i64;

    ::mini_linq::columnar_relation_loaders! {
        relation Reading(id: Integer @ bytes, amount: i32 @ plain);
        relation r#type(r#match: i64 @ plain);
    }

    mod ordinary {
        ::mini_linq::relation_loaders! {
            relation Reading(id: i64, amount: i32);
        }
    }

    pub(super) fn decode<T: OrdinalDecode, F: Family>(
        column: &ScanColumn<T, F>,
        initial: T,
    ) -> Vec<T> {
        let mut output = Vec::with_capacity(column.len());
        let mut values = [initial; VECTOR_ROWS];
        for (block, header) in column.headers().iter().enumerate() {
            for vector in 0..(header.rows as usize).div_ceil(VECTOR_ROWS) {
                let len = column.decode_vector(block, vector, &mut values);
                output.extend_from_slice(&values[..len]);
            }
        }
        output
    }

    #[test]
    fn explicit_mixed_families_match_plain_rows_across_batches_and_partial_blocks() {
        use std::fmt::Write;

        let mut contents = String::from("amount,id\n");
        for row in 0..(2 * VECTOR_ROWS + 17) {
            writeln!(contents, "{},{}", row % 7, row / 3).unwrap();
        }
        let file = File::new(&contents);
        let options = CsvOptions {
            headers: true,
            batch_size: 137,
            ..CsvOptions::default()
        };
        let block_rows = BlockRows::new(VECTOR_ROWS as u32).unwrap();
        let table = <relation_loader!(Reading)>::csv(file.path(), options, block_rows, true).unwrap();
        let expected = ordinary::ReadingLoader::csv(file.path(), options).unwrap();
        let (ids, amounts) = table.columns();
        cq_storage::scan::family::<Bytes, _>(ids);
        cq_storage::scan::family::<Plain, _>(amounts);
        assert_eq!(table.len(), expected.len());
        assert_eq!(table.block_rows(), block_rows);
        assert_eq!(ids.headers().len(), 3);
        assert_eq!(ids.headers()[2].rows, 17);
        assert_eq!(ids.summaries().unwrap().len(), 3);
        assert_eq!(amounts.summaries().unwrap().len(), 3);
        assert_eq!(
            decode(ids, 0).into_iter().zip(decode(amounts, 0)).collect::<Vec<_>>(),
            expected,
        );
    }

    #[test]
    fn empty_and_raw_named_columns_keep_authored_layout_without_summaries() {
        let block_rows = BlockRows::new(VECTOR_ROWS as u32).unwrap();
        let empty = File::new("");
        let table = ReadingLoader::csv(empty.path(), CsvOptions::default(), block_rows, false).unwrap();
        assert_eq!(table.len(), 0);
        assert!(table.columns().0.headers().is_empty());
        assert!(table.columns().0.summaries().is_none());
        let file = File::new("match\n-7\n-7\n");
        let table = <relation_loader!(r#type)>::csv(
            file.path(),
            CsvOptions { headers: true, ..CsvOptions::default() },
            block_rows,
            false,
        ).unwrap();
        assert_eq!(decode(&table.columns().0, 0), [-7, -7]);
        assert!(table.columns().0.summaries().is_none());
    }

    #[test]
    fn columnar_parse_errors_retain_source_coordinates_and_field_names() {
        let file = File::new("amount,id\n10,1\nwrong,2\n");
        let error = ReadingLoader::csv(
            file.path(),
            CsvOptions { headers: true, batch_size: 1, ..CsvOptions::default() },
            BlockRows::new(VECTOR_ROWS as u32).unwrap(),
            true,
        ).err().unwrap();
        let message = error.to_string();
        assert!(message.contains(file.path().to_str().unwrap()), "{message}");
        assert!(message.contains('3'), "{message}");
        assert!(message.contains("amount"), "{message}");
        assert!(message.contains("wrong"), "{message}");
    }
}

mod columnar_interned {
    use super::{CsvOptions, File, columnar::decode};
    use cq_storage::scan::{BlockRows, Bytes, Plain, VECTOR_ROWS};

    type Code = cq_load::StringCode;

    ::mini_linq::interned_relation_loaders! {
        columnar;
        relation Encoded(id: i64 @ plain, label: Code @ bytes, other: Code @ plain);
    }

    mod ordinary {
        ::mini_linq::interned_relation_loaders! {
            relation Encoded(id: i64, label: cq_load::StringCode, other: cq_load::StringCode);
        }
    }

    #[test]
    fn final_dictionary_codes_are_encoded_in_each_authored_family() {
        use std::fmt::Write;

        let mut contents = String::from("other,label,id\n");
        let mut source = Vec::new();
        for row in 0..(2 * VECTOR_ROWS + 9) {
            let label = if row < VECTOR_ROWS + 1 { "z" } else { "a" };
            let other = if row % 2 == 0 { "a" } else { "z" };
            let id = (row / 3) as i64;
            writeln!(contents, "{other},{label},{id}").unwrap();
            source.push((id, label.to_owned(), other.to_owned()));
        }
        let mut expected_dictionaries = cq_load::Dictionaries::default();
        let expected = ordinary::EncodedLoader::intern(source, &mut expected_dictionaries);
        let file = File::new(&contents);
        let mut dictionaries = cq_load::Dictionaries::default();
        let table = <relation_loader!(Encoded)>::csv_interned(
            file.path(),
            CsvOptions { headers: true, batch_size: 137, ..CsvOptions::default() },
            BlockRows::new(VECTOR_ROWS as u32).unwrap(),
            true,
            &mut dictionaries,
        ).unwrap();
        let (ids, labels, others) = table.columns();
        cq_storage::scan::family::<Bytes, _>(labels);
        cq_storage::scan::family::<Plain, _>(others);
        let actual = decode(ids, 0).into_iter()
            .zip(decode(labels, Code::MAX))
            .zip(decode(others, Code::MAX))
            .map(|((id, label), other)| (id, label, other))
            .collect::<Vec<_>>();
        assert_eq!(actual, expected);
        assert_eq!(labels.headers().len(), 3);
        assert_eq!(labels.headers()[2].rows, 9);
        assert_eq!(labels.summaries().unwrap().len(), 3);
        for column in ["label", "other"] {
            for value in ["a", "z", "absent"] {
                assert_eq!(
                    cq_storage::domain::exact(dictionaries.domain(column), value),
                    cq_storage::domain::exact(expected_dictionaries.domain(column), value),
                );
            }
        }
        assert!(dictionaries.get("id").is_none());
    }

    #[test]
    fn empty_columnar_interned_input_publishes_empty_dictionaries() {
        let file = File::new("");
        let mut dictionaries = cq_load::Dictionaries::default();
        let table = EncodedLoader::csv_interned(
            file.path(), CsvOptions::default(),
            BlockRows::new(VECTOR_ROWS as u32).unwrap(), false, &mut dictionaries,
        ).unwrap();
        assert_eq!(table.len(), 0);
        assert!(table.columns().1.headers().is_empty());
        assert!(table.columns().1.summaries().is_none());
        assert!(dictionaries.get("label").is_some());
        assert!(dictionaries.get("other").is_some());
        assert_eq!(cq_storage::domain::exact(dictionaries.domain("label"), ""), Code::MAX);
    }
}
