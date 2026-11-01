//! Device realizations of the existing checked Datalog storage records.
use cq_datalog_support::ensure;
use cq_lang_datalog_storage::{
    self as storage, DeviceRelation, DeviceScratch, DeviceState, DeviceStorage, Hasher, LayoutProtocol, Program,
    Provider, Providers, provider,
};
use proc_macro2::TokenStream;
use quote::quote;
use syn::{Expr, Ident, Type};

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
            if symbols.0[..index].iter().any(|(previous, _)| previous == ty) {
                return Err(syn::Error::new_spanned(ty, "symbol declarations require unique logical types"));
            }
        }
        symbols.validate(program)?;
        Ok(symbols)
    }
}

/// Emit the native packed word from recorded scalar metadata. Rustc resolves
/// aliases and selects the arena's scalar or most-significant-first lane ABI.
pub fn word_type<'a>(types: impl IntoIterator<Item = &'a Type>) -> syn::Result<Type> {
    let types: Vec<_> = types.into_iter().collect();
    let lanes = quote!({
        let bits = 0usize #(+ <#types as ::gpu_datalog_runtime::Scalar>::COLUMN.bits() as usize)*;
        let lanes = bits.div_ceil(64);
        if lanes == 0 { 1 } else { lanes }
    });
    syn::parse2(quote!(<::gpu_datalog_runtime::pack::WordWidth<
        #lanes, { (#lanes) <= 2 }
    > as ::gpu_datalog_runtime::pack::NativeWord>::Word))
}

