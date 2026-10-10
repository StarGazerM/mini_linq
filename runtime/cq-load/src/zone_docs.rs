//! Zone rows use a tuple schema with one policy per declared column. Supported
//! exact types use `MinMax<T>`; borrowed text uses `MinMax<String>` so extrema
//! are owned. All other types can use `NoBounds<T>`, without `Clone`/`Ord` bounds
//! on construction. The schema is explicit: Rust does not specialize unknown
//! user types or silently assign them a different ordering.
//!
//! ```
//! use cq_load::{column, MinMax, NoBounds, ZoneRows};
//! use cq_storage::{BlockRead, Point, PositionalRows};
//! use cq_storage::rows::Rows;
//! type Schema = (MinMax<i64>, MinMax<String>, NoBounds<f64>);
//! let rows = vec![(3, "é".to_string(), f64::NAN), (3, "a".into(), 1.0)];
//! let host = ZoneRows::from_rows::<Schema>(rows, 2048.try_into().unwrap());
//! let found: Vec<_> = (&host).read(column::<0>(), &Point(3))
//!     .map(|id| (&host).row_at(id).unwrap()).collect();
//! assert_eq!(found.len(), 2); // equal keys remain separate occurrences
//! assert_eq!((&host).rows().filter(|r| r.2.is_nan()).count(), 1);
//! ```
//!
//! Batches from `Loader::<Relation>::csv_batches` or `parquet_batches` feed
//! `ZoneRows::from_batches::<Schema, _, _>` directly. Block alignment does not
//! depend on their batch size. No source statistics are assumed compatible;
//! all bounds come from decoded values, including converted Date/Decimal data.
//!
//! Unknown ordered types fall back to exact per-row tests:
//!
//! ```
//! use cq_load::{column, NoBounds, ZoneRows};
//! use cq_storage::{BlockRead, Point};
//! #[derive(PartialEq, Eq, PartialOrd, Ord)]
//! struct UserKey(i64); // need not implement Clone
//! let host = ZoneRows::from_rows::<(NoBounds<UserKey>,)>(
//!     vec![(UserKey(4),)], 2.try_into().unwrap());
//! assert_eq!((&host).read(column::<0>(), &Point(&UserKey(4))).count(), 1);
//! ```
//!
//! A column outside the tuple cannot be read.
//!
//! ```compile_fail
//! use cq_load::{column, MinMax, ZoneRows};
//! use cq_storage::{BlockRead, Point};
//! let host = ZoneRows::from_rows::<(MinMax<i64>,)>(vec![(1,)], 2.try_into().unwrap());
//! (&host).read(column::<1>(), &Point(1)).count();
//! ```
//!
//! Predicates must have the column's exact key type.
//!
//! ```compile_fail
//! use cq_load::{column, MinMax, ZoneRows};
//! use cq_storage::{BlockRead, Point};
//! let host = ZoneRows::from_rows::<(MinMax<i64>,)>(vec![(1,)], 2.try_into().unwrap());
//! (&host).read(column::<0>(), &Point("1")).count();
//! ```
//!
//! Floats cannot acquire exact total-order summaries.
//!
//! ```compile_fail
//! use cq_load::{MinMax, ZoneRows};
//! let host = ZoneRows::from_rows::<(MinMax<f64>,)>(vec![(f64::NAN,)], 2.try_into().unwrap());
//! ```
//!
//! Unordered values can be stored with unavailable statistics, but storage's
//! `Column` contract still requires `Ord`; use `Rows` for their scalar filters.
//!
//! ```compile_fail
//! use cq_load::{column, NoBounds, ZoneRows};
//! use cq_storage::{Column, PositionalRows};
//! let host = ZoneRows::from_rows::<(NoBounds<f64>,)>(vec![(0.0,)], 2.try_into().unwrap());
//! column::<0>().key(&host, (&host).positions().next().unwrap());
//! ```
//!
//! Keys borrowed from the rows or owned metadata cannot outlive the host.
//!
//! ```compile_fail
//! use cq_load::{column, MinMax, ZoneRows};
//! use cq_storage::{BlockRead, PositionalRows};
//! let lo = {
//!     let host = ZoneRows::from_rows::<(MinMax<String>,)>(
//!         vec![(String::from("a"),)], 2.try_into().unwrap());
//!     (&host).bounds(column::<0>(), (&host).blocks().next().unwrap()).unwrap().0
//! };
//! println!("{lo}");
//! ```
//!
//! Schemas must match the declared tuple's columns.
//!
//! ```compile_fail
//! use cq_load::{MinMax, ZoneRows};
//! ZoneRows::from_rows::<(MinMax<i64>,)>(vec![(String::from("1"),)], 2.try_into().unwrap());
//! ```
