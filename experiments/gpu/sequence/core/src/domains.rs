//! Upload adapters and immutable bounds provenance, recorded before emission.
use cq_lang_sequence::gpu::bindings::{
    Bindings, ColumnBounds, UploadAdapter, UploadBounds, UploadCodec, UploadColumn,
};
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
        .flat_map(cq_lang_structures::syntax::Capture::binding_types)
        .filter(|(_, ty)| !matches!(ty, syn::Type::Infer(_)))
        .map(|(binder, ty)| (symbol_name(&binder), ty))
        .collect()
}

/// A variable-only head supplies Rust's expected type for an inferred binder.
/// Keep this context with the carrier facts: standalone scalar metadata has no
/// expression from which Rust could recover an `if let` binder's type.
fn head_types(program: &Program) -> Vec<(String, &syn::Type)> {
    let physical = &program.structures.physical.program;
    physical
        .declarations
        .iter()
        .filter(|declaration| declaration.name == physical.head.relation)
        .flat_map(|declaration| physical.head.args.iter().zip(&declaration.columns))
        .filter_map(|(argument, column)| {
            cq_lang_structures::syntax::rust::variable(argument)
                .map(|name| (symbol_name(name), &column.ty))
        })
        .collect()
}

/// The device carrier of a logical column type: the u32 code of its
/// declared codec, otherwise the type itself (GPU-Representation).
fn device_carrier(codecs: &crate::runs::Codecs, logical: &syn::Type) -> syn::Type {
    if codecs.get(logical).is_some() {
        syn::parse_quote!(u32)
    } else {
        logical.clone()
    }
}

/// Record the device carrier of every Physical column and capture answer
/// column; a capture answer publishes one host value of its declared type.
pub(crate) fn carriers(
    program: &Program,
    analysis: &Analysis,
) -> syn::Result<BTreeMap<String, syn::Type>> {
    let codecs = crate::runs::Codecs::check(&program.structures)?;
    let mut carriers = BTreeMap::new();
    let head = head_types(program);
    for row in analysis.structures.physical.rows.values() {
        for (column, ty) in &row.types {
            let ty = if matches!(ty, syn::Type::Infer(_)) {
                head.iter()
                    .find(|(name, _)| name == column)
                    .map_or(ty, |(_, ty)| *ty)
            } else {
                ty
            };
            carriers.insert(column.clone(), device_carrier(&codecs, ty));
        }
    }
    for (column, ty) in capture_types(program) {
        carriers.insert(column, device_carrier(&codecs, &ty));
    }
    Ok(carriers)
}

/// Every recorded carrier is its column's representation in every row (and
/// capture binder) that types it, and every typed column has one.
fn check_carriers(program: &Program, analysis: &Analysis, records: &Bindings) -> syn::Result<()> {
    let codecs = crate::runs::Codecs::check(&program.structures)?;
    let mut typed: BTreeMap<&String, Vec<&syn::Type>> = BTreeMap::new();
    for row in analysis.structures.physical.rows.values() {
        for (column, ty) in &row.types {
            typed.entry(column).or_default().push(ty);
        }
    }
    let captures = capture_types(program);
    for (column, ty) in &captures {
        typed.entry(column).or_default().push(ty);
    }
    // The checker consumes the authored head constraints independently of the
    // producer. Every available expected type must justify the inferred field.
    let head = head_types(program);
    for (column, logicals) in &mut typed {
        if logicals.iter().any(|ty| matches!(ty, syn::Type::Infer(_))) {
            let expected: Vec<_> = head
                .iter()
                .filter(|(name, _)| name == *column)
                .map(|(_, ty)| *ty)
                .collect();
            if !expected.is_empty() {
                logicals.retain(|ty| !matches!(ty, syn::Type::Infer(_)));
                logicals.extend(expected);
            }
        }
    }
    if records.carriers.len() != typed.len() {
        return Err(error("GPU-Representation: incomplete carrier records"));
    }
    for (column, logicals) in typed {
        let carrier = records
            .carriers
            .get(column)
            .ok_or_else(|| error("GPU-Representation: incomplete carrier records"))?;
        // A codec column's device carrier is its u32 code; any other column
        // is carried as its logical type.
        let legal = |logical: &&syn::Type| match codecs.get(logical) {
            Some(_) => *carrier == syn::parse_quote!(u32),
            None => carrier == *logical,
        };
        if !logicals.iter().all(legal) {
            return Err(error(
                "GPU-Representation: carrier is not its column's representation",
            ));
        }
    }
    Ok(())
}

pub(crate) fn select(
    program: &Program,
    analysis: &Analysis,
    records: &mut Bindings,
) -> syn::Result<()> {
    let types = records.carriers.clone();
    let mut unbounded = BTreeSet::new();
    for row in analysis.structures.physical.rows.values() {
        unbounded.extend(row.computed.iter().cloned());
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
                && analysis.structures.facts(&keyed.provider).bounds
                    == Some(cq_lang_structures::provider::BoundsSource::ResidentColumns)
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
                // Uploaded bounds feed provider descriptors' merges, so each
                // column is measured once at upload.
                UploadAdapter::NumericColumns {
                    types: layout.iter().map(|column| types[column].clone()).collect(),
                    columns: vec![
                        UploadColumn {
                            codec: UploadCodec::For,
                            bounds: UploadBounds::Measure,
                        };
                        layout.len()
                    ],
                }
            },
        );
    }
    Ok(())
}

