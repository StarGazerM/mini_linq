//! Loader metadata checked against each case’s literal query by catalog tests.
use crate::schema::{ColumnType::I32, Relation};

pub mod intersection {
    use super::*;
    pub const ARITY: usize = 1;
    pub const INPUTS: &[Relation] = &[
        Relation {
            name: "Left",
            columns: &[("c0", I32)],
        },
        Relation {
            name: "Right",
            columns: &[("c0", I32)],
        },
    ];
}

pub mod cartesian_product {
    use super::*;
    pub const ARITY: usize = 3;
    pub const INPUTS: &[Relation] = &[
        Relation {
            name: "A",
            columns: &[("c0", I32)],
        },
        Relation {
            name: "B",
            columns: &[("c0", I32), ("c1", I32)],
        },
    ];
}

pub mod oriented_chain {
    use super::*;
    pub const ARITY: usize = 2;
    pub const INPUTS: &[Relation] = &[Relation {
        name: "Edge",
        columns: &[("c0", I32), ("c1", I32)],
    }];
}

pub mod claw {
    use super::*;
    pub const ARITY: usize = 4;
    pub const INPUTS: &[Relation] = &[
        Relation {
            name: "A",
            columns: &[("c0", I32), ("c1", I32)],
        },
        Relation {
            name: "B",
            columns: &[("c0", I32), ("c1", I32)],
        },
        Relation {
            name: "C",
            columns: &[("c0", I32), ("c1", I32)],
        },
    ];
}

pub mod fact_star {
    use super::*;
    pub const ARITY: usize = 6;
    pub const INPUTS: &[Relation] = &[
        Relation {
            name: "Fact",
            columns: &[("c0", I32), ("c1", I32), ("c2", I32)],
        },
        Relation {
            name: "DimA",
            columns: &[("c0", I32), ("c1", I32)],
        },
        Relation {
            name: "DimB",
            columns: &[("c0", I32), ("c1", I32)],
        },
        Relation {
            name: "DimC",
            columns: &[("c0", I32), ("c1", I32)],
        },
    ];
}

pub mod triangle {
    use super::*;
    pub const ARITY: usize = 3;
    pub const INPUTS: &[Relation] = &[
        Relation {
            name: "R",
            columns: &[("c0", I32), ("c1", I32)],
        },
        Relation {
            name: "S",
            columns: &[("c0", I32), ("c1", I32)],
        },
        Relation {
            name: "T",
            columns: &[("c0", I32), ("c1", I32)],
        },
    ];
}

pub mod cycle_4 {
    use super::*;
    pub const ARITY: usize = 4;
    pub const INPUTS: &[Relation] = &[
        Relation {
            name: "R",
            columns: &[("c0", I32), ("c1", I32)],
        },
        Relation {
            name: "S",
            columns: &[("c0", I32), ("c1", I32)],
        },
        Relation {
            name: "T",
            columns: &[("c0", I32), ("c1", I32)],
        },
        Relation {
            name: "U",
            columns: &[("c0", I32), ("c1", I32)],
        },
    ];
}

pub mod spade {
    use super::*;
    pub const ARITY: usize = 4;
    pub const INPUTS: &[Relation] = &[
        Relation {
            name: "R",
            columns: &[("c0", I32), ("c1", I32)],
        },
        Relation {
            name: "S",
            columns: &[("c0", I32), ("c1", I32)],
        },
        Relation {
            name: "T",
            columns: &[("c0", I32), ("c1", I32)],
        },
        Relation {
            name: "H",
            columns: &[("c0", I32), ("c1", I32)],
        },
    ];
}

pub mod barbell {
    use super::*;
    pub const ARITY: usize = 6;
    pub const INPUTS: &[Relation] = &[
        Relation {
            name: "R",
            columns: &[("c0", I32), ("c1", I32)],
        },
        Relation {
            name: "S",
            columns: &[("c0", I32), ("c1", I32)],
        },
        Relation {
            name: "T",
            columns: &[("c0", I32), ("c1", I32)],
        },
        Relation {
            name: "Bridge",
            columns: &[("c0", I32), ("c1", I32)],
        },
        Relation {
            name: "U",
            columns: &[("c0", I32), ("c1", I32)],
        },
        Relation {
            name: "V",
            columns: &[("c0", I32), ("c1", I32)],
        },
        Relation {
            name: "W",
            columns: &[("c0", I32), ("c1", I32)],
        },
    ];
}

