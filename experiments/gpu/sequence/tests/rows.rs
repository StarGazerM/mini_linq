use cq_lang_sequence::{Program, check_grading};
use quote::{ToTokens, quote};

fn filtered_probe(condition: proc_macro2::TokenStream) -> Program {
    syn::parse2(quote! {
        physical {
            relation Driver(a:i32, value:i32, bound:i32) = (&data.driver);
            relation Provider(a:i32, payload:i32) = (&data.provider);
            driver = pipe rename Driver@0 {a -> a, value -> value, bound -> bound};
            provider = let rename Provider@1 {a -> a, payload -> payload};
            filtered = pipe select driver if (#condition);
            present = pipe semi eager [a] filtered, provider;
            projected = pipe project present keep {a, value};
            joined = pipe probe first [a] projected, provider;
            result = let canon project joined keep {a, value, payload};
            return result as answer(a, value, payload);
        }
        structures { providers { device_columns = ::cq_gpu_runtime::DeviceColumns<_> { reads [], build typed, abi columns, bounds resident_columns }; range_provider = ::cq_gpu_sequence_runtime::RangeProvider { reads [exact, range, membership], build inferred, abi range, contiguous_range }; packed_canon = ::cq_gpu_sequence_runtime::canon::PackedCanon { reads [distinct], build inferred, packed_bits 128 }; }
            driver host scan = @device_columns input { keys 0; columns [i32 => i32, i32 => i32, i32 => i32] } as ::cq_gpu_runtime::DeviceColumns [];
            provider host scan = @device_columns input { keys 0; columns [i32 => i32, i32 => i32] } as ::cq_gpu_runtime::DeviceColumns [];
            provider = @range_provider as ::cq_gpu_sequence_runtime::RangeProvider [a];
            result rows = @packed_canon as ::cq_gpu_sequence_runtime::canon::PackedCanon;
            result canon = rows;
        }
        sequence {
            input = store rows driver;
            index = store rows provider;
            filtered_rows = compact filtered flatten tile 256 input;
            present_rows = compact present lookup provider [a] filtered_rows;
            projected_rows = map projected present_rows;
            joined_rows = flatten lookup provider [a] projected_rows;
            output_rows = store canon map result joined_rows;
            output count;
        }
        graded gpu {
            input at host;
            index at host;
            filtered_rows at host.device.block;
            present_rows at host.device.block;
            projected_rows at host.device.block;
            joined_rows at host.device.block;
            output_rows at host.device;
        }
    })
    .unwrap()
}

fn translate(program: &Program) -> String {
    let program = cq_gpu_sequence_core::runs::record_representation(program).unwrap();
    let analysis = check_grading(&program, &cq_gpu_sequence::machine()).unwrap();
    cq_gpu_sequence::translate(&program, &analysis)
        .unwrap()
        .to_token_stream()
        .to_string()
}

#[test]
fn primitive_host_provider_uploads_columns_without_tuple_reencoding() {
    let mut program = filtered_probe(quote!(value > 0));
    program.structures.structures.entries[1] =
        syn::parse_quote!(provider host scan = @host_scan as ::core::iter::Map<_, _> [];);
    program.structures.structures.declare(&[syn::parse_quote!(
        host_scan = ::core::iter::Map<_, _> { reads [], build typed };
    )]);
    let generated = translate(&program);
    assert!(generated.contains("upload_numeric_columns"));
    assert!(generated.contains("__cq_host_column_0"));
    assert!(generated.contains("__cq_host_column_1"));
    assert!(!generated.contains("DeviceColumns :: from_rows"));
}

#[test]
fn compact_and_project_preserve_probe_row_provenance() {
    // The same provider is consumed first for membership and then for payload.
    // Its aggregate Exact capability must not turn Compact into a payload read.
    let generated = translate(&filtered_probe(quote!(value > 0)));
    assert!(generated.contains("__begin < __end"));
    assert!(generated.contains("__child_position"));
    assert!(!generated.contains("__row"));
}

#[test]
fn row_valued_bound_is_evaluated_per_lane_before_range_lookup() {
    let generated = translate(&filtered_probe(quote!(value > bound)));
    assert!(generated.contains("let value = &"));
    assert!(generated.contains("let bound = &"));
    assert!(generated.contains("* value > * bound"));
    assert!(generated.contains("device :: range"));
    assert!(!generated.contains("RangeSelection"));
    assert!(!generated.contains("__row"));
}

#[test]
fn encoded_integer_disjunction_is_a_device_or_condition() {
    let generated = translate(&filtered_probe(quote!((value = 1i32) or (value = 2i32))));
    assert!(generated.contains("let value = &"), "{generated}");
    assert!(
        generated.split(';').any(|statement| {
            statement.contains("* value == 1i32")
                && statement.contains("||")
                && statement.contains("* value == 2i32")
        }),
        "{generated}"
    );
    assert!(!generated.contains("relational_eq"), "{generated}");
    assert!(!generated.contains("RangeSelection"), "{generated}");
}

#[test]
fn published_row_bound_filter_is_not_rebuilt_as_a_host_interval() {
    let mut program = filtered_probe(quote!(value > bound));
    for definition in &mut program.sequence.definitions {
        if definition.name == "filtered_rows" {
            let inner = &definition.term;
            definition.term = syn::parse2(quote!(store #inner)).unwrap();
        }
        if definition.name == "present_rows" {
            definition.term = syn::parse_quote!(
                compact present lookup provider [a] flatten tile 256 filtered_rows
            );
        }
    }
    for grade in &mut program.graded.as_mut().unwrap().grades {
        if grade.name == "filtered_rows" {
            grade.level.pop();
        }
    }
    let generated = translate(&program);
    assert!(generated.contains("* value > * bound"));
    assert!(!generated.contains("RangeSelection"));
    assert!(!generated.contains("__row"));
}

#[test]
fn negative_membership_inverts_the_provider_probe_without_expanding_witnesses() {
    for (operator, negated) in [(quote!(semi), false), (quote!(anti), true)] {
        let program: Program = syn::parse2(quote! {
            physical {
                relation Driver(a:i32, b:i32) = (&data.driver);
                relation Provider(a:i32, b:i32) = (&data.provider);
                driver = pipe rename Driver@0 {a -> a, b -> b};
                provider = let rename Provider@1 {a -> a, b -> b};
                present = pipe #operator eager [a, b] driver, provider;
                result = let canon project present keep {a, b};
                return result as answer(a, b);
            }
            structures { providers { device_columns = ::cq_gpu_runtime::DeviceColumns<_> { reads [], build typed, abi columns, bounds resident_columns }; range_provider = ::cq_gpu_sequence_runtime::RangeProvider { reads [exact, range, membership], build inferred, abi range, contiguous_range }; packed_canon = ::cq_gpu_sequence_runtime::canon::PackedCanon { reads [distinct], build inferred, packed_bits 128 }; }
                driver host scan = @device_columns input { keys 0; columns [i32 => i32, i32 => i32] } as ::cq_gpu_runtime::DeviceColumns [];
                provider host scan = @device_columns input { keys 0; columns [i32 => i32, i32 => i32] } as ::cq_gpu_runtime::DeviceColumns [];
                provider = @range_provider as ::cq_gpu_sequence_runtime::RangeProvider [a, b];
                result rows = @packed_canon as ::cq_gpu_sequence_runtime::canon::PackedCanon;
                result canon = rows;
            }
            sequence {
                input = store rows driver;
                index = store rows provider;
                present_rows = compact present lookup provider [a, b] flatten tile 256 input;
                output_rows = store canon map result present_rows;
                output count;
            }
            graded gpu {
                input at host;
                index at host;
                present_rows at host.device.block;
                output_rows at host.device;
            }
        })
        .unwrap();
        let generated = translate(&program);
        assert!(
            generated.contains("device :: decode (__column"),
            "{generated}"
        );
        assert!(!generated.contains("contains_u32"), "{generated}");
        assert!(
            generated.contains(&format!("__member_found != {negated}")),
            "{generated}"
        );
        assert!(!generated.contains("__child_position"), "{generated}");
    }
}

#[test]
fn planned_retained_provider_filter_evaluates_row_columns_before_build() {
    let source: Program = syn::parse_quote! {
        physical {
            relation Driver(a:i32) = (&data.driver);
            relation Provider(a:i32,value:i32,bound:i32) = (&data.provider);
            driver = pipe rename Driver@0 {a -> a};
            provider = pipe rename Provider@1 {a -> a,value -> value,bound -> bound};
            selected = let select provider if (value < bound);
            present = pipe semi eager [a] driver,selected;
            result = let canon project present keep {a};
            return result as answer(a);
        }
        structures { providers { device_columns = ::cq_gpu_runtime::DeviceColumns<_> { reads [], build typed, abi columns, bounds resident_columns }; range_provider = ::cq_gpu_sequence_runtime::RangeProvider { reads [exact, range, membership], build inferred, abi range, contiguous_range }; packed_canon = ::cq_gpu_sequence_runtime::canon::PackedCanon { reads [distinct], build inferred, packed_bits 128 }; }
            driver host scan = @device_columns input { keys 0; columns [i32 => i32] } as ::cq_gpu_runtime::DeviceColumns [];
            provider host scan = @device_columns input { keys 0; columns [i32 => i32, i32 => i32, i32 => i32] } as ::cq_gpu_runtime::DeviceColumns [];
            selected = @range_provider as ::cq_gpu_sequence_runtime::RangeProvider [a];
            result rows = @packed_canon as ::cq_gpu_sequence_runtime::canon::PackedCanon;
            result canon = rows;
        }
        sequence {
            input = rows driver;
            provider_input = rows provider;
            index = store compact selected provider_input;
            present_rows = compact present lookup selected [a] input;
            output_rows = store canon map result present_rows;
            output count;
        }
    };
    let machine = cq_gpu_sequence::machine();
    let source = cq_gpu_sequence_core::runs::record_representation(&source).unwrap();
    let planned =
        cq_sequence_plan::decompose(&source, &machine, &cq_gpu_sequence::catalog()).unwrap();
    let analysis = check_grading(&planned.program, &machine).unwrap();
    assert_eq!(analysis.levels["index"], ["host", "device"]);
    let generated = translate(&planned.program);
    assert!(generated.contains("let value = &"));
    assert!(generated.contains("let bound = &"));
    assert!(generated.contains("* value < * bound"));
    assert!(generated.contains("ColumnDescriptor :: from_rows"));
    assert!(!generated.contains("RangeSelection"));
    assert!(!generated.contains("__row"));
}

fn segmented_binding(expression: proc_macro2::TokenStream) -> Program {
    syn::parse_quote! {
        physical {
            relation Driver(a:i32) = (&data.driver);
            relation Provider(a:i32, value:i32) = (&data.provider);
            driver = pipe rename Driver@0 {a -> a};
            provider = let rename Provider@1 {a -> a, value -> value};
            joined = pipe probe first [a] driver, provider;
            computed = pipe bind joined with doubled:i32 = (#expression);
            result = let canon project computed keep {a, doubled};
            return result as answer(a, doubled);
        }
        structures { providers { device_columns = ::cq_gpu_runtime::DeviceColumns<_> { reads [], build typed, abi columns, bounds resident_columns }; range_provider = ::cq_gpu_sequence_runtime::RangeProvider { reads [exact, range, membership], build inferred, abi range, contiguous_range }; packed_canon = ::cq_gpu_sequence_runtime::canon::PackedCanon { reads [distinct], build inferred, packed_bits 128 }; }
            driver host scan = @device_columns input { keys 0; columns [i32 => i32] } as ::cq_gpu_runtime::DeviceColumns [];
            provider host scan = @device_columns input { keys 0; columns [i32 => i32, i32 => i32] } as ::cq_gpu_runtime::DeviceColumns [];
            provider = @range_provider as ::cq_gpu_sequence_runtime::RangeProvider [a];
            result rows = @packed_canon as ::cq_gpu_sequence_runtime::canon::PackedCanon;
            result canon = rows;
        }
        sequence {
            input = store rows driver;
            index = store rows provider;
            joined_rows = flatten lookup provider [a] flatten tile 256 input;
            computed_rows = map computed joined_rows;
            output_rows = store canon map result computed_rows;
            output count;
        }
        graded gpu {
            input at host;
            index at host;
            joined_rows at host.device.block;
            computed_rows at host.device.block;
            output_rows at host.device;
        }
        // The authored output publishes its computed i32 as a native scalar.
        representation { identity [doubled]; encoded []; }
    }
}

#[test]
fn rust_bindings_use_recorded_scalar_bounds_without_expression_recognition() {
    for expression in [quote!(*value), quote!(*value * 2)] {
        let program =
            cq_gpu_sequence_core::runs::record_representation(&segmented_binding(expression))
                .unwrap();
        assert!(program.representation.identity.contains("doubled"));
        assert!(!program.representation.encoded.contains("doubled"));
        let generated = translate(&program);
        assert!(
            generated.contains("< i32 as :: gpu_datalog_runtime :: Scalar > :: MIN"),
            "{generated}"
        );
        assert!(
            generated.contains("< i32 as :: gpu_datalog_runtime :: Scalar > :: MAX"),
            "{generated}"
        );
    }
}

#[test]
fn segmented_arithmetic_bindings_are_lane_local() {
    let generated = translate(&segmented_binding(quote!(*value * 2)));
    assert!(generated.contains("__child_position"), "{generated}");
    assert!(generated.contains("let mut __map_code_"), "{generated}");
    // GPU-Capture: the literal is a closed value read from its host witness.
    assert!(
        generated.contains(":: MaybeUninit :: new (2)"),
        "{generated}"
    );
    assert!(
        generated.contains("* value * unsafe { __cq_closed_0 . assume_init () }"),
        "{generated}"
    );
    assert!(!generated.contains("wrapping_mul"), "{generated}");
    // Parent slots can be read by sibling lanes, never assigned by a map.
    for statement in generated.split(';') {
        if let Some((left, _)) = statement.split_once(" = ") {
            assert!(
                !(left.contains("* __parent_") && left.contains("__owner")),
                "{statement}"
            );
        }
    }
}

#[cfg(feature = "cuda-oracle")]
#[test]
fn row_regression_contradictory_provider_intervals_are_empty() -> cq_gpu_sequence_runtime::Result<()>
{
    use cq_gpu_runtime::{Codec, DeviceColumns, Dictionary, Pool};
    let pool = Pool::new(0, 0, 0)?;
    let data = DeviceColumns::from_rows(
        pool,
        &[(0i32,), (1,), (2,), (3,), (4,)],
        &mut Dictionary::default(),
        &[Codec::For],
    )?;
    macro_rules! selected_count {
        ([$($producer:tt)*] $($selection:tt)*) => {{
            let output = cq_gpu_sequence_test_macros::sequence! {
                physical {
                    relation R(x:i32) = (&data);
                    driver = pipe rename R@0 {x -> x};
                    provider = pipe rename R@1 {x -> x};
                    $($selection)*
                    present = pipe semi eager [x] driver,selected;
                    result = let canon project present keep {x};
                    return result as answer(x);
                }
                structures {
                    providers {
                        device_columns = ::cq_gpu_runtime::DeviceColumns<_> { reads [], build typed, abi columns, bounds resident_columns };
                        range_provider = ::cq_gpu_sequence_runtime::RangeProvider { reads [exact, range, membership], build inferred, abi range, contiguous_range };
                        packed_canon = ::cq_gpu_sequence_runtime::canon::PackedCanon { reads [distinct], build inferred, packed_bits 128 };
                    }
                    driver host scan = @device_columns input { keys 0; columns [i32 => i32] } as ::cq_gpu_runtime::DeviceColumns<'static,(i32,)> [];
                    provider host scan = @device_columns input { keys 0; columns [i32 => i32] } as ::cq_gpu_runtime::DeviceColumns<'static,(i32,)> [];
                    selected = @range_provider as ::cq_gpu_sequence_runtime::RangeProvider [x];
                    result rows = @packed_canon as ::cq_gpu_sequence_runtime::canon::PackedCanon;
                    result canon = rows;
                }
                sequence {
                    input = store rows driver;
                    index = store $($producer)*;
                    present_rows = compact present lookup selected [x] flatten tile 256 input;
                    output_rows = store canon map result present_rows;
                    output count;
                }
                graded gpu {
                    input at host;
                    index at host;
                    present_rows at host.device.block;
                    output_rows at host.device;
                }
            };
            let actual: Vec<(i32,)> = output.to_host()?;
            actual
        }};
    }
    let chained = selected_count!([compact selected compact bounded rows provider]
        bounded = pipe select provider if (x < 2i32);
        selected = let select bounded if (x = 3i32);
    );
    let combined = selected_count!([compact selected rows provider]
        selected = let select provider if (x < 2i32, x = 3i32);
    );
    assert!(
        chained.is_empty() && combined.is_empty(),
        "chained selects returned {chained:?}; same-select intersection returned {combined:?}",
    );
    Ok(())
}
