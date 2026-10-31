//! Upload adapters and immutable bounds provenance, recorded before emission.
use cq_lang_sequence::gpu::bindings::{Bindings, ColumnBounds, UploadAdapter};
use cq_lang_sequence::{Analysis, Program};
use cq_lang_structures::{
    ReaderRef, Structure,
    physical::{Operator, symbol_name},
};
use std::collections::{BTreeMap, BTreeSet};

fn error(message: &str) -> syn::Error {
    syn::Error::new(proc_macro2::Span::call_site(), message)
}

/// Typed `let` capture binders: the columns a capture answer may publish.
fn capture_types(program: &Program) -> Vec<(String, syn::Type)> {
    program
        .structures
        .physical
        .program
        .captures
        .items
        .iter()
        .filter_map(|item| match item {
            cq_lang_structures::syntax::Capture::Let(binding) => Some(binding.binding_types()),
            cq_lang_structures::syntax::Capture::Guard(_) => None,
        })
        .flatten()
        .filter(|(_, ty)| !matches!(ty, syn::Type::Infer(_)))
        .map(|(binder, ty)| (symbol_name(&binder), ty))
        .collect()
}

pub(crate) fn select(
    program: &Program,
    analysis: &Analysis,
    records: &mut Bindings,
) -> syn::Result<()> {
    let codecs = crate::runs::Codecs::check(&program.structures)?;
    let mut types = BTreeMap::new();
    let mut unbounded = BTreeSet::new();
    for row in analysis.structures.physical.rows.values() {
        for (column, ty) in &row.types {
            types.insert(
                column.clone(),
                if codecs.get(ty).is_some() {
                    syn::parse_quote!(u32)
                } else {
                    ty.clone()
                },
            );
        }
        unbounded.extend(row.computed.iter().cloned());
    }
    // A capture answer column publishes one host value of its declared type;
    // its bounds are that type's whole domain (IF-Capture).
    for (column, ty) in capture_types(program) {
        types.insert(
            column,
            if codecs.get(&ty).is_some() {
                syn::parse_quote!(u32)
            } else {
                ty
            },
        );
    }
    let resident: BTreeSet<_> = program
        .structures
        .structures
        .entries
        .iter()
        .filter_map(|entry| {
            if let Structure::Host {
                reader: ReaderRef::Scan,
                keyed,
                ..
            } = &entry.structure
                && crate::native_abi(&keyed.ty) == Some(crate::NativeAbi::Columns)
            {
                analysis
                    .structures
                    .physical
                    .hosts
                    .get(&symbol_name(&entry.name))
                    .cloned()
            } else {
                None
            }
        })
        .collect();
    let mut sources: BTreeMap<String, BTreeSet<(String, usize)>> = BTreeMap::new();
    for definition in &program.structures.physical.program.definitions {
        if let Operator::Rename(rename) = &definition.operator {
            let host = symbol_name(&rename.relation);
            for mapping in &rename.mapping {
                let column = symbol_name(&mapping.target);
                if resident.contains(&host) {
                    let index = analysis.structures.host_layouts[&host]
                        .iter()
                        .position(|name| name == &symbol_name(&mapping.source))
                        .ok_or_else(|| {
                            error("GPU-Bounds: resident column absent from input layout")
                        })?;
                    sources
                        .entry(column)
                        .or_default()
                        .insert((host.clone(), index));
                } else {
                    unbounded.insert(column);
                }
            }
        }
    }
    for (column, ty) in &types {
        let domain = match sources.remove(column) {
            Some(sources) if !unbounded.contains(column) => ColumnBounds::Resident {
                sources: sources.into_iter().collect(),
            },
            _ => ColumnBounds::Scalar { ty: ty.clone() },
        };
        records.bounds.insert(column.clone(), domain);
    }
    for (name, layout) in &analysis.structures.layouts {
        records.uploads.insert(
            name.clone(),
            if layout.is_empty() {
                UploadAdapter::EmptyRows
            } else {
                UploadAdapter::NumericColumns {
                    types: layout.iter().map(|column| types[column].clone()).collect(),
                }
            },
        );
    }
    Ok(())
}

