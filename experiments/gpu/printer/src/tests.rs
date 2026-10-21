use crate::{
    compiler::{compile, validate},
    options::{DEFAULT_RETRY_CAPACITY, Options, OutputPolicy},
    predicate::Constants,
};
use cq_lang_structures as ir;
use quote::{ToTokens, quote};
use std::collections::BTreeMap;

#[test]
fn host_set_checks_full_host_before_consumers_and_remains_opt_in() {
    let mut program: ir::Program = syn::parse_quote! { physical {
        relation R(a:i64,b:i64);
        host_set R;
        scan = pipe rename R@0 {a -> x,b -> y};
        output = pipe canon project scan keep {x};
        return output as answer(x);
    } structures {
        scan host scan = ::core::iter::Map<_,_> [];
        output canon = ::cq_gpu_runtime::DeviceSet<_>;
    } };
    let generated = compile(&program, Options::default())
        .unwrap()
        .to_token_stream()
        .to_string();
    let check = generated
        .find("certify_host_set")
        .expect("checked host-set fact");
    let binding = generated.find("bind_host").unwrap();
    assert!(binding < check, "{generated}");
    assert!(generated[check..].contains("\"R\""), "{generated}");
    assert_eq!(generated.matches("certify_host_set").count(), 1);
    // Losing a heading column still requires exact output canonicalization.
    assert!(
        generated.contains("DeviceSet") || generated.contains("dedup"),
        "{generated}"
    );
    program.physical.program.host_sets.clear();
    let ordinary = compile(&program, Options::default())
        .unwrap()
        .to_token_stream()
        .to_string();
    assert!(!ordinary.contains("certify_host_set"), "{ordinary}");
}

