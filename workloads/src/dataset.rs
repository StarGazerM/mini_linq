//! Strict row-oriented datasets shared by MiniLinq and DuckDB.
//!
//! A dataset directory contains one headerless integer CSV file per declared
//! input and a small, deterministic manifest. Raw duplicate rows are retained:
//! both MiniLinq and the SQL oracle apply input set semantics after loading.

use std::collections::BTreeSet;
use std::error::Error as StdError;
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

use cq_ir::{cq, symbol_name};

use crate::{GENERATOR_VERSION, GenerationConfig, Scale, Scenario, Suite};

pub const MANIFEST_FILE: &str = "manifest.txt";
const FORMAT_VERSION: u32 = 2;

/// One input relation in declaration-column order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RelationData {
    pub name: String,
    pub arity: usize,
    pub rows: Vec<Vec<i32>>,
}

impl RelationData {
    pub fn new(
        name: impl Into<String>,
        arity: usize,
        rows: Vec<Vec<i32>>,
    ) -> Result<Self, DatasetError> {
        let relation = Self {
            name: name.into(),
            arity,
            rows,
        };
        relation.validate_rows()?;
        Ok(relation)
    }

    pub fn raw_cardinality(&self) -> usize {
        self.rows.len()
    }

    pub fn distinct_cardinality(&self) -> usize {
        self.rows
            .iter()
            .map(Vec::as_slice)
            .collect::<BTreeSet<_>>()
            .len()
    }

    fn validate_rows(&self) -> Result<(), DatasetError> {
        validate_relation_name(&self.name)?;
        if self.arity == 0 {
            return Err(DatasetError::Schema(format!(
                "relation `{}` must have positive arity",
                self.name
            )));
        }
        for (row_index, row) in self.rows.iter().enumerate() {
            if row.len() != self.arity {
                return Err(DatasetError::Schema(format!(
                    "relation `{}` row {} has arity {}; expected {}",
                    self.name,
                    row_index + 1,
                    row.len(),
                    self.arity,
                )));
            }
        }
        Ok(())
    }
}

/// Cardinalities recorded from the actual rows written to disk.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RelationCardinality {
    pub name: String,
    pub arity: usize,
    pub raw_rows: usize,
    pub distinct_rows: usize,
}

/// Reproduction metadata stored beside a dataset.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Manifest {
    pub suite: Suite,
    pub case: String,
    pub scenario: Scenario,
    pub scale: Scale,
    pub seed: u64,
    pub generator_version: u32,
    pub relations: Vec<RelationCardinality>,
}

/// A complete set of inputs for one query case.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Dataset {
    pub manifest: Manifest,
    pub relations: Vec<RelationData>,
}

impl Dataset {
    /// Construct and validate generated data against the exact CQ schema.
    pub fn new(
        module: &cq::Module,
        suite: Suite,
        case: impl Into<String>,
        config: GenerationConfig,
        relations: Vec<RelationData>,
    ) -> Result<Self, DatasetError> {
        let case = case.into();
        validate_manifest_token("case name", &case)?;
        let manifest = Manifest {
            suite,
            case,
            scenario: config.scenario,
            scale: config.scale,
            seed: config.seed,
            generator_version: GENERATOR_VERSION,
            relations: relations.iter().map(relation_cardinality).collect(),
        };
        let dataset = Self {
            manifest,
            relations,
        };
        dataset.validate(module)?;
        Ok(dataset)
    }