/// Build a CQ binding in its recorded physical order; callers supply a checked key.
pub fn layout_type(types: &[&Type], key: usize) -> Type {
    let (keys, values) = types.split_at(key);
    syn::parse_quote!(::gpu_datalog_runtime::DeviceLayout<(#(#keys,)*), (#(#values,)*)>)
}

/// Device tokens (verbatim spellings; G2 owns their device facts): packed
/// bulk layout views and the relation-order delta/NEW views.
pub const DEVICE_LAYOUT: &str = "device_layout";
pub const DEVICE_VIEW: &str = "device_view";

fn device_tokens() -> [provider::Token; 2] {
    [
        provider::declared(
            DEVICE_LAYOUT,
            provider::Spelling::Verbatim,
            Some(LayoutProtocol::Bulk),
            provider::Admission::None,
            false,
            provider::TaskHasher::Fx,
        ),
        provider::declared(
            DEVICE_VIEW,
            provider::Spelling::Verbatim,
            None,
            provider::Admission::None,
            false,
            provider::TaskHasher::Fx,
        ),
    ]
}

fn instance(token: &str, ty: Type) -> Provider {
    Provider { token: Ident::new(token, proc_macro2::Span::call_site()), ty }
}

/// The default `rows_vec` buffer of a relation's full rows.
fn rows(types: &[&Type]) -> Provider {
    let token = provider::rows_vec();
    let ty = provider::spell(&token, &syn::parse_quote!((#(#types,)*)), &syn::parse_quote!(()), Hasher::None)
        .expect("rows_vec spells unhashed rows");
    Provider { token: token.token, ty }
}

/// The default Storage relation providers, keeping the source's canonical layout.
fn default_provider(types: &[&Type], canonical: usize) -> Providers {
    Providers {
        canonical,
        delta: rows(types),
        next: rows(types),
        delta_init: syn::parse_quote!(::std::vec::Vec::new()),
        next_init: syn::parse_quote!(::std::vec::Vec::new()),
    }
}

/// IDB delta and NEW are relation-order device views of fresh rows, so
/// publishing NEW as the next delta remains a move. EDB relations keep the
/// defaults required for their unused IDB fields.
fn device_provider(logical: &[&Type], device: &[&Type], edb: bool, canonical: usize) -> syn::Result<Providers> {
    if edb { return Ok(default_provider(logical, canonical)); }
    let view = layout_type(device, device.len());
    let init = empty_view(&view, &word_type(device.iter().copied())?);
    let view = instance(DEVICE_VIEW, view);
    Ok(Providers { canonical, delta: view.clone(), next: view, delta_init: init.clone(), next_init: init })
}

/// The default Storage layout: a layout keyed by every column is a `hash_set`,
/// otherwise a `hash_map`, both with the default Fx hasher.
fn default_layout(types: &[&Type], key: usize) -> (Provider, Hasher, LayoutProtocol) {
    let (keys, values) = types.split_at(key);
    let token = if values.is_empty() { provider::hash_set() } else { provider::hash_map() };
    let ty = provider::spell(&token, &syn::parse_quote!((#(#keys,)*)), &syn::parse_quote!((#(#values,)*)), Hasher::Fx)
        .expect("base tokens spell the Fx hasher");
    let protocol = token.protocol.expect("base layout tokens declare a protocol");
    (Provider { token: token.token, ty }, Hasher::Fx, protocol)
}

fn empty_view(provider: &Type, word: &Type) -> Expr {
    syn::parse_quote!(<#provider>::from_runs::<#word>(
        __cq_gpu_pool.clone(), <#provider>::derive_packing()?, ::std::vec::Vec::new())?)
}

/// A device view maintained as sorted packed runs in its own column order.
fn device_state(provider: Type, types: &[&Type], ratio: usize) -> syn::Result<DeviceState> {
    let word = word_type(types.iter().copied())?;
    let state: Type = syn::parse_quote!(::gpu_datalog_runtime::DeviceRuns<#word>);
    Ok(DeviceState {
        packing: syn::parse_quote!(<#provider>::derive_packing()?),
        state_init: syn::parse_quote!(<#state>::new(__cq_gpu_pool.clone(), #ratio)?),
        init: empty_view(&provider, &word),
        provider,
        word,
        state,
    })
}

fn device_relation(types: &[&Type]) -> syn::Result<DeviceRelation> {
    let columns = types.iter().map(|ty| quote!(<#ty as ::gpu_datalog_runtime::Scalar>::COLUMN));
    Ok(DeviceRelation {
        word: word_type(types.iter().copied())?,
        packing: syn::parse_quote!(::gpu_datalog_runtime::Packing::with_columns(&[#(#columns),*])?),
        candidates: syn::parse_quote!(::std::vec::Vec<::gpu_datalog_runtime::CandidateBatch>),
        candidates_init: syn::parse_quote!(::std::vec::Vec::new()),
        arena: syn::parse_quote!(::gpu_datalog_runtime::CandidateArena),
        arena_init: syn::parse_quote!(::gpu_datalog_runtime::CandidateArena::new(__cq_gpu_pool.clone())),
        next: syn::parse_quote!(fresh_run),
    })
}

/// Realize a default Storage program on the device: every layout becomes a
/// bulk-maintained `DeviceLayout` view over its own sorted runs, and every
/// relation, layout and negative key set records its state and initializers.
fn realize(source: &Program, symbols: &Symbols, ratio: usize) -> syn::Result<Program> {
    let analysis = storage::contract::check(source)?;
    let declarations = &source.incremental.strata.source.declarations;
    let device_types = |relation: usize, columns: &[usize]| -> Vec<Type> {
        columns.iter().map(|&column| symbols.device_type(&declarations[relation].columns[column].ty)).collect()
    };
    let mut target = source.clone();
    for token in device_tokens() {
        provider::declare(&mut target.tokens, token);
    }
    let mut relations = Vec::new();
    for (index, (provider, declaration)) in target.providers.iter_mut().zip(declarations).enumerate() {
        let logical = declaration.columns.iter().map(|column| &column.ty).collect::<Vec<_>>();
        let device = device_types(index, &(0..declaration.arity()).collect::<Vec<_>>());
        let device = device.iter().collect::<Vec<_>>();
        *provider = device_provider(&logical, &device, declaration.value.is_some(), provider.canonical)?;
        relations.push(device_relation(&device)?);
    }
    let mut states = [Vec::new(), Vec::new()];
    for (states, layouts) in states.iter_mut().zip([&mut target.layouts, &mut target.delta_layouts]) {
        for layout in layouts.iter_mut() {
            let types = device_types(analysis.positions[&cq_syntax::symbol_name(&layout.relation)], &layout.columns);
            let types = types.iter().collect::<Vec<_>>();
            layout.provider = instance(DEVICE_LAYOUT, layout_type(&types, layout.key));
            layout.hasher = Hasher::None;
            layout.protocol = LayoutProtocol::Bulk;
            states.push(device_state(layout.provider.ty.clone(), &types, ratio)?);
        }
    }
    let [layouts, delta_layouts] = states;
    let negations = source.negations.iter().map(|negative| {
        let types = device_types(negative.relation, &negative.columns);
        let types = types.iter().collect::<Vec<_>>();
        device_state(layout_type(&types, types.len()), &types, ratio)
    }).collect::<syn::Result<Vec<_>>>()?;
    target.device = Some(DeviceStorage {
        symbols: symbols.0.clone(),
        ratio,
        relations,
        layouts,
        delta_layouts,
        negations,
        scratch: DeviceScratch { arenas: syn::parse_quote!(component), release: syn::parse_quote!(component) },
    });
    Ok(target)
}

/// Restore default providers and erase the device records.
fn restore(target: &Program) -> syn::Result<Program> {
    let analysis = storage::contract::check(target)?;
    let declarations = &target.incremental.strata.source.declarations;
    let mut source = target.clone();
    source.device = None;
    source.tokens.retain(|token| token.token != DEVICE_LAYOUT && token.token != DEVICE_VIEW);
    for (provider, declaration) in source.providers.iter_mut().zip(declarations) {
        let types = declaration.columns.iter().map(|column| &column.ty).collect::<Vec<_>>();
        *provider = default_provider(&types, provider.canonical);
    }
    for layout in source.layouts.iter_mut().chain(&mut source.delta_layouts) {
        let declaration = &declarations[analysis.positions[&cq_syntax::symbol_name(&layout.relation)]];
        let types = layout.columns.iter().map(|&column| &declaration.columns[column].ty).collect::<Vec<_>>();
        (layout.provider, layout.hasher, layout.protocol) = default_layout(&types, layout.key);
    }
    Ok(source)
}

pub fn compile(source: &Program) -> syn::Result<Program> {
    compile_with(source, TokenStream::new())
}

pub fn compile_with(source: &Program, args: TokenStream) -> syn::Result<Program> {
    let Advice { symbols, ratio } = syn::parse2(args)?;
    symbols.validate(source)?;
    let target = realize(source, &symbols, ratio)?;
    check(source, &target)?;
    Ok(target)
}

/// Formal rule CK-DL-DeviceStorage: only provider realizations, layout
/// protocols and explicit device records change; restoring default providers
/// and erasing the device records gives the source exactly.
pub fn check(source: &Program, target: &Program) -> syn::Result<()> {
    ensure(source.device.is_none(), source, "device storage requires a source without device records")?;
    check_device(target)?;
    ensure(
        restore(target)? == *source,
        target,
        "device storage changed logical, read, layout, negative, policy, certificate, or default provider records",
    )
}

/// Validate the concrete realization at the printer boundary: the program is
/// exactly the device realization of its restored default source.
pub fn check_device(program: &Program) -> syn::Result<()> {
    let advice = program.device.as_ref().ok_or_else(||
        syn::Error::new_spanned(program, "missing device storage records"))?;
    let symbols = Symbols::from_program(program)?;
    ensure(
        realize(&restore(program)?, &symbols, advice.ratio)? == *program,
        program,
        "GPU printer requires checked device_storage providers and records",
    )
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
        assert!(syn::parse2::<Symbols>(quote!(symbols(u32 => A))).is_ok());
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
        let ty: Type = syn::parse_quote!(::gpu_datalog_runtime::Symbol<String, TextCodec>);
        let word = word_type([&ty, &ty]).unwrap();
        let device = target.device.as_ref().unwrap();
        assert_eq!(device.layouts[target.providers[0].canonical].state, syn::parse_quote!(
            ::gpu_datalog_runtime::DeviceRuns<#word>
        ));
        assert_eq!(device.relations[0].word, word);
        assert_eq!(Symbols::from_program(&target).unwrap().codec(&syn::parse_quote!(String)), Some(&syn::parse_quote!(TextCodec)));
        check(&source, &target).unwrap();
        check_device(&target).unwrap();
        let roundtrip: Program = syn::parse2(quote!(#target)).unwrap();
        assert_eq!(roundtrip, target);
        // Unmapped logical types are checked by rustc's Scalar bound, not by
        // recognizing the spelling of String in the query compiler.
        assert!(compile(&source).is_ok());
        assert!(compile_with(&source, quote!(symbols(other::String => TextCodec))).is_err());
        let mut changed = target.clone();
        changed.device.as_mut().unwrap().symbols[0].1 = syn::parse_quote!(WrongCodec);
        assert!(check(&source, &changed).is_err());
        assert!(check_device(&changed).is_err());
        let mut changed = target.clone();
        changed.layouts[0].provider.ty = syn::parse_quote!(::gpu_datalog_runtime::DeviceLayout<(), (u32, u32,)>);
        assert!(check(&source, &changed).is_err());
    }

    #[test]
    fn wide_symbol_schema_records_each_column_without_changing_host_types() {
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
        let word = word_type(&types).unwrap();
        assert_eq!(quote!(#word).to_string().matches("Scalar").count(), 10);
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
    fn scalar_aliases_are_resolved_by_rustc() {
        let ty: Type = syn::parse_quote!(MyScalar);
        let word = word_type([&ty]).unwrap();
        let text = quote!(#word).to_string();
        assert!(text.contains("< MyScalar as :: gpu_datalog_runtime :: Scalar > :: COLUMN"));
        assert!(text.contains("NativeWord"));
    }

    #[test]
    fn packing_boundaries_and_layout_keys() {
        let empty = word_type(std::iter::empty::<&Type>()).unwrap();
        assert!(matches!(empty, Type::Path(_)));
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
        let ty: Type = syn::parse_quote!(i32);
        let word = word_type([&ty, &ty]).unwrap();
        // EDB relations keep the defaults of their unused IDB fields.
        assert_eq!(target.providers[0], source.providers[0]);
        let view: Provider = syn::parse_quote!(@device_view as ::gpu_datalog_runtime::DeviceLayout<(i32, i32,), ()>);
        let idb = &target.providers[2];
        assert_eq!((&idb.delta, &idb.next, idb.canonical), (&view, &view, source.providers[2].canonical));
        assert_eq!(idb.delta_init, idb.next_init);
        let device = target.device.as_ref().unwrap();
        assert_eq!(device.layouts[idb.canonical].state, syn::parse_quote!(::gpu_datalog_runtime::DeviceRuns<#word>));
        assert_eq!(device.layouts.len(), target.layouts.len());
        assert_eq!(device.delta_layouts.len(), target.delta_layouts.len());
        assert_eq!(device.negations.len(), target.negations.len());
        assert_eq!(device.negations[0].provider, syn::parse_quote!(::gpu_datalog_runtime::DeviceLayout<(i32,), ()>));
        for (before, after) in source.layouts.iter().zip(&target.layouts)
            .chain(source.delta_layouts.iter().zip(&target.delta_layouts))
        {
            assert_eq!(after.protocol, LayoutProtocol::Bulk);
            let mut restored = after.clone();
            restored.provider = before.provider.clone();
            restored.hasher = before.hasher;
            restored.protocol = before.protocol;
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

    /// A `Custom` instance of an authored verbatim token declaring `protocol`.
    fn custom(program: &mut Program, protocol: Option<LayoutProtocol>) -> Provider {
        let name = match protocol {
            None => "custom_rows",
            Some(LayoutProtocol::Set) => "custom_set",
            Some(LayoutProtocol::GroupedMap) => "custom_map",
            Some(LayoutProtocol::Bulk) => "custom_bulk",
        };
        provider::declare(&mut program.tokens, provider::declared(
            name, provider::Spelling::Verbatim, protocol, provider::Admission::None, false, provider::TaskHasher::Fx,
        ));
        instance(name, syn::parse_quote!(Custom))
    }

    #[test]
    fn rejects_nondefault_sources_and_incorrect_device_providers() {
        // Layout providers are mirrored by their device records.
        let mutations: &[fn(&mut Program)] = &[
            |p| { let rows = custom(p, None); p.providers[2].delta = rows.clone(); p.providers[2].next = rows; },
            |p| p.providers[2].delta_init = syn::parse_quote!(custom()),
            |p| p.providers[2].next_init = syn::parse_quote!(custom()),
            |p| {
                p.layouts[0].provider = custom(p, Some(p.layouts[0].protocol));
                p.layouts[0].hasher = Hasher::None;
                if let Some(device) = &mut p.device { device.layouts[0].provider = syn::parse_quote!(Custom); }
            },
            |p| {
                p.delta_layouts[0].provider = custom(p, Some(p.delta_layouts[0].protocol));
                p.delta_layouts[0].hasher = Hasher::None;
                if let Some(device) = &mut p.device { device.delta_layouts[0].provider = syn::parse_quote!(Custom); }
            },
            |p| {
                let protocol = if p.layouts[0].protocol == LayoutProtocol::Bulk {
                    LayoutProtocol::GroupedMap
                } else {
                    LayoutProtocol::Bulk
                };
                // An authored token declaring the other protocol, same spelling.
                let ty = p.layouts[0].provider.ty.clone();
                p.layouts[0].provider = Provider { ty, ..custom(p, Some(protocol)) };
                p.layouts[0].hasher = Hasher::None;
                p.layouts[0].protocol = protocol;
            },
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
        let canonical = wrong_width.providers[0].canonical;
        wrong_width.device.as_mut().unwrap().layouts[canonical].state = syn::parse_quote!(::gpu_datalog_runtime::DeviceRuns<u128>);
        assert!(check(&source, &wrong_width).is_err());
        let records: &[fn(&mut DeviceStorage)] = &[
            |d| d.relations[2].candidates_init = syn::parse_quote!(custom()),
            |d| d.relations[2].arena = syn::parse_quote!(Custom),
            |d| d.relations[2].next = syn::parse_quote!(empty_then_extend),
            |d| d.relations[0].packing = syn::parse_quote!(custom()),
            |d| d.layouts[0].state_init = syn::parse_quote!(custom()),
            |d| d.layouts[0].packing = syn::parse_quote!(custom()),
            |d| d.delta_layouts[0].init = syn::parse_quote!(custom()),
            |d| d.negations[0].state = syn::parse_quote!(Custom),
            |d| d.scratch.release = syn::parse_quote!(never),
            |d| { d.negations.pop(); },
        ];
        for (index, mutate) in records.iter().enumerate() {
            let mut changed = target.clone();
            mutate(changed.device.as_mut().unwrap());
            assert!(check(&source, &changed).is_err(), "device record {index}");
            assert!(check_device(&changed).is_err(), "printer boundary {index}");
        }
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
    fn scalar_trait_eligibility_is_deferred_to_rustc() {
        for name in ["bool", "u64", "String", "Alias"] {
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
            let target = compile(&source).unwrap();
            check(&source, &target).unwrap();
        }
    }

    #[test]
    fn wide_relations_use_recorded_widths_without_changing_scalar_layout_types() {
        let mut incremental = source().incremental;
        for declaration in &mut incremental.strata.source.declarations {
            for column in &mut declaration.columns {
                column.ty = syn::parse_quote!(i64);
            }
        }
        let source = cq_datalog_storage::compile(&incremental).unwrap();
        let target = compile(&source).unwrap();
        let ty: Type = syn::parse_quote!(i64);
        let word = word_type([&ty, &ty]).unwrap();
        let device = target.device.as_ref().unwrap();
        assert_eq!(device.layouts[target.providers[0].canonical].state, syn::parse_quote!(::gpu_datalog_runtime::DeviceRuns<#word>));
        assert_eq!(device.layouts[target.providers[2].canonical].word, word);
        assert_eq!(target.layouts[0].provider, syn::parse_quote!(@device_layout as ::gpu_datalog_runtime::DeviceLayout<(), (i64, i64,)>));
        check(&source, &target).unwrap();
    }

    #[test]
    fn preserves_authored_extra_layouts_and_optional_certificates() {
        let mut source = source();
        source.certificates.clear();
        let mut extra = source.layouts[0].clone();
        extra.columns.reverse();
        extra.key = 0;
        let ty: Type = syn::parse_quote!(i32);
        (extra.provider, extra.hasher, extra.protocol) = default_layout(&[&ty, &ty], 0);
        source.layouts.push(extra);
        storage::contract::check(&source).unwrap();
        let target = compile(&source).unwrap();
        assert!(target.certificates.is_empty());
        assert_eq!(target.layouts.len(), source.layouts.len());
        assert_eq!(target.layouts.last().unwrap().columns, source.layouts.last().unwrap().columns);
        check(&source, &target).unwrap();
    }
}
