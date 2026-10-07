// Ownership accounting excludes allocator overhead and borrowed base storage.
// An allocation survives counter resets or thread transfer without subtracting
// from a different measurement epoch.
#[derive(Default)]
pub(crate) struct Allocation {
    #[cfg(feature = "counters")]
    bytes: usize,
    #[cfg(feature = "counters")]
    epoch: u64,
    #[cfg(feature = "counters")]
    thread: Option<std::thread::ThreadId>,
}
impl Allocation {
    pub(crate) fn new(bytes: usize) -> Self {
        let mut result = Self::default();
        #[cfg(feature = "counters")]
        {
            result.epoch = crate::counters::epoch();
            result.thread = Some(std::thread::current().id());
        }
        result.grow(bytes);
        result
    }
    pub(crate) fn grow(&mut self, bytes: usize) {
        #[cfg(feature = "counters")]
        {
            self.bytes += bytes;
            if self.live() {
                crate::counters::record(|c| {
                    c.bytes_retained_current += bytes as u64;
                    c.bytes_retained_peak = c.bytes_retained_peak.max(c.bytes_retained_current);
                });
            }
        }
        let _ = bytes;
    }
    pub(crate) fn shrink(&mut self, bytes: usize) {
        #[cfg(feature = "counters")]
        {
            self.bytes -= bytes;
            if self.live() {
                crate::counters::record(|c| c.bytes_retained_current -= bytes as u64);
            }
        }
        let _ = bytes;
    }
    #[cfg(feature = "counters")]
    fn live(&self) -> bool {
        self.thread == Some(std::thread::current().id()) && self.epoch == crate::counters::epoch()
    }
}
impl Drop for Allocation {
    fn drop(&mut self) {
        #[cfg(feature = "counters")]
        self.shrink(self.bytes);
    }
}