    /// Enforce exact declaration order, names, arities, row widths, and counts.
    pub fn validate(&self, module: &cq::Module) -> Result<(), DatasetError> {
        cq::contract::check(module).map_err(DatasetError::InvalidQuery)?;
        validate_manifest_token("case name", &self.manifest.case)?;
        if self.manifest.generator_version != GENERATOR_VERSION {
            return Err(DatasetError::Schema(format!(
                "dataset generator version is {}; this checkout requires {}",
                self.manifest.generator_version, GENERATOR_VERSION,
            )));
        }

        let declarations = &module.program.inputs;
        if self.relations.len() != declarations.len() {
            return Err(DatasetError::Schema(format!(
                "dataset has {} relations; query declares {}",
                self.relations.len(),
                declarations.len(),
            )));
        }
        if self.manifest.relations.len() != declarations.len() {
            return Err(DatasetError::Schema(format!(
                "manifest has {} relation entries; query declares {}",
                self.manifest.relations.len(),
                declarations.len(),
            )));
        }

        for (position, ((declaration, relation), recorded)) in declarations
            .iter()
            .zip(&self.relations)
            .zip(&self.manifest.relations)
            .enumerate()
        {
            relation.validate_rows()?;
            let expected_name = symbol_name(&declaration.name);
            let expected_arity = declaration.columns.len();
            if relation.name != expected_name || relation.arity != expected_arity {
                return Err(DatasetError::Schema(format!(
                    "relation {} is `{}/{}`; query declares `{}/{}` in that position",
                    position + 1,
                    relation.name,
                    relation.arity,
                    expected_name,
                    expected_arity,
                )));
            }

            let actual = relation_cardinality(relation);
            if *recorded != actual {
                return Err(DatasetError::Schema(format!(
                    "manifest cardinality for `{}` is stale: recorded raw/distinct {}/{}, actual {}/{}",
                    relation.name,
                    recorded.raw_rows,
                    recorded.distinct_rows,
                    actual.raw_rows,
                    actual.distinct_rows,
                )));
            }
        }
        let empty_relations = self
            .relations
            .iter()
            .filter(|relation| relation.rows.is_empty())
            .count();
        match self.manifest.scenario {
            Scenario::Coverage | Scenario::NoMatch if empty_relations != 0 => {
                return Err(DatasetError::Schema(format!(
                    "{} scenario requires every input to be nonempty; found {empty_relations} empty relations",
                    self.manifest.scenario.as_str(),
                )));
            }
            Scenario::EmptyInput if empty_relations != 1 => {
                return Err(DatasetError::Schema(format!(
                    "empty-input scenario requires exactly one empty relation; found {empty_relations}",
                )));
            }
            _ => {}
        }
        Ok(())
    }

    /// Match a directory manifest to the catalog case and generation request.
    ///
    /// Schema validation alone cannot distinguish a tiny dataset copied under
    /// a different scenario or scale path, or data produced with another seed.
    pub fn validate_provenance(
        &self,
        suite: Suite,
        case: &str,
        config: GenerationConfig,
    ) -> Result<(), DatasetError> {
        let actual = &self.manifest;
        if actual.suite != suite
            || actual.case != case
            || actual.scenario != config.scenario
            || actual.scale != config.scale
            || actual.seed != config.seed
        {
            return Err(DatasetError::Schema(format!(
                "dataset provenance is {}/{}/{}/{}/seed {}; expected {}/{}/{}/{}/seed {}",
                actual.suite.as_str(),
                actual.case,
                actual.scenario.as_str(),
                actual.scale.as_str(),
                actual.seed,
                suite.as_str(),
                case,
                config.scenario.as_str(),
                config.scale.as_str(),
                config.seed,
            )));
        }
        Ok(())
    }

    pub fn relation(&self, name: &str) -> Option<&RelationData> {
        self.relations.iter().find(|relation| relation.name == name)
    }

    /// Write deterministic headerless CSV and a deterministic line manifest.
    pub fn write_to_dir(
        &self,
        module: &cq::Module,
        directory: impl AsRef<Path>,
    ) -> Result<(), DatasetError> {
        self.validate(module)?;
        let directory = directory.as_ref();
        fs::create_dir_all(directory).map_err(|source| DatasetError::Io {
            path: directory.to_owned(),
            source,
        })?;
        reject_extra_csv_files(directory, self.relations.len())?;

        let manifest_path = directory.join(MANIFEST_FILE);
        fs::write(&manifest_path, encode_manifest(&self.manifest)).map_err(|source| {
            DatasetError::Io {
                path: manifest_path,
                source,
            }
        })?;
        for (input_index, relation) in self.relations.iter().enumerate() {
            let path = directory.join(input_file_name(input_index));
            fs::write(&path, encode_csv(relation))
                .map_err(|source| DatasetError::Io { path, source })?;
        }
        Ok(())
    }

