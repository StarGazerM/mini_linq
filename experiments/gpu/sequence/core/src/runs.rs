//! Checked bindings for declared packed-run inputs.
use std::collections::BTreeMap;
use cq_lang_structures::{Program, Structure, physical::symbol_name};
use quote::quote;
use syn::{GenericArgument, PathArguments, Type};

pub(crate) fn layout_types(ty: &Type) -> syn::Result<Option<(Vec<Type>, usize)>> {
    let Type::Path(path) = ty else { return Ok(None) };
    let Some(segment) = path.path.segments.last() else { return Ok(None) };
    if crate::native_abi(ty) != Some(crate::NativeAbi::Runs) { return Ok(None) }
    let PathArguments::AngleBracketed(arguments) = &segment.arguments else {
        return Err(syn::Error::new_spanned(ty, "GPU-Runs: DeviceLayout requires key and value tuple types"));
    };
    let arguments = arguments.args.iter().collect::<Vec<_>>();
    let [GenericArgument::Type(keys), GenericArgument::Type(values)] = arguments.as_slice() else {
        return Err(syn::Error::new_spanned(ty, "GPU-Runs: DeviceLayout requires exactly two tuple types"));
    };
    // A catalog admission pattern is not a concrete packed-run schema.
    if matches!(keys, Type::Infer(_)) && matches!(values, Type::Infer(_)) {
        return Ok(None);
    }
    fn fields(ty: &Type) -> syn::Result<Vec<Type>> {
        let Type::Tuple(tuple) = ty else { return Err(syn::Error::new_spanned(ty, "GPU-Runs: layout columns must be tuples")) };
        Ok(tuple.elems.iter().cloned().collect())
    }
    let mut columns = fields(keys)?;
    let key_count = columns.len();
    columns.extend(fields(values)?);
    Ok(Some((columns, key_count)))
}

pub(crate) fn symbol(ty: &Type) -> Option<(&Type, &Type)> {
    let Type::Path(path) = ty else { return None };
    let segment = path.path.segments.last()?;
    if crate::native_abi(ty) != Some(crate::NativeAbi::Symbol) { return None }
    let PathArguments::AngleBracketed(arguments) = &segment.arguments else { return None };
    let mut args = arguments.args.iter();
    match (args.next()?, args.next()?, args.next()) {
        (GenericArgument::Type(logical), GenericArgument::Type(codec), None) => Some((logical, codec)),
        _ => None,
    }
}

/// Compared columns share one checked encoding for each logical Rust type.
#[derive(Default)]
pub(crate) struct Codecs(Vec<(Type, Type)>);

impl Codecs {
    pub fn get(&self, logical: &Type) -> Option<&Type> {
        self.0.iter().find(|(ty, _)| ty == logical).map(|(_, codec)| codec)
    }

    /// Formal rule GPU-CodecIdentity: conflicting encodings of a logical symbol type are rejected before capture lowering.
    pub fn record(&mut self, logical: &Type, codec: &Type) -> syn::Result<()> {
        if let Some(previous) = self.get(logical) {
            if previous != codec {
                return Err(syn::Error::new_spanned(codec, "conflicting codecs for a logical symbol type"));
            }
        } else {
            self.0.push((logical.clone(), codec.clone()));
        }
        Ok(())
    }

    pub fn check(program: &Program) -> syn::Result<Self> {
        let mut codecs = Self::default();
        for entry in &program.structures.entries {
            let keyed = match &entry.structure {
                Structure::Host { keyed, .. } | Structure::Built { keyed, .. } => keyed,
                _ => continue,
            };
            if let Some((columns, _)) = layout_types(&keyed.ty)? {
                for column in columns {
                    if let Some((logical, codec)) = symbol(&column) {
                        codecs.record(logical, codec)?;
                    }
                }
            }
        }
        Ok(codecs)
    }
}

