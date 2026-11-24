//! Explicit byte-addressable item layouts for grouped index runs.
//!
//! A layout is selected by the provider type, never by inspecting a Rust row
//! type. A tuple of vectors stores independent columns; a vector stores one
//! opaque item column. Row materialization happens only at `item`/`into_rows`.

/// Formal rule IX-ItemColumns: positions name the same complete row in every column.
///
/// `push` appends exactly one row, `extend_range` appends the source occurrence
/// interval in order, and `item(i)` reconstructs exactly the row at position i.
/// Every component column has `len()` entries. Implementations must preserve
/// multiplicity and cannot deduplicate or reorder items.
pub trait ItemColumns: Sized {
    type Row: Copy;

    fn with_capacity(rows: usize) -> Self;
    fn len(&self) -> usize;
    fn is_empty(&self) -> bool {
        self.len() == 0
    }
    fn push(&mut self, row: Self::Row);
    fn item(&self, position: usize) -> Self::Row;
    fn extend_range(&mut self, source: &Self, range: std::ops::Range<usize>);

    /// A contiguous row view is available only when that is the chosen layout.
    fn row_slice(&self) -> Option<&[Self::Row]> {
        None
    }

    fn into_rows(self) -> Vec<Self::Row> {
        (0..self.len())
            .map(|position| self.item(position))
            .collect()
    }
}

impl<V: Copy> ItemColumns for Vec<V> {
    type Row = V;

    fn with_capacity(rows: usize) -> Self {
        Vec::with_capacity(rows)
    }
    fn len(&self) -> usize {
        Vec::len(self)
    }
    fn push(&mut self, row: V) {
        Vec::push(self, row);
    }
    fn item(&self, position: usize) -> V {
        self[position]
    }
    fn extend_range(&mut self, source: &Self, range: std::ops::Range<usize>) {
        self.extend_from_slice(&source[range]);
    }
    fn row_slice(&self) -> Option<&[V]> {
        Some(self)
    }
    fn into_rows(self) -> Vec<V> {
        self
    }
}

macro_rules! tuple_columns {
    ($($T:ident:$field:tt),+) => {
        impl<$($T: Copy),+> ItemColumns for ($(Vec<$T>,)+) {
            type Row = ($($T,)+);

            fn with_capacity(rows: usize) -> Self {
                ($(Vec::<$T>::with_capacity(rows),)+)
            }
            fn len(&self) -> usize {
                self.0.len()
            }
            fn push(&mut self, row: Self::Row) {
                $(self.$field.push(row.$field);)+
            }
            fn item(&self, position: usize) -> Self::Row {
                ($(self.$field[position],)+)
            }
            fn extend_range(&mut self, source: &Self, range: std::ops::Range<usize>) {
                $(self.$field.extend_from_slice(&source.$field[range.clone()]);)+
            }
        }

        impl<$($T: Copy),+> ColumnRow for ($($T,)+) {
            type Columns = ($(Vec<$T>,)+);
        }
    };
}

/// A tuple row with a tuple-of-vectors layout: one vector per field.
pub trait ColumnRow: Copy {
    type Columns: ItemColumns<Row = Self>;
}

/// The column-wise layout of any tuple row, `(Vec<A>, Vec<B>, ...)`, named
/// without its arity. This is the `items columns` layout recorded by index
/// selection; rustc derives the tuple vectors from the row type, so the
/// compiler never spells or inspects field types.
pub struct TupleColumns<V: ColumnRow>(V::Columns);

impl<V: ColumnRow> ItemColumns for TupleColumns<V> {
    type Row = V;

    fn with_capacity(rows: usize) -> Self {
        Self(V::Columns::with_capacity(rows))
    }
    fn len(&self) -> usize {
        self.0.len()
    }
    fn push(&mut self, row: V) {
        self.0.push(row);
    }
    #[inline]
    fn item(&self, position: usize) -> V {
        self.0.item(position)
    }
    fn extend_range(&mut self, source: &Self, range: std::ops::Range<usize>) {
        self.0.extend_range(&source.0, range);
    }
    fn into_rows(self) -> Vec<V> {
        self.0.into_rows()
    }
}

impl<V: ColumnRow> TupleColumns<V> {
    /// Borrow the per-field vectors for selective field reads.
    pub fn columns(&self) -> &V::Columns {
        &self.0
    }
}

tuple_columns!(A:0);
tuple_columns!(A:0, B:1);
tuple_columns!(A:0, B:1, C:2);
tuple_columns!(A:0, B:1, C:2, D:3);
tuple_columns!(A:0, B:1, C:2, D:3, E:4);
tuple_columns!(A:0, B:1, C:2, D:3, E:4, F:5);
tuple_columns!(A:0, B:1, C:2, D:3, E:4, F:5, G:6);
tuple_columns!(A:0, B:1, C:2, D:3, E:4, F:5, G:6, H:7);
tuple_columns!(A:0, B:1, C:2, D:3, E:4, F:5, G:6, H:7, I:8);
tuple_columns!(A:0, B:1, C:2, D:3, E:4, F:5, G:6, H:7, I:8, J:9);
tuple_columns!(A:0, B:1, C:2, D:3, E:4, F:5, G:6, H:7, I:8, J:9, K:10);
tuple_columns!(A:0, B:1, C:2, D:3, E:4, F:5, G:6, H:7, I:8, J:9, K:10, L:11);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tuple_columns_equal_the_tuple_of_vectors_layout() {
        let rows = [(1_i64, 'a', 3_u8), (2, 'b', 4), (5, 'c', 6)];
        let mut columns = TupleColumns::<(i64, char, u8)>::with_capacity(rows.len());
        for row in rows {
            columns.push(row);
        }
        let mut copied = TupleColumns::with_capacity(2);
        copied.extend_range(&columns, 1..3);
        assert_eq!(columns.columns().1, vec!['a', 'b', 'c']);
        assert_eq!((columns.len(), columns.item(2)), (3, rows[2]));
        assert!(columns.row_slice().is_none());
        assert_eq!(copied.into_rows(), rows[1..].to_vec());
        let run = crate::RadixRun::<u64, (i64, char), TupleColumns<_>>::from_column_rows(
            [(2, (1, 'x')), (1, (2, 'y')), (2, (3, 'z'))],
            4,
        );
        assert_eq!(
            crate::IndexRead::lookup(&&run, &2).collect::<Vec<_>>(),
            [(1, 'x'), (3, 'z')]
        );
    }
}