    /// Read a dataset directory and reject any drift from the CQ schema.
    pub fn read_from_dir(
        module: &cq::Module,
        directory: impl AsRef<Path>,
    ) -> Result<Self, DatasetError> {
        cq::contract::check(module).map_err(DatasetError::InvalidQuery)?;
        let directory = directory.as_ref();
        let manifest_path = directory.join(MANIFEST_FILE);
        let text = fs::read_to_string(&manifest_path).map_err(|source| DatasetError::Io {
            path: manifest_path.clone(),
            source,
        })?;
        let manifest = decode_manifest(&manifest_path, &text)?;

        reject_extra_csv_files(directory, manifest.relations.len())?;

        let mut relations = Vec::with_capacity(manifest.relations.len());
        for (input_index, entry) in manifest.relations.iter().enumerate() {
            let path = directory.join(input_file_name(input_index));
            let text = fs::read_to_string(&path).map_err(|source| DatasetError::Io {
                path: path.clone(),
                source,
            })?;
            relations.push(decode_csv(&path, &entry.name, entry.arity, &text)?);
        }
        let dataset = Self {
            manifest,
            relations,
        };
        dataset.validate(module)?;
        Ok(dataset)
    }
}

/// Write a canonical materialized result: exact width, strictly sorted, distinct rows.
pub fn write_result_rows(
    path: impl AsRef<Path>,
    rows: &[Vec<i32>],
    arity: usize,
) -> Result<(), DatasetError> {
    let relation = RelationData::new("result", arity, rows.to_vec())?;
    validate_materialized_result(&relation.rows)?;
    let path = path.as_ref();
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        fs::create_dir_all(parent).map_err(|source| DatasetError::Io {
            path: parent.to_owned(),
            source,
        })?;
    }
    fs::write(path, encode_csv(&relation)).map_err(|source| DatasetError::Io {
        path: path.to_owned(),
        source,
    })
}

/// Read a canonical materialized result with the query head's known arity.
pub fn read_result_rows(
    path: impl AsRef<Path>,
    arity: usize,
) -> Result<Vec<Vec<i32>>, DatasetError> {
    let path = path.as_ref();
    let text = fs::read_to_string(path).map_err(|source| DatasetError::Io {
        path: path.to_owned(),
        source,
    })?;
    let rows = decode_csv(path, "result", arity, &text)?.rows;
    validate_materialized_result(&rows)?;
    Ok(rows)
}

fn validate_materialized_result(rows: &[Vec<i32>]) -> Result<(), DatasetError> {
    if let Some((position, pair)) = rows
        .windows(2)
        .enumerate()
        .find(|(_, pair)| pair[0] >= pair[1])
    {
        return Err(DatasetError::Schema(format!(
            "materialized result rows {} and {} are not in strict lexicographic order: {:?}, {:?}",
            position + 1,
            position + 2,
            pair[0],
            pair[1],
        )));
    }
    Ok(())
}

fn relation_cardinality(relation: &RelationData) -> RelationCardinality {
    RelationCardinality {
        name: relation.name.clone(),
        arity: relation.arity,
        raw_rows: relation.raw_cardinality(),
        distinct_rows: relation.distinct_cardinality(),
    }
}

/// Collision-proof CSV filename for one declared input position.
///
/// Source relation identifiers remain in the manifest. Positional filenames
/// avoid `R`/`r` collisions on case-insensitive filesystems and remain valid
/// for raw Rust identifiers.
pub fn input_file_name(input_index: usize) -> String {
    format!("input{input_index}.csv")
}

fn reject_extra_csv_files(directory: &Path, relation_count: usize) -> Result<(), DatasetError> {
    let expected = (0..relation_count)
        .map(input_file_name)
        .collect::<BTreeSet<_>>();
    let entries = fs::read_dir(directory).map_err(|source| DatasetError::Io {
        path: directory.to_owned(),
        source,
    })?;
    for entry in entries {
        let entry = entry.map_err(|source| DatasetError::Io {
            path: directory.to_owned(),
            source,
        })?;
        let path = entry.path();
        if path.extension().and_then(|extension| extension.to_str()) != Some("csv") {
            continue;
        }
        let Some(file_name) = path.file_name().and_then(|name| name.to_str()) else {
            return Err(DatasetError::Schema(format!(
                "dataset contains a non-UTF-8 CSV filename at {}",
                path.display()
            )));
        };
        if !expected.contains(file_name) {
            return Err(DatasetError::Schema(format!(
                "dataset contains undeclared CSV file `{file_name}`"
            )));
        }
    }
    Ok(())
}

