#![doc = include_str!("../README.md")]
#![forbid(unsafe_code)]

use proc_macro::TokenStream;

macro_rules! physical_rewrite {
    ($name:ident) => {
        #[proc_macro]
        pub fn $name(input: TokenStream) -> TokenStream {
            cq_stage::rewrite::<cq_lang_physical::Program>(input.into(), cq_rewrite_passes::$name)
                .into()
        }
    };
}

physical_rewrite!(merge_joins);
physical_rewrite!(canon_prefixes);
physical_rewrite!(smallest_intersections);
physical_rewrite!(lazy_semijoins);

/// Change only the representation types matched by `selector => RustType`.
#[proc_macro]
pub fn retype(input: TokenStream) -> TokenStream {
    cq_stage::rewrite::<cq_lang_structures::Program>(input.into(), cq_rewrite_passes::retype).into()
}
