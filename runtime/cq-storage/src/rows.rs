//! Borrow host columns in declaration order, independent of physical reads.
use crate::{Answers, GroupedAnswers};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

/// A borrowed tuple normalized to a tuple of column references.
pub trait Columns {
    type Row: Copy;
    fn columns(self) -> Self::Row;
}
/// Concatenate a grouped host's leading key with its remaining columns.
pub trait Concat<R> {
    type Row: Copy;
    fn concat(self, rest: R) -> Self::Row;
}
/// Convert a borrowed probe tuple to an owned host key only on the direct path.
pub trait OwnKey {
    type Owned;
    fn own_key(&self) -> Self::Owned;
}

macro_rules! columns {
    () => {
        impl Columns for &() { type Row=(); fn columns(self) {} }
        impl OwnKey for () { type Owned=(); fn own_key(&self) {} }
    };
    ($($t:ident:$i:tt),*) => {
        impl<'a, $($t),*> Columns for &'a ($($t,)*) {
            type Row = ($(&'a $t,)*);
            fn columns(self) -> Self::Row { ($(&self.$i,)*) }
        }
        impl<$($t: Clone),*> OwnKey for ($(&$t,)*) {
            type Owned = ($($t,)*);
            fn own_key(&self) -> Self::Owned { ($(self.$i.clone(),)*) }
        }
    };
}
macro_rules! concat {
    ((); ()) => { impl Concat<()> for () { type Row=(); fn concat(self,_:()) {} } };
    (($($a:ident:$i:tt),*); ($($b:ident:$j:tt),*)) => {
        impl<$($a: Copy,)* $($b: Copy,)*> Concat<($($b,)*)> for ($($a,)*) {
            type Row = ($($a,)* $($b,)*);
            fn concat(self, rest: ($($b,)*)) -> Self::Row { let _ = (&self, &rest); ($(self.$i,)* $(rest.$j,)*) }
        }
    };
}
impl Columns for () {
    type Row = ();
    fn columns(self) {}
}
columns!();
columns!(T0:0);
columns!(T0:0,T1:1);
columns!(T0:0,T1:1,T2:2);
columns!(T0:0,T1:1,T2:2,T3:3);
columns!(T0:0,T1:1,T2:2,T3:3,T4:4);
columns!(T0:0,T1:1,T2:2,T3:3,T4:4,T5:5);
columns!(T0:0,T1:1,T2:2,T3:3,T4:4,T5:5,T6:6);
columns!(T0:0,T1:1,T2:2,T3:3,T4:4,T5:5,T6:6,T7:7);
columns!(T0:0,T1:1,T2:2,T3:3,T4:4,T5:5,T6:6,T7:7,T8:8);
columns!(T0:0,T1:1,T2:2,T3:3,T4:4,T5:5,T6:6,T7:7,T8:8,T9:9);
columns!(T0:0,T1:1,T2:2,T3:3,T4:4,T5:5,T6:6,T7:7,T8:8,T9:9,T10:10);
columns!(T0:0,T1:1,T2:2,T3:3,T4:4,T5:5,T6:6,T7:7,T8:8,T9:9,T10:10,T11:11);
concat!((); ());
concat!((); (B0:0));
concat!((A0:0); ());
concat!((); (B0:0,B1:1));
concat!((A0:0); (B0:0));
concat!((A0:0,A1:1); ());
concat!((); (B0:0,B1:1,B2:2));
concat!((A0:0); (B0:0,B1:1));
concat!((A0:0,A1:1); (B0:0));
concat!((A0:0,A1:1,A2:2); ());
concat!((); (B0:0,B1:1,B2:2,B3:3));
concat!((A0:0); (B0:0,B1:1,B2:2));
concat!((A0:0,A1:1); (B0:0,B1:1));
concat!((A0:0,A1:1,A2:2); (B0:0));
concat!((A0:0,A1:1,A2:2,A3:3); ());
concat!((); (B0:0,B1:1,B2:2,B3:3,B4:4));
concat!((A0:0); (B0:0,B1:1,B2:2,B3:3));
concat!((A0:0,A1:1); (B0:0,B1:1,B2:2));
concat!((A0:0,A1:1,A2:2); (B0:0,B1:1));
concat!((A0:0,A1:1,A2:2,A3:3); (B0:0));
concat!((A0:0,A1:1,A2:2,A3:3,A4:4); ());
concat!((); (B0:0,B1:1,B2:2,B3:3,B4:4,B5:5));
concat!((A0:0); (B0:0,B1:1,B2:2,B3:3,B4:4));
concat!((A0:0,A1:1); (B0:0,B1:1,B2:2,B3:3));
concat!((A0:0,A1:1,A2:2); (B0:0,B1:1,B2:2));
concat!((A0:0,A1:1,A2:2,A3:3); (B0:0,B1:1));
concat!((A0:0,A1:1,A2:2,A3:3,A4:4); (B0:0));
concat!((A0:0,A1:1,A2:2,A3:3,A4:4,A5:5); ());
concat!((); (B0:0,B1:1,B2:2,B3:3,B4:4,B5:5,B6:6));
concat!((A0:0); (B0:0,B1:1,B2:2,B3:3,B4:4,B5:5));
concat!((A0:0,A1:1); (B0:0,B1:1,B2:2,B3:3,B4:4));
concat!((A0:0,A1:1,A2:2); (B0:0,B1:1,B2:2,B3:3));
concat!((A0:0,A1:1,A2:2,A3:3); (B0:0,B1:1,B2:2));
concat!((A0:0,A1:1,A2:2,A3:3,A4:4); (B0:0,B1:1));
concat!((A0:0,A1:1,A2:2,A3:3,A4:4,A5:5); (B0:0));
concat!((A0:0,A1:1,A2:2,A3:3,A4:4,A5:5,A6:6); ());
concat!((); (B0:0,B1:1,B2:2,B3:3,B4:4,B5:5,B6:6,B7:7));
concat!((A0:0); (B0:0,B1:1,B2:2,B3:3,B4:4,B5:5,B6:6));
concat!((A0:0,A1:1); (B0:0,B1:1,B2:2,B3:3,B4:4,B5:5));
concat!((A0:0,A1:1,A2:2); (B0:0,B1:1,B2:2,B3:3,B4:4));
concat!((A0:0,A1:1,A2:2,A3:3); (B0:0,B1:1,B2:2,B3:3));
concat!((A0:0,A1:1,A2:2,A3:3,A4:4); (B0:0,B1:1,B2:2));
concat!((A0:0,A1:1,A2:2,A3:3,A4:4,A5:5); (B0:0,B1:1));
concat!((A0:0,A1:1,A2:2,A3:3,A4:4,A5:5,A6:6); (B0:0));
concat!((A0:0,A1:1,A2:2,A3:3,A4:4,A5:5,A6:6,A7:7); ());
concat!((); (B0:0,B1:1,B2:2,B3:3,B4:4,B5:5,B6:6,B7:7,B8:8));
concat!((A0:0); (B0:0,B1:1,B2:2,B3:3,B4:4,B5:5,B6:6,B7:7));
concat!((A0:0,A1:1); (B0:0,B1:1,B2:2,B3:3,B4:4,B5:5,B6:6));
concat!((A0:0,A1:1,A2:2); (B0:0,B1:1,B2:2,B3:3,B4:4,B5:5));
concat!((A0:0,A1:1,A2:2,A3:3); (B0:0,B1:1,B2:2,B3:3,B4:4));
concat!((A0:0,A1:1,A2:2,A3:3,A4:4); (B0:0,B1:1,B2:2,B3:3));
concat!((A0:0,A1:1,A2:2,A3:3,A4:4,A5:5); (B0:0,B1:1,B2:2));
concat!((A0:0,A1:1,A2:2,A3:3,A4:4,A5:5,A6:6); (B0:0,B1:1));
concat!((A0:0,A1:1,A2:2,A3:3,A4:4,A5:5,A6:6,A7:7); (B0:0));
concat!((A0:0,A1:1,A2:2,A3:3,A4:4,A5:5,A6:6,A7:7,A8:8); ());
concat!((); (B0:0,B1:1,B2:2,B3:3,B4:4,B5:5,B6:6,B7:7,B8:8,B9:9));
concat!((A0:0); (B0:0,B1:1,B2:2,B3:3,B4:4,B5:5,B6:6,B7:7,B8:8));
concat!((A0:0,A1:1); (B0:0,B1:1,B2:2,B3:3,B4:4,B5:5,B6:6,B7:7));
concat!((A0:0,A1:1,A2:2); (B0:0,B1:1,B2:2,B3:3,B4:4,B5:5,B6:6));
concat!((A0:0,A1:1,A2:2,A3:3); (B0:0,B1:1,B2:2,B3:3,B4:4,B5:5));
concat!((A0:0,A1:1,A2:2,A3:3,A4:4); (B0:0,B1:1,B2:2,B3:3,B4:4));
concat!((A0:0,A1:1,A2:2,A3:3,A4:4,A5:5); (B0:0,B1:1,B2:2,B3:3));
concat!((A0:0,A1:1,A2:2,A3:3,A4:4,A5:5,A6:6); (B0:0,B1:1,B2:2));
concat!((A0:0,A1:1,A2:2,A3:3,A4:4,A5:5,A6:6,A7:7); (B0:0,B1:1));
concat!((A0:0,A1:1,A2:2,A3:3,A4:4,A5:5,A6:6,A7:7,A8:8); (B0:0));
concat!((A0:0,A1:1,A2:2,A3:3,A4:4,A5:5,A6:6,A7:7,A8:8,A9:9); ());
concat!((); (B0:0,B1:1,B2:2,B3:3,B4:4,B5:5,B6:6,B7:7,B8:8,B9:9,B10:10));
concat!((A0:0); (B0:0,B1:1,B2:2,B3:3,B4:4,B5:5,B6:6,B7:7,B8:8,B9:9));
concat!((A0:0,A1:1); (B0:0,B1:1,B2:2,B3:3,B4:4,B5:5,B6:6,B7:7,B8:8));
concat!((A0:0,A1:1,A2:2); (B0:0,B1:1,B2:2,B3:3,B4:4,B5:5,B6:6,B7:7));
concat!((A0:0,A1:1,A2:2,A3:3); (B0:0,B1:1,B2:2,B3:3,B4:4,B5:5,B6:6));
concat!((A0:0,A1:1,A2:2,A3:3,A4:4); (B0:0,B1:1,B2:2,B3:3,B4:4,B5:5));
concat!((A0:0,A1:1,A2:2,A3:3,A4:4,A5:5); (B0:0,B1:1,B2:2,B3:3,B4:4));
concat!((A0:0,A1:1,A2:2,A3:3,A4:4,A5:5,A6:6); (B0:0,B1:1,B2:2,B3:3));
concat!((A0:0,A1:1,A2:2,A3:3,A4:4,A5:5,A6:6,A7:7); (B0:0,B1:1,B2:2));
concat!((A0:0,A1:1,A2:2,A3:3,A4:4,A5:5,A6:6,A7:7,A8:8); (B0:0,B1:1));
concat!((A0:0,A1:1,A2:2,A3:3,A4:4,A5:5,A6:6,A7:7,A8:8,A9:9); (B0:0));
concat!((A0:0,A1:1,A2:2,A3:3,A4:4,A5:5,A6:6,A7:7,A8:8,A9:9,A10:10); ());
concat!((); (B0:0,B1:1,B2:2,B3:3,B4:4,B5:5,B6:6,B7:7,B8:8,B9:9,B10:10,B11:11));
concat!((A0:0); (B0:0,B1:1,B2:2,B3:3,B4:4,B5:5,B6:6,B7:7,B8:8,B9:9,B10:10));
concat!((A0:0,A1:1); (B0:0,B1:1,B2:2,B3:3,B4:4,B5:5,B6:6,B7:7,B8:8,B9:9));
concat!((A0:0,A1:1,A2:2); (B0:0,B1:1,B2:2,B3:3,B4:4,B5:5,B6:6,B7:7,B8:8));
concat!((A0:0,A1:1,A2:2,A3:3); (B0:0,B1:1,B2:2,B3:3,B4:4,B5:5,B6:6,B7:7));
concat!((A0:0,A1:1,A2:2,A3:3,A4:4); (B0:0,B1:1,B2:2,B3:3,B4:4,B5:5,B6:6));
concat!((A0:0,A1:1,A2:2,A3:3,A4:4,A5:5); (B0:0,B1:1,B2:2,B3:3,B4:4,B5:5));
concat!((A0:0,A1:1,A2:2,A3:3,A4:4,A5:5,A6:6); (B0:0,B1:1,B2:2,B3:3,B4:4));
concat!((A0:0,A1:1,A2:2,A3:3,A4:4,A5:5,A6:6,A7:7); (B0:0,B1:1,B2:2,B3:3));
concat!((A0:0,A1:1,A2:2,A3:3,A4:4,A5:5,A6:6,A7:7,A8:8); (B0:0,B1:1,B2:2));
concat!((A0:0,A1:1,A2:2,A3:3,A4:4,A5:5,A6:6,A7:7,A8:8,A9:9); (B0:0,B1:1));
concat!((A0:0,A1:1,A2:2,A3:3,A4:4,A5:5,A6:6,A7:7,A8:8,A9:9,A10:10); (B0:0));
concat!((A0:0,A1:1,A2:2,A3:3,A4:4,A5:5,A6:6,A7:7,A8:8,A9:9,A10:10,A11:11); ());

/// Flat rows in declaration order; the handle is copied, the columns stay borrowed.
pub trait Rows: Copy {
    type Row: Copy;
    fn rows(self) -> impl Iterator<Item = Self::Row>;
}
impl<T: Rows> Rows for &T {
    type Row = T::Row;
    // Forward the copied handle, without retaining the borrow of that handle.
    #[allow(refining_impl_trait)]
    fn rows(self) -> impl Iterator<Item = T::Row> + use<T> {
        (*self).rows()
    }
}
macro_rules! flat {
    ([$($g:tt)*] $ty:ty) => {
        impl<'a, $($g)*> Rows for &'a $ty where &'a T: Columns, T: 'a {
            type Row = <&'a T as Columns>::Row;
            fn rows(self) -> impl Iterator<Item = Self::Row> { crate::instrument::cursor!(self.iter().map(Columns::columns)) }
        }
    };
}
flat!([T][T]);
flat!([T, const N: usize] [T; N]);
flat!([T] Vec<T>);
flat!([T, S] HashSet<T, S>);
flat!([T] BTreeSet<T>);
flat!([T] Answers<T>);
flat!([T, P] crate::dedup::PrefixOrdered<T, P>);
macro_rules! grouped {
    ([$($g:tt)*] $ty:ty) => {
        impl<'a, $($g)*> Rows for &'a $ty where
            K: 'a, V: 'a, &'a V: Rows, &'a K: Columns,
            <&'a K as Columns>::Row: Concat<<&'a V as Rows>::Row>,
        {
            type Row = <<&'a K as Columns>::Row as Concat<<&'a V as Rows>::Row>>::Row;
            fn rows(self) -> impl Iterator<Item = Self::Row> {
                crate::instrument::cursor!(self.iter().flat_map(|(k, vs)| vs.rows().map(move |v| k.columns().concat(v))))
            }
        }
    };
}
grouped!([K, V, S] HashMap<K, V, S>);
grouped!([K, V] BTreeMap<K, V>);
impl<'a, K: 'a, V: 'a> Rows for &'a GroupedAnswers<K, V>
where
    &'a K: Columns,
    &'a V: Columns,
    <&'a K as Columns>::Row: Concat<<&'a V as Columns>::Row>,
{
    type Row = <<&'a K as Columns>::Row as Concat<<&'a V as Columns>::Row>>::Row;
    fn rows(self) -> impl Iterator<Item = Self::Row> {
        crate::instrument::cursor!(
            self.iter()
                .flat_map(|(k, vs)| vs.iter().map(move |v| k.columns().concat(v.columns()))),
        )
    }
}

impl<'a, K: 'a, V: 'a> Rows for &'a crate::SingleGroup<K, V>
where
    &'a K: Columns,
    &'a V: Columns,
    <&'a K as Columns>::Row: Concat<<&'a V as Columns>::Row>,
{
    type Row = <<&'a K as Columns>::Row as Concat<<&'a V as Columns>::Row>>::Row;
    fn rows(self) -> impl Iterator<Item = Self::Row> {
        crate::instrument::cursor!(self.iter().flat_map(|(key, values)| {
            values
                .iter()
                .map(move |value| key.columns().concat(value.columns()))
        }),)
    }
}
