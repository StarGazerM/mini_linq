//! Byte layout of the checked GPU dispatch's temporary shared storage.
//!
//! Choices and unions retain every recorded chain. This is an allocation
//! contract, not a schedule transformation or a scratch-reuse policy.

use cq_lang_sequence::{DispatchShape, ResourceAllocation};
use proc_macro2::Span;

const LANES: usize = 256;

pub(crate) fn code_bytes(ty: &syn::Type, coded: bool) -> usize {
    if coded
        && matches!(ty, syn::Type::Path(path)
            if path.qself.is_none() && path.path.segments.len() == 1
                && ["u32", "u16", "u8", "bool"].iter().any(|name| path.path.segments[0].ident == *name))
    {
        4
    } else {
        8
    }
}

/// The same cursor places actual kernel buffers and sizes the machine premise.
pub(crate) struct SharedLayout {
    bytes: usize,
}

impl SharedLayout {
    pub(crate) fn new() -> Self {
        // Eight warp scan words, reservation, tile cursor and abort flag.
        Self { bytes: 48 }
    }

    fn allocate(&mut self, bytes: usize, align: usize) -> syn::Result<usize> {
        let overflow = || syn::Error::new(Span::call_site(), "GPU shared-memory layout overflow");
        let offset = self.bytes.checked_add(align - 1).ok_or_else(overflow)? / align * align;
        self.bytes = offset.checked_add(bytes).ok_or_else(overflow)?;
        Ok(offset)
    }

    pub(crate) fn payload(&mut self) -> syn::Result<(usize, usize)> {
        let begins = self.allocate(LANES * 4, 4)?;
        let offsets = self.allocate((LANES + 1) * 4, 4)?;
        Ok((begins, offsets))
    }

    pub(crate) fn parent(&mut self, width: usize) -> syn::Result<usize> {
        self.allocate(LANES * width, width)
    }

    pub(crate) fn fold(&mut self) -> syn::Result<usize> {
        self.allocate(LANES * 8, 8)
    }

    pub(crate) fn bytes(&self) -> syn::Result<usize> {
        self.bytes
            .checked_add(15)
            .map(|bytes| bytes / 16 * 16)
            .ok_or_else(|| syn::Error::new(Span::call_site(), "GPU shared-memory layout overflow"))
    }
}

/// Interpret the target-independent allocation stream using the GPU ABI.
pub fn shared_bytes(shape: &DispatchShape) -> syn::Result<usize> {
    let mut layout = SharedLayout::new();
    for chain in &shape.chains {
        for allocation in chain {
            match allocation {
                ResourceAllocation::Payload { parents } => {
                    layout.payload()?;
                    for column in parents {
                        let width = if column.coded
                            && column.types.iter().all(|ty| code_bytes(ty, true) == 4)
                        {
                            4
                        } else {
                            8
                        };
                        layout.parent(width)?;
                    }
                }
                ResourceAllocation::Fold => {
                    layout.fold()?;
                }
            }
        }
    }
    layout.bytes()
}
