//! One registry for macro adapters and programmatic checked rewrites.

#[macro_export]
macro_rules! physical_registry {
    ($callback:ident) => {
        $callback! {
            merge_joins => $crate::merge_joins;
            canon_prefixes => $crate::canon_prefixes;
            smallest_intersections => $crate::smallest_intersections;
            lazy_semijoins => $crate::lazy_semijoins;
            build_host_reads => $crate::build_host_reads;
            host_sets => $crate::host_sets;
            retain_streams => $crate::retain_streams;
            hoist => $crate::hoist;
        }
    };
}

#[macro_export]
macro_rules! structures_registry {
    ($callback:ident) => {
        $callback! {
            membership => $crate::membership;
            retype => $crate::retype;
            join_filter => $crate::join_filter;
            sorted_indexes => $crate::sorted_indexes;
            sorted_runs => $crate::sorted_runs;
            hashed_runs => $crate::hashed_runs;
            retain_lookups => $crate::retain_lookups;
            expand_lookups => $crate::expand_lookups;
        }
    };
}

/// Formal rule DR-Registry: Resolve registered paths and opaque arguments with one endpoint registry.
fn registered_name(path: &syn::Path) -> syn::Result<&syn::Ident> {
    let qualified = path.segments.len() == 2
        && path
            .segments
            .first()
            .is_some_and(|segment| segment.ident == "cq_rewrites");
    let bare = path.leading_colon.is_none() && path.segments.len() == 1;
    if (!bare && !qualified)
        || path
            .segments
            .iter()
            .any(|segment| !matches!(segment.arguments, syn::PathArguments::None))
    {
        return Err(syn::Error::new_spanned(
            path,
            "unregistered rewrite path; expected a bare name or cq_rewrites::name",
        ));
    }
    path.segments
        .last()
        .map(|segment| &segment.ident)
        .ok_or_else(|| syn::Error::new_spanned(path, "empty rewrite path"))
}

macro_rules! physical_dispatch {
    ($($name:ident => $implementation:path;)*) => {
        pub fn invoke_physical(
            source: &cq_lang_physical::Program,
            path: &syn::Path,
            arguments: proc_macro2::TokenStream,
        ) -> syn::Result<cq_lang_physical::Program> {
            let name = registered_name(path)?;
            $(if name == stringify!($name) { return $implementation(source, arguments); })*
            Err(syn::Error::new_spanned(path, "unregistered Physical rewrite"))
        }
    };
}
macro_rules! structures_dispatch {
    ($($name:ident => $implementation:path;)*) => {
        pub fn invoke_structures(
            source: &cq_lang_structures::Program,
            path: &syn::Path,
            arguments: proc_macro2::TokenStream,
        ) -> syn::Result<cq_lang_structures::Program> {
            let name = registered_name(path)?;
            $(if name == stringify!($name) { return $implementation(source, arguments); })*
            Err(syn::Error::new_spanned(path, "unregistered Structures rewrite"))
        }
    };
}
physical_registry!(physical_dispatch);
structures_registry!(structures_dispatch);

#[cfg(test)]
mod tests {
    use super::registered_name;

    #[test]
    fn registry_resolves_only_declared_namespaces() {
        for spelling in ["hoist", "cq_rewrites::hoist", "::cq_rewrites::hoist"] {
            let path = syn::parse_str(spelling).unwrap();
            assert_eq!(registered_name(&path).unwrap(), "hoist");
        }
        for spelling in [
            "::hoist",
            "other::hoist",
            "cq_rewrites::nested::hoist",
            "hoist::<i32>",
        ] {
            let path = syn::parse_str(spelling).unwrap();
            assert!(registered_name(&path).is_err());
        }
    }
}