fn assert_occurrence_build_boundary(generated: &syn::Expr) {
    let syn::Expr::Block(block) = generated else {
        panic!("GPU generation must return a block");
    };
    let module = block
        .block
        .stmts
        .iter()
        .find_map(|statement| match statement {
            syn::Stmt::Item(syn::Item::Mod(module)) => module.content.as_ref(),
            _ => None,
        })
        .expect("generated device module");
    let kernels = module
        .1
        .iter()
        .filter_map(|item| match item {
            syn::Item::Fn(function)
                if function.sig.ident.to_string().starts_with("cq_gpu_stream_") =>
            {
                Some((
                    function.sig.ident.to_string(),
                    function
                        .block
                        .to_token_stream()
                        .to_string()
                        .split_whitespace()
                        .collect::<String>(),
                ))
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(kernels.len(), 2, "{kernels:?}");
    let (_, build) = kernels
        .iter()
        .find(|(name, _)| name.ends_with("_r"))
        .expect("retained occurrence build kernel");
    let (_, query) = kernels
        .iter()
        .find(|(name, _)| name.ends_with("_out"))
        .expect("query kernel consuming retained occurrence");
    assert!(build.contains("==__cq_gpu_constant_0"), "{build}");
    assert!(!query.contains("==__cq_gpu_constant_0"), "{query}");
    let compact = generated
        .to_token_stream()
        .to_string()
        .split_whitespace()
        .collect::<String>();
    let build = compact
        .find("let__cq_gpu_table_r=")
        .expect("retained build");
    let query = compact
        .find("let__cq_gpu_table_out=")
        .expect("query output");
    assert!(build < query, "{compact}");
}

#[test]
fn occurrence_constants_filter_source_columns_before_retaining_projection() {
    for (ty, literal) in [
        (quote!(u32), quote!(4_000_000_000)),
        (quote!(i32), quote!(-7)),
        (quote!(bool), quote!(true)),
        (quote!(String), quote!("tag")),
    ] {
        let source: ir::Program = syn::parse2(quote! { physical {
            relation R(a:i64,b:#ty);
            r = let rename R@0 {a -> x} constants {b = #literal};
            out = pipe canon project r keep {x};
            return out as answer(x);
        } structures {
            r host scan = ::core::iter::Map<_,_> [];
            r rows = ::std::vec::Vec<_>;
            out canon = ::cq_gpu_runtime::DeviceSet<_>;
        } })
        .unwrap();
        for options in [quote!(), quote!(output(retry))] {
            let retry = !options.is_empty();
            let expression = compile(&source, syn::parse2(options).unwrap()).unwrap();
            assert_occurrence_build_boundary(&expression);
            let generated = expression.to_token_stream().to_string();
            let compact = generated.split_whitespace().collect::<String>();
            assert!(compact.contains("==__cq_gpu_constant_0"), "{generated}");
            assert!(
                !compact.contains("project_table(__cq_gpu_host_R,"),
                "{generated}"
            );
            assert!(
                compact.contains("table_view(__cq_gpu_table_r)"),
                "{generated}"
            );
            if !retry {
                assert!(
                    compact.contains("execute_selection(__cq_gpu_host_R,&[0usize]"),
                    "{generated}"
                );
                assert!(
                    compact.contains("column_view(__cq_gpu_host_R,1usize)"),
                    "{generated}"
                );
            }
            if ty.to_string() == "u32" || ty.to_string() == "i32" {
                assert!(
                    generated.contains(&quote!(let __cq_gpu_literal: #ty = #literal).to_string()),
                    "{generated}"
                );
            }
        }
    }
}

#[test]
fn occurrence_constants_build_indexes_over_only_retained_columns() {
    let source: ir::Program = syn::parse_quote! { physical {
        relation L(a:i64); relation R(b:i32,a:i64,v:i64);
        l = pipe rename L@0 {a -> x};
        r = let rename R@1 {a -> x,v -> y} constants {b = 7};
        out = pipe canon probe first [x] l,r;
        return out as answer(x,y);
    } structures {
        l host scan = ::core::iter::Map<_,_> [];
        r host scan = ::core::iter::Map<_,_> [];
        r = ::cq_storage::SortedRun<_,_,::cq_storage::BorrowedI64TupleCodec> [x];
        out canon = ::cq_gpu_runtime::DeviceSet<_>;
    } };
    for options in [quote!(), quote!(output(retry))] {
        let expression = compile(&source, syn::parse2(options).unwrap()).unwrap();
        assert_occurrence_build_boundary(&expression);
        let generated = expression.to_token_stream().to_string();
        let compact = generated.split_whitespace().collect::<String>();
        assert!(compact.contains("==__cq_gpu_constant_0"), "{generated}");
        let build = compact.find("let__cq_gpu_table_r=").unwrap();
        let index = compact
            .find("prepare_run(__cq_gpu_table_r,0usize)")
            .unwrap();
        let query = compact.find("let__cq_gpu_table_out=").unwrap();
        assert!(build < index && index < query, "{generated}");
        assert!(!compact.contains("resident_run("), "{generated}");
        assert!(
            compact.contains("run_column(__cq_gpu_run_view_r,1usize,")
                || compact.contains("__cq_gpu_run_view_r.columns.add(1usize).read()"),
            "{generated}"
        );
    }
}

#[test]
fn packed_fused_counts_preserve_both_block_prefixes() {
    for seed in 0..16u32 {
        let (mut packed, mut candidates, mut unique) = (0u32, 0u32, 0u32);
        for lane in 0..256u32 {
            assert_eq!(packed & 2047, unique);
            assert_eq!(packed >> 11, candidates);
            let count = if seed == 0 { 4 } else { (lane + seed) % 5 };
            let admitted = if seed == 0 {
                4
            } else {
                (lane * 3 + seed) % (count + 1)
            };
            packed += admitted | (count << 11);
            candidates += count;
            unique += admitted;
        }
        assert_eq!(packed & 2047, unique);
        assert_eq!(packed >> 11, candidates);
        assert!(unique <= candidates && candidates <= 1024);
    }
}

fn shared() -> ir::Program {
    syn::parse_quote! { physical {
        relation L(a:i64); relation R(a:i64,b:i64);
        l = pipe rename L@0 {a -> x};
        b = let rename R@1 {a -> x,b -> y};
        s = pipe semi eager [x] l,b;
        p = pipe canon probe first [x] s,b;
        return p as answer(x,y);
    } structures {
        l host scan = ::core::iter::Map<_,_> [];
        b host scan = ::core::iter::Map<_,_> [];
        b rows = ::std::vec::Vec<_>;
        b = ::cq_storage::SortedRun<_,_,::cq_storage::BorrowedI64TupleCodec> [x];
        p canon = ::cq_gpu_runtime::DeviceSet<_>;
        s share = lookup b [x] first s force p;
        s repr = tuples witness { b [x] at s as ::cq_storage::RunCursor<'_,_> };
    } }
}

fn frozen_host(levels: proc_macro2::TokenStream, keys: proc_macro2::TokenStream) -> ir::Program {
    let composite = keys.to_string().contains(',');
    let left_column = composite.then(|| quote!(,b:i64));
    let left_mapping = composite.then(|| quote!(,b -> y));
    syn::parse2(quote! { physical {
        relation L(a:i64 #left_column); relation R(b:i64,a:i64,v:i64);
        l = pipe rename L@0 {a -> x #left_mapping};
        r = pipe rename R@1 {b -> y,a -> x,v -> z};
        p = pipe canon probe first [#keys] l,r;
        return p as answer(x,y,z);
    } structures {
        l host scan = ::core::iter::Map<_,_> [];
        r host scan = ::core::iter::Map<_,_> [];
        r host p = ::cq_gpu_runtime::FrozenDeviceIndex<_> #levels;
        p canon = ::cq_gpu_runtime::DeviceSet<_>;
    }})
    .unwrap()
}

#[test]
fn frozen_host_reads_bind_resident_runs_without_building() {
    for (levels, keys, expected) in [
        (
            quote!([x]),
            quote!(x),
            "resident_run(&[&[1usize]as&[usize]])",
        ),
        (
            quote!([x, y]),
            quote!(x, y),
            "resident_run(&[&[0usize,1usize]as&[usize]])",
        ),
        (
            quote!([x][y]),
            quote!(x, y),
            "resident_run(&[&[1usize]as&[usize],&[0usize]as&[usize]])",
        ),
    ] {
        let program = frozen_host(levels, keys);
        for options in [quote!(), quote!(output(retry)), quote!(completion(round))] {
            let generated = compile(&program, syn::parse2(options).unwrap())
                .unwrap()
                .to_token_stream()
                .to_string();
            let physical = program.physical.to_token_stream().to_string();
            assert!(
                generated.contains(
                    &quote!(
                        const __CQ_GPU_RECORDED_PHYSICAL: &str = #physical;
                    )
                    .to_string()
                ),
                "{generated}"
            );
            let compact = generated.split_whitespace().collect::<String>();
            assert!(compact.contains(expected), "{generated}");
            assert!(!compact.contains("prepare_run("), "{generated}");
            assert!(!compact.contains("prepare_hashed_run("), "{generated}");
            assert!(!compact.contains("prepare_prefix_run("), "{generated}");
            assert!(
                !compact.contains("reencode_table(__cq_gpu_table_r"),
                "{generated}"
            );
            // The resident payload keeps R's original schema index.
            assert!(
                compact.contains("run_column(__cq_gpu_run_view_r,2usize,")
                    || compact.contains("__cq_gpu_run_view_r.columns.add(2usize).read()"),
                "{generated}"
            );
        }
    }
}

fn frozen_prefix_chain() -> ir::Program {
    syn::parse_quote! { physical {
        relation L(a:i64,b:i64); relation R(c:i64,a:i64,b:i64,v:i64);
        l = pipe rename L@0 {a -> x,b -> y};
        r = pipe rename R@1 {c -> z,a -> x,b -> y,v -> payload};
        pre = pipe semi eager [x,y] l,r;
        seeded = pipe bind pre with z:i64 = (1i64);
        out = pipe canon probe first [x,y,z] seeded,r;
        return out as answer(x,y,z,payload);
    } structures {
        l host scan = ::core::iter::Map<_,_> [];
        r host scan = ::core::iter::Map<_,_> [];
        r host pre = ::cq_gpu_runtime::FrozenDeviceIndex<_> [x,y];
        r host out = ::cq_gpu_runtime::FrozenDeviceIndex<_> [x,y,z];
        out canon = ::cq_gpu_runtime::DeviceSet<_>;
    }}
}

#[test]
fn frozen_chain_records_full_layout_and_shares_one_run_between_prefix_reads() {
    let source = frozen_prefix_chain();
    let recorded = crate::compiler::frozen_hosts(&source, quote!(R => [[1,2],[0]])).unwrap();
    let physical = recorded.physical.to_token_stream().to_string();
    assert_eq!(physical, source.physical.to_token_stream().to_string());
    let structures = recorded.structures.to_token_stream().to_string();
    assert!(structures.contains("FrozenKey < 1usize >"));
    assert!(structures.contains("FrozenKey < 2usize >"));
    assert!(structures.contains("FrozenKey < 0usize >"));
    for options in [quote!(), quote!(capacity(R))] {
        let generated = compile(&recorded, syn::parse2(options).unwrap())
            .unwrap()
            .to_token_stream()
            .to_string();
        let compact = generated.split_whitespace().collect::<String>();
        assert_eq!(
            compact
                .matches("resident_run(&[&[1usize,2usize]as&[usize],&[0usize]as&[usize]])")
                .count(),
            1,
            "{generated}"
        );
        assert!(!compact.contains("prepare_prefix_run"), "{generated}");
        assert!(!compact.contains("prepare_hashed_run"), "{generated}");
        // The early semijoin searches only x,y. The final probe also narrows z.
        assert!(
            compact
                .matches("run_column(__cq_gpu_run_view_r,2usize,__mid)")
                .count()
                >= 3,
            "{generated}"
        );
        assert_eq!(
            compact
                .matches("run_column(__cq_gpu_run_view_r,0usize,__mid)")
                .count(),
            2,
            "{generated}"
        );
    }
}

#[test]
fn frozen_chain_rejects_missing_prefixes_and_malformed_full_layouts() {
    for arguments in [
        quote!(R => [[0],[1,2]]),
        quote!(R => [[1,2],[2]]),
        quote!(R => [[1,2],[4]]),
        quote!(R => [[]]),
    ] {
        let error = crate::compiler::frozen_hosts(&frozen_prefix_chain(), arguments)
            .unwrap_err()
            .to_string();
        assert!(error.contains("CQ_GPU_FROZEN_"), "{error}");
    }
    let separate =
        crate::compiler::frozen_hosts(&frozen_prefix_chain(), quote!(R => [[1,2]], R => [[0,1,2]]))
            .unwrap();
    let generated = compile(&separate, Options::default())
        .unwrap()
        .to_token_stream()
        .to_string();
    assert_eq!(generated.matches("resident_run").count(), 2);
}

#[test]
fn leveled_hosts_record_exact_layout_without_changing_physical_reads() {
    let source = frozen_prefix_chain();
    let recorded = crate::compiler::leveled_hosts(&source, quote!(R => [[1,2],[0]])).unwrap();
    assert_eq!(source.physical, recorded.physical);
    let text = recorded.structures.to_token_stream().to_string();
    assert!(text.contains("LeveledDeviceIndex"));
    assert!(text.contains("FrozenKey < 1usize >"));
    assert!(text.contains("FrozenKey < 2usize >"));
    assert!(text.contains("FrozenKey < 0usize >"));
    assert!(!text.contains("FrozenDeviceIndex"));
    ir::contract::check(&recorded).unwrap();
}

#[test]
fn leveled_hosts_reject_wrong_layout_nonprefix_and_missing_host() {
    for (arguments, code) in [
        (quote!(R => [[1,2],[2]]), "CQ_GPU_FROZEN_LAYOUT"),
        (quote!(R => [[1,2],[4]]), "CQ_GPU_FROZEN_LAYOUT"),
        (quote!(R => [[0],[1,2]]), "CQ_GPU_FROZEN_READ_PREFIX"),
        (quote!(Missing => [[0]]), "CQ_GPU_FROZEN_HOST"),
        (quote!(L => [[0]]), "CQ_GPU_LEVELED_HOST"),
    ] {
        let error = crate::compiler::leveled_hosts(&frozen_prefix_chain(), arguments)
            .unwrap_err()
            .to_string();
        assert!(error.contains(code), "{error}");
    }
}

#[test]
fn leveled_prefix_reads_use_one_snapshot_and_any_run_membership() {
    let source =
        crate::compiler::leveled_hosts(&frozen_prefix_chain(), quote!(R => [[1,2],[0]])).unwrap();
    for options in [quote!(), quote!(capacity(R)), quote!(output(retry))] {
        let text = compile(&source, syn::parse2(options).unwrap())
            .unwrap()
            .to_token_stream()
            .to_string();
        let compact = text.split_whitespace().collect::<String>();
        assert_eq!(compact.matches(".snapshot_ptr(").count(), 1, "{text}");
        assert!(
            compact.contains("*const::cq_gpu_runtime::LeveledRunsView"),
            "{text}"
        );
        assert!(
            compact
                .contains("__leveled_run_id<::cq_gpu_runtime::leveled_count(__cq_gpu_runs_view_r)asusize&&!__leveled_found"),
            "{text}"
        );
        assert!(
            compact.contains("blocked_run::point(__cq_gpu_run_view_r.rank,"),
            "{text}"
        );
        assert!(
            compact.contains("run_column(__cq_gpu_run_view_r,0usize,__mid)"),
            "{text}"
        );
        assert_eq!(
            compact.matches("bind_host(").count(),
            1,
            "leveled inputs must not bind a contiguous table: {text}"
        );
        assert!(!compact.contains("prepare_prefix_run"), "{text}");
        assert!(!compact.contains("resident_run("), "{text}");
        assert!(
            !compact.contains("__cq_gpu_runs_view_r:::cq_gpu_runtime::LeveledRunsView"),
            "{text}"
        );
        assert!(!compact.contains("__cq_gpu_runs_view_r.runs"), "{text}");
    }
}

#[test]
fn leveled_expansion_counts_all_ranges_and_restores_run_and_position() {
    for staged in [false, true] {
        let mut source = frozen_host(quote!([x][y]), quote!(x, y));
        if staged {
            source
                .structures
                .entries
                .push(syn::parse_quote!(p stage = batch 256 expand;));
        }
        let source = crate::compiler::leveled_hosts(&source, quote!(R => [[1],[0]])).unwrap();
        for options in [
            quote!(),
            quote!(capacity(R)),
            quote!(output(retry)),
            quote!(tile(256, 4), output(retry)),
        ] {
            let resume = options.is_empty();
            let mut generated = compile(&source, syn::parse2(options).unwrap()).unwrap();
            if resume && staged {
                struct CheckedParentIdentity(usize);
                impl syn::visit_mut::VisitMut for CheckedParentIdentity {
                    fn visit_arm_mut(&mut self, arm: &mut syn::Arm) {
                        let body = arm.body.to_token_stream().to_string();
                        if body.contains("__driver_row_id as u32") {
                            assert!(
                                body.contains("let __driver_row_id ="),
                                "descriptor save must bind its parent identity in the same resume PC: {body}"
                            );
                            self.0 += 1;
                        }
                        syn::visit_mut::visit_arm_mut(self, arm);
                    }
                }
                let mut check = CheckedParentIdentity(0);
                syn::visit_mut::VisitMut::visit_expr_mut(&mut check, &mut generated);
                assert!(check.0 > 0);
            }
            let text = generated.to_token_stream().to_string();
            let compact = text.split_whitespace().collect::<String>();
            assert!(
                compact.contains("leveled_run(__cq_gpu_runs_view_r,__leveled_run_id)"),
                "{text}"
            );
            if resume && !staged {
                assert!(
                    compact.contains("__leveled_total+=(__run_end-__run_begin)asu64"),
                    "{text}"
                );
                assert!(
                    compact.contains("__leveled_position-=__run_end-__run_begin"),
                    "{text}"
                );
                assert!(
                    compact.contains("let__build_row=__leveled_build_row"),
                    "{text}"
                );
            }
            if staged {
                assert!(compact.contains("__descriptor.add("), "{text}");
                assert!(
                    compact.contains("__leveled_run_id=*__descriptor.add(1)asusize"),
                    "{text}"
                );
                assert!(compact.contains("::cq_gpu_runtime::leveled_any("), "{text}");
                assert!(!compact.contains("2*__leveled_run_id*"), "{text}");
                assert!(compact.contains("tiles::scan("), "{text}");
            }
            if resume {
                assert!(compact.contains("__resume_registers"), "{text}");
                assert!(compact.contains("let__resume_shared_words="), "{text}");
            }
        }
    }
}

#[test]
fn leveled_full_row_probe_and_anti_are_any_run() {
    for operator in [quote!(probe first), quote!(semi eager), quote!(anti eager)] {
        let source: ir::Program = syn::parse2(quote! { physical {
            relation L(a:i64,b:i64); relation R(a:i64,b:i64);
            l = pipe rename L@0 {a -> x,b -> y};
            r = pipe rename R@1 {a -> x,b -> y};
            p = pipe canon #operator [x,y] l,r;
            return p as answer(x,y);
        } structures {
            l host scan = ::core::iter::Map<_,_> [];
            r host scan = ::core::iter::Map<_,_> [];
            r host p = ::cq_gpu_runtime::FrozenDeviceIndex<_> [x][y];
            p canon = ::cq_gpu_runtime::DeviceSet<_>;
        }})
        .unwrap();
        let source = crate::compiler::leveled_hosts(&source, quote!(R => [[0],[1]])).unwrap();
        for options in [quote!(), quote!(capacity(L)), quote!(output(retry))] {
            let text = compile(&source, syn::parse2(options).unwrap())
                .unwrap()
                .to_token_stream()
                .to_string();
            assert!(text.contains("__leveled_found"), "{text}");
            assert!(text.contains("__contains_lo"), "{text}");
        }
    }
}

#[test]
fn leveled_hosts_reject_actual_scans() {
    let scan: ir::Program = syn::parse_quote! { physical {
        relation R(a:i64,b:i64);
        l = pipe rename R@0 {a -> x,b -> w};
        r = pipe rename R@1 {a -> x,b -> y};
        p = pipe canon probe first [x] l,r;
        return p as answer(x,w,y);
    } structures {
        l host scan = ::core::iter::Map<_,_> [];
        r host scan = ::core::iter::Map<_,_> [];
        r host p = ::cq_gpu_runtime::FrozenDeviceIndex<_> [x];
        p canon = ::cq_gpu_runtime::DeviceSet<_>;
    }};
    let error = crate::compiler::leveled_hosts(&scan, quote!(R => [[0]]))
        .unwrap_err()
        .to_string();
    assert!(error.contains("CQ_GPU_LEVELED_SCAN"), "{error}");
}

#[test]
fn leveled_witness_searches_once_and_preserves_range_sets() {
    for staged in [false, true] {
        let mut source: ir::Program = syn::parse_quote! { physical {
            relation L(a:i64,b:i64); relation R(a:i64,b:i64,v:i64);
            l = pipe rename L@0 {a -> x,b -> y};
            r = pipe rename R@1 {a -> x,b -> y,v -> z};
            s = pipe semi eager [x,y] l,r;
            p = pipe canon probe first [x,y] s,r;
            return p as answer(x,y,z);
        } structures {
            l host scan = ::core::iter::Map<_,_> [];
            r host scan = ::core::iter::Map<_,_> [];
            r host s = ::cq_gpu_runtime::LeveledDeviceIndex<_> [x][y];
            r host p = ::cq_gpu_runtime::LeveledDeviceIndex<_> [x][y];
            s share = lookup r [x,y] first s force p;
            s repr = tuples witness {r [x,y] at s as ::cq_storage::RunCursor<'_,_>};
            p canon = ::cq_gpu_runtime::DeviceSet<_>;
        }};
        if staged {
            source
                .structures
                .entries
                .push(syn::parse_quote!(p stage = batch 256 expand;));
        }
        let source = crate::compiler::leveled_hosts(&source, quote!(R => [[0],[1]])).unwrap();
        for options in [
            quote!(),
            quote!(capacity(L)),
            quote!(output(retry)),
            quote!(tile(256, 4), output(retry)),
        ] {
            let text = compile(&source, syn::parse2(options).unwrap())
                .unwrap()
                .to_token_stream()
                .to_string();
            assert_eq!(
                text.matches("blocked_run :: point").count(),
                1,
                "forcing a witness must not search again: {text}"
            );
            assert!(text.contains("witness:s"), "{text}");
            assert!(text.contains("32"), "{text}");
            assert!(text.contains("__leveled_run_id"), "{text}");
            if staged {
                assert!(text.contains("leveled_any"), "{text}");
                assert!(text.contains("__descriptor"), "{text}");
            }
        }
    }
}

#[test]
fn frozen_host_output_is_byte_identical_when_leveled_pass_has_no_binding() {
    fn stable(tokens: proc_macro2::TokenStream) -> String {
        tokens
            .into_iter()
            .map(|token| match token {
                proc_macro2::TokenTree::Group(group) => {
                    format!("{:?}[{}]", group.delimiter(), stable(group.stream()))
                }
                proc_macro2::TokenTree::Ident(ident) => {
                    let text = ident.to_string();
                    if text.contains("__cq_gpu_module_")
                        || text.contains("cq_gpu_stream_")
                        || text.contains("cq_gpu_expand_")
                    {
                        text.split('_')
                            .map(|part| {
                                if !part.is_empty() && part.chars().all(|c| c.is_ascii_digit()) {
                                    "MODULE"
                                } else {
                                    part
                                }
                            })
                            .collect::<Vec<_>>()
                            .join("_")
                    } else {
                        text
                    }
                }
                proc_macro2::TokenTree::Literal(literal) => {
                    if let Ok(text) = syn::parse2::<syn::LitStr>(quote!(#literal))
                        && text.value().starts_with("cq_gpu_stream_")
                    {
                        format!(
                            "{:?}",
                            text.value()
                                .split('_')
                                .map(|part| {
                                    if !part.is_empty() && part.chars().all(|c| c.is_ascii_digit())
                                    {
                                        "MODULE"
                                    } else {
                                        part
                                    }
                                })
                                .collect::<Vec<_>>()
                                .join("_")
                        )
                    } else {
                        literal.to_string()
                    }
                }
                other => other.to_string(),
            })
            .collect::<Vec<_>>()
            .join(" ")
    }
    for staged in [false, true] {
        let mut source = frozen_host(quote!([x][y]), quote!(x, y));
        if staged {
            source
                .structures
                .entries
                .push(syn::parse_quote!(p stage = batch 256 expand;));
        }
        let untouched = crate::compiler::leveled_hosts(&source, quote!()).unwrap();
        for options in [quote!(), quote!(capacity(R)), quote!(output(retry))] {
            let before = compile(&source, syn::parse2(options.clone()).unwrap())
                .unwrap()
                .to_token_stream();
            let after = compile(&untouched, syn::parse2(options).unwrap())
                .unwrap()
                .to_token_stream();
            assert!(!after.to_string().contains("__leveled"));
            assert_eq!(stable(before), stable(after));
        }
    }
}

#[test]
fn frozen_sorted_ranges_narrow_before_cooperative_expansion() {
    for (levels, slot) in [(quote!([x, y]), 1usize), (quote!([x][y]), 0usize)] {
        let mut program = frozen_host(levels, quote!(x, y));
        program
            .structures
            .entries
            .push(syn::parse_quote!(p stage = batch 256 expand;));
        for options in [quote!(), quote!(capacity(R)), quote!(output(retry))] {
            let frontier = options.to_string().starts_with("capacity");
            let generated = compile(&program, syn::parse2(options).unwrap())
                .unwrap()
                .to_token_stream()
                .to_string();
            let compact = generated.split_whitespace().collect::<String>();
            assert!(compact.contains("resident_run("), "{generated}");
            assert!(
                compact.contains("blocked_run::point(__cq_gpu_run_view_r.rank,"),
                "{generated}"
            );
            for comparison in ["<", "<="] {
                assert!(
                    compact.contains(&format!(
                        "run_column(__cq_gpu_run_view_r,{slot}usize,__mid){comparison}__prefix_key"
                    )),
                    "{generated}"
                );
            }
            assert!(
                compact.contains("(__prefix_begin,__prefix_end)"),
                "{generated}"
            );
            assert!(compact.contains("tiles::scan("), "{generated}");
            assert!(!compact.contains("hashed_run_step"), "{generated}");
            assert!(!compact.contains("prepare_prefix_run"), "{generated}");
            assert!(!compact.contains("prepare_hashed_run"), "{generated}");
            if frontier {
                assert!(compact.contains("frontier_claim"), "{generated}");
            }
        }
    }
}

#[test]
fn frozen_hosts_rewrite_records_only_selected_existing_host_reads() {
    let mut program = frozen_host(quote!([x]), quote!(x));
    for entry in &mut program.structures.entries {
        if let ir::Structure::Host { keyed, reader, .. } = &mut entry.structure {
            if reader == "p" {
                keyed.ty = syn::parse_quote!(::cq_storage::SortedRun<_,_,_>);
            }
        }
    }
    let frozen = crate::compiler::frozen_hosts(&program, quote!(R)).unwrap();
    assert!(compile(&frozen, Options::default()).is_ok());
    assert!(crate::compiler::frozen_hosts(&program, quote!()).is_ok());
    assert!(
        crate::compiler::frozen_hosts(&program, quote!(Missing))
            .unwrap_err()
            .to_string()
            .contains("CQ_GPU_FROZEN_HOST")
    );
    assert!(
        compile(&program, Options::default())
            .unwrap_err()
            .to_string()
            .contains("CQ_GPU_HOST_KEYED_FALLBACK")
    );
}

#[test]
fn frozen_hosts_reject_missing_or_mismatched_recorded_levels() {
    let wrong = frozen_host(quote!([y]), quote!(x));
    assert!(
        compile(&wrong, Options::default())
            .unwrap_err()
            .to_string()
            .contains("fallback does not match a host read")
    );
    let missing = frozen_host(quote!([]), quote!(x));
    assert!(
        compile(&missing, Options::default())
            .unwrap_err()
            .to_string()
            .contains("fallback does not match a host read")
    );
    let invalid = frozen_host(quote!([x,y][]), quote!(x, y));
    assert!(compile(&invalid, Options::default()).is_err());
}

#[test]
fn ordinary_nested_expansions_use_four_item_tiles_and_compact_carriers() {
    let mut source: ir::Program = syn::parse_quote! { physical {
        relation Customer(customer:i64,nation:i64);
        relation Nation(nation:i64,region:i64);
        relation Orders(customer:i64,order:i64);
        relation Lines(order:i64,supplier:i64);
        relation Supplier(supplier:i64,nation:i64);
        c = pipe rename Customer@0 {customer -> customer,nation -> nation};
        nr = let rename Nation@1 {nation -> nation,region -> region};
        orders = let rename Orders@2 {customer -> customer,order -> order};
        lines = let rename Lines@3 {order -> order,supplier -> supplier};
        suppliers = let rename Supplier@4 {supplier -> supplier,nation -> nation};
        n = pipe probe first [nation] c,nr;
        o = pipe probe first [customer] n,orders;
        l = pipe probe first [order] o,lines;
        s = pipe probe first [nation,supplier] l,suppliers;
        out = pipe canon project s keep {nation,order,supplier};
        return out as answer(nation,order,supplier);
    } structures {
        c host scan = ::core::iter::Map<_,_> [];
        nr host scan = ::core::iter::Map<_,_> [];
        orders host scan = ::core::iter::Map<_,_> [];
        lines host scan = ::core::iter::Map<_,_> [];
        suppliers host scan = ::core::iter::Map<_,_> [];
        nr = ::cq_storage::SortedRun<_,_,::cq_storage::BorrowedI64TupleCodec> [nation];
        orders = ::cq_storage::SortedRun<_,_,::cq_storage::BorrowedI64TupleCodec> [customer];
        lines = ::cq_storage::SortedRun<_,_,::cq_storage::BorrowedI64TupleCodec> [order];
        suppliers = ::cq_gpu_runtime::PrefixRun<_,_,::cq_storage::BorrowedI64TupleCodec,1> [supplier][nation];
        n stage = batch 256 expand;
        o stage = batch 256 expand;
        l stage = batch 256 expand;
        out canon = ::cq_gpu_runtime::DeviceSet<_>;
    }};
    for (options, fused) in [
        (quote!(), false),
        (quote!(capacity(Lines)), false),
        (quote!(output(retry)), false),
        (quote!(output(retry)), true),
        (quote!(output(retry), capacity(Lines)), true),
    ] {
        let ir::Structure::Dedup { ty, .. } =
            &mut source.structures.entries.last_mut().unwrap().structure
        else {
            unreachable!()
        };
        *ty = if fused {
            syn::parse_quote!(::cq_gpu_runtime::DeviceSet<_,::cq_gpu_runtime::For,::cq_gpu_runtime::Fused>)
        } else {
            syn::parse_quote!(::cq_gpu_runtime::DeviceSet<_>)
        };
        let generated = compile(&source, syn::parse2(options).unwrap())
            .unwrap()
            .to_token_stream()
            .to_string();
        let compact = generated.split_whitespace().collect::<String>();
        assert!(
            compact.contains("dynamic_shared_range=(36976u32,36976u32)"),
            "{generated}"
        );
        assert_eq!(compact.matches("tiles::scan(__count,").count(), 3);
        assert_output_chunk_scan(&generated);
        if fused {
            assert_fused_retry_kernel(&generated);
            assert!(!compact.contains("__execution.dedup("), "{generated}");
            assert_eq!(compact.matches("cq_gpu_runtime::reserve(").count(), 1);
        }
        assert_eq!(
            compact.matches("while__contains_lo<__contains_hi").count(),
            1
        );
        assert!(!compact.contains("__overflows"));
        assert!(compact.contains("owner(__descriptor.add(1026usize),1024usizeasu32,"));
        assert!(compact.contains("column(__driver,1usize,__driver_row)"));
        assert!(compact.contains("run_column(__cq_gpu_run_view_lines,0usize,__build_row)"));
        assert!(!compact.contains("as*muti64).add(__input_slot)"));
        if compact.contains("execute_resumable") {
            assert!(compact.contains("=*__descriptor.add(__word)"));
        } else {
            assert_eq!(compact.matches("unsafefncq_gpu_expand_").count(), 3);
            assert!(!compact.contains("_out_s("));
        }
    }
}

#[test]
fn nested_claim_reuse_keeps_room_for_maximum_descriptor_publication() {
    let source: ir::Program = syn::parse_quote! { physical {
        relation L(a:i64,b:i64); relation R(b:i64,v:i64);
        relation S(a:i64,v:i64); relation T(v:i64,w:i64);
        l = pipe rename L@0 {a -> a,b -> b};
        r = let rename R@1 {b -> b,v -> v};
        s = let rename S@2 {a -> a,v -> v};
        t = let rename T@3 {v -> v,w -> w};
        candidate = pipe probe smallest [b] l,r;
        checked = pipe probe first [a,v] candidate,s;
        out = pipe canon probe first [v] checked,t;
        return out as answer(a,b,v,w);
    } structures {
        l host scan = ::core::iter::Map<_,_> [];
        r host scan = ::core::iter::Map<_,_> [];
        s host scan = ::core::iter::Map<_,_> [];
        t host scan = ::core::iter::Map<_,_> [];
        r = ::cq_gpu_runtime::PrefixRun<_,_,::cq_storage::BorrowedI64TupleCodec,0> [b][v];
        s = ::cq_gpu_runtime::PrefixRun<_,_,::cq_storage::BorrowedI64TupleCodec,0> [a][v];
        t = ::cq_storage::SortedRun<_,_,::cq_storage::BorrowedI64TupleCodec> [v];
        candidate stage = batch 256 expand;
        out stage = batch 256 expand;
        out canon = ::cq_gpu_runtime::DeviceSet<_>;
    }};
    for options in [quote!(), quote!(capacity(L))] {
        let generated = compile(&source, syn::parse2(options).unwrap())
            .unwrap()
            .to_token_stream()
            .to_string();
        let compact = generated.split_whitespace().collect::<String>();
        // Root=3588 words; child=3076. Its max-sized publication needs another
        // 512 words of padding, but no separate 3588-word claim allocation.
        assert!(
            compact.contains("dynamic_shared_range=(28768u32,28768u32)"),
            "{generated}"
        );
        assert!(compact.contains("let__descriptor=__scratch.add(16usize)"));
        assert!(compact.contains("let__descriptor=__scratch.add(3604usize)"));
    }
}

#[test]
fn full_key_probe_uses_membership_only_when_canon_erases_duplicate_witnesses() {
    for canon in [false, true] {
        let mode = if canon {
            quote!(pipe canon)
        } else {
            quote!(let)
        };
        let after = (!canon).then(|| quote!(out = pipe canon project p keep {x,y};));
        let returned = if canon { quote!(p) } else { quote!(out) };
        let dedup = if canon {
            quote!(p canon = ::cq_gpu_runtime::DeviceSet<_>;)
        } else {
            quote!(p rows = ::std::vec::Vec<_>; out canon = ::cq_gpu_runtime::DeviceSet<_>;)
        };
        let source: ir::Program = syn::parse2(quote! { physical {
            relation L(a:i64,b:i64); relation R(a:i64,b:i64);
            l = pipe rename L@0 {a -> x,b -> y};
            r = let rename R@1 {a -> x,b -> y};
            p = #mode probe first [x,y] l,r;
            #after
            return #returned as answer(x,y);
        } structures {
            l host scan = ::core::iter::Map<_,_> [];
            r host scan = ::core::iter::Map<_,_> [];
            r = ::cq_gpu_runtime::PrefixRun<_,_,::cq_storage::BorrowedI64TupleCodec,0> [x][y];
            #dedup
        }})
        .unwrap();
        let generated = compile(&source, syn::parse2(quote!(capacity(L))).unwrap())
            .unwrap()
            .to_token_stream()
            .to_string();
        assert_eq!(
            generated.contains("while __contains_lo < __contains_hi"),
            canon
        );
        assert_eq!(
            generated.contains("while __cq_gpu_row_p < __cq_gpu_end_p"),
            !canon
        );
        assert!(!generated.contains("while __candidate"));
    }
}

#[test]
fn composite_hashed_runs_require_recorded_type_and_check_every_key() {
    let mut source: ir::Program = syn::parse_quote! { physical {
        relation L(a:i64,b:i64); relation R(a:i64,b:i64,c:i64);
        l = pipe rename L@0 {a -> x,b -> y};
        r = let rename R@1 {a -> x,b -> y,c -> z};
        p = pipe canon probe first [x,y] l,r;
        return p as answer(x,y,z);
    } structures {
        l host scan = ::core::iter::Map<_,_> [];
        r host scan = ::core::iter::Map<_,_> [];
        r rows = ::std::vec::Vec<_>;
        r = ::cq_gpu_runtime::HashedRun<_,_> [x,y];
        p canon = ::cq_gpu_runtime::DeviceSet<_>;
    }};
    for options in [quote!(), quote!(capacity(R))] {
        let text = compile(&source, syn::parse2(options).unwrap())
            .unwrap()
            .to_token_stream()
            .to_string();
        assert!(text.contains("prepare_hashed_run"), "{text}");
        assert_eq!(text.matches("hashed_run_step").count(), 2);
        assert!(text.contains("run_column (__cq_gpu_run_view_r , 0usize"));
        assert!(text.contains("run_column (__cq_gpu_run_view_r , 1usize"));
    }
    let ir::Structure::Built { keyed, .. } = &mut source.structures.entries[3].structure else {
        unreachable!()
    };
    keyed.ty = syn::parse_quote!(::cq_storage::HashIndex<'_, _, _>);
    assert!(
        compile(&source, Options::default())
            .unwrap_err()
            .to_string()
            .contains("CQ_GPU_BUILD_TYPE")
    );
}

#[test]
fn dedup_execution_policy_is_read_only_from_the_recorded_type() {
    for (policy, compacting, fused) in [
        (quote!(::cq_gpu_runtime::Staged), false, false),
        (quote!(::cq_gpu_runtime::Compacting), true, false),
        (quote!(::cq_gpu_runtime::Fused), false, true),
    ] {
        let mut source = shared();
        let ir::Structure::Dedup { ty, .. } = &mut source.structures.entries[4].structure else {
            unreachable!()
        };
        *ty = syn::parse_quote!(::cq_gpu_runtime::DeviceSet<_, ::cq_gpu_runtime::For, #policy>);
        let text = compile(&source, Options::default())
            .unwrap()
            .to_token_stream()
            .to_string();
        assert_eq!(
            text.contains("execute_resumable_compacting"),
            compacting,
            "{text}"
        );
        assert_eq!(text.contains("execute_resumable_fused"), fused, "{text}");
        assert_eq!(text.contains("fused_emit"), fused, "{text}");
        if fused {
            assert!(!text.contains("__execution . dedup"), "{text}");
            let bounded = compile(&source, syn::parse2(quote!(capacity(R))).unwrap())
                .unwrap()
                .to_token_stream()
                .to_string();
            assert!(bounded.contains("execute_resumable_fused"), "{bounded}");
            assert!(!bounded.contains("execute_bounded"), "{bounded}");
        }
    }
    let mut source = shared();
    let ir::Structure::Dedup { ty, .. } = &mut source.structures.entries[4].structure else {
        unreachable!()
    };
    *ty = syn::parse_quote!(::cq_gpu_runtime::DeviceSet<_, ::cq_gpu_runtime::Plain, ::cq_gpu_runtime::UnknownPolicy>);
    assert!(
        compile(&source, Options::default())
            .unwrap_err()
            .to_string()
            .contains("CQ_GPU_DEDUP_POLICY")
    );
}

#[test]
fn compacting_resumption_records_the_full_pending_reservation_bound() {
    for (stage, options, maximum) in [
        (quote!(), quote!(tile(256, 4)), 1),
        (
            quote!(p stage = batch 32 expand;),
            quote!(tile(256, 4)),
            256,
        ),
        (quote!(p stage = batch 256 expand;), quote!(), 1024),
        (
            quote!(p stage = batch 256 expand;),
            quote!(tile(256, 4)),
            1024,
        ),
    ] {
        let mut source = shared();
        let ir::Structure::Dedup { ty, .. } = &mut source.structures.entries[4].structure else {
            unreachable!()
        };
        *ty = syn::parse_quote!(
            ::cq_gpu_runtime::DeviceSet<_, ::cq_gpu_runtime::Plain, ::cq_gpu_runtime::Compacting>
        );
        if !stage.is_empty() {
            source.structures.entries.push(syn::parse2(stage).unwrap());
        }
        let generated = compile(&source, syn::parse2(options).unwrap())
            .unwrap()
            .to_token_stream()
            .to_string();
        let compact = generated.split_whitespace().collect::<String>();
        let (_, call) = compact.split_once("execute_resumable_compacting(").unwrap();
        assert!(
            call.contains(&format!(",{maximum}usize,|__stream,__output,__resume,|")),
            "{generated}"
        );
    }
}

#[test]
fn retry_preserves_fused_canon_and_stages_other_recorded_policies() {
    for (policy, fused) in [
        (quote!(::cq_gpu_runtime::Staged), false),
        (quote!(::cq_gpu_runtime::Compacting), false),
        (quote!(::cq_gpu_runtime::Fused), true),
    ] {
        let mut source = shared();
        let ir::Structure::Dedup { ty, .. } = &mut source.structures.entries[4].structure else {
            unreachable!()
        };
        *ty = syn::parse_quote!(::cq_gpu_runtime::DeviceSet<_, ::cq_gpu_runtime::For, #policy>);
        for options in [quote!(output(retry)), quote!(output(retry), capacity(R))] {
            let generated = compile(&source, syn::parse2(options).unwrap())
                .unwrap()
                .to_token_stream()
                .to_string();
            assert_retry_kernel(&generated);
            let compact = generated.split_whitespace().collect::<String>();
            if fused {
                assert_fused_retry_kernel(&generated);
                assert!(!compact.contains("__execution.dedup("), "{generated}");
            } else {
                assert!(compact.contains("execute_retry("), "{generated}");
                assert!(
                    compact.contains("execute_retry(concat!(module_path!(),\"::\",stringify!("),
                    "bounded history must be isolated by generated kernel site: {generated}"
                );
                assert!(!compact.contains("fused_retry_insert"), "{generated}");
                assert!(!compact.contains("FusedRetryView"), "{generated}");
                assert!(!compact.contains("__fused"), "{generated}");
                assert!(
                    compact.find("execute_retry(").unwrap()
                        < compact.find("__execution.dedup(").unwrap(),
                    "canon must consume only the successful retry result: {generated}"
                );
                assert_eq!(compact.matches("__execution.dedup(").count(), 1);
            }
        }
    }
}

#[test]
fn pure_projection_uses_runtime_without_a_redundant_copy_kernel() {
    let source: ir::Program = syn::parse_quote! { physical {
        relation Arbitrary(k:i64);
        scan = pipe rename Arbitrary@0 {k -> renamed};
        output = pipe canon project scan keep {renamed};
        return output as result(renamed);
    } structures {
        scan host scan = ::core::iter::Map<_,_> [];
        output canon = ::cq_gpu_runtime::DeviceSet<_>;
    } };
    let generated = compile(&source, Options::default())
        .unwrap()
        .to_token_stream()
        .to_string();
    assert!(!generated.contains("cuda_module"));
    assert!(generated.contains("__execution . dedup"));
    assert!(generated.contains("bind_host"));
    assert!(generated.contains("into_columns :: < (i64 ,) , :: cq_gpu_runtime :: Plain >"));
    assert!(!generated.contains("download"));
    assert!(!generated.contains("decode_value"));
    assert!(!generated.contains("into_iter"));
    assert_eq!(generated.matches("pub fn cq_gpu_stream_").count(), 0);
    assert!(generated.contains("project_table"));
    assert!(!generated.contains("prepare_run"));
    assert!(!generated.contains("blocked_run :: point"));
}

#[test]
fn every_keyed_read_has_exactly_one_provenance_entry_and_witness_does_not_reprobe() {
    let source = shared();
    let facts = ir::contract::check(&source).unwrap();
    let reads = validate(&source, &facts).unwrap();
    assert_eq!(
        reads.len(),
        facts
            .physical
            .keyed_reads
            .values()
            .map(Vec::len)
            .sum::<usize>()
    );
    assert_eq!(
        reads.iter().find(|r| r.reader == "s").unwrap().source,
        "rank-point"
    );
    assert_eq!(
        reads.iter().find(|r| r.reader == "p").unwrap().source,
        "witness:s"
    );
    assert!(reads.iter().all(|r| r.build == "b" && r.keys == ["x"]));
    let generated = compile(&source, syn::parse2(quote!(capacity(R))).unwrap())
        .unwrap()
        .to_token_stream()
        .to_string();
    assert_eq!(generated.matches("blocked_run :: point").count(), 1);
    assert_eq!(generated.matches("prepare_run").count(), 1);
    assert_eq!(generated.matches("pub fn cq_gpu_stream_").count(), 1);
    assert!(generated.contains("cuda_module"));
    assert!(generated.contains("p:b[x]:witness:s"));
    assert!(
        generated.contains(
            "let (__cq_gpu_begin_p , __cq_gpu_end_p) = (__cq_gpu_begin_s , __cq_gpu_end_s)"
        )
    );
    assert!(generated.contains("while __cq_gpu_row_p < __cq_gpu_end_p"));
    assert!(!generated.contains("tiles :: owner"));
    assert!(generated.contains("let __cq_gpu_payload_p_y = __cq_gpu_run_view_b . columns"));
    assert!(generated.contains("__cq_gpu_payload_p_y . add (__cq_gpu_row_p as usize) . read ()"));
}

#[test]
fn unshared_first_probe_still_expands_every_range_row() {
    let mut source = shared();
    source.structures.entries.retain(|entry| {
        !matches!(
            entry.structure,
            ir::Structure::Share { .. } | ir::Structure::Repr { .. }
        )
    });
    let generated = compile(&source, syn::parse2(quote!(capacity(R))).unwrap())
        .unwrap()
        .to_token_stream()
        .to_string();
    assert_eq!(generated.matches("blocked_run :: point").count(), 2);
    assert!(generated.contains("while __cq_gpu_row_p < __cq_gpu_end_p"));
    assert!(generated.contains("__cq_gpu_row_p += 1"));
}

#[test]
fn recorded_expand_enables_frontier_and_zero_disables_it() {
    let mut source = shared();
    source
        .structures
        .entries
        .push(syn::parse_quote!(p stage = batch 32 expand;));
    let enabled = compile(&source, syn::parse2(quote!(capacity(R))).unwrap())
        .unwrap()
        .to_token_stream()
        .to_string();
    assert!(enabled.contains("frontier_claim"));
    assert!(enabled.contains("32usize"));
    let disabled = compile(
        &source,
        syn::parse2(quote!(capacity(R), frontier(0))).unwrap(),
    )
    .unwrap()
    .to_token_stream()
    .to_string();
    assert!(!disabled.contains("frontier_claim"));
    assert!(disabled.contains("false &&"));
}

#[test]
fn backend_rejection_codes_name_every_unsupported_operator() {
    let source = shared();
    let facts = ir::contract::check(&source).unwrap();
    // Exercise the backend policy independently: the language checker continues
    // to own validity of the complete graph; unsupported choices cannot hide in
    // an unused definition or require reaching a CUDA invocation to be rejected.
    for (definition, code) in [
        (quote!(bad = pipe merge [x] l,b;), "CQ_GPU_MERGE"),
        (quote!(bad = pipe union l,b;), "CQ_GPU_UNION"),
        (quote!(bad = pipe anti lazy [x] l,b;), "CQ_GPU_ANTI_CHOICE"),
        (quote!(bad = pipe guard l if (true);), "CQ_GPU_OPAQUE_GUARD"),
        (
            quote!(bad = pipe bind l with z:i64 = (opaque(x));),
            "CQ_GPU_OPAQUE_BIND",
        ),
        (
            quote!(bad = pipe bind l with for z:i64 = ([x, x + 1]);),
            "CQ_GPU_GENERATOR",
        ),
    ] {
        let mut target = source.clone();
        target
            .physical
            .program
            .definitions
            .push(syn::parse2(definition).unwrap());
        assert!(
            validate(&target, &facts)
                .unwrap_err()
                .to_string()
                .contains(code),
            "{code}"
        );
    }
}

#[test]
fn generators_are_rejected_before_device_lowering() {
    for binding in [
        quote!(for y:i64 = ([x, x + 1])),
        quote!(for y = ([x, x + 1])),
        quote!(for (y, z) = ([(x, x + 1)])),
    ] {
        let source: ir::Program = syn::parse2(quote! { physical {
            relation L(a:i64);
            l = pipe rename L@0 {a -> x};
            g = pipe bind l with #binding;
            p = pipe canon project g keep {y};
            return p as answer(y);
        } structures {
            l host scan = ::core::iter::Map<_,_> [];
            p canon = ::cq_gpu_runtime::DeviceSet<_>;
        } })
        .unwrap();
        assert!(
            compile(&source, Options::default())
                .unwrap_err()
                .to_string()
                .contains("CQ_GPU_GENERATOR")
        );
    }
}

#[test]
fn backend_rejection_codes_name_unsupported_recorded_storage() {
    let source = shared();
    let facts = ir::contract::check(&source).unwrap();
    for (entry, code) in [
        (
            quote!(b = ::cq_storage::Trie<_,_> [x][y];),
            "CQ_GPU_MULTILEVEL_BUILD",
        ),
        (
            quote!(b = ::cq_storage::HashIndex<'_,_,_> [x];),
            "CQ_GPU_BUILD_TYPE",
        ),
        (
            quote!(p canon = ::cq_storage::HashSet<_>;),
            "CQ_GPU_NON_DEVICE_CANON",
        ),
        (
            quote!(l host s = ::cq_storage::HashIndex<'_,_,_> [x];),
            "CQ_GPU_HOST_KEYED_FALLBACK",
        ),
    ] {
        let mut target = source.clone();
        target
            .structures
            .entries
            .insert(0, syn::parse2(entry).unwrap());
        assert!(
            validate(&target, &facts)
                .unwrap_err()
                .to_string()
                .contains(code),
            "{code}"
        );
    }
    let mut bad_types = facts;
    bad_types
        .physical
        .rows
        .get_mut("b")
        .unwrap()
        .types
        .insert("x".into(), syn::parse_quote!(String));
    assert!(
        validate(&source, &bad_types)
            .unwrap_err()
            .to_string()
            .contains("CQ_GPU_NON_INTEGRAL_KEY")
    );
}

#[test]
fn printer_options_are_explicit_and_checked() {
    assert_eq!(Options::default().items, 4);
    assert_eq!(syn::parse2::<Options>(quote!()).unwrap().items, 4);
    assert_eq!(Options::default().output, OutputPolicy::Resumable);
    assert_eq!(
        syn::parse2::<Options>(quote!()).unwrap().output,
        OutputPolicy::Resumable
    );
    assert_eq!(
        syn::parse2::<Options>(quote!(output(retry)))
            .unwrap()
            .output,
        OutputPolicy::Retry
    );
    assert_eq!(DEFAULT_RETRY_CAPACITY, 1024);
    let options: Options = syn::parse2(quote!(tile(256, 4), frontier(0), directory(rank))).unwrap();
    assert_eq!(options.frontier, 0);
    assert_eq!(options.items, 4);
    for (args, code) in [
        (quote!(tile(128, 4)), "CQ_GPU_TILE"),
        (quote!(tile(256, 1)), "CQ_GPU_TILE"),
        (quote!(tile(256, 8)), "CQ_GPU_TILE"),
        (quote!(directory(fences)), "CQ_GPU_DIRECTORY"),
        (quote!(output(unknown)), "CQ_GPU_OUTPUT_POLICY"),
        (quote!(output(resumable)), "CQ_GPU_OUTPUT_POLICY"),
        (
            quote!(output(retry), output(retry)),
            "CQ_GPU_DUPLICATE_ARGUMENT",
        ),
        (
            quote!(output(retry), output(unknown)),
            "CQ_GPU_DUPLICATE_ARGUMENT",
        ),
        (
            quote!(frontier(1), frontier(2)),
            "CQ_GPU_DUPLICATE_ARGUMENT",
        ),
    ] {
        assert!(
            syn::parse2::<Options>(args)
                .unwrap_err()
                .to_string()
                .contains(code)
        );
    }
}

#[test]
fn dictionary_order_is_never_silently_integer_order() {
    let vars = BTreeMap::from([("name".into(), quote!(__name))]);
    let types = BTreeMap::from([("name".into(), syn::parse_quote!(String))]);
    let theta = syn::parse_quote!(name < "z");
    assert!(
        Constants::default()
            .theta(&theta, &vars, &types)
            .unwrap_err()
            .to_string()
            .contains("CQ_GPU_STRING_ORDER")
    );
}

fn smallest_intersection(second: proc_macro2::TokenStream) -> ir::Program {
    syn::parse2(quote! { physical {
        relation L(a:i64,b:i64); relation R(b:i64,v:i64); relation S(a:i64,v:i64);
        l = pipe rename L@0 {a -> a,b -> b};
        r = let rename R@1 {b -> b,v -> v};
        s = let rename S@2 {a -> a,v -> v};
        candidate = pipe probe smallest [b] l,r;
        checked = pipe canon #second [a,v] candidate,s;
        return checked as answer(a,b,v);
    } structures {
        l host scan = ::core::iter::Map<_,_> [];
        r host scan = ::core::iter::Map<_,_> [];
        s host scan = ::core::iter::Map<_,_> [];
        r rows = ::std::vec::Vec<_>;
        r = ::cq_gpu_runtime::PrefixRun<_,_,::cq_storage::BorrowedI64TupleCodec,0> [b][v];
        s rows = ::std::vec::Vec<_>;
        s = ::cq_gpu_runtime::PrefixRun<_,_,::cq_storage::BorrowedI64TupleCodec,0> [a][v];
        checked canon = ::cq_gpu_runtime::DeviceSet<_>;
    }})
    .unwrap()
}

fn assert_smallest_cooperative_chunks(generated: &str) {
    let compact = generated.split_whitespace().collect::<String>();
    assert!(
        compact.contains("__item<4usize") || compact.contains("__item+1<4usize"),
        "{generated}"
    );
    assert!(
        compact.contains("__input_slot=__item*256+__tid"),
        "{generated}"
    );
    assert!(compact.contains("1024"), "{generated}");
    assert!(compact.contains("__descriptor"), "{generated}");
    assert!(compact.contains("__smallest_right=__root_parent&0x8000u16!=0"));
    assert!(compact.contains("__root_parent&0x7fffu16"));
    assert!(compact.contains("as*mutu16).add(__input_slot)="));
    assert!(compact.contains("__input_slotasu16|((__smallest_rightasu16)<<15)"));
    assert!(compact.contains("let__driver_row="));
    for slot in [0usize, 1usize] {
        assert!(
            compact.contains(&format!(
                "cq_gpu_runtime::column(__driver,{slot}usize,__driver_row)"
            )),
            "{generated}"
        );
    }
    assert!(
        compact.contains("let(__smallest_other_begin,__smallest_other_end)=if__valid"),
        "{generated}"
    );
    for bound in ["__smallest_other_begin", "__smallest_other_end"] {
        assert!(
            !compact.split(';').any(|statement| {
                statement.contains("__descriptor.add(") && statement.contains(&format!("={bound}"))
            }),
            "membership bounds must be reprobed, not stored: {generated}"
        );
    }
    // Two root prefix lookups choose the side; each candidate executes only
    // one of the two emitted branches that recover the other prefix range.
    assert_eq!(compact.matches("blocked_run::point").count(), 4);
    assert!(compact.contains("__count>u32::MAX/256"), "{generated}");
    assert!(
        compact.contains("__expansion_sum:u64") || compact.contains("__expansion_sum=0u64"),
        "{generated}"
    );
    assert!(
        compact.contains("__expansion_sum>u32::MAXasu64"),
        "{generated}"
    );
    assert!(compact.contains("flag_error(__output,2)"), "{generated}");
    assert!(!compact.contains("__overflows"), "{generated}");
    assert!(
        compact.contains("while__contains_lo<__contains_hi"),
        "{generated}"
    );
    assert_output_chunk_scan(generated);
}

fn assert_single_smallest_descriptor(generated: &str, carried_values: usize) {
    let compact = generated.split_whitespace().collect::<String>();
    // Header, starts, exclusive offsets, alignment, driver row IDs and packed
    // u16 parent/side entries. Only demanded non-driver values add i64 arrays.
    let descriptor_words = 2 + 1024 + 1025 + 1 + 1024 + 512 + carried_values * 2048;
    let shared_bytes = (16 + descriptor_words) * 4;
    assert!(
        compact.contains(&format!(
            "dynamic_shared_range=({shared_bytes}u32,{shared_bytes}u32)"
        )),
        "{generated}"
    );
    assert_eq!(
        compact.matches("as*muti64).add(__input_slot)=").count(),
        carried_values,
        "driver columns must reload instead of becoming i64 carriers: {generated}"
    );
    let descriptors: Vec<_> = compact
        .split("let__descriptor=__scratch.add(")
        .skip(1)
        .collect();
    let minimum = if compact.contains("frontier_claim(") {
        2
    } else {
        1
    };
    assert!(descriptors.len() >= minimum, "{generated}");
    assert!(
        descriptors.iter().all(|rest| rest.starts_with("16usize)")),
        "root and stolen tasks must share one descriptor: {generated}"
    );
    assert!(
        !compact.contains("=*__descriptor.add(__word)"),
        "claim dispatch must not copy a second descriptor: {generated}"
    );
}

#[test]
fn staged_smallest_capacity_uses_four_item_cooperative_chunks() {
    for second in [quote!(probe first), quote!(semi eager)] {
        let mut source = smallest_intersection(second);
        source.structures.entries.push(syn::parse_quote! {
            candidate stage = batch 256 expand;
        });
        let generated = compile(
            &source,
            syn::parse2(quote!(tile(256, 4), capacity(L))).unwrap(),
        )
        .unwrap()
        .to_token_stream()
        .to_string();
        assert_smallest_cooperative_chunks(&generated);
        assert_single_smallest_descriptor(&generated, 0);
        assert!(generated.contains("__tile_hits"), "{generated}");
        assert!(generated.contains("__tile_values"), "{generated}");
        assert!(generated.contains("__tile_count"), "{generated}");
        assert_eq!(generated.matches("cq_gpu_runtime :: reserve (").count(), 1);
        assert!(generated.contains("frontier_publish"), "{generated}");
        assert!(generated.contains("frontier_claim"), "{generated}");
        assert!(generated.contains("execute_bounded"), "{generated}");
        assert!(!generated.contains("execute_resumable"), "{generated}");
    }
}

fn assert_output_chunk_scan(generated: &str) {
    let compact = generated.split_whitespace().collect::<String>();
    let count = if compact.contains("execute_retry_fused(") {
        "__tile_count|(__tile_candidates<<11)"
    } else {
        "__tile_count"
    };
    assert_eq!(
        compact.matches(&format!("tiles::scan({count},")).count(),
        1,
        "each generated candidate-chunk output path needs one count scan: {generated}"
    );
}

fn assert_fused_retry_kernel(generated: &str) {
    assert_retry_kernel(generated);
    let compact = generated.split_whitespace().collect::<String>();
    for required in [
        "execute_retry_fused(concat!(module_path!(),\"::\",stringify!(",
        "FusedRetryView",
        "fused_retry_insert(",
        "fused_retry_full(",
        "fused_retry_count(",
        "|__stream,__output,__fused|{",
        "__tile_candidates+=__validasu32;",
        "if__valid&&!__fused_full{",
        "ballot_sync(u32::MAX,__valid&&!__fused_full)",
        "__fused_full=__admission==2;",
        "__valid=__admission==1;",
        "}else{__valid=false;}",
    ] {
        assert!(compact.contains(required), "{required}: {generated}");
    }
    let (_, call) = compact
        .split_once("execute_retry_fused(concat!(module_path!(),\"::\",stringify!(")
        .expect("fused retry site identifies the generated kernel");
    assert!(
        call.contains(")),&[") || call.contains(")),&__head_bounds,"),
        "the site must precede the output bounds: {generated}"
    );
    for forbidden in ["execute_retry(", "FusedView", "fused_emit", "__pending"] {
        assert!(!compact.contains(forbidden), "{forbidden}: {generated}");
    }
    assert_output_chunk_scan(generated);
    assert_eq!(compact.matches("fused_retry_full(").count(), 1);
    assert_eq!(compact.matches("fused_retry_count(").count(), 1);
    assert!(
        compact.find("__tile_candidates+=__validasu32;").unwrap()
            < compact.find("if__valid&&!__fused_full{").unwrap(),
        "duplicates and candidates after overflow must still be counted: {generated}"
    );
    assert!(
        compact.find("let__fused_participants=").unwrap()
            < compact.find("if__valid&&!__fused_full{").unwrap(),
        "the admission mask must be recorded before branching: {generated}"
    );
    assert!(
        compact.find("fused_retry_count(").unwrap()
            < compact.find("cq_gpu_runtime::reserve(").unwrap(),
        "count every valid candidate before reserving unique output: {generated}"
    );
}

fn assert_retry_kernel(generated: &str) {
    let compact = generated.split_whitespace().collect::<String>();
    assert!(
        compact.contains("const__CQ_GPU_OUTPUT_POLICY:&str=\"retry\";"),
        "{generated}"
    );
    assert!(
        compact.contains("const__CQ_GPU_RETRY_DEFAULT_CAPACITY:usize=1024usize;"),
        "{generated}"
    );
    assert!(
        compact.contains("execute_retry(") || compact.contains("execute_retry_fused("),
        "{generated}"
    );
    for forbidden in [
        "execute_resumable",
        "execute_bounded",
        "__resume",
        "ResumeView",
        "__checkpoint",
        "__lane_checkpoint",
        "__saved_shared",
        "resume_reserve",
        "resume_requested",
    ] {
        assert!(!compact.contains(forbidden), "{forbidden}: {generated}");
    }
    let callback = compact
        .split("|__stream,__output")
        .nth(1)
        .and_then(|arguments| arguments.split_once('|'))
        .map(|(_, body)| body)
        .expect("retry launch callback");
    if compact.contains("__execution.frontier(") {
        assert!(
            callback.starts_with("{__pool.reset("),
            "each launch must reset its frontier inside the callback: {generated}"
        );
        assert_eq!(compact.matches("__pool.reset(").count(), 1);
    } else {
        // A projected-head kernel has no root/steal state to reset. Its set,
        // ticket arena and output are freshly allocated by execute_retry_fused.
        assert_eq!(compact.matches("__pool.reset(").count(), 0);
    }
}

#[test]
fn retry_fused_smallest_keeps_four_item_descriptors_and_one_packed_output_scan() {
    for second in [quote!(probe first), quote!(semi eager)] {
        let mut source = smallest_intersection(second);
        let ir::Structure::Dedup { ty, .. } =
            &mut source.structures.entries.last_mut().unwrap().structure
        else {
            unreachable!()
        };
        *ty = syn::parse_quote!(::cq_gpu_runtime::DeviceSet<_,::cq_gpu_runtime::For,::cq_gpu_runtime::Fused>);
        source.structures.entries.push(syn::parse_quote! {
            candidate stage = batch 256 expand;
        });
        for (options, capacity_hint) in [
            (quote!(output(retry)), "None"),
            (
                quote!(output(retry), capacity(L)),
                "Some(__execution.table_view(__cq_gpu_host_L)?.lenasusize)",
            ),
        ] {
            let generated = compile(&source, syn::parse2(options).unwrap())
                .unwrap()
                .to_token_stream()
                .to_string();
            assert_fused_retry_kernel(&generated);
            assert_smallest_cooperative_chunks(&generated);
            assert_single_smallest_descriptor(&generated, 0);
            let compact = generated.split_whitespace().collect::<String>();
            assert!(
                compact.contains(&format!("],{capacity_hint},|__stream,__output,")),
                "{generated}"
            );
            for token in [
                "__tile_hits",
                "__tile_values",
                "__tile_candidates",
                "frontier_publish",
                "frontier_claim",
            ] {
                assert!(compact.contains(token), "{generated}");
            }
            assert_eq!(compact.matches("cq_gpu_runtime::reserve(").count(), 1);
            assert!(!compact.contains("__execution.dedup("), "{generated}");
            assert!(!compact.contains("download"), "{generated}");
        }
    }
}

#[test]
fn retry_smallest_uses_the_compact_bounded_kernel_and_initial_capacity_hint() {
    for second in [quote!(probe first), quote!(semi eager)] {
        let mut source = smallest_intersection(second);
        source.structures.entries.push(syn::parse_quote! {
            candidate stage = batch 256 expand;
        });
        for (options, capacity) in [
            (quote!(output(retry)), "__CQ_GPU_RETRY_DEFAULT_CAPACITY"),
            (
                quote!(output(retry), capacity(L)),
                "__execution.table_view(__cq_gpu_host_L)?.lenasusize",
            ),
        ] {
            let generated = compile(&source, syn::parse2(options).unwrap())
                .unwrap()
                .to_token_stream()
                .to_string();
            assert_retry_kernel(&generated);
            assert_smallest_cooperative_chunks(&generated);
            assert_single_smallest_descriptor(&generated, 0);
            let compact = generated.split_whitespace().collect::<String>();
            assert!(
                compact.contains("dynamic_shared_range=(14416u32,14416u32)"),
                "{generated}"
            );
            assert!(
                compact.contains(&format!("],{capacity},|__stream,__output|")),
                "{generated}"
            );
            for token in [
                "__tile_hits",
                "__tile_values",
                "__tile_count",
                "frontier_publish",
                "frontier_claim",
            ] {
                assert!(compact.contains(token), "{generated}");
            }
            assert_eq!(compact.matches("cq_gpu_runtime::reserve(").count(), 1);
            assert!(!compact.contains("download"), "{generated}");
        }
    }
}

#[test]
fn default_smallest_resumes_four_item_cooperative_chunks() {
    for second in [quote!(probe first), quote!(semi eager)] {
        let mut source = smallest_intersection(second.clone());
        source.structures.entries.push(syn::parse_quote! {
            candidate stage = batch 256 expand;
        });
        let generated = compile(&source, Options::default())
            .unwrap()
            .to_token_stream()
            .to_string();
        assert_smallest_cooperative_chunks(&generated);
        assert_single_smallest_descriptor(&generated, 0);
        assert!(generated.contains("__tile_item"), "{generated}");
        assert!(generated.contains("__resume_pc"), "{generated}");
        assert!(generated.contains("__resume_registers"), "{generated}");
        assert_eq!(
            generated
                .matches("cq_gpu_runtime :: resume_reserve (")
                .count(),
            1
        );
        assert!(generated.contains("execute_resumable"), "{generated}");
        assert!(!generated.contains("execute_bounded"), "{generated}");
        assert!(!generated.contains("execute_retry"), "{generated}");
        assert!(!generated.contains("download"), "{generated}");
    }
}

#[test]
fn smallest_expands_one_chosen_side_and_binary_searches_the_other() {
    for second in [quote!(probe first), quote!(semi eager)] {
        let source = smallest_intersection(second);
        for options in [quote!(), quote!(capacity(L))] {
            let generated = compile(&source, syn::parse2(options).unwrap())
                .unwrap()
                .to_token_stream()
                .to_string();
            assert_eq!(generated.matches("prepare_prefix_run").count(), 2);
            assert!(
                generated.contains("prepare_prefix_run (__cq_gpu_table_r , & [0usize , 1usize])")
            );
            assert!(
                generated.contains("prepare_prefix_run (__cq_gpu_table_s , & [0usize , 1usize])")
            );
            assert!(
                generated.contains(
                    &quote!(
                        __smallest_right = __smallest_b_end - __smallest_b_begin
                            < __smallest_a_end - __smallest_a_begin
                    )
                    .to_string()
                ),
                "{generated}"
            );
            assert!(generated.contains("if __smallest_right"), "{generated}");
            assert!(generated.contains("__smallest_other_begin"), "{generated}");
            assert!(generated.contains("__smallest_other_end"), "{generated}");
            // Both prefix directories are consulted once; the checked read is
            // consumed by the intersection, not emitted as a separate lookup.
            assert_eq!(generated.matches("blocked_run :: point").count(), 2);
            assert!(
                generated.contains("while __contains_lo < __contains_hi"),
                "{generated}"
            );
            assert!(generated.contains("__contains_lo ="), "{generated}");
            assert!(generated.contains("__contains_hi ="), "{generated}");
            assert!(!generated.contains("while __candidate"));
            assert!(!generated.contains("while __cq_gpu_row_checked"));
            assert!(!generated.contains("let mut __found"));
        }
    }
}

#[test]
fn staged_smallest_reloads_roots_and_reprobes_membership_range() {
    for second in [quote!(probe first), quote!(semi eager)] {
        let mut source = smallest_intersection(second);
        source
            .structures
            .entries
            .push(syn::parse_quote!(candidate stage = batch 256 expand;));
        for options in [quote!(), quote!(capacity(L))] {
            let generated = compile(&source, syn::parse2(options).unwrap())
                .unwrap()
                .to_token_stream()
                .to_string();
            assert!(generated.contains("frontier_publish"), "{generated}");
            assert!(generated.contains("frontier_claim"), "{generated}");
            assert_smallest_cooperative_chunks(&generated);
            assert_single_smallest_descriptor(&generated, 0);
            assert!(!generated.contains("while __cq_gpu_row_checked"));
            assert_eq!(generated.matches("prepare_prefix_run").count(), 2);
        }
    }
}

#[test]
fn compact_smallest_carries_only_demanded_non_driver_bindings() {
    for (expression, demanded, carried_values, reloads) in [
        (quote!(7), true, 1usize, 1usize),
        (quote!(7), false, 0, 1),
        (quote!(*a), true, 0, 2),
    ] {
        let output = if demanded {
            quote!(a, b, v, extra)
        } else {
            quote!(a, b, v)
        };
        let source: ir::Program = syn::parse2(quote! { physical {
            relation L(a:i64,b:i64); relation R(b:i64,v:i64); relation S(a:i64,v:i64);
            l = pipe rename L@0 {a -> a,b -> b};
            bound = pipe bind l with extra:i64 = (#expression);
            r = let rename R@1 {b -> b,v -> v};
            s = let rename S@2 {a -> a,v -> v};
            candidate = pipe probe smallest [b] bound,r;
            checked = pipe probe first [a,v] candidate,s;
            output = pipe canon project checked keep {#output};
            return output as answer(#output);
        } structures {
            l host scan = ::core::iter::Map<_,_> [];
            r host scan = ::core::iter::Map<_,_> [];
            s host scan = ::core::iter::Map<_,_> [];
            r rows = ::std::vec::Vec<_>;
            r = ::cq_gpu_runtime::PrefixRun<_,_,::cq_storage::BorrowedI64TupleCodec,0> [b][v];
            s rows = ::std::vec::Vec<_>;
            s = ::cq_gpu_runtime::PrefixRun<_,_,::cq_storage::BorrowedI64TupleCodec,0> [a][v];
            l stage = batch 256;
            output canon = ::cq_gpu_runtime::DeviceSet<_>;
        }})
        .unwrap();
        for options in [quote!(), quote!(capacity(L))] {
            let generated = compile(&source, syn::parse2(options).unwrap())
                .unwrap()
                .to_token_stream()
                .to_string();
            assert_smallest_cooperative_chunks(&generated);
            assert_single_smallest_descriptor(&generated, carried_values);
            let compact = generated.split_whitespace().collect::<String>();
            assert_eq!(
                compact
                    .matches("cq_gpu_runtime::column(__driver,0usize,__driver_row)")
                    .count(),
                reloads,
                "copied driver aliases must reload from the same column: {generated}"
            );
        }
    }
}

#[test]
fn compact_smallest_does_not_reload_a_reintroduced_constant() {
    let source: ir::Program = syn::parse_quote! { physical {
        relation L(a:i64,b:i64); relation M(a:i64,b:i64);
        relation R(b:i64,v:i64); relation S(a:i64,v:i64);
        l = pipe rename L@0 {a -> a,b -> b};
        live = pipe select l if (a = 1);
        projected = pipe project live keep {b};
        m = pipe rename M@1 {a -> a,b -> b};
        member = let point select m if (a = 7);
        restored = pipe probe first [b] projected,member;
        r = let rename R@2 {b -> b,v -> v};
        s = let rename S@3 {a -> a,v -> v};
        candidate = pipe probe smallest [b] restored,r;
        checked = pipe canon probe first [a,v] candidate,s;
        return checked as answer(a,b,v);
    } structures {
        l host scan = ::core::iter::Map<_,_> [];
        l stage = batch 256;
        m host scan = ::core::iter::Map<_,_> [];
        m host member = ::core::iter::Map<_,_> [a];
        member = ::cq_storage::AdaptiveKeySet<_,::cq_storage::BorrowedI64TupleCodec,4096> [b]
            membership payload [b] constants {a = 7};
        r host scan = ::core::iter::Map<_,_> [];
        s host scan = ::core::iter::Map<_,_> [];
        r rows = ::std::vec::Vec<_>;
        s rows = ::std::vec::Vec<_>;
        r = ::cq_gpu_runtime::PrefixRun<_,_,::cq_storage::BorrowedI64TupleCodec,0> [b][v];
        s = ::cq_gpu_runtime::PrefixRun<_,_,::cq_storage::BorrowedI64TupleCodec,0> [a][v];
        checked canon = ::cq_gpu_runtime::DeviceSet<_>;
    }};
    for options in [quote!(), quote!(capacity(L))] {
        let generated = compile(&source, syn::parse2(options).unwrap())
            .unwrap()
            .to_token_stream()
            .to_string();
        let compact = generated.split_whitespace().collect::<String>();
        assert!(!compact.contains("cq_gpu_runtime::column(__driver,0usize,__driver_row)"));
        assert!(compact.contains("cq_gpu_runtime::column(__driver,1usize,__driver_row)"));
    }
}

#[test]
fn smallest_reuses_a_recorded_first_side_witness() {
    let source: ir::Program = syn::parse_quote! { physical {
        relation L(a:i64,b:i64); relation R(b:i64,v:i64); relation S(a:i64,v:i64);
        l = pipe rename L@0 {a -> a,b -> b};
        r = let rename R@1 {b -> b,v -> v};
        s = let rename S@2 {a -> a,v -> v};
        pre = pipe semi eager [b] l,r;
        candidate = pipe probe smallest [b] pre,r;
        checked = pipe canon probe first [a,v] candidate,s;
        return checked as answer(a,b,v);
    } structures {
        l host scan = ::core::iter::Map<_,_> [];
        r host scan = ::core::iter::Map<_,_> [];
        s host scan = ::core::iter::Map<_,_> [];
        r rows = ::std::vec::Vec<_>;
        s rows = ::std::vec::Vec<_>;
        r = ::cq_storage::SortedRun<_,_,::cq_storage::BorrowedI64TupleCodec> [b];
        r = ::cq_gpu_runtime::PrefixRun<_,_,::cq_storage::BorrowedI64TupleCodec,0> [b][v];
        s = ::cq_gpu_runtime::PrefixRun<_,_,::cq_storage::BorrowedI64TupleCodec,0> [a][v];
        pre share = lookup r [b] first pre force candidate;
        pre repr = tuples witness { r [b] at pre as ::cq_storage::RunCursor<'_,_> };
        candidate stage = batch 256 expand;
        checked canon = ::cq_gpu_runtime::DeviceSet<_>;
    }};
    for options in [quote!(), quote!(capacity(L))] {
        let generated = compile(&source, syn::parse2(options).unwrap())
            .unwrap()
            .to_token_stream()
            .to_string();
        // One eager lookup and the second root prefix lookup, followed by
        // two alternative candidate-range reprobe branches.
        assert_eq!(generated.matches("blocked_run :: point").count(), 4);
        assert!(generated.contains("__cq_gpu_run_view_r_1 . columns"));
        assert!(generated.contains("__smallest_a_begin"));
        assert!(generated.contains("while __contains_lo < __contains_hi"));
    }
}

#[test]
fn smallest_reuses_both_recorded_prefix_witnesses_at_the_introducing_probe() {
    for second in [quote!(probe first), quote!(semi eager)] {
        let source: ir::Program = syn::parse2(quote! { physical {
            relation L(a:i64,b:i64); relation R(b:i64,v:i64); relation S(a:i64,v:i64);
            l = pipe rename L@0 {a -> a,b -> b};
            r = let rename R@1 {b -> b,v -> v};
            s = let rename S@2 {a -> a,v -> v};
            pre_r = pipe semi eager [b] l,r;
            pre_s = pipe semi eager [a] pre_r,s;
            candidate = pipe probe smallest [b] pre_s,r;
            checked = pipe canon #second [a,v] candidate,s;
            return checked as answer(a,b,v);
        } structures {
            l host scan = ::core::iter::Map<_,_> [];
            r host scan = ::core::iter::Map<_,_> [];
            s host scan = ::core::iter::Map<_,_> [];
            r rows = ::std::vec::Vec<_>;
            s rows = ::std::vec::Vec<_>;
            r = ::cq_gpu_runtime::PrefixRun<_,_,::cq_storage::BorrowedI64TupleCodec,0> [b][v];
            s = ::cq_gpu_runtime::PrefixRun<_,_,::cq_storage::BorrowedI64TupleCodec,0> [a][v];
            pre_r share = lookup r [b] first pre_r force candidate;
            pre_r repr = tuples witness { r [b] at pre_r as ::cq_storage::RunCursor<'_,_> };
            pre_s share = lookup s [a] first pre_s force candidate;
            pre_s repr = tuples witness { s [a] at pre_s as ::cq_storage::RunCursor<'_,_> };
            checked canon = ::cq_gpu_runtime::DeviceSet<_>;
        }})
        .unwrap();
        for staged in [false, true] {
            let mut source = source.clone();
            if staged {
                source.structures.entries.push(syn::parse_quote! {
                    candidate stage = batch 256 expand;
                });
            }
            for options in [quote!(), quote!(capacity(L))] {
                let generated = compile(&source, syn::parse2(options).unwrap())
                    .unwrap()
                    .to_token_stream()
                    .to_string();
                assert_eq!(generated.matches("prepare_prefix_run").count(), 2);
                // Eager semis supply both root prefix witnesses. Staged
                // candidates recover the other range in one of two branches;
                // unstaged serial traversal keeps the original ranges.
                assert_eq!(
                    generated.matches("blocked_run :: point").count(),
                    if staged { 4 } else { 2 }
                );
                assert!(generated.contains("__smallest_a_begin"), "{generated}");
                assert!(generated.contains("__smallest_b_begin"), "{generated}");
                assert!(generated.contains("while __contains_lo < __contains_hi"));
                assert!(!generated.contains("while __cq_gpu_row_checked"));
            }
        }
    }
}

#[test]
fn smallest_requires_explicit_prefix_runs_on_both_sides() {
    for second in [quote!(probe first), quote!(semi eager)] {
        for (entry, side, provider) in [(4, "side 1", "r"), (6, "side 2", "s")] {
            let mut source = smallest_intersection(second.clone());
            let ir::Structure::Built { keyed, .. } =
                &mut source.structures.entries[entry].structure
            else {
                unreachable!()
            };
            // Even a recorded sorted run is not a sorted-trie candidate index.
            keyed.ty =
                syn::parse_quote!(::cq_storage::SortedRun<_,_,::cq_storage::BorrowedI64TupleCodec>);
            for options in [quote!(), quote!(capacity(L))] {
                let error = compile(&source, syn::parse2(options).unwrap())
                    .unwrap_err()
                    .to_string();
                assert!(error.contains("CQ_GPU_SMALLEST_TRIE"), "{error}");
                assert!(
                    error.contains(&format!("{side} provider {provider}")),
                    "{error}"
                );
                assert!(error.contains("selection"), "{error}");
                assert!(error.contains("prefix_runs"), "{error}");
            }
        }
    }
}

#[test]
fn recorded_prefix_run_binary_searches_before_expanding() {
    let source: ir::Program = syn::parse_quote! {physical {
        relation L(a:i64,b:i64); relation R(a:i64,b:i64,c:i64);
        l = pipe rename L@0 {a -> x,b -> y};
        r = let rename R@1 {a -> x,b -> y,c -> z};
        p = pipe canon probe first [x,y] l,r;
        return p as answer(x,y,z);
    } structures {
        l host scan = ::core::iter::Map<_,_> [];
        r host scan = ::core::iter::Map<_,_> [];
        r rows = ::std::vec::Vec<_>;
        r = ::cq_gpu_runtime::PrefixRun<_,_,::cq_storage::BorrowedI64TupleCodec,0> [x][y];
        p canon = ::cq_gpu_runtime::DeviceSet<_>;
    }};
    let facts = ir::contract::check(&source).unwrap();
    let reads = validate(&source, &facts).unwrap();
    assert_eq!(reads.len(), 1);
    assert_eq!(reads[0].keys, ["x", "y"]);
    assert_eq!(reads[0].source, "rank-point+binary-search");
    for options in [quote!(), quote!(capacity(R))] {
        let generated = compile(&source, syn::parse2(options).unwrap())
            .unwrap()
            .to_token_stream()
            .to_string();
        assert_eq!(generated.matches("prepare_prefix_run").count(), 1);
        assert!(generated.contains("prepare_prefix_run (__cq_gpu_table_r , & [0usize , 1usize])"));
        assert!(
            generated.contains("run_column (__cq_gpu_run_view_r , 1usize , __mid) < __prefix_key")
        );
        assert!(
            generated.contains("run_column (__cq_gpu_run_view_r , 1usize , __mid) <= __prefix_key")
        );
        assert!(!generated.contains("while __candidate"));
        assert!(!generated.contains("== __cq_gpu_tile_column_1"));
        assert!(generated.contains("(__prefix_begin , __prefix_end)"));
    }
}

#[test]
fn prefix_membership_narrows_each_recorded_column_without_residual_scan() {
    for operation in [quote!(semi), quote!(anti)] {
        for levels in [quote!([x][y][z]), quote!([x][y,z])] {
            let source: ir::Program = syn::parse2(quote! { physical {
                relation L(a:i64,b:i64,c:i64); relation R(a:i64,b:i64,c:i64);
                l = pipe rename L@0 {a -> x,b -> y,c -> z};
                r = let rename R@1 {a -> x,b -> y,c -> z};
                p = pipe canon #operation eager [x,y,z] l,r;
                return p as answer(x,y,z);
            } structures {
                l host scan = ::core::iter::Map<_,_> [];
                r host scan = ::core::iter::Map<_,_> [];
                r rows = ::std::vec::Vec<_>;
                r = ::cq_gpu_runtime::PrefixRun<_,_,::cq_storage::BorrowedI64TupleCodec,0> #levels;
                p canon = ::cq_gpu_runtime::DeviceSet<_>;
            }})
            .unwrap();
            for options in [quote!(), quote!(capacity(L))] {
                let generated = compile(&source, syn::parse2(options).unwrap())
                    .unwrap()
                    .to_token_stream()
                    .to_string();
                assert!(generated.contains(
                    "prepare_prefix_run (__cq_gpu_table_r , & [0usize , 1usize , 2usize])"
                ));
                assert_eq!(generated.matches("while __lo < __hi").count(), 4);
                for slot in [1usize, 2usize] {
                    let lower = quote!(::cq_gpu_runtime::run_column(__cq_gpu_run_view_r, #slot, __mid) < __prefix_key).to_string();
                    let upper = quote!(::cq_gpu_runtime::run_column(__cq_gpu_run_view_r, #slot, __mid) <= __prefix_key).to_string();
                    assert!(generated.contains(&lower));
                    assert!(generated.contains(&upper));
                }
                assert!(generated.contains("let __found ="));
                assert!(!generated.contains("while __candidate"));
                assert!(!generated.contains("let mut __found"));
            }
        }
    }
}

#[test]
fn prefix_reads_search_only_the_requested_levels() {
    for (columns, rename, keys, searches) in [
        (quote!(a:i64), quote!(a -> x), quote!(x), 0),
        (quote!(a:i64,b:i64), quote!(a -> x,b -> y), quote!(x, y), 2),
    ] {
        let source: ir::Program = syn::parse2(quote! { physical {
            relation L(#columns); relation R(a:i64,b:i64,c:i64);
            l = pipe rename L@0 {#rename};
            r = let rename R@1 {a -> x,b -> y,c -> z};
            p = pipe canon semi eager [#keys] l,r;
            return p as answer(#keys);
        } structures {
            l host scan = ::core::iter::Map<_,_> [];
            r host scan = ::core::iter::Map<_,_> [];
            r rows = ::std::vec::Vec<_>;
            r = ::cq_gpu_runtime::PrefixRun<_,_,::cq_storage::BorrowedI64TupleCodec,0> [x][y][z];
            p canon = ::cq_gpu_runtime::DeviceSet<_>;
        }})
        .unwrap();
        for options in [quote!(), quote!(capacity(R))] {
            let generated = compile(&source, syn::parse2(options).unwrap())
                .unwrap()
                .to_token_stream()
                .to_string();
            assert!(
                generated.contains(
                    "prepare_prefix_run (__cq_gpu_table_r , & [0usize , 1usize , 2usize])"
                )
            );
            assert_eq!(generated.matches("blocked_run :: point").count(), 1);
            assert_eq!(generated.matches("while __lo < __hi").count(), searches);
            assert!(!generated.contains("run_column (__cq_gpu_run_view_r , 2usize , __mid)"));
            assert!(!generated.contains("while __candidate"));
        }
    }
}

#[test]
fn recorded_membership_uses_its_bitmap_budget_and_reconstructs_constants() {
    let source: ir::Program = syn::parse_quote! {physical {
        relation L(k:i64); relation R(k:i64,name:String);
        l = pipe rename L@0 {k -> x};
        r = pipe rename R@1 {k -> x,name -> label};
        b = let point select r if (label = "tag");
        p = pipe canon probe first [x] l,b;
        return p as answer(x,label);
    } structures {
        l host scan = ::core::iter::Map<_,_> [];
        r host scan = ::core::iter::Map<_,_> [];
        r host b = ::core::iter::Map<_,_> [label];
        b = ::cq_storage::AdaptiveKeySet<_,::cq_storage::BorrowedI64TupleCodec,4096> [x]
            membership payload [x] constants {label = "tag"};
        p canon = ::cq_gpu_runtime::DeviceSet<_>;
    }};
    let generated = compile(&source, Options::default())
        .unwrap()
        .to_token_stream()
        .to_string();
    assert!(generated.contains("prepare_membership"));
    assert!(generated.contains("4096usize"));
    assert!(generated.contains("membership_contains"));
    assert!(!generated.contains("prepare_run"));
    assert!(generated.contains("bitmap-membership"));
}

#[test]
fn recorded_join_filter_keeps_its_exact_covering_read_and_budget() {
    let mut source = shared();
    source.structures.entries.push(syn::parse_quote! {
        l join_filter = ::cq_storage::AdaptiveKeySet<i64, ::cq_storage::I64Codec, 8192>
            build b [x] covering s marker ::cq_load::ColumnAt<0>;
    });
    let facts = ir::contract::check(&source).unwrap();
    let reads = validate(&source, &facts).unwrap();
    assert_eq!(reads.len(), 2);
    assert_eq!(
        reads.iter().find(|read| read.reader == "s").unwrap().source,
        "rank-point"
    );
    let generated = compile(&source, syn::parse2(quote!(capacity(R))).unwrap())
        .unwrap()
        .to_token_stream()
        .to_string();
    assert_eq!(generated.matches("prepare_membership").count(), 1);
    assert_eq!(generated.matches("membership_contains").count(), 1);
    assert_eq!(generated.matches("prepare_run").count(), 1);
    assert_eq!(generated.matches("blocked_run :: point").count(), 1);
    assert!(generated.contains("8192usize"));
    assert!(generated.contains("__CQ_GPU_JOIN_FILTER_READS"));
    assert!(generated.contains("l:join-filter:0:b[x]:covering=s"));
    assert!(generated.contains("s:b[x]:rank-point"));
    assert!(!generated.contains("positional_handle"));
    assert!(
        generated.find("membership_contains").unwrap()
            < generated.find("blocked_run :: point").unwrap()
    );
}

#[test]
fn join_filter_does_not_silently_replace_bloom_or_opaque_providers() {
    for ty in [
        quote!(::cq_join_filters::BloomFilter<16>),
        quote!(::user::AdaptiveKeySet<i64, ::cq_storage::I64Codec, 8192>),
    ] {
        let mut source = shared();
        source.structures.entries.push(syn::parse_quote! {
            l join_filter = #ty build b [x] covering s marker ::cq_load::ColumnAt<0>;
        });
        let error = compile(&source, Options::default())
            .unwrap_err()
            .to_string();
        assert!(error.contains("CQ_GPU_JOIN_FILTER_TYPE"), "{error}");
    }
}

#[test]
fn provider_names_must_resolve_to_the_supported_recorded_paths() {
    for ty in [
        quote!(::user::SortedRun<_, _, ::cq_storage::BorrowedI64TupleCodec>),
        quote!(SortedRun<_, _, ::cq_storage::BorrowedI64TupleCodec>),
        quote!(::user::AdaptiveKeySet<_, ::cq_storage::BorrowedI64TupleCodec, 4096>),
    ] {
        let mut source = shared();
        let ir::Structure::Built { keyed, .. } = &mut source.structures.entries[3].structure else {
            unreachable!("fixture build");
        };
        keyed.ty = syn::parse2(ty).unwrap();
        let facts = ir::contract::check(&shared()).unwrap();
        let error = validate(&source, &facts).unwrap_err().to_string();
        assert!(error.contains("CQ_GPU_BUILD_TYPE"), "{error}");
    }
    let mut source = shared();
    let ir::Structure::Repr {
        witness: Some(witness),
        ..
    } = &mut source.structures.entries[6].structure
    else {
        unreachable!("fixture witness");
    };
    witness.ty = syn::parse_quote!(::user::RunCursor<'_, _>);
    let error = compile(&source, Options::default())
        .unwrap_err()
        .to_string();
    assert!(error.contains("CQ_GPU_WITNESS_TYPE"), "{error}");

    for ty in [
        quote!(::std::vec::Vec<_>),
        quote!(::alloc::vec::Vec<_>),
        quote!(Vec<_>),
    ] {
        let mut source = shared();
        let ir::Structure::Rows { ty: rows, .. } = &mut source.structures.entries[2].structure
        else {
            unreachable!("fixture rows");
        };
        *rows = syn::parse2(ty).unwrap();
        compile(&source, Options::default()).unwrap();
    }
    let mut source = shared();
    let ir::Structure::Rows { ty, .. } = &mut source.structures.entries[2].structure else {
        unreachable!("fixture rows");
    };
    *ty = syn::parse_quote!(::user::Vec<_>);
    let error = compile(&source, Options::default())
        .unwrap_err()
        .to_string();
    assert!(error.contains("CQ_GPU_ROWS_TYPE"), "{error}");
}

#[test]
fn resident_host_binding_preserves_codec_ownership_and_explicit_download() {
    let source: ir::Program = syn::parse_quote! { physical {
        relation Input(k:i64,label:&str) = (&resident);
        scan = pipe rename Input@0 {k -> x,label -> name};
        output = pipe canon project scan keep {x,name};
        return output as answer(name,x,name);
    } structures {
        scan host scan = ::cq_gpu_runtime::DeviceColumns<'_,(i64,&str),::cq_gpu_runtime::Delta> [];
        output canon = ::cq_gpu_runtime::DeviceSet<_>;
    }};
    let generated = compile(&source, Options::default())
        .unwrap()
        .to_token_stream()
        .to_string();
    assert!(generated.contains("HostColumns :: pool"));
    assert!(generated.contains("with_runtime_for"));
    assert!(generated.contains("bind_host"));
    assert!(generated.contains("HostColumns :: seed_dictionary"));
    assert!(generated.find("seed_dictionary").unwrap() < generated.find("bind_host").unwrap());
    assert!(
        generated
            .contains("into_columns :: < (& str , i64 , & str ,) , :: cq_gpu_runtime :: Plain >")
    );
    assert!(generated.contains("& [0usize , 1usize , 0usize]"));
    assert!(generated.contains("__execution . dedup"));
    assert!(!generated.contains("Codec :: Delta"));
    assert!(!generated.contains("encode_slice"));
    assert!(!generated.contains("decode_value"));
    assert!(!generated.contains("download"));
}

#[test]
fn derived_codecs_follow_only_recorded_provider_types() {
    for (marker, codec) in [
        (quote!(::cq_gpu_runtime::Plain), "Plain"),
        (quote!(::cq_gpu_runtime::For), "For"),
        (quote!(::cq_gpu_runtime::Delta), "Delta"),
        (quote!(::cq_gpu_runtime::DictionaryCodec), "Dictionary"),
    ] {
        let mut source = shared();
        source.structures.entries[4] =
            syn::parse_quote!(p canon = ::cq_gpu_runtime::DeviceSet<_,#marker>;);
        let generated = compile(&source, Options::default())
            .unwrap()
            .to_token_stream()
            .to_string();
        assert!(
            generated.contains(&format!("Codec :: {codec}")),
            "{generated}"
        );
        assert!(generated.contains("__CQ_GPU_DERIVED_CODECS"));
        assert_eq!(generated.matches("__execution . dedup").count(), 1);
        assert!(generated.contains(&format!("(\"p\" , \"{codec}\")")));
    }
    let mut source = shared();
    source.structures.entries[2] = syn::parse_quote! {
        b rows = ::cq_gpu_runtime::DeviceColumns<_,::cq_gpu_runtime::For>;
    };
    let generated = compile(&source, Options::default())
        .unwrap()
        .to_token_stream()
        .to_string();
    assert!(generated.contains("(\"b\" , \"For\")"));
    assert!(generated.contains("into_columns :: < (i64 , i64 ,) , :: cq_gpu_runtime :: Plain >"));
    assert_eq!(generated.matches("__execution . dedup").count(), 1);
}

#[test]
fn unknown_codec_aliases_and_conflicting_materializations_are_rejected() {
    for marker in [
        quote!(For),
        quote!(::user::For),
        quote!(::cq_gpu_runtime::Unknown),
    ] {
        let mut source = shared();
        source.structures.entries[4] =
            syn::parse_quote!(p canon = ::cq_gpu_runtime::DeviceSet<_,#marker>;);
        let error = compile(&source, Options::default())
            .unwrap_err()
            .to_string();
        assert!(error.contains("CQ_GPU_CODEC_TYPE"), "{error}");
    }
    let mut source = shared();
    source.physical.program.definitions[1].canon = Some(Default::default());
    source.structures.entries.push(syn::parse_quote! {
        b canon = ::cq_gpu_runtime::DeviceSet<_,::cq_gpu_runtime::For>;
    });
    let error = compile(&source, Options::default())
        .unwrap_err()
        .to_string();
    assert!(error.contains("CQ_GPU_CODEC_CONFLICT"), "{error}");
}

#[test]
fn capacity_is_explicit_and_names_a_declared_input() {
    let options: Options = syn::parse2(quote!(capacity(R), frontier(0))).unwrap();
    assert_eq!(options.capacity.as_ref().unwrap(), "R");
    let generated = compile(&shared(), options)
        .unwrap()
        .to_token_stream()
        .to_string();
    assert!(generated.contains("__cq_gpu_host_R"));
    assert!(generated.contains("__CQ_GPU_CAPACITY"));
    assert!(generated.contains("execute_bounded"));
    assert!(!generated.contains("execute_resumable"));
    let options: Options = syn::parse2(quote!(capacity(invented))).unwrap();
    let error = compile(&shared(), options).unwrap_err().to_string();
    assert!(error.contains("CQ_GPU_CAPACITY"), "{error}");
    let retry: Options = syn::parse2(quote!(output(retry), capacity(invented))).unwrap();
    let error = compile(&shared(), retry).unwrap_err().to_string();
    assert!(error.contains("CQ_GPU_CAPACITY"), "{error}");
    assert!(syn::parse2::<Options>(quote!(capacity(L), capacity(R))).is_err());
    assert!(syn::parse2::<Options>(quote!(completion(invented))).is_err());
}

#[test]
fn completion_boundary_is_explicit_and_query_default_is_unchanged() {
    let query = compile(&shared(), Options::default())
        .unwrap()
        .to_token_stream()
        .to_string();
    assert!(query.contains(". finish"));
    let round = compile(&shared(), syn::parse2(quote!(completion(round))).unwrap())
        .unwrap()
        .to_token_stream()
        .to_string();
    assert!(!round.contains(". finish"));
    assert!(round.contains("__CQ_GPU_COMPLETION"));
    assert!(round.contains("\"round\""));
}

#[test]
fn kernels_read_resident_logical_count_not_allocation_capacity() {
    let generated = compile(&shared(), syn::parse2(quote!(capacity(R))).unwrap())
        .unwrap()
        .to_token_stream()
        .to_string();
    assert!(generated.contains("table_len (__driver)"));
    assert!(!generated.contains("__row < __driver . len"));
    assert!(generated.contains("__driver . len . div_ceil"));
}

#[test]
fn unbounded_expansion_uses_saved_device_execution_state() {
    let generated = compile(&shared(), Options::default())
        .unwrap()
        .to_token_stream()
        .to_string();
    assert!(generated.contains("execute_resumable"));
    assert!(generated.contains("__resume_pc"));
    assert!(generated.contains("__resume_registers"));
    assert!(!generated.contains("execute_bounded"));
    assert!(!generated.contains("download"));
}

#[test]
fn resumable_expansion_preserves_recorded_batch_and_frontier() {
    let mut source = shared();
    source.structures.entries.push(syn::parse_quote! {
        p stage = batch 32 expand;
    });
    let generated = compile(&source, Options::default())
        .unwrap()
        .to_token_stream()
        .to_string();
    assert!(generated.contains("execute_resumable"));
    assert!(generated.contains("frontier_claim"));
    assert!(generated.contains("32usize"));
    assert!(generated.contains("p:b[x]:witness:s"));
}

#[test]
fn unstaged_probe_after_expansion_uses_a_uniform_lane_cursor_without_another_frame() {
    let source: ir::Program = syn::parse_quote! {
        physical {
            relation L(a:i64); relation R(a:i64,b:i64); relation S(b:i64,c:i64);
            l = pipe rename L@0 {a -> x};
            r = let rename R@1 {a -> x,b -> y};
            s = let rename S@2 {b -> y,c -> z};
            p = pipe probe first [x] l,r;
            q = pipe canon probe first [y] p,s;
            return q as answer(x,y,z);
        }
        structures {
            l host scan = ::core::iter::Map<_,_> [];
            r host scan = ::core::iter::Map<_,_> [];
            s host scan = ::core::iter::Map<_,_> [];
            r = ::cq_storage::SortedRun<_,_> [x];
            s = ::cq_storage::SortedRun<_,_> [y];
            q canon = ::cq_gpu_runtime::DeviceSet<_>;
            p stage = batch 256 expand;
        }
    };
    for options in [quote!(), quote!(output(retry))] {
        let generated = compile(&source, syn::parse2(options).unwrap())
            .unwrap()
            .to_token_stream()
            .to_string();
        assert!(generated.contains("__remaining"));
        assert!(generated.contains("frontier_claim"));
        assert!(generated.contains("12368u32"), "{generated}");
        assert!(!generated.contains("24672u32"), "{generated}");
    }
}

#[test]
fn fixed_full_tile_scan_keeps_only_two_decoded_registers_live_per_column() {
    let generated = compile(&shared(), syn::parse2(quote!(capacity(R))).unwrap())
        .unwrap()
        .to_token_stream()
        .to_string();
    assert_eq!(generated.matches("decode_tile_pair (").count(), 1);
    assert!(generated.contains("launch_bounds (256 , 4u32)"));
    assert!(generated.contains("__cq_gpu_root_column_0 : :: cq_gpu_runtime :: ColumnView"));
    assert!(generated.contains("column_view (__cq_gpu_host_L , 0usize)"));
    assert!(generated.contains("decode_tile_pair (__cq_gpu_root_column_0"));
    assert!(generated.contains("let [__cq_gpu_tile_register_0_0"));
    assert!(generated.contains("let __cq_gpu_tile_column_0 = if __stripe % 512 == 0"));
    assert!(generated.contains("while __pair < 2"));
    assert!(generated.contains("while __stripe < (__pair + 1) * 512"));
    assert!(!generated.contains("__cq_gpu_tile_register_0_2"));
    assert!(!generated.contains("[__stripe / 256]"));
    assert!(!generated.contains("sync_threads"));
    assert!(
        generated.find("decode_tile_pair (").unwrap() < generated.find("let mut __stripe").unwrap()
    );
    assert!(!generated.contains("column (__driver"));
    assert!(generated.contains("let __driver_len = :: cq_gpu_runtime :: table_len (__driver)"));
    assert!(generated.contains("while __tile < __driver_len . div_ceil"));
    assert!(generated.contains("__row < __driver_len"));
    let paired_slot = quote!(
        let __slot = (__tid / 32) * 128 + (__tid % 32) * 2
            + (__stripe / 512) * 64 + (__stripe / 256) % 2;
    )
    .to_string();
    assert!(generated.contains(&paired_slot));

    let mut source = shared();
    source.structures.entries.push(syn::parse_quote! {
        p stage = batch 32 expand;
    });
    let partial = compile(&source, syn::parse2(quote!(capacity(R))).unwrap())
        .unwrap()
        .to_token_stream()
        .to_string();
    assert!(!partial.contains("decode_tile_pair ("));
    assert!(!partial.contains("__cq_gpu_root_column_0"));
    assert!(partial.contains("column (__driver"));
    assert!(!partial.contains(&paired_slot));
}

#[test]
fn selection_decodes_one_convergent_stripe_before_row_predicates() {
    let source: ir::Program = syn::parse_quote! { physical {
        relation Arbitrary(k:i64,v:i64);
        scan = pipe rename Arbitrary@0 {k -> key,v -> value};
        selected = pipe select scan if (key > 0);
        output = pipe canon project selected keep {key,value};
        return output as result(key,value);
    } structures {
        scan host scan = ::core::iter::Map<_,_> [];
        output canon = ::cq_gpu_runtime::DeviceSet<_>;
    } };
    let generated = compile(&source, Options::default())
        .unwrap()
        .to_token_stream()
        .to_string();
    assert!(generated.contains("execute_selection"));
    assert_eq!(generated.matches("decode_stripe (").count(), 2);
    assert!(generated.contains("column_view (__cq_gpu_host_Arbitrary , 0usize)"));
    assert!(generated.contains("column_view (__cq_gpu_host_Arbitrary , 1usize)"));
    assert!(generated.contains("__row / 1024"));
    assert!(generated.contains("(__row % 1024) / 256"));
    assert!(
        generated.find("decode_stripe (").unwrap() < generated.find("let mut __valid").unwrap()
    );
    assert!(!generated.contains("decode_tile_pair ("));
    assert!(!generated.contains("column (__driver"));
    assert!(!generated.contains("execute_resumable"));
    for options in [
        quote!(output(retry)),
        quote!(output(retry), capacity(Arbitrary)),
    ] {
        let retry = compile(&source, syn::parse2(options).unwrap())
            .unwrap()
            .to_token_stream()
            .to_string();
        assert_retry_kernel(&retry);
        assert!(!retry.contains("execute_selection"), "{retry}");
        assert!(!retry.contains("__scatter"), "{retry}");
        assert!(
            retry.find("execute_retry").unwrap() < retry.find("__execution . dedup").unwrap(),
            "{retry}"
        );
    }
}

#[test]
fn decoded_root_descriptors_come_from_the_materialized_table() {
    let source: ir::Program = syn::parse_quote! { physical {
        relation Input(k:i64);
        stored = let rename Input@0 {k -> x};
        filtered = pipe select stored if (x > 0);
        output = pipe canon project filtered keep {x};
        return output as result(x);
    } structures {
        stored host scan = ::core::iter::Map<_,_> [];
        stored rows = ::std::vec::Vec<_>;
        output canon = ::cq_gpu_runtime::DeviceSet<_>;
    } };
    let generated = compile(&source, syn::parse2(quote!(capacity(Input))).unwrap())
        .unwrap()
        .to_token_stream()
        .to_string();
    assert!(generated.contains("decode_tile_pair (__cq_gpu_root_column_0"));
    assert!(generated.contains("column_view (__cq_gpu_table_stored , 0usize)"));
    assert!(!generated.contains("column_view (__cq_gpu_host_Input"));
}

#[test]
fn generated_module_cache_is_thread_local_and_context_owned() {
    let generated = compile(&shared(), syn::parse2(quote!(capacity(R))).unwrap())
        .unwrap()
        .to_token_stream()
        .to_string();
    assert!(generated.contains(":: std :: thread_local !"));
    assert!(generated.contains(":: std :: sync :: Arc < :: cuda_core :: CudaContext >"));
    assert!(generated.contains(":: std :: rc :: Rc <"));
    assert!(generated.contains("Arc :: ptr_eq (__context , __runtime . context ())"));
    assert_eq!(
        generated
            .matches(":: load (__runtime . context ())")
            .count(),
        1
    );
    assert!(generated.contains("__cache . borrow_mut ()"));
}

#[test]
fn range_reads_are_truthfully_scan_filters() {
    let source: ir::Program = syn::parse_quote! { physical {
        relation L(a:i64);
        l = pipe rename L@0 {a -> x};
        r = pipe range select l if (x >= -7);
        p = pipe canon project r keep {x};
        return p as answer(x);
    } structures {
        l host scan = ::core::iter::Map<_,_> [];
        l host r = ::core::iter::Map<_,_> [x];
        p canon = ::cq_gpu_runtime::DeviceSet<_>;
    } };
    let facts = ir::contract::check(&source).unwrap();
    let reads = validate(&source, &facts).unwrap();
    assert_eq!(reads.len(), 1);
    assert_eq!(reads[0].source, "scan-filter");
}

#[test]
fn recorded_stage_sizes_are_not_silently_changed() {
    for size in [257usize, 1024, 2048] {
        let mut source = shared();
        source
            .structures
            .entries
            .push(syn::parse2(quote!(p stage = batch #size expand;)).unwrap());
        assert!(
            compile(&source, Options::default())
                .unwrap_err()
                .to_string()
                .contains("CQ_GPU_STAGE_SIZE")
        );
    }
}

#[test]
fn typed_literals_follow_declared_types_in_relational_predicates() {
    let vars = BTreeMap::from([("x".to_owned(), quote!(x)), ("y".to_owned(), quote!(y))]);
    let types = BTreeMap::from([
        ("x".to_owned(), syn::parse_quote!(i64)),
        ("y".to_owned(), syn::parse_quote!(u32)),
    ]);
    for theta in [
        syn::parse_quote!(x >= -3000000000, y >= 4000000000),
        syn::parse_quote!(x = 3000000000),
    ] {
        let mut constants = Constants::default();
        constants.theta(&theta, &vars, &types).unwrap();
        let values = constants
            .values
            .iter()
            .map(|(_, value)| value.to_string())
            .collect::<Vec<_>>()
            .join(" ");
        assert!(values.contains("let __cq_gpu_literal : i64"));
        if values.contains("4000000000") {
            assert!(values.contains("let __cq_gpu_literal : u32"));
        }
    }
}

#[test]
fn output_bounds_follow_driver_and_run_payload_without_full_domain_fallback() {
    for options in [
        Options::default(),
        syn::parse2(quote!(capacity(R))).unwrap(),
    ] {
        let generated = compile(&shared(), options)
            .unwrap()
            .to_token_stream()
            .to_string();
        assert!(generated.contains("column_bounds (__cq_gpu_host_L , 0usize)"));
        assert!(generated.contains("column_bounds (__cq_gpu_table_b , 1usize)"));
        assert!(!generated.contains("i64 :: MIN"));
        assert!(!generated.contains("i64 :: MAX"));
        assert!(generated.contains("__cq_gpu_check_schema :: < (i64 , i64 ,) , _ >"));
    }
}

#[test]
fn integer_literal_encoding_compiles_large_values_but_rejects_declared_type_overflow() {
    let directory = std::env::temp_dir().join(format!("cq-gpu-literals-{}", std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
    for (literal, ty, valid) in [
        (quote!(3000000000), quote!(i64), true),
        (quote!(-9223372036854775808), quote!(i64), true),
        (quote!(4000000000), quote!(u32), true),
        (quote!(256), quote!(u8), false),
        (quote!(-1), quote!(u32), false),
    ] {
        let mut constants = Constants::default();
        constants
            .literal(
                &syn::parse2(literal).unwrap(),
                Some(&syn::parse2(ty).unwrap()),
            )
            .unwrap();
        let value = &constants.values[0].1;
        let source = directory.join("literal.rs");
        std::fs::write(&source, quote!(fn main() { let _ = #value; }).to_string()).unwrap();
        let output = std::process::Command::new("rustc")
            .arg("--emit=metadata")
            .arg("--out-dir")
            .arg(&directory)
            .arg(&source)
            .output()
            .unwrap();
        assert_eq!(
            output.status.success(),
            valid,
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        if !valid {
            let diagnostic = String::from_utf8_lossy(&output.stderr);
            assert!(
                diagnostic.contains("out of range")
                    || diagnostic.contains("cannot apply unary operator")
            );
        }
    }
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn copied_and_literal_binds_preserve_types_and_bounds() {
    let source: ir::Program = syn::parse_quote! { physical {
        relation R(a:i32);
        scan = pipe rename R@0 {a -> x};
        copied = pipe bind scan with y:i32 = (*x);
        literal = pipe bind copied with z:u8 = (7);
        output = pipe canon project literal keep {y,z};
        return output as answer(y,z,y);
    } structures {
        scan host scan = ::core::iter::Map<_,_> [];
        output canon = ::cq_gpu_runtime::DeviceSet<_>;
    }};
    for options in [
        Options::default(),
        syn::parse2(quote!(capacity(R))).unwrap(),
    ] {
        let generated = compile(&source, options)
            .unwrap()
            .to_token_stream()
            .to_string();
        assert!(generated.contains("let __cq_gpu_literal : u8 = 7"));
        assert!(generated.contains("Bounds :: singleton"));
        assert!(generated.contains("into_columns :: < (i32 , u8 , i32 ,)"));
        assert!(generated.contains("__execution . dedup"));
        assert!(!generated.contains("opaque"));
    }
}

#[test]
fn closed_typed_scalar_binds_are_encoded_once_on_host() {
    for (expression, ty) in [
        (quote!(meta.x), quote!(i32)),
        (quote!(meta.enabled), quote!(bool)),
        (quote!(external()), quote!(u64)),
        (quote!(*external), quote!(i32)),
    ] {
        let source: ir::Program = syn::parse_quote! { physical {
            relation R(a:i32);
            scan = pipe rename R@0 {a -> x};
            output = pipe canon bind scan with y:#ty = (#expression);
            return output as answer(x,y);
        } structures {
            scan host scan = ::core::iter::Map<_,_> [];
            output canon = ::cq_gpu_runtime::DeviceSet<_>;
        }};
        for options in [
            Options::default(),
            syn::parse2(quote!(capacity(R))).unwrap(),
        ] {
            let generated = compile(&source, options)
                .unwrap()
                .to_token_stream()
                .to_string();
            let capture = quote!(let __cq_gpu_literal: #ty = #expression).to_string();
            assert_eq!(generated.matches(&capture).count(), 1, "{generated}");
            assert!(generated.contains("encode_value"));
            assert!(generated.contains("Bounds :: singleton"));
        }
    }
}

#[test]
fn closed_capture_respects_written_bind_scope() {
    let source: ir::Program = syn::parse_quote! { physical {
        relation R(a:i32);
        scan = pipe rename R@0 {a -> x};
        output = pipe canon bind scan with y:i32 = (*x) scope [];
        return output as answer(x,y);
    } structures {
        scan host scan = ::core::iter::Map<_,_> [];
        output canon = ::cq_gpu_runtime::DeviceSet<_>;
    }};
    let generated = compile(&source, Options::default())
        .unwrap()
        .to_token_stream()
        .to_string();
    assert_eq!(
        generated
            .matches("let __cq_gpu_literal : i32 = * x")
            .count(),
        1
    );
    assert!(generated.contains("Bounds :: singleton"));
}

#[test]
fn typed_bind_rejects_opaque_numeric_and_type_changing_expressions() {
    for (expression, ty, code) in [
        (quote!(*x + 1), quote!(i32), "CQ_GPU_OPAQUE_BIND"),
        (quote!(x.clone()), quote!(i32), "CQ_GPU_OPAQUE_BIND"),
        (quote!(*x), quote!(i64), "CQ_GPU_BIND_TYPE"),
        (quote!(meta.x + *x), quote!(i32), "CQ_GPU_OPAQUE_BIND"),
        (quote!(opaque!(x)), quote!(i32), "CQ_GPU_OPAQUE_BIND"),
        // Physical rejects floating-point relational columns before GPU lowering.
        (quote!(meta.x), quote!(f64), "requires Eq + Hash or Ord"),
        (quote!(meta.name), quote!(&str), "CQ_GPU_OPAQUE_BIND"),
    ] {
        let source: ir::Program = syn::parse_quote! { physical {
            relation R(a:i32);
            scan = pipe rename R@0 {a -> x};
            output = pipe canon bind scan with y:#ty = (#expression);
            return output as answer(x,y);
        } structures {
            scan host scan = ::core::iter::Map<_,_> [];
            output canon = ::cq_gpu_runtime::DeviceSet<_>;
        }};
        let error = compile(&source, Options::default())
            .unwrap_err()
            .to_string();
        assert!(error.contains(code), "{error}");
    }
}

#[test]
fn certified_exact_membership_is_device_only_and_schema_checked() {
    let source: ir::Program = syn::parse_quote! { physical {
        relation R(a:i32,b:bool);
        scan = pipe rename R@0 {a -> x,b -> flag};
        selected = pipe select scan if (
            #[total] { x = x, flag = flag } (!::cq_gpu_runtime::datalog::contains(&keys, &(x,flag,)))
        );
        output = pipe canon project selected keep {x,flag};
        return output as answer(x,flag);
    } structures {
        scan host scan = ::core::iter::Map<_,_> [];
        output canon = ::cq_gpu_runtime::DeviceSet<_>;
    }};
    let options = syn::parse2(quote!(datalog_keys(keys: (i32, bool)))).unwrap();
    let mut expression = compile(&source, options).unwrap();
    // The evidence snapshot intentionally retains the source predicate as text;
    // the executable expression must contain only the device intrinsic.
    let syn::Expr::Block(block) = &mut expression else {
        panic!("query block")
    };
    block.block.stmts.retain(|statement| {
        !matches!(statement,
        syn::Stmt::Item(syn::Item::Const(item)) if item.ident == "__CQ_GPU_RECORDED_PHYSICAL")
    });
    let generated = expression.to_token_stream().to_string();
    assert!(!generated.contains("prepare_exact_membership"));
    assert!(
        generated.contains("resident_membership (2usize)"),
        "{generated}"
    );
    assert!(generated.contains("bind_host (& keys"), "{generated}");
    assert!(generated.contains("ExactMembershipView"));
    assert!(generated.contains("exact_contains"));
    assert!(!generated.contains("datalog :: contains"));
    assert!(!generated.contains("download"));
    let error = compile(&source, Options::default())
        .unwrap_err()
        .to_string();
    assert!(error.contains("bridge-certified"), "{error}");
    let options = syn::parse2(quote!(datalog_keys(keys: (i64, bool)))).unwrap();
    let error = compile(&source, options).unwrap_err().to_string();
    assert!(error.contains("CQ_GPU_DATALOG_KEY_TYPE"), "{error}");
}

#[test]
fn nullary_frozen_membership_borrows_an_existence_view_without_building() {
    let source: ir::Program = syn::parse_quote! { physical {
        relation R(a:i32);
        scan = pipe rename R@0 {a -> x};
        selected = pipe select scan if (
            #[total] { x = x } (!::cq_gpu_runtime::datalog::contains(&keys, &()))
        );
        output = pipe canon project selected keep {x};
        return output as answer(x);
    } structures {
        scan host scan = ::core::iter::Map<_,_> [];
        output canon = ::cq_gpu_runtime::DeviceSet<_>;
    }};
    let generated = compile(
        &source,
        syn::parse2(quote!(datalog_keys(keys: ()))).unwrap(),
    )
    .unwrap()
    .to_token_stream()
    .to_string();
    assert!(
        generated.contains("resident_membership (0usize)"),
        "{generated}"
    );
    assert!(generated.contains("Row = (u8 ,)"), "{generated}");
    assert!(
        !generated.contains("prepare_exact_membership"),
        "{generated}"
    );
    assert!(generated.contains("exact_contains"), "{generated}");
}

#[test]
fn lossy_and_repeated_heads_canon_after_projection() {
    let mut source: ir::Program = syn::parse_quote! { physical {
        relation R(a:i32);
        scan = pipe canon rename R@0 {a -> x};
        return scan as answer(*x,*x);
    } structures {
        scan host scan = ::core::iter::Map<_,_> [];
        scan canon = ::cq_gpu_runtime::DeviceSet<_>;
        scan head = ::cq_gpu_runtime::DeviceSet<_,::cq_gpu_runtime::Delta>;
    }};
    let generated = compile(&source, Options::default())
        .unwrap()
        .to_token_stream()
        .to_string();
    assert!(generated.contains("project_table (__cq_gpu_table_scan , & [0usize , 0usize])"));
    assert_eq!(generated.matches("__execution . dedup").count(), 2);
    assert!(generated.contains("into_columns :: < (i32 , i32 ,) , :: cq_gpu_runtime :: Delta >"));
    let ir::Structure::HeadDedup { ty, .. } = &mut source.structures.entries[2].structure else {
        unreachable!()
    };
    *ty = syn::parse_quote!(::cq_gpu_runtime::DeviceSet<_,::cq_gpu_runtime::Delta,::cq_gpu_runtime::Fused>);
    let generated = compile(&source, Options::default())
        .unwrap()
        .to_token_stream()
        .to_string();
    assert!(generated.contains("cq_gpu_head_"));
    assert!(generated.contains("execute_resumable_fused"));
    assert_eq!(generated.matches("__execution . dedup").count(), 1);
    assert!(generated.contains("project_table (__cq_gpu_table_scan , & [0usize , 0usize])"));
    for (options, capacity_hint) in [
        (quote!(output(retry)), "None"),
        (
            quote!(output(retry), capacity(R)),
            "Some(__execution.table_view(__cq_gpu_host_R)?.lenasusize)",
        ),
    ] {
        let retry = compile(&source, syn::parse2(options).unwrap())
            .unwrap()
            .to_token_stream()
            .to_string();
        assert_fused_retry_kernel(&retry);
        let compact = retry.split_whitespace().collect::<String>();
        assert!(
            compact.contains(&format!(
                "&__head_bounds,{capacity_hint},|__stream,__output,"
            )),
            "{retry}"
        );
        assert!(retry.contains("cq_gpu_head_"), "{retry}");
        assert!(retry.contains("project_table (__cq_gpu_table_scan , & [0usize , 0usize])"));
        assert!(retry.contains("into_columns :: < (i32 , i32 ,) , :: cq_gpu_runtime :: Delta >"));
        assert_eq!(retry.matches("__execution . dedup").count(), 1);
        assert!(
            retry.find("__execution . dedup").unwrap() < retry.find("execute_retry_fused").unwrap(),
            "only the upstream non-fused canon needs staged dedup: {retry}"
        );
    }
}

#[test]
fn physical_anti_reuses_recorded_run_and_inverts_exact_membership() {
    let source: ir::Program = syn::parse_quote! { physical {
        relation L(a:i64); relation R(a:i64,b:i64);
        l = pipe rename L@0 {a -> x};
        b = let rename R@1 {a -> x,b -> y};
        output = pipe canon anti eager [x] l,b;
        return output as answer(x);
    } structures {
        l host scan = ::core::iter::Map<_,_> [];
        b host scan = ::core::iter::Map<_,_> [];
        b rows = ::cq_gpu_runtime::DeviceColumns<_>;
        b = ::cq_storage::SortedRun<_,_,::cq_storage::BorrowedI64TupleCodec> [x];
        output canon = ::cq_gpu_runtime::DeviceSet<_>;
    }};
    for options in [
        Options::default(),
        syn::parse2(quote!(capacity(L))).unwrap(),
    ] {
        let generated = compile(&source, options)
            .unwrap()
            .to_token_stream()
            .to_string();
        assert!(generated.contains("__found != true"));
        assert_eq!(generated.matches("prepare_run").count(), 1);
        assert!(!generated.contains("prepare_exact_membership"));
        assert!(generated.contains("output:b[x]:rank-point"));
    }
}

#[test]
fn shared_datalog_dictionary_seeds_input_free_literal_rules() {
    let source: ir::Program = syn::parse_quote! { physical {
        root = pipe unit;
        output = pipe canon bind root with x:&str = ("literal");
        return output as answer(x);
    } structures {
        output canon = ::cq_gpu_runtime::DeviceSet<_>;
    }};
    let options = syn::parse2(quote!(datalog_dictionary(shared), datalog_pool(pool))).unwrap();
    let generated = compile(&source, options)
        .unwrap()
        .to_token_stream()
        .to_string();
    let compact: String = generated.chars().filter(|c| !c.is_whitespace()).collect();
    assert!(compact.contains("&::std::sync::Arc<::cq_gpu_runtime::Dictionary<'_>>=&shared"));
    assert!(compact.contains("&::std::sync::Arc<::cq_gpu_runtime::Pool>=&pool"));
    assert!(compact.contains("Some(__cq_gpu_shared_pool.clone())"));
    assert!(
        generated.find("__cq_gpu_shared_pool").unwrap()
            < generated.find("with_runtime_for").unwrap()
    );
    assert!(
        generated.find("dictionary_mut () . merge").unwrap()
            < generated.find("encode_value").unwrap()
    );
    let generated = compile(
        &shared(),
        syn::parse2(quote!(datalog_dictionary(dictionary))).unwrap(),
    )
    .unwrap()
    .to_token_stream()
    .to_string();
    assert!(
        generated.find("dictionary_mut () . merge").unwrap()
            < generated.find("HostColumns :: seed_dictionary").unwrap()
    );
    assert!(syn::parse2::<Options>(quote!(datalog_dictionary(shared, extra))).is_err());
    assert!(
        syn::parse2::<Options>(quote!(
            datalog_dictionary(shared),
            datalog_dictionary(other)
        ))
        .is_err()
    );
    assert!(syn::parse2::<Options>(quote!(datalog_dictionary(make_dictionary()))).is_err());
    assert!(syn::parse2::<Options>(quote!(datalog_pool(pool), datalog_pool(other))).is_err());
    assert!(syn::parse2::<Options>(quote!(datalog_pool(make_pool()))).is_err());
}