fn encode_csv(relation: &RelationData) -> String {
    let mut output = String::new();
    for row in &relation.rows {
        for (column, value) in row.iter().enumerate() {
            if column != 0 {
                output.push(',');
            }
            output.push_str(&value.to_string());
        }
        output.push('\n');
    }
    output
}

fn decode_csv(
    path: &Path,
    name: &str,
    arity: usize,
    text: &str,
) -> Result<RelationData, DatasetError> {
    if !text.is_empty() && !text.ends_with('\n') {
        return Err(DatasetError::Csv {
            path: path.to_owned(),
            line: text.lines().count(),
            message: "nonempty CSV must end with a newline".to_owned(),
        });
    }
    let mut rows = Vec::new();
    for (line_index, line) in text.split_terminator('\n').enumerate() {
        if line.is_empty() {
            return Err(DatasetError::Csv {
                path: path.to_owned(),
                line: line_index + 1,
                message: "blank rows are not allowed".to_owned(),
            });
        }
        let fields = line.split(',').collect::<Vec<_>>();
        if fields.len() != arity {
            return Err(DatasetError::Csv {
                path: path.to_owned(),
                line: line_index + 1,
                message: format!("row has {} columns; expected {arity}", fields.len()),
            });
        }
        let mut row = Vec::with_capacity(arity);
        for (column, field) in fields.into_iter().enumerate() {
            let value = field.parse::<i32>().map_err(|_| DatasetError::Csv {
                path: path.to_owned(),
                line: line_index + 1,
                message: format!("column {} is not an i32: `{field}`", column + 1),
            })?;
            if value.to_string() != field {
                return Err(DatasetError::Csv {
                    path: path.to_owned(),
                    line: line_index + 1,
                    message: format!(
                        "column {} is not in canonical decimal form: `{field}`",
                        column + 1
                    ),
                });
            }
            row.push(value);
        }
        rows.push(row);
    }
    RelationData::new(name, arity, rows)
}

fn encode_manifest(manifest: &Manifest) -> String {
    let mut output = format!(
        "mini-linq-dataset\t{FORMAT_VERSION}\nsuite\t{}\ncase\t{}\nscenario\t{}\nscale\t{}\nseed\t{}\ngenerator-version\t{}\nrelations\t{}\n",
        manifest.suite.as_str(),
        manifest.case,
        manifest.scenario.as_str(),
        manifest.scale.as_str(),
        manifest.seed,
        manifest.generator_version,
        manifest.relations.len(),
    );
    for relation in &manifest.relations {
        output.push_str(&format!(
            "relation\t{}\t{}\t{}\t{}\n",
            relation.name, relation.arity, relation.raw_rows, relation.distinct_rows,
        ));
    }
    output
}

