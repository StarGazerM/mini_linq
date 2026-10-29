#[allow(dead_code)]
#[path = "support/h05_program.rs"]
mod h05_program;

use cq_lang_sequence::{Capability, Len, OutputProtocol, Term, Tiling};
use quote::ToTokens;
use syn::visit::{self, Visit};

fn checked() -> (cq_lang_sequence::Program, cq_lang_sequence::Analysis) {
    let program = h05_program::program();
    let program = cq_gpu_sequence_core::runs::record_representation(&program).unwrap();
    let analysis = cq_lang_sequence::check_grading(&program, &cq_gpu_sequence::machine())
        .expect("the written H05 program must be graded without planner decisions");
    (program, analysis)
}

#[test]
fn repeated_translations_have_distinct_cuda_exports() {
    fn export_names(expression: syn::Expr) -> (syn::Ident, Vec<String>) {
        let syn::Expr::Block(block) = &expression else {
            panic!("translation must emit a scoped block")
        };
        let Some(syn::Stmt::Item(syn::Item::Mod(module))) = block.block.stmts.first() else {
            panic!("translation must declare its CUDA module")
        };
        let mut functions = Functions::default();
        functions.visit_expr(&expression);
        let kernels = functions
            .queries
            .iter()
            .map(|function| function.sig.ident.to_string())
            .collect();
        (module.ident.clone(), kernels)
    }
    let (program, analysis) = checked();
    let first = export_names(cq_gpu_sequence::translate(&program, &analysis).unwrap());
    let second = export_names(cq_gpu_sequence::translate(&program, &analysis).unwrap());
    assert_ne!(first.0, second.0, "query expansions need distinct modules");
    assert!(
        first.1.iter().all(|name| !second.1.contains(name)),
        "CUDA kernel exports must not collide across query expansions"
    );
}

#[test]
fn later_dispatch_omits_unreferenced_earlier_store_parameters() {
    let mut program = h05_program::program();
    program.graded.as_mut().unwrap().grades[0] = syn::parse_quote!(RegionASIA at device;);
    let analysis = cq_lang_sequence::check_grading(&program, &cq_gpu_sequence::machine()).unwrap();
    let expression = cq_gpu_sequence::translate(&program, &analysis).unwrap();
    let mut functions = Functions::default();
    functions.visit_expr(&expression);
    assert_eq!(functions.queries.len(), 3);
    let later = functions.queries[2];
    for name in [
        "__cq_store_rows_0",
        "__cq_store_begin_0",
        "__cq_store_offsets_0",
    ] {
        assert!(!later.block.to_token_stream().to_string().contains(name));
        assert!(
            !later
                .sig
                .inputs
                .to_token_stream()
                .to_string()
                .contains(name)
        );
    }
}

#[test]
fn every_generated_kernel_fits_the_parameter_area() {
    struct ParameterBytes;
    impl<'ast> Visit<'ast> for ParameterBytes {
        fn visit_item_fn(&mut self, function: &'ast syn::ItemFn) {
            if function
                .attrs
                .iter()
                .any(|attribute| attribute.path().is_ident("kernel"))
            {
                let mut bytes = 0usize;
                for input in &function.sig.inputs {
                    let syn::FnArg::Typed(input) = input else {
                        panic!("kernel receiver")
                    };
                    let (size, align) = match &*input.ty {
                        syn::Type::Ptr(_) => (8, 8),
                        syn::Type::Path(path) => match path
                            .path
                            .segments
                            .last()
                            .unwrap()
                            .ident
                            .to_string()
                            .as_str()
                        {
                            "bool" => (1, 1),
                            "u32" => (4, 4),
                            "usize" | "i64" => (8, 8),
                            "ColumnView" => (40, 8),
                            "RangeView" => (64, 8),
                            other => panic!("unknown emitted ABI type {other}"),
                        },
                        other => panic!("unknown emitted ABI type {}", other.to_token_stream()),
                    };
                    bytes = bytes.next_multiple_of(align) + size;
                }
                assert!(
                    bytes < 4096,
                    "{} has {bytes} parameter bytes",
                    function.sig.ident
                );
            }
            visit::visit_item_fn(self, function);
        }
    }
    let (program, analysis) = checked();
    ParameterBytes.visit_expr(&cq_gpu_sequence::translate(&program, &analysis).unwrap());
}

