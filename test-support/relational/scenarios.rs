//! Complete input programs for pass tests. Answer oracles are authored in cases.rs.
#![allow(dead_code)]
pub struct Scenario {
    pub name: &'static str,
    pub cq: &'static str,
    pub vo: &'static str,
    pub ra: &'static str,
    pub indexes: &'static str,
    pub physical: &'static str,
    pub structures: &'static str,
    pub policy: &'static str,
}
impl Scenario {
    pub fn ix(&self) -> String {
        format!("{} {}", self.ra, self.indexes)
    }
    pub fn structured(&self) -> String {
        format!("{} {}", self.physical, self.structures)
    }
}
macro_rules! scenario {
    ($name:literal) => {
        Scenario {
            name: $name,
            cq: include_str!(concat!("scenarios/", $name, ".cq")),
            vo: include_str!(concat!("scenarios/", $name, ".vo")),
            ra: include_str!(concat!("scenarios/", $name, ".ra")),
            indexes: include_str!(concat!("scenarios/", $name, ".ix")),
            physical: include_str!(concat!("scenarios/", $name, ".physical")),
            structures: include_str!(concat!("scenarios/", $name, ".structures")),
            policy: include_str!(concat!("scenarios/", $name, ".policy")),
        }
    };
}
pub const SCENARIOS: &[Scenario] = &[
    scenario!("barbell"),
    scenario!("bushy"),
    scenario!("cartesian"),
    scenario!("cartesian_first"),
    scenario!("chain_projection"),
    scenario!("claw"),
    scenario!("composite"),
    scenario!("cycle4"),
    scenario!("existential"),
    scenario!("fact_star"),
    scenario!("fine_triangle"),
    scenario!("intersection"),
    scenario!("loomis_whitney_4"),
    scenario!("oriented_chain"),
    scenario!("projection"),
    scenario!("return_trip"),
    scenario!("scan"),
    scenario!("self_chain"),
    scenario!("spade"),
    scenario!("ternary_guard"),
    scenario!("triangle"),
    scenario!("unary_guard"),
    scenario!("ya_composite"),
    scenario!("ya_composite_reduced"),
    scenario!("ya_deep"),
    scenario!("ya_deep_reduced"),
    scenario!("ya_delivery"),
    scenario!("ya_delivery_reduced"),
    scenario!("ya_disconnected"),
    scenario!("ya_disconnected_reduced"),
];