fn decode_manifest(path: &Path, text: &str) -> Result<Manifest, DatasetError> {
    if !text.ends_with('\n') {
        return Err(manifest_error(path, 1, "manifest must end with a newline"));
    }
    let lines = text.split_terminator('\n').collect::<Vec<_>>();
    if lines.len() < 8 {
        return Err(manifest_error(
            path,
            lines.len() + 1,
            "manifest is truncated",
        ));
    }
    expect_manifest_line(
        path,
        1,
        lines[0],
        "mini-linq-dataset",
        &FORMAT_VERSION.to_string(),
    )?;
    let suite = match manifest_value(path, 2, lines[1], "suite")? {
        "classic" => Suite::Classic,
        "retail" => Suite::Retail,
        value => return Err(manifest_error(path, 2, format!("unknown suite `{value}`"))),
    };
    let case = manifest_value(path, 3, lines[2], "case")?.to_owned();
    validate_manifest_token("case name", &case)?;
    let scenario = match manifest_value(path, 4, lines[3], "scenario")? {
        "coverage" => Scenario::Coverage,
        "empty-input" => Scenario::EmptyInput,
        "no-match" => Scenario::NoMatch,
        value => {
            return Err(manifest_error(
                path,
                4,
                format!("unknown scenario `{value}`"),
            ));
        }
    };
    let scale = match manifest_value(path, 5, lines[4], "scale")? {
        "tiny" => Scale::Tiny,
        "medium" => Scale::Medium,
        "large" => Scale::Large,
        value => return Err(manifest_error(path, 5, format!("unknown scale `{value}`"))),
    };
    let seed = parse_manifest_number(path, 6, lines[5], "seed")?;
    let generator_version = parse_manifest_number(path, 7, lines[6], "generator-version")?;
    let relation_count: usize = parse_manifest_number(path, 8, lines[7], "relations")?;
    if lines.len() != 8 + relation_count {
        return Err(manifest_error(
            path,
            lines.len() + 1,
            format!(
                "manifest declares {relation_count} relation entries but contains {}",
                lines.len().saturating_sub(8)
            ),
        ));
    }

    let mut relations = Vec::with_capacity(relation_count);
    for (offset, line) in lines[8..].iter().enumerate() {
        let line_number = offset + 9;
        let fields = line.split('\t').collect::<Vec<_>>();
        if fields.len() != 5 || fields[0] != "relation" {
            return Err(manifest_error(
                path,
                line_number,
                "relation entry must be `relation<TAB>name<TAB>arity<TAB>raw<TAB>distinct`",
            ));
        }
        validate_relation_name(fields[1])?;
        let arity = parse_number_field(path, line_number, "arity", fields[2])?;
        let raw_rows = parse_number_field(path, line_number, "raw rows", fields[3])?;
        let distinct_rows = parse_number_field(path, line_number, "distinct rows", fields[4])?;
        if distinct_rows > raw_rows {
            return Err(manifest_error(
                path,
                line_number,
                "distinct cardinality exceeds raw cardinality",
            ));
        }
        relations.push(RelationCardinality {
            name: fields[1].to_owned(),
            arity,
            raw_rows,
            distinct_rows,
        });
    }
    Ok(Manifest {
        suite,
        case,
        scenario,
        scale,
        seed,
        generator_version,
        relations,
    })
}

fn expect_manifest_line(
    path: &Path,
    line: usize,
    actual: &str,
    key: &str,
    value: &str,
) -> Result<(), DatasetError> {
    let actual_value = manifest_value(path, line, actual, key)?;
    if actual_value != value {
        return Err(manifest_error(
            path,
            line,
            format!("expected `{key}\t{value}`"),
        ));
    }
    Ok(())
}

fn manifest_value<'a>(
    path: &Path,
    line: usize,
    actual: &'a str,
    key: &str,
) -> Result<&'a str, DatasetError> {
    let fields = actual.split('\t').collect::<Vec<_>>();
    if fields.len() != 2 || fields[0] != key {
        return Err(manifest_error(
            path,
            line,
            format!("expected `{key}<TAB>value`"),
        ));
    }
    Ok(fields[1])
}

fn parse_manifest_number<T>(
    path: &Path,
    line: usize,
    actual: &str,
    key: &str,
) -> Result<T, DatasetError>
where
    T: std::str::FromStr + ToString,
{
    parse_number_field(path, line, key, manifest_value(path, line, actual, key)?)
}

fn parse_number_field<T>(
    path: &Path,
    line: usize,
    field: &str,
    value: &str,
) -> Result<T, DatasetError>
where
    T: std::str::FromStr + ToString,
{
    let parsed = value.parse::<T>().map_err(|_| {
        manifest_error(
            path,
            line,
            format!("{field} is not a canonical nonnegative integer: `{value}`"),
        )
    })?;
    if parsed.to_string() != value {
        return Err(manifest_error(
            path,
            line,
            format!("{field} is not in canonical decimal form: `{value}`"),
        ));
    }
    Ok(parsed)
}

fn validate_relation_name(value: &str) -> Result<(), DatasetError> {
    validate_manifest_token("relation name", value)?;
    if value.contains('/') || value.contains('\\') {
        return Err(DatasetError::Schema(format!(
            "relation name `{value}` is not a safe dataset filename"
        )));
    }
    Ok(())
}