#[test]
fn physical_and_sequence_contracts_preserve_the_written_join_chain() {
    let program = h05_program::program();
    let analysis = cq_lang_sequence::contract::check(&program).unwrap();
    assert!(program.structures.physical.program.host_sets.is_empty());
    assert_eq!(analysis.results["C"], "rnc");
    assert_eq!(analysis.results["result"], "answer");
    assert_eq!(
        analysis.output,
        Some(("result".into(), OutputProtocol::Bounded))
    );
    assert!(matches!(analysis.lengths["C"], Len::Scan(_)));
    assert!(matches!(analysis.lengths["result"], Len::Scan(_)));
    assert_eq!(analysis.shapes["result"].len(), 1);
    for name in [
        "RegionASIA",
        "Nation",
        "Customer",
        "Orders",
        "Lineitem",
        "Supplier",
        "C",
        "result",
    ] {
        assert!(
            analysis.stores.contains_key(name),
            "missing explicit store {name}"
        );
    }
    // Regression: the original formal C accidentally stopped after Nation.
    let c = &program
        .sequence
        .definitions
        .iter()
        .find(|d| d.name == "C")
        .unwrap()
        .term;
    let Term::Store { inner, .. } = c else {
        panic!("C must publish storage")
    };
    let Term::Flatten(inner) = inner.as_ref() else {
        panic!("C must store flatten offsets")
    };
    let Term::Lookup {
        provider,
        keys,
        inner,
    } = inner.as_ref()
    else {
        panic!("C must enumerate Customer")
    };
    assert_eq!(provider, "customer_index");
    assert_eq!(
        keys.iter().map(ToString::to_string).collect::<Vec<_>>(),
        ["nation"]
    );
    let Term::Flatten(inner) = inner.as_ref() else {
        panic!("Customer must follow flattened Nation")
    };
    let Term::Lookup { provider, .. } = inner.as_ref() else {
        panic!("Nation lookup missing")
    };
    assert_eq!(provider, "nation_index");
}

#[test]
fn grading_records_two_queries_ranges_membership_and_bounded_publication() {
    let (_, analysis) = checked();
    assert_eq!(analysis.dispatches.len(), 2);
    assert_eq!(analysis.dispatches[0].outer_store, "RegionASIA");
    assert_eq!(analysis.dispatches[1].outer_store, "C");
    assert_eq!(analysis.dispatches[0].inner_stores, ["C"]);
    assert_eq!(analysis.dispatches[1].inner_stores, ["result"]);
    for dispatch in &analysis.dispatches {
        assert_eq!(dispatch.level, ["host", "device"]);
        assert_eq!(dispatch.producer_level, ["host", "device", "block"]);
        assert_eq!(
            dispatch.failure_channel.last().map(String::as_str),
            Some("host")
        );
    }
    assert_eq!(analysis.dispatches[1].parts, Some(256));
    assert_eq!(
        analysis.dispatches[1].concatenation,
        OutputProtocol::Bounded
    );
    assert_eq!(
        analysis.tilings["result"],
        [Tiling::Strip, Tiling::Strip, Tiling::Distribute]
    );
    for (provider, key) in [
        ("nation_index", "region"),
        ("customer_index", "nation"),
        ("orders_index", "customer"),
        ("lineitem_index", "order"),
    ] {
        assert_eq!(
            analysis.capabilities[&(provider.into(), vec![key.into()])],
            Capability::Range
        );
    }
    assert_eq!(
        analysis.capabilities[&(
            "supplier_index".into(),
            vec!["nation".into(), "supplier".into()]
        )],
        Capability::Membership
    );
}

#[derive(Default)]
struct Functions<'ast> {
    queries: Vec<&'ast syn::ItemFn>,
}
impl<'ast> Visit<'ast> for Functions<'ast> {
    fn visit_item_fn(&mut self, function: &'ast syn::ItemFn) {
        if function
            .sig
            .ident
            .to_string()
            .starts_with("__cq_sequence_dispatch_")
        {
            self.queries.push(function);
        }
        visit::visit_item_fn(self, function);
    }
}

