//! Embedded stage fixtures and their authored data cases.
//!
//! Texts are byte-preserving copies, not outputs recomputed by a compiler.
//! Parsed accessors only parse; callers can separately run language contracts.
//! IX and Structures are suffix fragments paired with RA and Physical.

pub use crate::cases::{CASES, Case};

/// One query represented at each published language stage.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Scenario {
    pub name: &'static str,
    pub cq: &'static str,
    pub vo: &'static str,
    pub ra: &'static str,
    /// Index-demand suffix to append to `ra`.
    pub ix: &'static str,
    pub physical: &'static str,
    /// Storage-choice suffix to append to `physical`.
    pub structures: &'static str,
}

impl Scenario {
    pub fn parse_cq(&self) -> syn::Result<crate::cq::Program> {
        syn::parse_str(self.cq)
    }

    pub fn parse_vo(&self) -> syn::Result<crate::vo::Program> {
        syn::parse_str(self.vo)
    }

    pub fn parse_ra(&self) -> syn::Result<crate::ra::Program> {
        syn::parse_str(self.ra)
    }

    pub fn parse_ix(&self) -> syn::Result<crate::ix::Program> {
        syn::parse_str(&format!("{}\n{}", self.ra, self.ix))
    }

    pub fn parse_physical(&self) -> syn::Result<crate::physical::Program> {
        syn::parse_str(self.physical)
    }

    pub fn parse_structures(&self) -> syn::Result<crate::structures::Program> {
        syn::parse_str(&format!("{}\n{}", self.physical, self.structures))
    }

    /// Find authored data for this scenario, including alternative plans.
    ///
    /// Every built-in scenario has a case. User-created scenarios may not.
    pub fn case(&self) -> Option<&'static Case> {
        let name = match self.name {
            "fine_triangle" => "triangle",
            "ya_composite_reduced" => "ya_composite",
            "ya_deep_reduced" => "ya_deep",
            "ya_delivery_reduced" => "ya_delivery",
            "ya_disconnected_reduced" => "ya_disconnected",
            name => name,
        };
        CASES.iter().find(|case| case.name == name)
    }
}

macro_rules! scenario {
    ($name:literal) => {
        Scenario {
            name: $name,
            cq: include_str!(concat!("scenarios/", $name, ".cq")),
            vo: include_str!(concat!("scenarios/", $name, ".vo")),
            ra: include_str!(concat!("scenarios/", $name, ".ra")),
            ix: include_str!(concat!("scenarios/", $name, ".ix")),
            physical: include_str!(concat!("scenarios/", $name, ".physical")),
            structures: include_str!(concat!("scenarios/", $name, ".structures")),
        }
    };
}

/// The complete 30-scenario corpus, in stable name order.
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
