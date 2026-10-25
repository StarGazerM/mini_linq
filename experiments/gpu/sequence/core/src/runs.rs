//! Checked bindings for declared packed-run inputs.
use std::collections::BTreeMap;
use cq_lang_structures::{Program, Structure, physical::symbol_name};
use quote::quote;
use syn::{GenericArgument, PathArguments, Type};

pub(crate) fn layout_types(ty: &Type) -> syn::Result<Option<(Vec<Type>, usize)>> {
    let Type::Path(path) = ty else { return Ok(None) };
    let Some(segment) = path.path.segments.last() else { return Ok(None) };
    if segment.ident != "DeviceLayout" { return Ok(None) }
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
    let mut bits = 0;
    for column in &columns { bits += scalar(column)?.0; }
    if bits > 128 { return Err(syn::Error::new_spanned(ty, "GPU-Runs: packed layout exceeds 128 bits")); }
    Ok(Some((columns, key_count)))
}

pub(crate) fn scalar(ty: &Type) -> syn::Result<(u32, bool, proc_macro2::TokenStream)> {
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
        if reader != "scan" { continue }
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
                if *ty != types[position] { return Err(syn::Error::new_spanned(entry, "GPU-Runs: declared packed column type differs from input")); }
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
        if origin.selection.is_some() { return Err(syn::Error::new_spanned(&entry.name, "GPU-Runs: a supplied run index cannot incorporate a computed selection")); }
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
        assert!(layout_types(&wide).is_err());
        let unsupported: Type = syn::parse_quote!(::gpu_datalog_runtime::DeviceLayout<(f32,), (u32,)>);
        assert!(layout_types(&unsupported).is_err());
        let ordinary: Type = syn::parse_quote!(::cq_gpu_sequence_runtime::RangeProvider);
        assert!(layout_types(&ordinary).unwrap().is_none());
    }
}