struct NoQueryConstants;
impl<'ast> Visit<'ast> for NoQueryConstants {
    fn visit_expr_binary(&mut self, expression: &'ast syn::ExprBinary) {
        if matches!(
            expression.op,
            syn::BinOp::Eq(_)
                | syn::BinOp::Ne(_)
                | syn::BinOp::Lt(_)
                | syn::BinOp::Le(_)
                | syn::BinOp::Gt(_)
                | syn::BinOp::Ge(_)
        ) && ![&*expression.left, &*expression.right]
            .iter()
            .any(|operand| {
                // Generated runtime parameters describe provider plumbing, not
                // query predicates (for example key lanes and packing shifts).
                matches!(operand, syn::Expr::Path(path)
                    if path.path.get_ident().is_some_and(|ident| ident.to_string().starts_with("__cq_")))
            })
        {
            for operand in [&*expression.left, &*expression.right] {
                if let syn::Expr::Lit(syn::ExprLit {
                    lit: syn::Lit::Int(literal),
                    ..
                }) = operand
                {
                    assert!(
                        !["2", "8766", "9131", "8000", "8200"].contains(&literal.base10_digits()),
                        "query predicate leaked into expansion kernel: {}",
                        expression.to_token_stream()
                    );
                }
            }
        }
        visit::visit_expr_binary(self, expression);
    }
}

fn check_generated(program: &cq_lang_sequence::Program) {
    let program = cq_gpu_sequence_core::runs::record_representation(program).unwrap();
    let analysis = cq_lang_sequence::check_grading(&program, &cq_gpu_sequence::machine()).unwrap();
    let expression = cq_gpu_sequence::translate(&program, &analysis).unwrap();
    let mut functions = Functions::default();
    functions.visit_expr(&expression);
    assert_eq!(
        functions.queries.len(),
        2,
        "one query kernel per recorded dispatch"
    );
    for (index, function) in functions.queries.iter().enumerate() {
        assert!(
            function
                .sig
                .ident
                .to_string()
                .starts_with(&format!("__cq_sequence_dispatch_{index}_")),
            "dispatch names must retain their index and an expansion-specific suffix"
        );
        NoQueryConstants.visit_block(&function.block);
        let attributes = function
            .attrs
            .iter()
            .map(|a| a.to_token_stream().to_string())
            .collect::<Vec<_>>()
            .join(" ");
        assert!(attributes.contains("kernel"));
        assert!(
            attributes.contains("256"),
            "query kernels must record 256 lanes"
        );
    }
    let tokens = expression.to_token_stream().to_string();
    assert!(
        tokens.contains("RangeBuilder") || tokens.contains("RangeProvider"),
        "providers must be built before query execution"
    );
    assert!(
        tokens.contains("PackedCanon"),
        "canonicalization must use the runtime packed-key protocol"
    );
    assert!(!tokens.contains("cq_gpu :: print"));
    assert!(!tokens.contains("h05_reference"));
}

#[test]
fn translation_emits_two_query_kernels_and_builds_without_query_constant_filters() {
    for capacity in [1024, 1048576] {
        let program = h05_program::program_with_capacity(capacity);
        assert!(matches!(
            &program.sequence.definitions.last().unwrap().term,
            Term::Store { capacity: Some(recorded), .. } if *recorded == capacity
        ));
        check_generated(&program);
    }
}

#[test]
fn translation_is_not_specialized_to_h05_date_literals_or_store_names() {
    let source = h05_program::program()
        .to_token_stream()
        .to_string()
        .replace("8766", "8000")
        .replace("9131", "8200")
        .replace("RegionASIA", "region_driver")
        .replace(" C ", " customer_positions ");
    let program = syn::parse_str(&source).unwrap();
    check_generated(&program);
}