pub mod loomis_whitney_4 {
    use super::*;
    pub const ARITY: usize = 4;
    pub const INPUTS: &[Relation] = &[
        Relation {
            name: "ABC",
            columns: &[("c0", I32), ("c1", I32), ("c2", I32)],
        },
        Relation {
            name: "BCD",
            columns: &[("c0", I32), ("c1", I32), ("c2", I32)],
        },
        Relation {
            name: "ACD",
            columns: &[("c0", I32), ("c1", I32), ("c2", I32)],
        },
        Relation {
            name: "ABD",
            columns: &[("c0", I32), ("c1", I32), ("c2", I32)],
        },
    ];
}

pub mod retail_h03 {
    use super::*;
    pub const ARITY: usize = 2;
    pub const INPUTS: &[Relation] = &[
        Relation {
            name: "RetailH03Customer",
            columns: &[("c0", I32), ("c1", I32)],
        },
        Relation {
            name: "RetailH03Orders",
            columns: &[("c0", I32), ("c1", I32)],
        },
        Relation {
            name: "RetailH03Lineitem",
            columns: &[("c0", I32), ("c1", I32), ("c2", I32), ("c3", I32)],
        },
    ];
}

pub mod retail_h05 {
    use super::*;
    pub const ARITY: usize = 3;
    pub const INPUTS: &[Relation] = &[
        Relation {
            name: "RetailH05Customer",
            columns: &[("c0", I32), ("c1", I32)],
        },
        Relation {
            name: "RetailH05Orders",
            columns: &[("c0", I32), ("c1", I32)],
        },
        Relation {
            name: "RetailH05Lineitem",
            columns: &[("c0", I32), ("c1", I32), ("c2", I32), ("c3", I32)],
        },
        Relation {
            name: "RetailH05Supplier",
            columns: &[("c0", I32), ("c1", I32)],
        },
        Relation {
            name: "RetailH05Nation",
            columns: &[("c0", I32), ("c1", I32)],
        },
        Relation {
            name: "RetailH05Region",
            columns: &[("c0", I32)],
        },
    ];
}

pub mod retail_h07 {
    use super::*;
    pub const ARITY: usize = 3;
    pub const INPUTS: &[Relation] = &[
        Relation {
            name: "RetailH07Supplier",
            columns: &[("c0", I32), ("c1", I32)],
        },
        Relation {
            name: "RetailH07Lineitem",
            columns: &[("c0", I32), ("c1", I32), ("c2", I32), ("c3", I32)],
        },
        Relation {
            name: "RetailH07Orders",
            columns: &[("c0", I32), ("c1", I32)],
        },
        Relation {
            name: "RetailH07Customer",
            columns: &[("c0", I32), ("c1", I32)],
        },
        Relation {
            name: "RetailH07Nation",
            columns: &[("c0", I32), ("c1", I32)],
        },
    ];
}

pub mod retail_h08 {
    use super::*;
    pub const ARITY: usize = 3;
    pub const INPUTS: &[Relation] = &[
        Relation {
            name: "RetailH08Lineitem",
            columns: &[("c0", I32), ("c1", I32), ("c2", I32), ("c3", I32)],
        },
        Relation {
            name: "RetailH08Part",
            columns: &[("c0", I32)],
        },
        Relation {
            name: "RetailH08Supplier",
            columns: &[("c0", I32), ("c1", I32)],
        },
        Relation {
            name: "RetailH08Orders",
            columns: &[("c0", I32), ("c1", I32)],
        },
        Relation {
            name: "RetailH08Customer",
            columns: &[("c0", I32), ("c1", I32)],
        },
        Relation {
            name: "RetailH08Nation",
            columns: &[("c0", I32), ("c1", I32)],
        },
        Relation {
            name: "RetailH08Region",
            columns: &[("c0", I32)],
        },
    ];
}

pub mod retail_h09 {
    use super::*;
    pub const ARITY: usize = 3;
    pub const INPUTS: &[Relation] = &[
        Relation {
            name: "RetailH09Lineitem",
            columns: &[("c0", I32), ("c1", I32), ("c2", I32), ("c3", I32)],
        },
        Relation {
            name: "RetailH09PartSupp",
            columns: &[("c0", I32), ("c1", I32)],
        },
        Relation {
            name: "RetailH09Part",
            columns: &[("c0", I32)],
        },
        Relation {
            name: "RetailH09Supplier",
            columns: &[("c0", I32), ("c1", I32)],
        },
        Relation {
            name: "RetailH09Orders",
            columns: &[("c0", I32), ("c1", I32)],
        },
        Relation {
            name: "RetailH09Nation",
            columns: &[("c0", I32), ("c1", I32)],
        },
    ];
}

