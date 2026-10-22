use cq_gpu_runtime::DeviceColumns;

fn bind(_: &DeviceColumns<'_, (i64,)>) {}

fn wrong_signedness(rows: &DeviceColumns<'_, (u64,)>) {
    bind(rows);
}

fn main() {}