/// Validate source coverage and carrier legality; never call selection or require its preference.
pub(crate) fn check(program: &Program, analysis: &Analysis, records: &Bindings) -> syn::Result<()> {
    let codecs = crate::runs::Codecs::check(&program.structures)?;
    let captures = capture_types(program);
    let mut columns: BTreeSet<_> = analysis
        .structures
        .physical
        .rows
        .values()
        .flat_map(|row| row.types.keys())
        .collect();
    columns.extend(captures.iter().map(|(column, _)| column));
    if records.bounds.keys().collect::<BTreeSet<_>>() != columns
        || records.uploads.len() != analysis.structures.layouts.len()
    {
        return Err(error("GPU-Bounds: incomplete bounds or upload records"));
    }
    for (column, logical) in &captures {
        let expected: syn::Type = if codecs.get(logical).is_some() {
            syn::parse_quote!(u32)
        } else {
            logical.clone()
        };
        if records.bounds.get(column) != Some(&ColumnBounds::Scalar { ty: expected }) {
            return Err(error("GPU-Bounds: a capture column has its type's scalar bounds"));
        }
    }
    for (column, domain) in &records.bounds {
        match domain {
            ColumnBounds::Scalar { ty } => {
                for row in analysis
                    .structures
                    .physical
                    .rows
                    .values()
                    .filter(|row| row.types.contains_key(column))
                {
                    let logical = &row.types[column];
                    let expected = if codecs.get(logical).is_some() {
                        syn::parse_quote!(u32)
                    } else {
                        logical.clone()
                    };
                    if *ty != expected {
                        return Err(error("GPU-Bounds: scalar bounds use a different carrier"));
                    }
                }
            }
            ColumnBounds::Resident { sources } => {
                let recorded: BTreeSet<_> = sources.iter().cloned().collect();
                if sources.is_empty()
                    || recorded.len() != sources.len()
                    || analysis
                        .structures
                        .physical
                        .rows
                        .values()
                        .any(|row| row.computed.contains(column))
                {
                    return Err(error(
                        "GPU-Bounds: resident bounds cannot enclose a computed column",
                    ));
                }
                let mut required = BTreeSet::new();
                for definition in &program.structures.physical.program.definitions {
                    let Operator::Rename(rename) = &definition.operator else {
                        continue;
                    };
                    for mapping in rename
                        .mapping
                        .iter()
                        .filter(|mapping| symbol_name(&mapping.target) == *column)
                    {
                        let host = symbol_name(&rename.relation);
                        let declared = program.structures.structures.entries.iter().any(|entry|
                            analysis.structures.physical.hosts.get(&symbol_name(&entry.name)) == Some(&host)
                                && matches!(&entry.structure, Structure::Host { reader: ReaderRef::Scan, keyed, .. }
                                    if crate::native_abi(&keyed.ty) == Some(crate::NativeAbi::Columns)));
                        if !declared {
                            return Err(error(
                                "GPU-Bounds: resident bounds omit a nonresident origin",
                            ));
                        }
                        let layout = &analysis.structures.host_layouts[&host];
                        let index = layout
                            .iter()
                            .position(|name| name == &symbol_name(&mapping.source))
                            .ok_or_else(|| error("GPU-Bounds: unknown resident bound column"))?;
                        required.insert((host, index));
                    }
                }
                if recorded != required {
                    return Err(error(
                        "GPU-Bounds: incomplete or unrelated resident origins",
                    ));
                }
            }
        }
    }
    for (name, layout) in &analysis.structures.layouts {
        let upload = records
            .uploads
            .get(name)
            .ok_or_else(|| error("GPU-Upload: missing adapter"))?;
        match upload {
            UploadAdapter::EmptyRows if layout.is_empty() => {}
            UploadAdapter::NumericColumns { types }
                if !layout.is_empty() && types.len() == layout.len() =>
            {
                let row = &analysis.structures.physical.rows[name];
                for (column, ty) in layout.iter().zip(types) {
                    let logical = &row.types[column];
                    let expected = if codecs.get(logical).is_some() {
                        syn::parse_quote!(u32)
                    } else {
                        logical.clone()
                    };
                    if *ty != expected {
                        return Err(error("GPU-Upload: adapter carrier differs from its column"));
                    }
                }
            }
            _ => {
                return Err(error(
                    "GPU-Upload: adapter shape differs from its publication",
                ));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bounds_and_upload_records_check_original_provenance() {
        let program: Program = syn::parse_quote! {
            physical {
                relation R(a:i64) = (&data);
                r = pipe rename R@0 {a -> a};
                result = let canon project r keep {a};
                return result as answer(a);
            }
            structures {
                r host scan = ::cq_gpu_runtime::DeviceColumns<'static,(i64,)> [];
                result rows = ::cq_gpu_sequence_runtime::canon::PackedCanon;
                result canon = rows;
            }
            sequence {
                input = store rows r;
                output_rows = store canon map result flatten tile 256 input;
                output count;
            }
            graded gpu { input at host; output_rows at host.device; }
        };
        let program = crate::runs::record_representation(&program).unwrap();
        let analysis = cq_lang_sequence::check_grading(&program, &crate::machine()).unwrap();
        let records = &analysis.gpu_bindings;
        assert_eq!(
            records.bounds["a"],
            ColumnBounds::Resident {
                sources: vec![("R".into(), 0)]
            }
        );
        assert_eq!(
            records.uploads["r"],
            UploadAdapter::NumericColumns {
                types: vec![syn::parse_quote!(i64)]
            }
        );
        let mut wrong = records.clone();
        wrong.bounds.insert(
            "a".into(),
            ColumnBounds::Resident {
                sources: vec![("R".into(), 1)],
            },
        );
        assert!(check(&program, &analysis, &wrong).is_err());
        let mut missing = records.clone();
        missing.bounds.clear();
        assert!(check(&program, &analysis, &missing).is_err());
        let mut wrong = records.clone();
        wrong.uploads.insert("r".into(), UploadAdapter::EmptyRows);
        assert!(check(&program, &analysis, &wrong).is_err());
        let mut conservative = records.clone();
        conservative.bounds.insert(
            "a".into(),
            ColumnBounds::Scalar {
                ty: syn::parse_quote!(i64),
            },
        );
        check(&program, &analysis, &conservative).unwrap();
    }
}
