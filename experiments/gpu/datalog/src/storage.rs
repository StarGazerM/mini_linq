//! Device realizations of the existing checked Datalog storage records.
use cq_lang_datalog_storage::{self as storage, Program, Providers};
use proc_macro2::TokenStream;
use quote::quote;
use syn::{Expr, Type};

/// Return the native packed width and the runtime column descriptor.
/// The runtime also packs u64, but GPU CQ expressions cannot represent its full domain.
pub fn scalar(ty: &Type) -> syn::Result<(u32, TokenStream)> {
    if let Type::Path(path) = ty {
        if path.qself.is_none() && path.path.segments.len() == 1 {
            let segment = &path.path.segments[0];
            if matches!(segment.arguments, syn::PathArguments::None) {
                let (bits, variant) = match segment.ident.to_string().as_str() {
                    "u8" => (8, quote!(U8)),
                    "i8" => (8, quote!(I8)),
                    "u16" => (16, quote!(U16)),
                    "i16" => (16, quote!(I16)),
                    "u32" => (32, quote!(U32)),
                    "i32" => (32, quote!(I32)),
                    "i64" => (64, quote!(I64)),
                    _ => return Err(unsupported_scalar(ty)),
                };
                return Ok((bits, quote!(::gpu_datalog_runtime::Column::#variant)));
            }
        }
    }
    Err(unsupported_scalar(ty))
}

fn unsupported_scalar(ty: &Type) -> syn::Error {
    syn::Error::new_spanned(
        ty,
        "device storage supports only u8/i8/u16/i16/u32/i32/i64 scalar columns; u64 is outside the GPU CQ expression domain",
    )
}

/// Choose the minimal runtime word, including u64 for the empty tuple.
pub fn word_type<'a>(types: impl IntoIterator<Item = &'a Type>) -> syn::Result<Type> {
    let mut bits = 0;
    for ty in types {
        let (width, _) = scalar(ty)?;
        if width > 128 - bits {
            return Err(syn::Error::new_spanned(
                ty,
                "device storage tuple exceeds 128 packed bits",
            ));
        }
        bits += width;
    }
    Ok(if bits <= 64 {
        syn::parse_quote!(u64)
    } else {
        syn::parse_quote!(u128)
    })
}

/// Build a CQ binding in its recorded physical order; callers supply a checked key.
pub fn layout_type(types: &[&Type], key: usize) -> Type {
    let (keys, values) = types.split_at(key);
    syn::parse_quote!(::gpu_datalog_runtime::DeviceLayout<(#(#keys,)*), (#(#values,)*)>)
}

fn default_provider(types: &[&Type], edb: bool) -> Providers {
    Providers {
        canonical_name: syn::parse_quote!(Canonical),
        canonical_set: if edb {
            syn::parse_quote!(::std::collections::HashSet<R, ::std::hash::BuildHasherDefault<::cq_datalog::rustc_hash::FxHasher>>)
        } else {
            syn::parse_quote!(::cq_storage::HashSet<R>)
        },
        all: syn::parse_quote!(::std::vec::Vec<(#(#types,)*)>),
        seen: if edb {
            syn::parse_quote!(::cq_storage::HashSet<(#(#types,)*)>)
        } else {
            syn::parse_quote!(::std::collections::HashSet<(#(#types,)*), ::std::hash::BuildHasherDefault<::cq_datalog::rustc_hash::FxHasher>>)
        },
        delta: syn::parse_quote!(::std::vec::Vec<(#(#types,)*)>),
        next: syn::parse_quote!(::std::vec::Vec<(#(#types,)*)>),
        all_init: syn::parse_quote!(::std::vec::Vec::new()),
        delta_init: syn::parse_quote!(::std::vec::Vec::new()),
        next_init: syn::parse_quote!(::std::vec::Vec::new()),
    }
}

fn device_provider(types: &[&Type], edb: bool) -> syn::Result<Providers> {
    let word = word_type(types.iter().copied())?;
    let runs: Type = syn::parse_quote!(::gpu_datalog_runtime::DeviceRuns<#word>);
    let mut provider = default_provider(types, edb);
    if edb {
        provider.canonical_set = runs;
    } else {
        provider.all = runs.clone();
        provider.seen = runs.clone();
        provider.delta = runs.clone();
        provider.next = runs;
        let init: Expr = syn::parse_quote!(::gpu_datalog_runtime::DeviceRuns::<#word>::new(__cq_gpu_pool.clone())?);
        provider.all_init = init.clone();
        provider.delta_init = init.clone();
        provider.next_init = init;
    }
    Ok(provider)
}

fn default_layout(types: &[&Type], key: usize) -> Type {
    let (keys, values) = types.split_at(key);
    syn::parse_quote!(
        ::std::collections::HashMap<
            (#(#keys,)*),
            ::std::vec::Vec<(#(#values,)*)>,
            ::std::hash::BuildHasherDefault<::cq_datalog::rustc_hash::FxHasher>
        >
    )
}

pub fn compile(source: &Program) -> syn::Result<Program> {
    let analysis = storage::contract::check(source)?;
    let declarations = &source.incremental.strata.source.declarations;
    let mut target = source.clone();
    for (provider, declaration) in target.providers.iter_mut().zip(declarations) {
        let types: Vec<_> = declaration.columns.iter().map(|column| &column.ty).collect();
        *provider = device_provider(&types, declaration.value.is_some())?;
    }
    for layout in target.layouts.iter_mut().chain(&mut target.delta_layouts) {
        let declaration = &declarations[analysis.positions[&cq_syntax::symbol_name(&layout.relation)]];
        let types: Vec<_> = layout.columns.iter().map(|&column| &declaration.columns[column].ty).collect();
        layout.provider = layout_type(&types, layout.key);
    }
    check(source, &target)?;
    Ok(target)
}

/// Formal rule CK-DL-DeviceStorage:
/// Only default provider realizations change; every logical and provenance record stays exact.
pub fn check(source: &Program, target: &Program) -> syn::Result<()> {
    let analysis = storage::contract::check(source)?;
    storage::contract::check(target)?;
    if source.incremental != target.incremental
        || source.reads != target.reads
        || source.negations != target.negations
        || source.certificates != target.certificates
        || source.policy != target.policy
        || source.layouts.len() != target.layouts.len()
        || source.delta_layouts.len() != target.delta_layouts.len()
    {
        return Err(syn::Error::new_spanned(
            target,
            "device storage changed logical, read, layout, negative, policy, or certificate records",
        ));
    }
    let declarations = &source.incremental.strata.source.declarations;
    for ((before, after), declaration) in source.providers.iter().zip(&target.providers).zip(declarations) {
        let types: Vec<_> = declaration.columns.iter().map(|column| &column.ty).collect();
        let edb = declaration.value.is_some();
        if before != &default_provider(&types, edb) {
            return Err(syn::Error::new_spanned(before, "device storage requires default relation providers"));
        }
        if after != &device_provider(&types, edb)? {
            return Err(syn::Error::new_spanned(after, "device storage relation provider differs from its packed realization"));
        }
    }
    for (before, after) in source.layouts.iter().zip(&target.layouts)
        .chain(source.delta_layouts.iter().zip(&target.delta_layouts))
    {
        if before.relation != after.relation
            || before.columns != after.columns
            || before.key != after.key
            || before.maintenance != after.maintenance
        {
            return Err(syn::Error::new_spanned(after, "device storage changed a layout record or permutation"));
        }
        let declaration = &declarations[analysis.positions[&cq_syntax::symbol_name(&before.relation)]];
        let types: Vec<_> = before.columns.iter().map(|&column| &declaration.columns[column].ty).collect();
        if before.provider != default_layout(&types, before.key) {
            return Err(syn::Error::new_spanned(before, "device storage requires default layout providers"));
        }
        if after.provider != layout_type(&types, before.key) {
            return Err(syn::Error::new_spanned(after, "device storage layout provider differs from its recorded heading"));
        }
    }
    Ok(())
}

/// Validate the concrete realization at the printer boundary.
pub fn check_device(program: &Program) -> syn::Result<()> {
    let analysis = storage::contract::check(program)?;
    let declarations = &program.incremental.strata.source.declarations;
    for (provider, declaration) in program.providers.iter().zip(declarations) {
        let types = declaration.columns.iter().map(|column| &column.ty).collect::<Vec<_>>();
        if provider != &device_provider(&types, declaration.value.is_some())? {
            return Err(syn::Error::new_spanned(provider, "GPU printer requires checked device_storage providers"));
        }
    }
    for layout in program.layouts.iter().chain(&program.delta_layouts) {
        let declaration = &declarations[analysis.positions[&cq_syntax::symbol_name(&layout.relation)]];
        let types = layout.columns.iter().map(|&i| &declaration.columns[i].ty).collect::<Vec<_>>();
        if layout.provider != layout_type(&types, layout.key) {
            return Err(syn::Error::new_spanned(layout, "GPU printer requires checked device layout providers"));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn source() -> Program {
        cq_datalog_storage::compile(&syn::parse_quote! {
            struct Paths;
            relation Edge(x:i32,y:i32) = (edges);
            relation Blocked(x:i32,y:i32) = (blocked);
            relation Reach(x:i32,y:i32);
            relation Result(x:i32,y:i32);
            output Result;
            Reach(*x,*y) :- Edge(x,y);
            Reach(*x,*z) :- Reach(x,y), Reach(y,z);
            Result(*x,*y) :- Reach(x,y), !Blocked(x,_);
            strata { component [Reach]; component [Result]; }
            incremental { scheme seminaive_covering;
                delta Reach = ReachChanges;
                delta Result = ResultChanges;
                variant 0 seed { Reach(*x,*y) :- Edge(x,y); }
                variant 1 delta 0 { Reach(*x,*z) :- ReachChanges(x,y), Reach(y,z); }
                variant 1 delta 1 { Reach(*x,*z) :- Reach(x,y), ReachChanges(y,z); }
                variant 2 seed { Result(*x,*y) :- Reach(x,y), !Blocked(x,_); }
            }
        }).unwrap()
    }

    #[test]
    fn scalar_domains_match_runtime_and_gpu_expressions() {
        for (name, bits, variant) in [
            ("u8", 8, "U8"), ("i8", 8, "I8"),
            ("u16", 16, "U16"), ("i16", 16, "I16"),
            ("u32", 32, "U32"), ("i32", 32, "I32"), ("i64", 64, "I64"),
        ] {
            let (actual_bits, descriptor) = scalar(&syn::parse_str(name).unwrap()).unwrap();
            assert_eq!(actual_bits, bits);
            let expected: syn::Path = syn::parse_str(&format!("::gpu_datalog_runtime::Column::{variant}")).unwrap();
            assert_eq!(syn::parse2::<syn::Path>(descriptor).unwrap(), expected);
        }
        for name in ["u64", "u128", "i128", "usize", "isize", "bool", "char", "f32", "f64", "String", "&i32", "(i32,)", "[u8; 4]", "MyScalar", "u8<u16>"] {
            assert!(scalar(&syn::parse_str(name).unwrap()).is_err(), "{name}");
        }
    }

    #[test]
    fn packing_boundaries_and_layout_keys() {
        for (types, expected) in [
            (vec![], syn::parse_quote!(u64)),
            (vec![syn::parse_quote!(i64)], syn::parse_quote!(u64)),
            (vec![syn::parse_quote!(i64), syn::parse_quote!(u8)], syn::parse_quote!(u128)),
            (vec![syn::parse_quote!(i64), syn::parse_quote!(i64)], syn::parse_quote!(u128)),
            (vec![syn::parse_quote!(u8); 16], syn::parse_quote!(u128)),
        ] {
            assert_eq!(word_type(&types).unwrap(), expected);
        }
        let too_wide: Vec<Type> = vec![syn::parse_quote!(i64), syn::parse_quote!(i64), syn::parse_quote!(u8)];
        assert!(word_type(&too_wide).is_err());
        let types: Vec<Type> = vec![syn::parse_quote!(i16), syn::parse_quote!(u32)];
        let types: Vec<_> = types.iter().collect();
        assert_eq!(layout_type(&types, 0), syn::parse_quote!(::gpu_datalog_runtime::DeviceLayout<(), (i16, u32,)>));
        assert_eq!(layout_type(&types, 1), syn::parse_quote!(::gpu_datalog_runtime::DeviceLayout<(i16,), (u32,)>));
        assert_eq!(layout_type(&types, 2), syn::parse_quote!(::gpu_datalog_runtime::DeviceLayout<(i16, u32,), ()>));
        assert_eq!(layout_type(&[], 0), syn::parse_quote!(::gpu_datalog_runtime::DeviceLayout<(), ()>));
    }

    #[test]
    fn rewrite_preserves_all_nonprovider_records_and_unused_defaults() {
        let source = source();
        let target = compile(&source).unwrap();
        check(&source, &target).unwrap();
        storage::contract::check(&target).unwrap();
        assert_eq!(target.incremental, source.incremental);
        assert_eq!(target.reads, source.reads);
        assert_eq!(target.negations, source.negations);
        assert_eq!(target.certificates, source.certificates);
        assert_eq!(target.policy, source.policy);
        assert_eq!(target.providers[0].canonical_set, syn::parse_quote!(::gpu_datalog_runtime::DeviceRuns<u64>));
        let mut edb = target.providers[0].clone();
        edb.canonical_set = source.providers[0].canonical_set.clone();
        assert_eq!(edb, source.providers[0]);
        assert_eq!(target.providers[2].all, syn::parse_quote!(::gpu_datalog_runtime::DeviceRuns<u64>));
        assert_eq!(target.providers[2].canonical_set, source.providers[2].canonical_set);
        assert_eq!(target.providers[2].canonical_name, source.providers[2].canonical_name);
        for (before, after) in source.layouts.iter().zip(&target.layouts)
            .chain(source.delta_layouts.iter().zip(&target.delta_layouts))
        {
            let mut restored = after.clone();
            restored.provider = before.provider.clone();
            assert_eq!(&restored, before);
        }
        assert!(compile(&target).is_err());
    }

    #[test]
    fn rejects_nondefault_sources_and_incorrect_device_providers() {
        let mutations: &[fn(&mut Program)] = &[
            |p| p.providers[0].canonical_name = syn::parse_quote!(Custom),
            |p| p.providers[0].canonical_set = syn::parse_quote!(Custom<R>),
            |p| p.providers[2].all = syn::parse_quote!(Custom),
            |p| p.providers[2].seen = syn::parse_quote!(Custom),
            |p| { p.providers[2].delta = syn::parse_quote!(Custom); p.providers[2].next = syn::parse_quote!(Custom); },
            |p| p.providers[2].all_init = syn::parse_quote!(custom()),
            |p| p.providers[2].delta_init = syn::parse_quote!(custom()),
            |p| p.providers[2].next_init = syn::parse_quote!(custom()),
            |p| p.layouts[0].provider = syn::parse_quote!(Custom),
            |p| p.delta_layouts[0].provider = syn::parse_quote!(Custom),
        ];
        let source = source();
        let target = compile(&source).unwrap();
        for (index, mutate) in mutations.iter().enumerate() {
            let mut changed = source.clone();
            mutate(&mut changed);
            storage::contract::check(&changed).unwrap();
            assert!(compile(&changed).is_err(), "source {index}");
            assert!(check(&changed, &target).is_err(), "checker source {index}");
            let mut changed = target.clone();
            mutate(&mut changed);
            storage::contract::check(&changed).unwrap();
            assert!(check(&source, &changed).is_err(), "target {index}");
        }
        let mut wrong_width = target.clone();
        wrong_width.providers[0].canonical_set = syn::parse_quote!(::gpu_datalog_runtime::DeviceRuns<u128>);
        assert!(check(&source, &wrong_width).is_err());
        assert!(check(&source, &source).is_err());
    }

    #[test]
    fn checker_rejects_record_mutations() {
        let source = source();
        let target = compile(&source).unwrap();
        let mutations: &[fn(&mut Program)] = &[
            |p| p.incremental.strata.source.name = syn::parse_quote!(Other),
            |p| p.reads.reverse(),
            |p| p.reads[0].columns.reverse(),
            |p| p.layouts[0].columns.reverse(),
            |p| p.layouts[0].key = 1,
            |p| p.layouts[0].maintenance = syn::parse_quote!(replace_each_run),
            |p| { p.layouts.pop(); },
            |p| { p.delta_layouts.pop(); },
            |p| p.negations[0].columns.clear(),
            |p| p.negations[0].filters.push((0, syn::parse_quote!(7))),
            |p| p.negations[0].equalities.push((0, 1)),
            |p| p.certificates.clear(),
            |p| p.certificates.reverse(),
            |p| p.policy.insertion = syn::parse_quote!(keys_only),
        ];
        for (index, mutate) in mutations.iter().enumerate() {
            let mut changed = target.clone();
            mutate(&mut changed);
            assert!(check(&source, &changed).is_err(), "mutation {index}");
        }
    }

    #[test]
    fn rejects_unsupported_and_overwide_relation_schemas() {
        for (name, message) in [
            ("bool", "supports only"),
            ("u64", "supports only"),
            ("String", "supports only"),
            ("i64", "exceeds 128"),
        ] {
            let ty: Type = syn::parse_str(name).unwrap();
            let source = cq_datalog_storage::compile(&syn::parse_quote! {
                struct Schema;
                relation E(x:#ty,y:#ty,z:#ty) = (input);
                relation R(x:#ty,y:#ty,z:#ty);
                output R;
                R(*x,*y,*z) :- E(x,y,z);
                strata { component [R]; }
                incremental { scheme seminaive_covering;
                    delta R = Changes;
                    variant 0 seed { R(*x,*y,*z) :- E(x,y,z); }
                }
            }).unwrap();
            assert!(compile(&source).unwrap_err().to_string().contains(message), "{name}");
            assert!(check(&source, &source).unwrap_err().to_string().contains(message), "{name}");
        }
    }

    #[test]
    fn wide_relations_use_u128_without_changing_scalar_layout_types() {
        let mut incremental = source().incremental;
        for declaration in &mut incremental.strata.source.declarations {
            for column in &mut declaration.columns {
                column.ty = syn::parse_quote!(i64);
            }
        }
        let source = cq_datalog_storage::compile(&incremental).unwrap();
        let target = compile(&source).unwrap();
        assert_eq!(target.providers[0].canonical_set, syn::parse_quote!(::gpu_datalog_runtime::DeviceRuns<u128>));
        assert_eq!(target.providers[2].all, syn::parse_quote!(::gpu_datalog_runtime::DeviceRuns<u128>));
        assert_eq!(target.layouts[0].provider, syn::parse_quote!(::gpu_datalog_runtime::DeviceLayout<(), (i64, i64,)>));
        check(&source, &target).unwrap();
    }

    #[test]
    fn preserves_authored_extra_layouts_and_optional_certificates() {
        let mut source = source();
        source.certificates.clear();
        let mut extra = source.layouts[0].clone();
        extra.columns.reverse();
        extra.key = 0;
        // Both columns are i32, so this permutation retains the default Rust type.
        source.layouts.push(extra);
        storage::contract::check(&source).unwrap();
        let target = compile(&source).unwrap();
        assert!(target.certificates.is_empty());
        assert_eq!(target.layouts.len(), source.layouts.len());
        assert_eq!(target.layouts.last().unwrap().columns, source.layouts.last().unwrap().columns);
        check(&source, &target).unwrap();
    }
}
