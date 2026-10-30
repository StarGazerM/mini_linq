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
    };
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