pub mod retail_h21_positive {
    use super::*;
    pub const ARITY: usize = 3;
    pub const INPUTS: &[Relation] = &[
        Relation {
            name: "RetailH21PositiveSupplier",
            columns: &[("c0", I32), ("c1", I32)],
        },
        Relation {
            name: "RetailH21PositiveLineitem",
            columns: &[("c0", I32), ("c1", I32), ("c2", I32), ("c3", I32)],
        },
        Relation {
            name: "RetailH21PositiveOrders",
            columns: &[("c0", I32), ("c1", I32)],
        },
        Relation {
            name: "RetailH21PositiveNation",
            columns: &[("c0", I32), ("c1", I32)],
        },
    ];
}

pub mod retail_d19 {
    use super::*;
    pub const ARITY: usize = 4;
    pub const INPUTS: &[Relation] = &[
        Relation {
            name: "RetailD19StoreSales",
            columns: &[
                ("c0", I32),
                ("c1", I32),
                ("c2", I32),
                ("c3", I32),
                ("c4", I32),
            ],
        },
        Relation {
            name: "RetailD19DateDim",
            columns: &[("c0", I32)],
        },
        Relation {
            name: "RetailD19Item",
            columns: &[("c0", I32)],
        },
        Relation {
            name: "RetailD19Customer",
            columns: &[("c0", I32), ("c1", I32)],
        },
        Relation {
            name: "RetailD19CustomerAddress",
            columns: &[("c0", I32)],
        },
        Relation {
            name: "RetailD19Store",
            columns: &[("c0", I32)],
        },
    ];
}

pub mod retail_d27 {
    use super::*;
    pub const ARITY: usize = 3;
    pub const INPUTS: &[Relation] = &[
        Relation {
            name: "RetailD27StoreSales",
            columns: &[
                ("c0", I32),
                ("c1", I32),
                ("c2", I32),
                ("c3", I32),
                ("c4", I32),
            ],
        },
        Relation {
            name: "RetailD27DateDim",
            columns: &[("c0", I32)],
        },
        Relation {
            name: "RetailD27Item",
            columns: &[("c0", I32)],
        },
        Relation {
            name: "RetailD27Store",
            columns: &[("c0", I32)],
        },
        Relation {
            name: "RetailD27CustomerDemo",
            columns: &[("c0", I32)],
        },
    ];
}

pub mod retail_d72_inner {
    use super::*;
    pub const ARITY: usize = 4;
    pub const INPUTS: &[Relation] = &[
        Relation {
            name: "RetailD72InnerCatalogSales",
            columns: &[
                ("c0", I32),
                ("c1", I32),
                ("c2", I32),
                ("c3", I32),
                ("c4", I32),
                ("c5", I32),
            ],
        },
        Relation {
            name: "RetailD72InnerDateDim",
            columns: &[("c0", I32), ("c1", I32)],
        },
        Relation {
            name: "RetailD72InnerInventory",
            columns: &[("c0", I32), ("c1", I32), ("c2", I32)],
        },
        Relation {
            name: "RetailD72InnerWarehouse",
            columns: &[("c0", I32)],
        },
        Relation {
            name: "RetailD72InnerItem",
            columns: &[("c0", I32)],
        },
        Relation {
            name: "RetailD72InnerCustomerDemo",
            columns: &[("c0", I32)],
        },
        Relation {
            name: "RetailD72InnerHouseholdDemo",
            columns: &[("c0", I32)],
        },
    ];
}

pub mod retail_d85 {
    use super::*;
    pub const ARITY: usize = 3;
    pub const INPUTS: &[Relation] = &[
        Relation {
            name: "RetailD85WebSales",
            columns: &[("c0", I32), ("c1", I32), ("c2", I32), ("c3", I32)],
        },
        Relation {
            name: "RetailD85WebReturns",
            columns: &[
                ("c0", I32),
                ("c1", I32),
                ("c2", I32),
                ("c3", I32),
                ("c4", I32),
                ("c5", I32),
            ],
        },
        Relation {
            name: "RetailD85WebPage",
            columns: &[("c0", I32)],
        },
        Relation {
            name: "RetailD85CustomerDemo",
            columns: &[("c0", I32)],
        },
        Relation {
            name: "RetailD85CustomerAddress",
            columns: &[("c0", I32)],
        },
        Relation {
            name: "RetailD85DateDim",
            columns: &[("c0", I32)],
        },
        Relation {
            name: "RetailD85Reason",
            columns: &[("c0", I32)],
        },
    ];
}
