use cq_gpu_runtime::DeviceColumns;

fn bind(_: &DeviceColumns<'_, (i64, i64)>) {}

fn wrong_type(rows: &DeviceColumns<'_, (i64, String)>) {
    bind(rows);
}

fn main() {}