pub(crate) fn scalar(ty: &Type) -> proc_macro2::TokenStream {
    quote!(<#ty as ::gpu_datalog_runtime::Scalar>::COLUMN)
}


// Formal rule GPU-Runs: only declared run handles bind packed inputs; every
// lookup uses a leading packed prefix and every physical run remains visible.
pub fn bind(source: &Program) -> syn::Result<Program> {
    let analysis = cq_lang_structures::contract::check(source)?;
    let definitions = source.physical.program.definitions.iter()
        .map(|definition| (symbol_name(&definition.name), definition.clone())).collect();
    let mut hosts = BTreeMap::new();
    for entry in &source.structures.entries {
        let Structure::Host { reader, keyed, .. } = &entry.structure else { continue };
        if !matches!(reader, cq_lang_structures::ReaderRef::Scan) { continue }
        let Some((types, key_count)) = layout_types(&keyed.ty)? else { continue };
        let host = &analysis.physical.hosts[&symbol_name(&entry.name)];
        let layout = &analysis.host_layouts[host];
        let row = &analysis.physical.rows[&symbol_name(&entry.name)];
        let origin = crate::rows::origin(&symbol_name(&entry.name), &definitions)?
            .ok_or_else(|| syn::Error::new_spanned(entry, "GPU-Runs: scan has no declared input origin"))?;
        if layout.len() != types.len() { return Err(syn::Error::new_spanned(entry, "GPU-Runs: declared packed arity differs from input")); }
        for (column, ty) in &row.types {
            if let Some(source) = origin.columns.get(column) {
                let position = layout.iter().position(|name| name == source).unwrap();
                let logical = symbol(&types[position]).map_or(&types[position], |(logical, _)| logical);
                if ty != logical { return Err(syn::Error::new_spanned(entry, "GPU-Runs: declared packed column type differs from input")); }
            }
        }
        hosts.insert(host.clone(), (keyed.ty.clone(), key_count));
    }
    let mut target = source.clone();
    for entry in &mut target.structures.entries {
        let Structure::Built { keyed, .. } = &mut entry.structure else { continue };
        let name = symbol_name(&entry.name);
        let Some(origin) = crate::rows::origin(&name, &definitions)? else { continue };
        let Some((ty, _key_count)) = hosts.get(&origin.host) else { continue };
        // A selected source is not the raw run: keep the computed Built index.
        if origin.selection.is_some() || origin.predicate { continue; }
        let keys = keyed.level_names().concat();
        let layout = &analysis.host_layouts[&origin.host];
        if keys.len() > layout.len() || keys.iter().enumerate().any(|(i,key)| origin.columns.get(key) != layout.get(i)) {
            return Err(syn::Error::new_spanned(&entry.name, format!(
                "GPU-Runs: lookup key {:?} (source columns {:?}) must be a leading prefix of sorted packed layout {:?} for input {}",
                keys, keys.iter().map(|key| origin.columns.get(key)).collect::<Vec<_>>(),
                layout, origin.host,
            )));
        }
        keyed.ty = ty.clone();
    }
    cq_lang_structures::contract::check(&target)?;
    Ok(target)
}

/// Formal rule GPU-Representation: provider and codec declarations, never
/// logical Rust type spellings, supply the facts checked by BE-RustRepresentation.
pub fn record_representation(source: &cq_lang_sequence::Program) -> syn::Result<cq_lang_sequence::Program> {
    let analysis = cq_lang_structures::contract::check(&source.structures)?;
    let mut hosts = BTreeMap::new();
    for entry in &source.structures.structures.entries {
        let Structure::Host { reader: cq_lang_structures::ReaderRef::Scan, keyed, .. } = &entry.structure else { continue };
        let name = symbol_name(&entry.name);
        let Some(host) = analysis.physical.hosts.get(&name) else { continue };
        if let Some((columns, _)) = layout_types(&keyed.ty)? {
            let layout = &analysis.host_layouts[host];
            if columns.len() != layout.len() {
                return Err(syn::Error::new_spanned(entry, "GPU-Runs: declared packed arity differs from input"));
            }
            for (column, ty) in layout.iter().zip(columns) {
                hosts.insert((host.clone(), column.clone()), symbol(&ty).is_some());
            }
        } else if crate::native_abi(&keyed.ty) == Some(crate::NativeAbi::Columns) {
            for column in &analysis.host_layouts[host] {
                hosts.insert((host.clone(), column.clone()), false);
            }
        }
    }
    let mut target = source.clone();
    // An authored binding annotation declares the identity of its Rust result.
    // Keep the type tokens opaque: rustc checks its Scalar ABI and supplies the
    // layout when the generated boundary or callable uses that result.
    for definition in &source.structures.physical.program.definitions {
        if let cq_lang_structures::physical::Operator::Bind(bind) = &definition.operator
            && bind.annotation.is_some()
        {
            for binding in cq_lang_structures::syntax::rust::bindings(&bind.pattern) {
                let column = symbol_name(&binding);
                if !target.representation.encoded.contains(&column) {
                    target.representation.identity.insert(column);
                }
            }
        }
    }
    for definition in &source.structures.physical.program.definitions {
        let cq_lang_structures::physical::Operator::Rename(rename) = &definition.operator else { continue };
        let host = symbol_name(&rename.relation);
        // Rename establishes the column identity once. Selections preserve it;
        // recording a representation must not demand a single-column interval.
        for mapping in &rename.mapping {
            let column = symbol_name(&mapping.target);
            match hosts.get(&(host.clone(), symbol_name(&mapping.source))) {
                Some(true) => {
                    target.representation.identity.remove(&column);
                    target.representation.encoded.insert(column.clone());
                }
                Some(false) if !target.representation.encoded.contains(&column) => {
                    target.representation.identity.insert(column.clone());
                }
                _ => {}
            }
        }
    }
    for entry in &source.structures.structures.entries {
        let Structure::Built { keyed, .. } = &entry.structure else { continue };
        if crate::native_abi(&keyed.ty) != Some(crate::NativeAbi::Range) {
            continue;
        }
        // The native range provider publishes Scalar-ABI rows, including
        // computed columns. Explicit codec facts still take precedence.
        let row = &analysis.physical.rows[&symbol_name(&entry.name)];
        for column in row.types.keys() {
            if !target.representation.encoded.contains(column) {
                target.representation.identity.insert(column.clone());
            }
        }
    }
    let codecs = Codecs::check(&source.structures)?;
    for row in analysis.physical.rows.values() {
        for (column, logical) in &row.types {
            if codecs.get(logical).is_some() {
                target.representation.identity.remove(column);
                target.representation.encoded.insert(column.clone());
            }
        }
    }
    Ok(target)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn authored_binding_annotations_record_identity_without_type_inspection() {
        let program: cq_lang_sequence::Program = syn::parse_quote! {
            physical {
                relation R(x:i64) = (&data.r);
                r = pipe rename R@0 {x -> x};
                bound = pipe bind r with value:UserDefinedScalar = (convert(x));
                result = pipe canon project bound keep {value};
                return result as answer(value);
            }
            structures {
                r host scan = ::cq_gpu_runtime::DeviceColumns<'static,(i64,)> [];
                result canon = sort using ::cq_gpu_sequence_runtime::canon::PackedCanon;
            }
            sequence {
                input = store rows r;
                output_rows = store canon map result map bound flatten tile 256 input;
                output count;
            }
        };
        let recorded = record_representation(&program).unwrap();
        assert!(recorded.representation.identity.contains("value"));
        let facts = cq_lang_sequence::contract::check(&recorded).unwrap();
        let mut operator = recorded.structures.physical.program.definitions[1].operator.clone();
        assert!(!facts.rust_requires_host(&operator));
        let cq_lang_structures::physical::Operator::Bind(bind) = &mut operator else {
            unreachable!()
        };
        bind.annotation = None;
        assert!(facts.rust_requires_host(&operator));
    }

    #[test]
    fn representation_provenance_does_not_require_single_column_intervals() {
        let program: cq_lang_sequence::Program = syn::parse_quote! {
            physical {
                relation R(a:i64,b:i64) = (&data.r);
                r = pipe rename R@0 {a -> x,b -> y};
                selected = pipe select r if (x > 1,y > 2);
                result = pipe canon project selected keep {x,y};
                return result as answer(x,y);
            }
            structures {
                r host scan = ::cq_gpu_runtime::DeviceColumns<'static,(i64,i64)> [];
                result canon = sort using ::cq_gpu_sequence_runtime::canon::PackedCanon;
            }
            sequence {
                input = store rows r;
                result_rows = store canon map result compact selected flatten tile 256 input;
                output count;
            }
        };
        let recorded = record_representation(&program).unwrap();
        assert!(recorded.representation.identity.contains("x"));
        assert!(recorded.representation.identity.contains("y"));
    }

    #[test]
    fn checks_declared_layout_width_and_scalar_domains() {
        let valid: Type = syn::parse_quote!(::gpu_datalog_runtime::DeviceLayout<(i64,), (i64,)>);
        let (columns, keys) = layout_types(&valid).unwrap().unwrap();
        assert_eq!((columns.len(), keys), (2, 1));
        let wide: Type = syn::parse_quote!(::gpu_datalog_runtime::DeviceLayout<(i64,), (i64,u8)>);
        assert_eq!(layout_types(&wide).unwrap().unwrap().0.len(), 3);
        let alias: Type = syn::parse_quote!(::gpu_datalog_runtime::DeviceLayout<(MyScalar,), (u32,)>);
        assert!(layout_types(&alias).unwrap().is_some());
        let template: Type = syn::parse_quote!(::gpu_datalog_runtime::DeviceLayout<_, _>);
        assert!(layout_types(&template).unwrap().is_none());
        let ordinary: Type = syn::parse_quote!(::cq_gpu_sequence_runtime::RangeProvider);
        assert!(layout_types(&ordinary).unwrap().is_none());
    }
}
