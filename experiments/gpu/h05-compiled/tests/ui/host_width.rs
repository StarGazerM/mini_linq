use cq_gpu_runtime::DeviceColumns;

fn bind(_: &DeviceColumns<'_, (i64, i64)>) {}

fn wrong_width(rows: &DeviceColumns<'_, (i64,)>) {
    bind(rows);
}

fn main() {}
