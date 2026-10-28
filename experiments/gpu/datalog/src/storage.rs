//! Device realizations of the existing checked Datalog storage records.
use cq_lang_datalog_storage::{self as storage, Program, Providers};
use proc_macro2::TokenStream;
use quote::quote;
use syn::{Expr, Type};

/// Explicit, syntactically matched logical-type to host-codec declarations.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Symbols(pub Vec<(Type, Type)>);

impl syn::parse::Parse for Symbols {
    fn parse(input: syn::parse::ParseStream<'_>) -> syn::Result<Self> {
        if input.is_empty() { return Ok(Self::default()); }
        let name: syn::Ident = input.parse()?;
        if name != "symbols" { return Err(syn::Error::new_spanned(name, "expected symbols(T => Codec, ...)")); }
        let content;
        syn::parenthesized!(content in input);
        let mut symbols = Self::default();
        while !content.is_empty() {
            let ty: Type = content.parse()?;
            content.parse::<syn::Token![=>]>()?;
            let codec: Type = content.parse()?;
            if symbols.codec(&ty).is_some() {
                return Err(syn::Error::new_spanned(ty, "duplicate symbol type"));
            }
            if scalar(&ty).is_ok() {
                return Err(syn::Error::new_spanned(ty, "native device scalars cannot have symbol codecs"));
            }
            symbols.0.push((ty, codec));
            if content.is_empty() { break; }
            content.parse::<syn::Token![,]>()?;
        }
        if !input.is_empty() { return Err(input.error("unexpected device_storage arguments")); }
        Ok(symbols)
    }
}
struct Advice {
    symbols: Symbols,
    ratio: usize,
}
impl syn::parse::Parse for Advice {
    fn parse(input: syn::parse::ParseStream<'_>) -> syn::Result<Self> {
        let mut advice = Self { symbols: Symbols::default(), ratio: 4 };
        let mut seen_symbols = false;
        let mut seen_ratio = false;
        while !input.is_empty() {
            let name: syn::Ident = input.parse()?;
            let content;
            syn::parenthesized!(content in input);
            match name.to_string().as_str() {
                "symbols" if !seen_symbols => {
                    let tokens: TokenStream = content.parse()?;
                    advice.symbols = syn::parse2(quote!(symbols(#tokens)))?;
                    seen_symbols = true;
                }
                "ratio" if !seen_ratio => {
                    advice.ratio = content.parse::<syn::LitInt>()?.base10_parse()?;
                    if !content.is_empty() { return Err(content.error("unexpected ratio contents")); }
                    if advice.ratio < 2 { return Err(syn::Error::new_spanned(name, "device maintenance ratio must be at least two")); }
                    seen_ratio = true;
                }
                _ => return Err(syn::Error::new_spanned(name, "expected unique symbols(...) or ratio(...) advice")),
            }
            if !input.is_empty() { input.parse::<syn::Token![,]>()?; }
        }
        Ok(advice)
    }
}

impl Symbols {
    pub fn codec(&self, ty: &Type) -> Option<&Type> {
        self.0.iter().find(|(logical, _)| logical == ty).map(|(_, codec)| codec)
    }

    pub fn device_type(&self, ty: &Type) -> Type {
        match self.codec(ty) {
            Some(codec) => syn::parse_quote!(::gpu_datalog_runtime::Symbol<#ty, #codec>),
            None => ty.clone(),
        }
    }

    fn validate(&self, program: &Program) -> syn::Result<()> {
        for (ty, _) in &self.0 {
            if !program.incremental.strata.source.declarations.iter()
                .any(|declaration| declaration.columns.iter().any(|column| column.ty == *ty))
            {
                return Err(syn::Error::new_spanned(ty, "symbol type does not occur in a relation; use its declared type spelling"));
            }
        }
        Ok(())
    }

    /// Formal rule DL-DeviceSymbols: codecs are the declared logical-type map,
    /// independent of opaque provider spelling and generic arguments.
    pub fn from_program(program: &Program) -> syn::Result<Self> {
        let advice = program.device.as_ref().ok_or_else(||
            syn::Error::new_spanned(program, "missing device storage advice"))?;
        let symbols = Self(advice.symbols.clone());
        for (index, (ty, _)) in symbols.0.iter().enumerate() {
            if scalar(ty).is_ok() || symbols.0[..index].iter().any(|(previous, _)| previous == ty) {
                return Err(syn::Error::new_spanned(ty, "symbol declarations require unique nonnative logical types"));
            }
        }
        symbols.validate(program)?;
        Ok(symbols)
    }
}

fn symbol_parts(ty: &Type) -> Option<(&Type, &Type)> {
    let Type::Path(path) = ty else { return None };
    if path.qself.is_some() || path.path.segments.len() != 2
        || path.path.segments[0].ident != "gpu_datalog_runtime"
        || path.path.segments[1].ident != "Symbol" { return None; }
    let syn::PathArguments::AngleBracketed(arguments) = &path.path.segments[1].arguments else { return None };
    let mut arguments = arguments.args.iter();
    let (Some(syn::GenericArgument::Type(logical)), Some(syn::GenericArgument::Type(codec)), None) =
        (arguments.next(), arguments.next(), arguments.next()) else { return None };
    Some((logical, codec))
}

/// Return the native packed width and the runtime column descriptor.
/// The runtime also packs u64, but GPU CQ expressions cannot represent its full domain.
pub fn scalar(ty: &Type) -> syn::Result<(u32, TokenStream)> {
    if symbol_parts(ty).is_some() {
        return Ok((32, quote!(::gpu_datalog_runtime::Column::U32)));
    }
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
    let mut bits = 0_usize;
    for ty in types {
        let (width, _) = scalar(ty)?;
        bits = bits.checked_add(width as usize)
            .ok_or_else(|| syn::Error::new_spanned(ty, "device storage tuple width overflow"))?;
    }
    Ok(if bits <= 64 {
        syn::parse_quote!(u64)
    } else if bits <= 128 {
        syn::parse_quote!(u128)
    } else {
        let lanes = syn::Index::from(bits.div_ceil(64));
        syn::parse_quote!([u64; #lanes])
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

fn device_provider(types: &[&Type], edb: bool, symbols: &Symbols, ratio: usize) -> syn::Result<Providers> {
    let device_types = types.iter().map(|ty| symbols.device_type(ty)).collect::<Vec<_>>();
    let word = word_type(&device_types)?;
    let runs: Type = syn::parse_quote!(::gpu_datalog_runtime::DeviceRuns<#word>);
    let mut provider = default_provider(types, edb);
    if edb {
        provider.canonical_set = runs;
    } else {
        provider.all = runs.clone();
        provider.seen = runs.clone();
        provider.delta = runs.clone();
        provider.next = runs.clone();
        let init: Expr = syn::parse_quote!(<#runs>::new(__cq_gpu_pool.clone(), #ratio)?);
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
    compile_with(source, TokenStream::new())
}

pub fn compile_with(source: &Program, args: TokenStream) -> syn::Result<Program> {
    let Advice { symbols, ratio } = syn::parse2(args)?;
    symbols.validate(source)?;
    let analysis = storage::contract::check(source)?;
    let declarations = &source.incremental.strata.source.declarations;
    let mut target = source.clone();
    target.device = Some(storage::DeviceStorage { symbols: symbols.0.clone(), ratio });
    for (provider, declaration) in target.providers.iter_mut().zip(declarations) {
        let types: Vec<_> = declaration.columns.iter().map(|column| &column.ty).collect();
        *provider = device_provider(&types, declaration.value.is_some(), &symbols, ratio)?;
    }
    for layout in target.layouts.iter_mut().chain(&mut target.delta_layouts) {
        let declaration = &declarations[analysis.positions[&cq_syntax::symbol_name(&layout.relation)]];
        let device_types = layout.columns.iter().map(|&column| symbols.device_type(&declaration.columns[column].ty)).collect::<Vec<_>>();
        let types = device_types.iter().collect::<Vec<_>>();
        layout.provider = layout_type(&types, layout.key);
    }
    check(source, &target)?;
    Ok(target)
}

/// Formal rule CK-DL-DeviceStorage:
/// Only provider realizations and explicit device advice change; logical records stay exact.
pub fn check(source: &Program, target: &Program) -> syn::Result<()> {
    let analysis = storage::contract::check(source)?;
    let symbols = Symbols::from_program(target)?;
    storage::contract::check(target)?;
    if source.incremental != target.incremental
        || source.reads != target.reads
        || source.negations != target.negations
        || source.device.is_some()
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
        if after != &device_provider(&types, edb, &symbols, target.device.as_ref().expect("checked advice").ratio)? {
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
            || before.protocol != after.protocol
        {
            return Err(syn::Error::new_spanned(after, "device storage changed a layout record or permutation"));
        }
        let declaration = &declarations[analysis.positions[&cq_syntax::symbol_name(&before.relation)]];
        let types: Vec<_> = before.columns.iter().map(|&column| &declaration.columns[column].ty).collect();
        if before.provider != default_layout(&types, before.key) {
            return Err(syn::Error::new_spanned(before, "device storage requires default layout providers"));
        }
        let device_types = types.iter().map(|ty| symbols.device_type(ty)).collect::<Vec<_>>();
        let types = device_types.iter().collect::<Vec<_>>();
        if after.provider != layout_type(&types, before.key) {
            return Err(syn::Error::new_spanned(after, "device storage layout provider differs from its recorded heading"));
        }
    }
    Ok(())
}

/// Validate the concrete realization at the printer boundary.
pub fn check_device(program: &Program) -> syn::Result<()> {
    let analysis = storage::contract::check(program)?;
    let symbols = Symbols::from_program(program)?;
    let declarations = &program.incremental.strata.source.declarations;
    for (provider, declaration) in program.providers.iter().zip(declarations) {
        let types = declaration.columns.iter().map(|column| &column.ty).collect::<Vec<_>>();
        if provider != &device_provider(&types, declaration.value.is_some(), &symbols, program.device.as_ref().expect("checked advice").ratio)? {
            return Err(syn::Error::new_spanned(provider, "GPU printer requires checked device_storage providers"));
        }
    }
    for layout in program.layouts.iter().chain(&program.delta_layouts) {
        let declaration = &declarations[analysis.positions[&cq_syntax::symbol_name(&layout.relation)]];
        let device_types = layout.columns.iter().map(|&i| symbols.device_type(&declaration.columns[i].ty)).collect::<Vec<_>>();
        let types = device_types.iter().collect::<Vec<_>>();
        if layout.provider != layout_type(&types, layout.key) {
            return Err(syn::Error::new_spanned(layout, "GPU printer requires checked device layout providers"));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_explicit_symbol_codecs_and_rejects_ambiguous_arguments() {
        let symbols: Symbols = syn::parse_quote!(symbols(String => TextCodec, SymbolId => ids::Codec,));
        assert_eq!(symbols.codec(&syn::parse_quote!(String)), Some(&syn::parse_quote!(TextCodec)));
        assert_eq!(symbols.codec(&syn::parse_quote!(SymbolId)), Some(&syn::parse_quote!(ids::Codec)));
        assert!(syn::parse2::<Symbols>(quote!(symbols(String => A, String => B))).is_err());
        assert!(syn::parse2::<Symbols>(quote!(symbols(u32 => A))).is_err());
        assert!(syn::parse2::<Symbols>(quote!(symbols(String))).is_err());
        assert!(syn::parse2::<Symbols>(quote!(symbols(String => A) extra)).is_err());
        assert!(syn::parse2::<Symbols>(quote!(codecs(String => A))).is_err());
    }

    #[test]
    fn symbol_storage_preserves_logic_and_checks_codec_metadata() {
        let mut incremental = source().incremental;
        for declaration in &mut incremental.strata.source.declarations {
            for column in &mut declaration.columns {
                column.ty = syn::parse_quote!(String);
            }
        }
        let source = cq_datalog_storage::compile(&incremental).unwrap();
        let target = compile_with(&source, quote!(symbols(String => TextCodec))).unwrap();
        assert_eq!(source.incremental, target.incremental);
        assert_eq!(target.providers[0].canonical_set, syn::parse_quote!(
            ::gpu_datalog_runtime::DeviceRuns<u64>
        ));
        assert_eq!(Symbols::from_program(&target).unwrap().codec(&syn::parse_quote!(String)), Some(&syn::parse_quote!(TextCodec)));
        check(&source, &target).unwrap();
        check_device(&target).unwrap();
        let roundtrip: Program = syn::parse2(quote!(#target)).unwrap();
        assert_eq!(roundtrip, target);
        assert!(compile(&source).is_err());
        assert!(compile_with(&source, quote!(symbols(other::String => TextCodec))).is_err());
        let mut changed = target.clone();
        changed.device.as_mut().unwrap().symbols[0].1 = syn::parse_quote!(WrongCodec);
        assert!(check(&source, &changed).is_err());
        assert!(check_device(&changed).is_err());
        let mut changed = target.clone();
        changed.layouts[0].provider = syn::parse_quote!(::gpu_datalog_runtime::DeviceLayout<(), (u32, u32,)>);
        assert!(check(&source, &changed).is_err());
    }

    #[test]
    fn wide_symbol_schema_uses_three_lanes_without_changing_host_types() {
        let source = cq_datalog_storage::compile(&syn::parse_quote! {
            struct Wide;
            relation E(a:String,b:String,c:String,d:String,e:String) = (input);
            relation R(a:String,b:String,c:String,d:String,e:String);
            output R;
            R(*a,*b,*c,*d,*e) :- E(a,b,c,d,e);
            strata { component [R]; }
            incremental { scheme seminaive_covering;
                delta R = Changes;
                variant 0 seed { R(*a,*b,*c,*d,*e) :- E(a,b,c,d,e); }
            }
        }).unwrap();
        let target = compile_with(&source, quote!(symbols(String => TextCodec))).unwrap();
        assert_eq!(source.incremental, target.incremental);
        let schema = Symbols::from_program(&target).unwrap();
        let types = source.incremental.strata.source.declarations[0].columns.iter()
            .map(|column| schema.device_type(&column.ty)).collect::<Vec<_>>();
        assert_eq!(word_type(&types).unwrap(), syn::parse_quote!([u64; 3]));
        check_device(&target).unwrap();
    }

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
            (vec![syn::parse_quote!(i64), syn::parse_quote!(i64), syn::parse_quote!(u8)], syn::parse_quote!([u64; 3])),
            (vec![syn::parse_quote!(i64); 4], syn::parse_quote!([u64; 4])),
        ] {
            assert_eq!(word_type(&types).unwrap(), expected);
        }
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
    fn stored_advice_roundtrips_and_rejects_invalid_ratios() {
        let source = source();
        let default = compile(&source).unwrap();
        assert_eq!(default.device.as_ref().unwrap().ratio, 4);
        let target = compile_with(&source, quote!(ratio(8))).unwrap();
        let roundtrip: Program = syn::parse2(quote!(#target)).unwrap();
        assert_eq!(roundtrip, target);
        check_device(&roundtrip).unwrap();
        assert!(compile_with(&source, quote!(ratio(1))).is_err());
        assert!(compile_with(&source, quote!(ratio(0))).is_err());
        assert!(compile_with(&source, quote!(ratio(2), ratio(8))).is_err());
        let mut forged = target.clone();
        forged.device.as_mut().unwrap().ratio = 4;
        assert!(check_device(&forged).is_err(), "initializers must consume the recorded ratio");
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
    fn rejects_unsupported_relation_schemas() {
        for (name, message) in [
            ("bool", "supports only"),
            ("u64", "supports only"),
            ("String", "supports only"),
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
