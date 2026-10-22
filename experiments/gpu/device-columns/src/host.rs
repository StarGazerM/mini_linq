use crate::{
    Bounds, Buffer, Codec, ColumnView, DecodeValue, Dictionary, EncodeValue, EncodedColumn, Pool,
    Result, decode_value, encode_value, failure,
};
use cuda_core::DeviceCopy;
use std::{marker::PhantomData, sync::Arc};

/// Representation markers are read from explicitly recorded derived types.
/// Host values instead carry the codec selected when they were loaded.
pub trait Representation {
    const CODEC: Codec;
}
pub struct Plain;
pub struct For;
pub struct Delta;
pub struct DictionaryCodec;
impl Representation for Plain {
    const CODEC: Codec = Codec::Plain;
}
impl Representation for For {
    const CODEC: Codec = Codec::For;
}
impl Representation for Delta {
    const CODEC: Codec = Codec::Delta;
}
impl Representation for DictionaryCodec {
    const CODEC: Codec = Codec::Dictionary;
}

pub trait RowCodec<'data>: Sized {
    const WIDTH: usize;
    fn encode(&'data self, dictionary: &mut Dictionary<'data>, output: &mut [i64]) -> Result<()>;
    fn decode(values: &[i64], dictionary: &Dictionary<'data>) -> Result<Self>;
}
impl RowCodec<'_> for () {
    const WIDTH: usize = 0;
    fn encode(&self, _: &mut Dictionary<'_>, _: &mut [i64]) -> Result<()> {
        Ok(())
    }
    fn decode(_: &[i64], _: &Dictionary<'_>) -> Result<Self> {
        Ok(())
    }
}
macro_rules! tuple_codec {
    ($($T:ident:$i:tt),+) => {
        impl<'data, $($T),+> RowCodec<'data> for ($($T,)+)
        where $($T: DecodeValue<'data>, &'data $T: EncodeValue<'data>, $T: 'data),+ {
            const WIDTH: usize = [$(stringify!($T)),+].len();
            fn encode(&'data self, dictionary: &mut Dictionary<'data>, output: &mut [i64]) -> Result<()> {
                $(output[$i] = encode_value(&self.$i, dictionary)?;)+ Ok(())
            }
            fn decode(values: &[i64], dictionary: &Dictionary<'data>) -> Result<Self> {
                Ok(($(decode_value::<$T>(values[$i], dictionary)?,)+))
            }
        }
    };
}
tuple_codec!(A:0);
tuple_codec!(A:0,B:1);
tuple_codec!(A:0,B:1,C:2);
tuple_codec!(A:0,B:1,C:2,D:3);
tuple_codec!(A:0,B:1,C:2,D:3,E:4);
tuple_codec!(A:0,B:1,C:2,D:3,E:4,F:5);
tuple_codec!(A:0,B:1,C:2,D:3,E:4,F:5,G:6);
tuple_codec!(A:0,B:1,C:2,D:3,E:4,F:5,G:6,H:7);
tuple_codec!(A:0,B:1,C:2,D:3,E:4,F:5,G:6,H:7,I:8);
tuple_codec!(A:0,B:1,C:2,D:3,E:4,F:5,G:6,H:7,I:8,J:9);
tuple_codec!(A:0,B:1,C:2,D:3,E:4,F:5,G:6,H:7,I:8,J:9,K:10);

