use std::collections::{BTreeMap, HashMap};
use std::hash::Hash;
use std::marker::PhantomData;
use std::path::Path;

use crate::{Csv, CsvOptions, Relation, Result};

/// Canonicalize complete relation rows in place before passing them to a query.
///
/// Formal rule LD-RelationSet: sorting and adjacent deduplication preserve
/// full-row support and establish the unconditional input-set contract.
/// `Ord` equality must be the relation's equality. No row is cloned and no
/// additional row buffer is allocated.
pub fn canonicalize<T: Ord>(rows: &mut Vec<T>) {
    rows.sort_unstable();
    rows.dedup();
}

/// Formal rule IO-CpuLoadKey: only a hash-matching verified key retained by
/// equality-preserving decoded columns discharges load-time canonicalization.
#[cfg(feature = "parquet")]
pub(crate) fn canonicalize_unless_certified<T: Ord>(rows: &mut Vec<T>, certified: bool) {
    if !certified {
        canonicalize(rows);
    }
}

/// Split a declared row at a leading column prefix, without cloning fields.
///
/// The declaration macro implements every prefix from zero through the arity.
/// Keys satisfy the chosen collection's equality/order bounds; payloads must
/// implement `Ord` so grouped loaders can deduplicate complete rows in place.
/// Raw batch decoders remain available for unordered types such as `f64`.
pub trait Group<const PREFIX: usize>: Relation {
    type Key;
    type Value;

    fn split(row: Self::Row) -> (Self::Key, Self::Value);
}

/// A typed source reader generated from a relation's authored declaration.
pub struct Loader<S>(PhantomData<S>);

impl<S: Relation> Loader<S> {
    /// Open a headerless or headed CSV stream with bounded row batches.
    pub fn csv_batches(path: impl AsRef<Path>, options: CsvOptions) -> Result<Csv<S>> {
        Csv::open(path, options)
    }

    /// Read a full-row set into the dispatch rule's tuple-vector host.
    pub fn csv(path: impl AsRef<Path>, options: CsvOptions) -> Result<Vec<S::Row>>
    where
        S::Row: Ord,
    {
        // DECODE ?: read_csv reconstructs the entire host relation; this generic convenience API records no final consumer.
        crate::read_csv::<S>(path, options)
    }

    /// Group a CSV stream by its first `PREFIX` declared columns.
    pub fn csv_hash<const PREFIX: usize>(
        path: impl AsRef<Path>,
        options: CsvOptions,
    ) -> Result<HashMap<S::Key, Vec<S::Value>>>
    where
        S: Group<PREFIX>,
        S::Key: Eq + Hash,
        S::Value: Ord,
    {
        // DECODE V: CSV row decoding precedes hash grouping; P4 owns retained index keys.
        collect_hash::<S, PREFIX>(Self::csv_batches(path, options)?)
    }

    /// Group a CSV stream into ordered leading-prefix keys.
    pub fn csv_btree<const PREFIX: usize>(
        path: impl AsRef<Path>,
        options: CsvOptions,
    ) -> Result<BTreeMap<S::Key, Vec<S::Value>>>
    where
        S: Group<PREFIX>,
        S::Key: Ord,
        S::Value: Ord,
    {
        // DECODE V: CSV row decoding precedes ordered grouping; P4 owns retained index keys.
        collect_btree::<S, PREFIX>(Self::csv_batches(path, options)?)
    }

    /// Open a Parquet stream; only the runtime's `parquet` feature is needed.
    #[cfg(feature = "parquet")]
    pub fn parquet_batches(
        path: impl AsRef<Path>,
        options: crate::ParquetOptions,
    ) -> Result<crate::Parquet<S>> {
        crate::Parquet::open(path, options)
    }

    /// Read Arrow batches and canonicalize the complete declared rows once.
    #[cfg(feature = "parquet")]
    pub fn parquet(path: impl AsRef<Path>, options: crate::ParquetOptions) -> Result<Vec<S::Row>>
    where
        S::Row: Ord,
    {
        // DECODE ?: read_parquet reconstructs a tuple-vector host but this generic loader records no publication or Rust-read boundary.
        crate::read_parquet::<S>(path, options)
    }

