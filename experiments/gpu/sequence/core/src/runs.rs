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
    fn fields(ty: &Type) -> syn::Result<Vec<Type>> {
        let Type::Tuple(tuple) = ty else { return Err(syn::Error::new_spanned(ty, "GPU-Runs: layout columns must be tuples")) };
        Ok(tuple.elems.iter().cloned().collect())
    }
    let mut columns = fields(keys)?;
    let key_count = columns.len();
    columns.extend(fields(values)?);
    for column in &columns { scalar(column)?; }
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

pub(crate) fn scalar(ty: &Type) -> syn::Result<(u32, bool, proc_macro2::TokenStream)> {
    if symbol(ty).is_some() {
        return Ok((32, false, quote!(::gpu_datalog_runtime::Column::U32)));
    }
    let name = quote!(#ty).to_string();
    let (bits, signed, variant) = match name.as_str() {
        "i8" => (8, true, quote!(I8)), "u8" => (8, false, quote!(U8)),
        "i16" => (16, true, quote!(I16)), "u16" => (16, false, quote!(U16)),
        "i32" => (32, true, quote!(I32)), "u32" => (32, false, quote!(U32)),
        "i64" => (64, true, quote!(I64)),
        _ => return Err(syn::Error::new_spanned(ty, "GPU-Runs: supported columns are i8/i16/i32/i64/u8/u16/u32")),
    };
    Ok((bits, signed, quote!(::gpu_datalog_runtime::Column::#variant)))
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
        let Some((ty, key_count)) = hosts.get(&origin.host) else { continue };
        // A selected source is not the raw run: keep the computed Built index.
        if origin.selection.is_some() || origin.predicate { continue; }
        let keys = keyed.level_names().concat();
        let layout = &analysis.host_layouts[&origin.host];
        if keys.len() > *key_count || keys.iter().enumerate().any(|(i,key)| origin.columns.get(key) != layout.get(i)) {
            return Err(syn::Error::new_spanned(&entry.name, "GPU-Runs: lookup key must be a leading prefix of the declared packed layout key"));
        }
        keyed.ty = ty.clone();
    }
    cq_lang_structures::contract::check(&target)?;
    Ok(target)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checks_declared_layout_width_and_scalar_domains() {
        let valid: Type = syn::parse_quote!(::gpu_datalog_runtime::DeviceLayout<(i64,), (i64,)>);
        let (columns, keys) = layout_types(&valid).unwrap().unwrap();
        assert_eq!((columns.len(), keys), (2, 1));
        let wide: Type = syn::parse_quote!(::gpu_datalog_runtime::DeviceLayout<(i64,), (i64,u8)>);
        assert_eq!(layout_types(&wide).unwrap().unwrap().0.len(), 3);
        let unsupported: Type = syn::parse_quote!(::gpu_datalog_runtime::DeviceLayout<(f32,), (u32,)>);
        assert!(layout_types(&unsupported).is_err());
        let ordinary: Type = syn::parse_quote!(::cq_gpu_sequence_runtime::RangeProvider);
        assert!(layout_types(&ordinary).unwrap().is_none());
    }
}