/// Loader encoding for owned relations: string bytes are retained by the
/// dataset dictionary, so dropping the temporary source rows is safe.
pub trait OwnedEncodeValue {
    fn encode_owned(&self, dictionary: &mut Dictionary<'static>) -> Result<i64>;
}
macro_rules! owned_scalar {
    ($($T:ty),+) => {$(
        impl OwnedEncodeValue for $T {
            fn encode_owned(&self, dictionary: &mut Dictionary<'static>) -> Result<i64> {
                encode_value(*self, dictionary)
            }
        }
    )+};
}
owned_scalar!(
    i8, i16, i32, i64, i128, isize, u8, u16, u32, u64, u128, usize, bool
);
impl OwnedEncodeValue for String {
    fn encode_owned(&self, dictionary: &mut Dictionary<'static>) -> Result<i64> {
        dictionary.intern_owned(self)
    }
}
impl OwnedEncodeValue for &str {
    fn encode_owned(&self, dictionary: &mut Dictionary<'static>) -> Result<i64> {
        dictionary.intern_owned(self)
    }
}
pub trait OwnedRowCodec {
    const WIDTH: usize;
    fn encode_owned(&self, dictionary: &mut Dictionary<'static>, output: &mut [i64]) -> Result<()>;
}
impl OwnedRowCodec for () {
    const WIDTH: usize = 0;
    fn encode_owned(&self, _: &mut Dictionary<'static>, _: &mut [i64]) -> Result<()> {
        Ok(())
    }
}
macro_rules! owned_tuple {
    ($($T:ident:$i:tt),+) => {
        impl<$($T: OwnedEncodeValue),+> OwnedRowCodec for ($($T,)+) {
            const WIDTH: usize = [$(stringify!($T)),+].len();
            fn encode_owned(&self, dictionary: &mut Dictionary<'static>, output: &mut [i64]) -> Result<()> {
                $(output[$i] = self.$i.encode_owned(dictionary)?;)+ Ok(())
            }
        }
    };
}
owned_tuple!(A:0);
owned_tuple!(A:0,B:1);
owned_tuple!(A:0,B:1,C:2);
owned_tuple!(A:0,B:1,C:2,D:3);
owned_tuple!(A:0,B:1,C:2,D:3,E:4);
owned_tuple!(A:0,B:1,C:2,D:3,E:4,F:5);
owned_tuple!(A:0,B:1,C:2,D:3,E:4,F:5,G:6);
owned_tuple!(A:0,B:1,C:2,D:3,E:4,F:5,G:6,H:7);
owned_tuple!(A:0,B:1,C:2,D:3,E:4,F:5,G:6,H:7,I:8);
owned_tuple!(A:0,B:1,C:2,D:3,E:4,F:5,G:6,H:7,I:8,J:9);
owned_tuple!(A:0,B:1,C:2,D:3,E:4,F:5,G:6,H:7,I:8,J:9,K:10);
owned_tuple!(A:0,B:1,C:2,D:3,E:4,F:5,G:6,H:7,I:8,J:9,K:10,L:11);
tuple_codec!(A:0,B:1,C:2,D:3,E:4,F:5,G:6,H:7,I:8,J:9,K:10,L:11);

#[doc(hidden)]
#[derive(Clone, Copy, Default)]
#[repr(C)]
pub struct OutputCount {
    pub rows: u64,
    pub error: u32,
    pub padding: u32,
}
unsafe impl DeviceCopy for OutputCount {}

/// Internal immutable storage shared by projections and typed host values.
/// The runtime constructs this only after device writes have been submitted on
/// the retained pool's stream. Status owners keep ancestor failures observable.
#[doc(hidden)]
#[derive(Clone)]
pub struct Table {
    pub columns: Vec<Arc<EncodedColumn>>,
    pub views: Arc<Buffer<ColumnView>>,
    pub metadata: Arc<Buffer<OutputCount>>,
    pub capacity: usize,
    pub bounds: Vec<Bounds>,
    pub statuses: Vec<Arc<Buffer<OutputCount>>>,
}

/// An immutable columnar host retaining its device allocations. The lifetime
/// tracks borrowed string values, never Runtime or Execution. `C` records the
/// derived representation; loaded host columns carry their own actual codecs.
pub struct DeviceColumns<'data, Row, C = Plain> {
    table: Table,
    dictionary: Arc<Dictionary<'data>>,
    pool: Arc<Pool>,
    marker: PhantomData<(Row, C)>,
}
impl<Row, C> Clone for DeviceColumns<'_, Row, C> {
    fn clone(&self) -> Self {
        Self {
            table: self.table.clone(),
            dictionary: self.dictionary.clone(),
            pool: self.pool.clone(),
            marker: PhantomData,
        }
    }
}
impl<Row: OwnedRowCodec> DeviceColumns<'static, Row> {
    /// Build a host once from temporary owned rows. The supplied dataset
    /// dictionary assigns IDs shared by every relation loaded with it.
    pub fn from_rows(
        pool: Arc<Pool>,
        rows: &[Row],
        dictionary: &mut Dictionary<'static>,
        codecs: &[Codec],
    ) -> Result<Self> {
        if rows.len() > u32::MAX as usize {
            return Err(failure(
                "CQ_GPU_OUTPUT_ROWS",
                "relation cardinality exceeds u32::MAX",
            ));
        }
        if codecs.len() != Row::WIDTH {
            return Err(failure("CQ_GPU_SCHEMA", "one codec is required per column"));
        }
        let mut raw: Vec<Vec<i64>> = (0..Row::WIDTH)
            .map(|_| Vec::with_capacity(rows.len()))
            .collect();
        let mut values = vec![0; Row::WIDTH];
        for row in rows {
            row.encode_owned(dictionary, &mut values)?;
            for (column, &value) in raw.iter_mut().zip(&values) {
                column.push(value);
            }
        }
        let columns = raw
            .iter()
            .zip(codecs)
            .map(|(values, &codec)| {
                EncodedColumn::from_host(pool.clone(), values, codec).map(Arc::new)
            })
            .collect::<Result<Vec<_>>>()?;
        let bounds = columns.iter().map(|column| column.bounds()).collect();
        let host_views: Vec<_> = columns
            .iter()
            .map(|column| unsafe { column.view() })
            .collect();
        let views = Arc::new(Buffer::from_host(pool.clone(), &host_views)?);
        let metadata = Arc::new(Buffer::from_host(
            pool.clone(),
            &[OutputCount {
                rows: rows.len() as u64,
                error: 0,
                padding: 0,
            }],
        )?);
        Ok(Self {
            table: Table {
                columns,
                views,
                metadata,
                capacity: rows.len(),
                bounds,
                statuses: Vec::new(),
            },
            dictionary: Arc::new(dictionary.clone()),
            pool,
            marker: PhantomData,
        })
    }
}
impl<'data, Row, C> DeviceColumns<'data, Row, C> {
    /// Construct a completed device result without transferring data.
    ///
    /// # Safety
    /// All columns, views and metadata must agree on schema/capacity, belong to
    /// `pool`, and be initialized in stream order. Every raw count/view pointer
    /// must remain valid through an owner in `table`. Bounds must enclose all
    /// values, dictionary IDs must share `dictionary`, and statuses must retain
    /// failures from every consumed ancestor.
    pub unsafe fn from_parts(
        table: Table,
        dictionary: Arc<Dictionary<'data>>,
        pool: Arc<Pool>,
    ) -> Self {
        Self {
            table,
            dictionary,
            pool,
            marker: PhantomData,
        }
    }
    #[doc(hidden)]
    pub fn storage(&self) -> &Table {
        &self.table
    }
    pub fn dictionary(&self) -> &Dictionary<'data> {
        &self.dictionary
    }
    /// Share this resident relation with a common, compatible dictionary.
    /// The replacement must contain the complete existing ID prefix and retain
    /// borrowed text lifetimes. Columns, statuses, pool, stream and context are
    /// unchanged; no tuple remapping, copying or device transfer occurs.
    pub fn with_dictionary(&self, dictionary: Arc<Dictionary<'data>>) -> Result<Self> {
        self.dictionary.check_extension(&dictionary)?;
        Ok(Self {
            table: self.table.clone(),
            dictionary,
            pool: self.pool.clone(),
            marker: PhantomData,
        })
    }
    pub fn pool(&self) -> &Arc<Pool> {
        &self.pool
    }
    pub fn capacity(&self) -> usize {
        self.table.capacity
    }
    pub fn raw_bytes(&self) -> usize {
        self.table.capacity * self.table.columns.len() * 8
    }
    pub fn encoded_bytes(&self) -> usize {
        self.table.columns.iter().map(|c| c.bytes()).sum()
    }
    pub fn column_bytes(&self) -> Vec<usize> {
        self.table.columns.iter().map(|c| c.bytes()).collect()
    }
    /// Read the logical row count after checking all retained device statuses.
    /// Transfers only metadata; unlike `to_host`, allocates no decoded columns.
    pub fn checked_len(&self) -> Result<usize> {
        checked_count(&self.table)
    }
    /// Download on explicit demand, checking this result and all ancestor
    /// statuses first. No query binding or chaining operation calls this.
    pub fn to_host(&self) -> Result<Vec<Row>>
    where
        Row: RowCodec<'data>,
    {
        let count = checked_count(&self.table)?;
        let columns = self
            .table
            .columns
            .iter()
            .map(|column| column.decoded()?.to_host_prefix(count))
            .collect::<Result<Vec<_>>>()?;
        let mut values = vec![0; columns.len()];
        let mut rows = Vec::with_capacity(count);
        for row in 0..count {
            for (index, column) in columns.iter().enumerate() {
                values[index] = column[row];
            }
            rows.push(Row::decode(&values, &self.dictionary)?);
        }
        Ok(rows)
    }
}

