// Ownership accounting excludes allocator overhead and borrowed base storage.
// Allocation ownership follows its query epoch across task/thread transfer;
// resets detach old epochs rather than charging their later drops to new work.
#[derive(Default)]
pub(crate) struct Allocation {
    #[cfg(feature = "counters")]
    bytes: usize,
    #[cfg(feature = "counters")]
    owner: Option<crate::counters::TaskContext>,
}
impl Allocation {
    pub(crate) fn new(bytes: usize) -> Self {
        let mut result = Self::default();
        #[cfg(feature = "counters")]
        {
            result.owner = Some(crate::counters::TaskContext::capture());
        }
        result.grow(bytes);
        result
    }
    pub(crate) fn grow(&mut self, bytes: usize) {
        #[cfg(feature = "counters")]
        {
            self.bytes += bytes;
            if let Some(owner) = &self.owner {
                owner.grow(bytes);
            }
        }
        let _ = bytes;
    }
    pub(crate) fn shrink(&mut self, bytes: usize) {
        #[cfg(feature = "counters")]
        {
            self.bytes -= bytes;
            if let Some(owner) = &self.owner {
                owner.shrink(bytes);
            }
        }
        let _ = bytes;
    }
}
impl Drop for Allocation {
    fn drop(&mut self) {
        #[cfg(feature = "counters")]
        self.shrink(self.bytes);
    }
}