    /// Select a preparation manifest explicitly; absent/invalid evidence dedups.
    #[cfg(feature = "parquet")]
    pub fn parquet_with_manifest(
        path: impl AsRef<Path>,
        options: crate::ParquetOptions,
        manifest: &Path,
    ) -> Result<Vec<S::Row>>
    where
        S::Row: Ord,
    {
        crate::read_parquet_with_manifest::<S>(path, options, manifest)
    }
}

impl<S: crate::InternedRelation> Loader<S> {
    /// Encode owned source rows once, retaining sorted dictionaries separately.
    ///
    /// Useful for in-memory fixtures and callers with an existing source reader.
    /// String fields in the source schema become `StringCode` fields only where
    /// the authored relation explicitly declares that type.
    pub fn intern(
        rows: Vec<<S::Source as Relation>::Row>,
        dictionaries: &mut crate::Dictionaries,
    ) -> Vec<S::Row>
    where
        S::Row: Ord,
    {
        let mut rows = S::intern(rows, dictionaries);
        canonicalize(&mut rows);
        rows
    }

    /// Borrow source string cells directly into per-column dictionaries and
    /// retain only temporary integer codes in rows during loading.
    ///
    /// Finalization sorts distinct entries and remaps codes, making them
    /// independent of batch size, source order and hash-table layout. Source
    /// errors leave dictionaries unchanged. Retain `dictionaries` for literals
    /// and publication.
    #[cfg(feature = "parquet")]
    pub fn parquet_interned(
        path: impl AsRef<Path>,
        options: crate::ParquetOptions,
        dictionaries: &mut crate::Dictionaries,
    ) -> Result<Vec<S::Row>>
    where
        S::Row: Ord,
    {
        let path = path.as_ref();
        Self::parquet_interned_with_manifest(path, options, dictionaries, &crate::set_manifest::manifest_path(path))
    }

    /// Intern with a caller-selected certificate; dictionaries retain the same
    /// transactional publication and exact sorted-code semantics.
    #[cfg(feature = "parquet")]
    pub fn parquet_interned_with_manifest(
        path: impl AsRef<Path>,
        options: crate::ParquetOptions,
        dictionaries: &mut crate::Dictionaries,
        manifest: &Path,
    ) -> Result<Vec<S::Row>>
    where
        S::Row: Ord,
    {
        let mut source = crate::Parquet::<S::Source>::open_certified(
            path.as_ref(), options, S::Source::COLUMNS, manifest, S::KEY_COLUMNS,
        )?;
        let certified_set = source.certified_set();
        let mut state = S::state();
        let mut rows = Vec::new();
        while let Some(batch) = source.next_with(|batch| S::decode(batch, &mut state)) {
            rows.extend(batch?);
        }
        S::finish(&mut rows, state, dictionaries);
        canonicalize_unless_certified(&mut rows, certified_set);
        Ok(rows)
    }
}

fn collect_hash<S, const PREFIX: usize>(
    batches: impl Iterator<Item = Result<Vec<S::Row>>>,
) -> Result<HashMap<S::Key, Vec<S::Value>>>
where
    S: Group<PREFIX>,
    S::Key: Eq + Hash,
    S::Value: Ord,
{
    let mut groups = HashMap::<_, Vec<_>>::new();
    for batch in batches {
        for row in batch? {
            let (key, value) = S::split(row);
            groups.entry(key).or_default().push(value);
        }
    }
    for values in groups.values_mut() {
        canonicalize(values);
    }
    Ok(groups)
}

fn collect_btree<S, const PREFIX: usize>(
    batches: impl Iterator<Item = Result<Vec<S::Row>>>,
) -> Result<BTreeMap<S::Key, Vec<S::Value>>>
where
    S: Group<PREFIX>,
    S::Key: Ord,
    S::Value: Ord,
{
    let mut groups = BTreeMap::<_, Vec<_>>::new();
    for batch in batches {
        for row in batch? {
            let (key, value) = S::split(row);
            groups.entry(key).or_default().push(value);
        }
    }
    for values in groups.values_mut() {
        canonicalize(values);
    }
    Ok(groups)
}
