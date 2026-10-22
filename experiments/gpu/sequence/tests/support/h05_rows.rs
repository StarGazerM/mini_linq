use cq_gpu_runtime::{DeviceColumns, For};
use gpu_compress::Catalog;

/// Typed resident bindings in the fixture's declared column order.
pub struct Rows {
    pub customer: DeviceColumns<'static, (u32, u32), For>,
    pub orders: DeviceColumns<'static, (u32, u32, i32), For>,
    pub lineitem: DeviceColumns<'static, (u32, u32), For>,
    pub supplier: DeviceColumns<'static, (u32, u32), For>,
    pub nation: DeviceColumns<'static, (u32, u32), For>,
    pub region: DeviceColumns<'static, (u32, i32), For>,
}

impl Rows {
    pub fn bind(catalog: &Catalog) -> Result<Self, Box<dyn std::error::Error>> {
        Ok(Self {
            customer: catalog.device_columns("customer")?,
            orders: catalog.device_columns("orders")?,
            lineitem: catalog.device_columns("lineitem")?,
            supplier: catalog.device_columns("supplier")?,
            nation: catalog.device_columns("nation")?,
            region: catalog.device_columns("region")?,
        })
    }
}