#[test]
fn expansion_uses_live_code_columns_and_one_reservation_per_item_batch() {
    let (program, analysis) = checked();
    let expression = cq_gpu_sequence::translate(&program, &analysis).unwrap();
    let mut functions = Functions::default();
    functions.visit_expr(&expression);
    let query = functions.queries[1].to_token_stream().to_string();
    assert!(!query.contains("__row"));
    assert!(!query.contains("__stored"));
    assert!(query.contains("device :: decode ("));
    assert!(!query.contains("decode_u32"), "parent carriers use the recorded i64 ABI");
    assert!(query.contains("__parent_"));
    assert!(query.contains("__found_count"));

    #[derive(Default)]
    struct NestedItems(bool);
    impl<'ast> Visit<'ast> for NestedItems {
        fn visit_expr_while(&mut self, loop_: &'ast syn::ExprWhile) {
            self.0 |= loop_
                .cond
                .to_token_stream()
                .to_string()
                .starts_with("__item <");
            visit::visit_expr_while(self, loop_);
        }
    }
    #[derive(Default)]
    struct Batches(usize);
    impl<'ast> Visit<'ast> for Batches {
        fn visit_expr_while(&mut self, loop_: &'ast syn::ExprWhile) {
            let condition = loop_.cond.to_token_stream().to_string();
            let mut nested = NestedItems::default();
            nested.visit_block(&loop_.body);
            if condition == "__item < 4usize" && !nested.0 {
                self.0 += 1;
                let body = loop_.body.to_token_stream().to_string();
                assert!(!body.contains("checked_scan"), "scan inside per-item loop");
                assert!(
                    !body.contains("fetch_add"),
                    "reservation inside per-item loop"
                );
            }
            visit::visit_expr_while(self, loop_);
        }
    }
    let mut batches = Batches::default();
    batches.visit_block(&functions.queries[1].block);
    assert_eq!(batches.0, 2, "one gather loop and one output loop");
}

#[test]
fn semijoin_device_store_builds_provider_after_publication() {
    let program: cq_lang_sequence::Program = syn::parse_quote! {
        physical {
            relation Region(region:u32) = (&data.region);
            relation Nation(nation:u32,region:u32) = (&data.nation);
            relation Customer(customer:u32,nation:u32) = (&data.customer);
            r1 = let rename Region@0 {region -> region};
            nation_base = pipe rename Nation@1 {nation -> nation,region -> region};
            r2 = let semi eager [region] nation_base,r1;
            customer = pipe rename Customer@2 {customer -> customer,nation -> nation};
            answer = pipe canon semi eager [nation] customer,r2;
            return answer as answer(customer,nation);
        }
        structures {
            r1 host scan = ::cq_gpu_runtime::DeviceColumns<'static,(u32,)> [];
            nation_base host scan = ::cq_gpu_runtime::DeviceColumns<'static,(u32,u32)> [];
            customer host scan = ::cq_gpu_runtime::DeviceColumns<'static,(u32,u32)> [];
            r1 = ::cq_gpu_sequence_runtime::RangeProvider [region];
            r2 = ::cq_gpu_sequence_runtime::RangeProvider [nation];
            answer canon = ::cq_gpu_sequence_runtime::canon::PackedCanon;
        }
        sequence {
            R1 = store rows r1;
            N = store rows nation_base;
            R2 = store compact r2 lookup r1 [region] flatten tile 256 N;
            C = store rows customer;
            result = store canon compact answer lookup r2 [nation] flatten tile 256 C;
            output count;
        }
        graded gpu {
            R1 at host;
            N at host;
            R2 at device;
            C at host;
            result at device;
        }
    };
    // The authored source must satisfy BE-Scope before any scheduling rewrite.
    // A later EXT-SeqStore pass cannot supply the earlier materialization fact.
    let contract = cq_lang_sequence::contract::check(&program)
        .expect("lookup r2 must have an earlier authored Sequence store");
    assert_eq!(contract.results["R2"], "r2");
    assert!(contract.stores.contains_key("R2"));
    let program = cq_gpu_sequence_core::runs::record_representation(&program).unwrap();
    let analysis = cq_lang_sequence::check_grading(&program, &cq_gpu_sequence::machine())
        .expect("the semijoin store and its provider capability are explicitly recorded");
    assert_eq!(analysis.results["R2"], "r2");
    assert!(analysis.stores.contains_key("R2"));
    assert_eq!(
        analysis.capabilities[&("r2".into(), vec!["nation".into()])],
        Capability::Membership
    );
    let generated = cq_gpu_sequence::translate(&program, &analysis)
        .unwrap()
        .to_token_stream()
        .to_string();
    let provider = generated
        .find("let __cq_sequence_provider_r2")
        .expect("the device semijoin must build its recorded provider");
    let publication = generated[..provider]
        .rfind("let (__cq_store_rows_0")
        .expect("device-store publication must precede provider construction");
    assert!(
        generated[publication..].contains("ColumnDescriptor :: from_rows"),
        "the provider must describe published device rows"
    );
    assert!(publication < provider);
}
