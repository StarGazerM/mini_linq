use std::collections::{BTreeMap, HashMap};
use std::hash::Hash;
use std::marker::PhantomData;
use std::path::Path;

use crate::{Csv, CsvOptions, Relation, Result};

/// Split a declared row at a leading column prefix, without cloning fields.
///
/// The declaration macro implements every prefix from zero through the arity.
/// Key constraints belong to the selected collection, so an unordered payload
/// (including `f64`) does not prevent grouping by an ordered leading column.
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

    /// Read rows directly into the dispatch rule's tuple-vector host.
    pub fn csv(path: impl AsRef<Path>, options: CsvOptions) -> Result<Vec<S::Row>> {
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
    {
        // DECODE V: CSV batch decoding precedes hash grouping; P4 builds indexes over retained representations and P5 loads columnar scans directly.
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
    {
        // DECODE V: CSV batch decoding precedes ordered grouping; P4 retains encoded index keys and P5 avoids eager logical scan tuples.
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

    /// Read Arrow batches into the dispatch rule's tuple-vector host.
    #[cfg(feature = "parquet")]
    pub fn parquet(path: impl AsRef<Path>, options: crate::ParquetOptions) -> Result<Vec<S::Row>> {
        // DECODE ?: read_parquet reconstructs a tuple-vector host but this generic loader records no publication or Rust-read boundary.
        crate::read_parquet::<S>(path, options)
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
    ) -> Vec<S::Row> {
        S::intern(rows, dictionaries)
    }

    /// Load input-only source rows, prepare sorted per-column dictionaries, and
    /// return encoded relation rows. Temporary source strings are then dropped.
    ///
    /// All source batches are loaded before interning so codes are independent
    /// of batch size and source row order. A source error leaves dictionaries
    /// unchanged. Retain `dictionaries` for literal translation and publication.
    #[cfg(feature = "parquet")]
    pub fn parquet_interned(
        path: impl AsRef<Path>,
        options: crate::ParquetOptions,
        dictionaries: &mut crate::Dictionaries,
    ) -> Result<Vec<S::Row>> {
        let rows = crate::read_parquet::<S::Source>(path, options)?;
        Ok(Self::intern(rows, dictionaries))
    }
}

fn collect_hash<S, const PREFIX: usize>(
    batches: impl Iterator<Item = Result<Vec<S::Row>>>,
) -> Result<HashMap<S::Key, Vec<S::Value>>>
where
    S: Group<PREFIX>,
    S::Key: Eq + Hash,
{
    let mut groups = HashMap::<_, Vec<_>>::new();
    for batch in batches {
        for row in batch? {
            let (key, value) = S::split(row);
            groups.entry(key).or_default().push(value);
        }
    }
    Ok(groups)
}

fn collect_btree<S, const PREFIX: usize>(
    batches: impl Iterator<Item = Result<Vec<S::Row>>>,
) -> Result<BTreeMap<S::Key, Vec<S::Value>>>
where
    S: Group<PREFIX>,
    S::Key: Ord,
{
    let mut groups = BTreeMap::<_, Vec<_>>::new();
    for batch in batches {
        for row in batch? {
            let (key, value) = S::split(row);
            groups.entry(key).or_default().push(value);
        }
    }
    Ok(groups)
}