#[doc(hidden)]
pub fn checked_count(table: &Table) -> Result<usize> {
    let mut count = 0;
    for status in table
        .statuses
        .iter()
        .chain(std::iter::once(&table.metadata))
    {
        let record = status.to_host()?[0];
        let named = match record.error {
            0 => None,
            1 => Some((
                "CQ_GPU_OUTPUT_OVERFLOW",
                "logical output count exceeded u64",
            )),
            2 => Some((
                "CQ_GPU_EXPANSION_OVERFLOW",
                "tile expansion exceeded u32 offset domain",
            )),
            3 => Some((
                "CQ_GPU_SET_FULL",
                "exact device set exhausted its bounded probe table",
            )),
            4 => Some((
                "CQ_GPU_OUTPUT_CAPACITY",
                "logical output exceeds allocated capacity",
            )),
            5 => Some(("CQ_GPU_OUTPUT_ROWS", "output cardinality exceeds u32::MAX")),
            6 => Some((
                "CQ_GPU_MEMBERSHIP_DOMAIN",
                "recorded membership bitmap budget exceeded",
            )),
            7 => Some(("CQ_GPU_RUN_BUILD", "invalid device run count or key domain")),
            _ => Some(("CQ_GPU_KERNEL_ERROR", "unknown device error code")),
        };
        if let Some((code, detail)) = named {
            return Err(failure(code, detail));
        }
        count = record.rows;
    }
    if count > table.capacity as u64 {
        return Err(failure(
            "CQ_GPU_OUTPUT_CAPACITY",
            "logical count exceeds capacity",
        ));
    }
    Ok(count as usize)
}
