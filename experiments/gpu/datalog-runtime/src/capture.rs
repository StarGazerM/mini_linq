//! Host-only inspection of the immutable layouts passed to an observed hole.
use std::io::Write;

use crate::{RadixLayout, Result, Word};

pub trait RowSource {
    fn rows(&self) -> Result<Vec<Vec<i128>>>;
    fn len(&self) -> usize;
    fn runs(&self) -> usize;
    fn columns(&self) -> usize;
    /// Same row and column order as `rows`, without a heap allocation per row.
    /// Fails if any field cannot be represented as an unsigned 32-bit code.
    fn write_u32_le(&self, out: &mut dyn Write) -> Result<()>;
}

impl<K, W: Word + 'static> RowSource for RadixLayout<K, (), W, u64, ()> {
    fn rows(&self) -> Result<Vec<Vec<i128>>> {
        self.to_rows()
    }
    fn len(&self) -> usize {
        self.len()
    }
    fn runs(&self) -> usize {
        self.run_count()
    }
    fn columns(&self) -> usize {
        self.packing().len()
    }
    fn write_u32_le(&self, out: &mut dyn Write) -> Result<()> {
        let packing = self.packing();
        let keys = self.to_host()?;
        let mut buf = Vec::with_capacity(64 * 1024);
        for key in keys {
            let row = packing.row(key);
            for column in 0..packing.len() {
                buf.extend_from_slice(&u32::try_from(row.field(column))?.to_le_bytes());
            }
            if buf.len() >= 64 * 1024 {
                out.write_all(&buf)?;
                buf.clear();
            }
        }
        out.write_all(&buf)?;
        Ok(())
    }
}
