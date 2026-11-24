//! First-warmup storage accounting from existing host sizes; no GPU readbacks.
use std::cell::{Cell, RefCell};

#[derive(Debug)]
pub struct IndexBytes {
    pub kind: &'static str,
    pub allocated: usize,
    pub live: Option<usize>,
}
thread_local! {
    static ACTIVE: Cell<bool> = const { Cell::new(false) };
    static PUBLICATIONS: RefCell<Vec<(&'static str, Vec<gpu_device_columns::Bounds>)>> = const { RefCell::new(Vec::new()) };
    static RECORDS: RefCell<Vec<IndexBytes>> = const { RefCell::new(Vec::new()) };
}

pub fn begin() {
    PUBLICATIONS.with(|records| records.borrow_mut().clear());
    RECORDS.with(|records| records.borrow_mut().clear());
    ACTIVE.with(|active| active.set(true));
}
pub fn enabled() -> bool { ACTIVE.with(Cell::get) }
pub fn record(kind: &'static str, allocated: usize, live: usize) {
    if enabled() {
        RECORDS.with(|records| records.borrow_mut().push(IndexBytes { kind, allocated, live: Some(live) }));
    }
}
pub fn record_allocated(kind: &'static str, allocated: usize) {
    if enabled() {
        RECORDS.with(|records| records.borrow_mut().push(IndexBytes { kind, allocated, live: None }));
    }
}
pub fn finish() -> Vec<IndexBytes> {
    ACTIVE.with(|active| active.set(false));
    RECORDS.with(|records| std::mem::take(&mut *records.borrow_mut()))
}

pub fn canonical(keys: &gpu_datalog_runtime::RadixLayout<
    gpu_datalog_runtime::radix_run::EncodedRow, (),
    gpu_datalog_runtime::radix_run::PackedRow, u64, (),
>) {
    if enabled() {
        record_allocated("canonical", keys.keys().bytes() + keys.metadata().bytes());
    }
}

pub fn publication(producer: &'static str, bounds: &[gpu_device_columns::Bounds]) {
    if enabled() {
        PUBLICATIONS.with(|records| records.borrow_mut().push((producer, bounds.to_vec())));
    }
}
pub fn take_publications() -> Vec<(&'static str, Vec<gpu_device_columns::Bounds>)> {
    PUBLICATIONS.with(|records| std::mem::take(&mut *records.borrow_mut()))
}