/// Validate source coverage and carrier legality; never call selection or require its preference.
pub(crate) fn check(program: &Program, analysis: &Analysis, records: &Bindings) -> syn::Result<()> {
    check_carriers(program, analysis, records)?;
    let carrier = |column: &String| &records.carriers[column];
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
    for (column, _) in &captures {
        let expected = carrier(column).clone();
        if records.bounds.get(column) != Some(&ColumnBounds::Scalar { ty: expected }) {
            return Err(error(
                "GPU-Bounds: a capture column has its type's scalar bounds",
            ));
        }
    }
    for (column, domain) in &records.bounds {
        match domain {
            ColumnBounds::Scalar { ty } => {
                if ty != carrier(column) {
                    return Err(error("GPU-Bounds: scalar bounds use a different carrier"));
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
                                    if analysis.structures.facts(&keyed.provider).bounds
                                        == Some(cq_lang_structures::provider::BoundsSource::ResidentColumns)));
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
            UploadAdapter::NumericColumns { types, columns }
                if !layout.is_empty()
                    && types.len() == layout.len()
                    && columns.len() == layout.len() =>
            {
                // Any recorded bounds source is sound: a measurement is
                // exact and a carrier range encloses every value.
                for (column, ty) in layout.iter().zip(types) {
                    if ty != carrier(column) {
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
    fn inferred_output_carrier_uses_the_declared_head_context() {
        let program: Program = syn::parse_quote! {
            physical {
                relation R(a:i32) = (&data);
                relation answer(value:i32);
                r = pipe rename R@0 {a -> a};
                bound = pipe bind r with b = (*a);
                computed = let canon project bound keep {b};
                return computed as answer(b);
            }
            structures {
                providers {
                    device_columns = ::cq_gpu_runtime::DeviceColumns<_> {
                        reads [], build typed, abi columns, bounds resident_columns
                    };
                    radix_set = ::gpu_datalog_runtime::RadixLayout<::gpu_datalog_runtime::radix_run::EncodedRow, (), ::gpu_datalog_runtime::radix_run::PackedRow, u64, ()> {
                        reads [distinct, resident], build inferred
                    };
                }
                r host scan = @device_columns input {
                    keys 0; columns [i32 => i32]
                } as ::cq_gpu_runtime::DeviceColumns<'static,(i32,)> [];
                computed rows = @radix_set as ::gpu_datalog_runtime::RadixLayout<::gpu_datalog_runtime::radix_run::EncodedRow, (), ::gpu_datalog_runtime::radix_run::PackedRow, u64, ()>;
                computed canon = rows;
            }
            sequence {
                input = store rows r;
                values = store canon map computed map bound input;
                output_rows = store flatten tile 256 values;
                output count;
            }
            graded gpu { input at host; values at host; output_rows at host.device; }
        };
        let program = crate::runs::record_representation(&program).unwrap();
        let analysis = cq_lang_sequence::check_grading(&program, &crate::machine()).unwrap();
        assert!(matches!(
            analysis.structures.physical.rows["computed"].types["b"],
            syn::Type::Infer(_)
        ));
        let records = &analysis.gpu_bindings;
        assert_eq!(records.carriers["b"], syn::parse_quote!(i32));
        assert_eq!(
            records.bounds["b"],
            ColumnBounds::Scalar {
                ty: syn::parse_quote!(i32)
            }
        );
        let arena = analysis.gpu_publications["output_rows"]
            .arena
            .as_ref()
            .unwrap();
        assert_eq!(arena.columns[0].column, "b");
        assert_eq!(arena.columns[0].carrier, syn::parse_quote!(i32));
        for ty in [syn::parse_quote!(_), syn::parse_quote!(u32)] {
            let mut wrong = records.clone();
            wrong.carriers.insert("b".into(), ty);
            assert!(check_carriers(&program, &analysis, &wrong).is_err());
        }
    }

    #[test]
    fn bounds_and_upload_records_check_original_provenance() {
        let program: Program = syn::parse_quote! {
            physical {
                relation R(a:i64) = (&data);
                r = pipe rename R@0 {a -> a};
                result = let canon project r keep {a};
                return result as answer(a);
            }
            structures { providers { device_columns = ::cq_gpu_runtime::DeviceColumns<_> { reads [], build typed, abi columns, bounds resident_columns }; radix_set = ::gpu_datalog_runtime::RadixLayout<::gpu_datalog_runtime::radix_run::EncodedRow, (), ::gpu_datalog_runtime::radix_run::PackedRow, u64, ()> { reads [distinct, resident], build inferred }; }
                r host scan = @device_columns input { keys 0; columns [i64 => i64] } as ::cq_gpu_runtime::DeviceColumns<'static,(i64,)> [];
                result rows = @radix_set as ::gpu_datalog_runtime::RadixLayout<::gpu_datalog_runtime::radix_run::EncodedRow, (), ::gpu_datalog_runtime::radix_run::PackedRow, u64, ()>;
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
                types: vec![syn::parse_quote!(i64)],
                columns: vec![UploadColumn {
                    codec: UploadCodec::For,
                    bounds: UploadBounds::Measure,
                }],
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
