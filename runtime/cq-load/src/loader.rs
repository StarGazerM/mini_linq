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
        crate::read_parquet::<S>(path, options)
    }

    /// Group a Parquet stream by its first `PREFIX` declared columns.
    #[cfg(feature = "parquet")]
    pub fn parquet_hash<const PREFIX: usize>(
        path: impl AsRef<Path>,
        options: crate::ParquetOptions,
    ) -> Result<HashMap<S::Key, Vec<S::Value>>>
    where
        S: Group<PREFIX>,
        S::Key: Eq + Hash,
    {
        collect_hash::<S, PREFIX>(Self::parquet_batches(path, options)?)
    }

    /// Group a Parquet stream into ordered leading-prefix keys.
    #[cfg(feature = "parquet")]
    pub fn parquet_btree<const PREFIX: usize>(
        path: impl AsRef<Path>,
        options: crate::ParquetOptions,
    ) -> Result<BTreeMap<S::Key, Vec<S::Value>>>
    where
        S: Group<PREFIX>,
        S::Key: Ord,
    {
        collect_btree::<S, PREFIX>(Self::parquet_batches(path, options)?)
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