fn validate_manifest_token(context: &str, value: &str) -> Result<(), DatasetError> {
    if value.is_empty()
        || value.contains('\t')
        || value.contains('\n')
        || value.contains('\r')
        || value.contains('\0')
    {
        return Err(DatasetError::Schema(format!(
            "{context} `{value}` cannot be represented in the dataset manifest"
        )));
    }
    Ok(())
}

fn manifest_error(path: &Path, line: usize, message: impl Into<String>) -> DatasetError {
    DatasetError::Manifest {
        path: path.to_owned(),
        line,
        message: message.into(),
    }
}

#[derive(Debug)]
pub enum DatasetError {
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    InvalidQuery(syn::Error),
    Schema(String),
    Manifest {
        path: PathBuf,
        line: usize,
        message: String,
    },
    Csv {
        path: PathBuf,
        line: usize,
        message: String,
    },
}

impl fmt::Display for DatasetError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io { path, source } => write!(formatter, "{}: {source}", path.display()),
            Self::InvalidQuery(error) => write!(formatter, "invalid workload query: {error}"),
            Self::Schema(message) => formatter.write_str(message),
            Self::Manifest {
                path,
                line,
                message,
            } => write!(
                formatter,
                "{}:{line}: invalid dataset manifest: {message}",
                path.display()
            ),
            Self::Csv {
                path,
                line,
                message,
            } => write!(
                formatter,
                "{}:{line}: invalid relation CSV: {message}",
                path.display()
            ),
        }
    }
}

impl StdError for DatasetError {
    fn source(&self) -> Option<&(dyn StdError + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            Self::InvalidQuery(error) => Some(error),
            Self::Schema(_) | Self::Manifest { .. } | Self::Csv { .. } => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicU64, Ordering};

    use super::*;

    static NEXT_DIRECTORY: AtomicU64 = AtomicU64::new(0);

    fn module() -> cq::Module {
        syn::parse_str(
            "struct Data; relation r#type(c0: i32); relation Edge(c0: i32, c1: i32); answer(x, z) :- r#type(x), Edge(x, z).",
        )
        .unwrap()
    }

    fn config(scenario: Scenario, scale: Scale, seed: u64) -> GenerationConfig {
        GenerationConfig {
            scenario,
            scale,
            seed,
        }
    }

    fn dataset() -> Dataset {
        Dataset::new(
            &module(),
            Suite::Classic,
            "round-trip",
            config(Scenario::Coverage, Scale::Tiny, 17),
            vec![
                RelationData::new("type", 1, vec![vec![2], vec![1], vec![1]]).unwrap(),
                RelationData::new("Edge", 2, vec![vec![1, -3], vec![2, 4]]).unwrap(),
            ],
        )
        .unwrap()
    }

    #[test]
    fn directory_round_trip_preserves_raw_order_duplicates_and_manifest_counts() {
        let temporary = TemporaryDirectory::new();
        let expected = dataset();
        expected.write_to_dir(&module(), &temporary.path).unwrap();
        let actual = Dataset::read_from_dir(&module(), &temporary.path).unwrap();

        assert_eq!(actual, expected);
        assert_eq!(actual.manifest.relations[0].raw_rows, 3);
        assert_eq!(actual.manifest.relations[0].distinct_rows, 2);
        assert_eq!(
            fs::read_to_string(temporary.path.join("input0.csv")).unwrap(),
            "2\n1\n1\n"
        );
        assert!(temporary.path.join("input1.csv").is_file());
        assert!(!temporary.path.join("type.csv").exists());
    }

    #[test]
    fn positional_files_preserve_case_distinct_relation_names() {
        let module: cq::Module = syn::parse_str(
            "struct CaseNames; relation R(c0: i32); relation r(c0: i32); q(x) :- R(x), r(x).",
        )
        .unwrap();
        let expected = Dataset::new(
            &module,
            Suite::Classic,
            "case-names",
            config(Scenario::Coverage, Scale::Tiny, 3),
            vec![
                RelationData::new("R", 1, vec![vec![1]]).unwrap(),
                RelationData::new("r", 1, vec![vec![2]]).unwrap(),
            ],
        )
        .unwrap();
        let temporary = TemporaryDirectory::new();
        expected.write_to_dir(&module, &temporary.path).unwrap();

        assert_eq!(
            fs::read_to_string(temporary.path.join("input0.csv")).unwrap(),
            "1\n"
        );
        assert_eq!(
            fs::read_to_string(temporary.path.join("input1.csv")).unwrap(),
            "2\n"
        );
        assert_eq!(
            Dataset::read_from_dir(&module, &temporary.path).unwrap(),
            expected
        );
    }

    #[test]
    fn schema_and_manifest_drift_are_rejected() {
        let mut wrong_order = dataset();
        wrong_order.relations.swap(0, 1);
        assert!(wrong_order.validate(&module()).is_err());

        let mut stale = dataset();
        stale.relations[0].rows.push(vec![9]);
        assert!(stale.validate(&module()).is_err());

        let dataset = dataset();
        assert!(
            dataset
                .validate_provenance(
                    Suite::Classic,
                    "round-trip",
                    config(Scenario::Coverage, Scale::Tiny, 17),
                )
                .is_ok()
        );
        assert!(
            dataset
                .validate_provenance(
                    Suite::Retail,
                    "round-trip",
                    config(Scenario::Coverage, Scale::Tiny, 17),
                )
                .is_err()
        );
        assert!(
            dataset
                .validate_provenance(
                    Suite::Classic,
                    "other",
                    config(Scenario::Coverage, Scale::Tiny, 17),
                )
                .is_err()
        );
        assert!(
            dataset
                .validate_provenance(
                    Suite::Classic,
                    "round-trip",
                    config(Scenario::EmptyInput, Scale::Tiny, 17),
                )
                .is_err()
        );
        assert!(
            dataset
                .validate_provenance(
                    Suite::Classic,
                    "round-trip",
                    config(Scenario::Coverage, Scale::Medium, 17),
                )
                .is_err()
        );
        assert!(
            dataset
                .validate_provenance(
                    Suite::Classic,
                    "round-trip",
                    config(Scenario::Coverage, Scale::Tiny, 18),
                )
                .is_err()
        );

        let mut mislabeled_empty = dataset.clone();
        mislabeled_empty.manifest.scenario = Scenario::EmptyInput;
        assert!(mislabeled_empty.validate(&module()).is_err());

        let mut mislabeled_no_match = dataset;
        mislabeled_no_match.relations[0].rows.clear();
        mislabeled_no_match.manifest.relations[0] =
            relation_cardinality(&mislabeled_no_match.relations[0]);
        mislabeled_no_match.manifest.scenario = Scenario::NoMatch;
        assert!(mislabeled_no_match.validate(&module()).is_err());
    }

    #[test]
    fn csv_reader_rejects_noncanonical_or_wrong_width_rows() {
        let path = Path::new("R.csv");
        assert!(decode_csv(path, "R", 1, "01\n").is_err());
        assert!(decode_csv(path, "R", 2, "1\n").is_err());
        assert!(decode_csv(path, "R", 1, "1").is_err());
        assert!(decode_csv(path, "R", 1, "\n").is_err());
        assert!(parse_number_field::<usize>(path, 1, "arity", "01").is_err());
        assert!(validate_relation_name("../R").is_err());
    }

    #[test]
    fn result_csv_preserves_and_enforces_materialized_set_order() {
        let temporary = TemporaryDirectory::new();
        let path = temporary.path.join("golden.csv");
        let rows = vec![vec![-1, 9], vec![2, 3]];
        write_result_rows(&path, &rows, 2).unwrap();
        assert_eq!(read_result_rows(&path, 2).unwrap(), rows);

        assert!(write_result_rows(&path, &[vec![2], vec![1]], 1).is_err());
        assert!(write_result_rows(&path, &[vec![1], vec![1]], 1).is_err());
    }

    struct TemporaryDirectory {
        path: PathBuf,
    }

    impl TemporaryDirectory {
        fn new() -> Self {
            let number = NEXT_DIRECTORY.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "mini-linq-dataset-test-{}-{number}",
                std::process::id()
            ));
            fs::create_dir(&path).unwrap();
            Self { path }
        }
    }

    impl Drop for TemporaryDirectory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }
}
