//! Source-only regression tests: no gpu_blocked_run import, CUDA initialization,
//! kernel execution, or oracle invocation. Baseline: 5e09a15 Run::build.
use syn::{
    visit::{self, Visit},
    Block, Expr, ImplItem, Item, Stmt,
};

fn source() -> syn::File {
    syn::parse_file(include_str!("../src/lib.rs")).expect("blocked-run source parses")
}

fn run_method(file: &syn::File, name: &str) -> Block {
    file.items
        .iter()
        .find_map(|item| {
            let Item::Impl(item) = item else { return None };
            let syn::Type::Path(ty) = item.self_ty.as_ref() else {
                return None;
            };
            if !ty.path.segments.last().is_some_and(|s| s.ident == "Run") {
                return None;
            }
            item.items.iter().find_map(|item| match item {
                ImplItem::Fn(method) if method.sig.ident == name => Some(method.block.clone()),
                _ => None,
            })
        })
        .unwrap_or_else(|| panic!("missing Run::{name}"))
}

fn kernel(file: &syn::File, name: &str) -> Block {
    file.items
        .iter()
        .find_map(|item| {
            let Item::Mod(module) = item else { return None };
            if module.ident != "kernels" {
                return None;
            }
            module
                .content
                .as_ref()?
                .1
                .iter()
                .find_map(|item| match item {
                    Item::Fn(function) if function.sig.ident == name => {
                        Some((*function.block).clone())
                    }
                    _ => None,
                })
        })
        .unwrap_or_else(|| panic!("missing kernel {name}"))
}

fn path_is(expr: &Expr, name: &str) -> bool {
    matches!(expr, Expr::Path(path) if path.path.is_ident(name))
}

#[derive(Default)]
struct Trace {
    prefix: bool,
    events: Vec<String>,
}

impl<'ast> Visit<'ast> for Trace {
    fn visit_expr_closure(&mut self, _: &'ast syn::ExprClosure) {
        // The local exclusive_scan adapter is defined here, not executed here.
    }

    fn visit_expr_if(&mut self, expr: &'ast syn::ExprIf) {
        let selected = match expr.cond.as_ref() {
            Expr::Let(binding) if path_is(&binding.expr, "secondary") => Some(self.prefix),
            Expr::MethodCall(call) if path_is(&call.receiver, "secondary") => {
                match call.method.to_string().as_str() {
                    "is_some" => Some(self.prefix),
                    "is_none" => Some(!self.prefix),
                    _ => None,
                }
            }
            _ => None,
        };
        match selected {
            Some(true) => self.visit_block(&expr.then_branch),
            Some(false) => {
                if let Some((_, branch)) = &expr.else_branch {
                    self.visit_expr(branch);
                }
            }
            None => visit::visit_expr_if(self, expr),
        }
    }

    fn visit_expr_method_call(&mut self, call: &'ast syn::ExprMethodCall) {
        if path_is(&call.receiver, "module") && !call.method.to_string().starts_with("prepare_") {
            self.events.push(call.method.to_string());
        }
        visit::visit_expr_method_call(self, call);
    }

    fn visit_expr_call(&mut self, call: &'ast syn::ExprCall) {
        if path_is(&call.func, "exclusive_scan") {
            let Some(Expr::Reference(input)) = call.args.first() else {
                panic!("scan must borrow its input");
            };
            let Expr::Path(input) = input.expr.as_ref() else {
                panic!("scan input must be named");
            };
            self.events
                .push(format!("scan:{}", input.path.get_ident().unwrap()));
        }
        visit::visit_expr_call(self, call);
    }

    fn visit_local(&mut self, local: &'ast syn::Local) {
        visit::visit_local(self, local);
        if let syn::Pat::Ident(name) = &local.pat {
            if name.ident == "offsets" || name.ident == "unique_keys" {
                self.events.push(format!("allocate:{}", name.ident));
            }
        }
    }

    fn visit_expr_for_loop(&mut self, expr: &'ast syn::ExprForLoop) {
        if path_is(&expr.expr, "payloads") {
            // Validation loops contain no launches. Record only the gather loop.
            let start = self.events.len();
            visit::visit_expr_for_loop(self, expr);
            if self.events.len() != start {
                self.events.insert(start, "payload_loop".into());
                self.events.push("end_payload_loop".into());
            }
        } else {
            visit::visit_expr_for_loop(self, expr);
        }
    }
}

fn expected(prefix: bool) -> Vec<String> {
    // Captured from git show 5e09a15:experiments/gpu/blocked-run/src/lib.rs.
    // Allocation markers intentionally require BOTH scans before directory allocation.
    [
        "initialize",
        if prefix {
            "prefix_sorted_check"
        } else {
            "sorted_check"
        },
        "clear_histogram",
        "sorted_starts",
        "histogram",
        "word_offsets",
        "scan:word_rows",
        "scan:prefix",
        "allocate:offsets",
        "allocate:unique_keys",
        "complete_stats",
        "scatter",
        if prefix {
            "order_prefix_bins"
        } else {
            "order_bins"
        },
        "sorted_directory",
        "gather",
        "fences",
        "payload_loop",
        "gather",
        "end_payload_loop",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect()
}

#[test]
fn single_key_preserves_baseline_launch_and_allocation_order() {
    let file = source();
    let mut trace = Trace::default();
    trace.visit_block(&run_method(&file, "build_inner"));
    assert_eq!(trace.events, expected(false));
}

#[test]
fn prefix_only_substitutes_check_and_in_bin_ordering() {
    let file = source();
    let mut trace = Trace {
        prefix: true,
        ..Trace::default()
    };
    trace.visit_block(&run_method(&file, "build_inner"));
    assert_eq!(trace.events, expected(true));
}

#[test]
fn unsorted_kernels_return_before_any_work_on_sorted_or_invalid_input() {
    let file = source();
    let guard: Stmt =
        syn::parse_str("if unsafe { (*stats).sorted != 0 || (*stats).invalid != 0 } { return; }")
            .unwrap();
    for name in [
        "clear_histogram",
        "histogram",
        "scatter",
        "order_bins",
        "order_prefix_bins",
    ] {
        let body = kernel(&file, name);
        assert_eq!(
            body.stmts.first(),
            Some(&guard),
            "{name}: exact early gate must be first"
        );
    }
}

#[test]
fn sorted_directory_kernels_use_the_complementary_early_gate() {
    let file = source();
    let guard: Stmt =
        syn::parse_str("if unsafe { (*stats).sorted == 0 || (*stats).invalid != 0 } { return; }")
            .unwrap();
    for name in ["sorted_starts", "sorted_directory"] {
        assert_eq!(kernel(&file, name).stmts.first(), Some(&guard), "{name}");
    }
}

#[test]
fn wrappers_add_no_kernel_launches_or_scans() {
    let file = source();
    for name in ["build", "build_prefix"] {
        let mut trace = Trace::default();
        trace.visit_block(&run_method(&file, name));
        assert!(
            trace.events.is_empty(),
            "{name} adds pipeline work: {:?}",
            trace.events
        );
    }
}

#[test]
fn word_offsets_sorted_gate_initializes_scan_inputs_before_returning() {
    let file = source();
    // Unlike the other unsorted kernels, this stage must still supply both
    // scan inputs on the sorted path without reading uninitialized bins.
    let expected: Block = syn::parse_str(
        r#"{
        let word = thread::index_1d().get() / 32;
        let lane = warp::lane_id();
        if unsafe { (*stats).sorted != 0 || (*stats).invalid != 0 } {
            if lane == 31 && word < bits.len() {
                unsafe {
                    word_rows.add(word).write(0);
                    prefix.add(word).write(bits[word].count_ones());
                }
            }
            return;
        }
    }"#,
    )
    .unwrap();
    let body = kernel(&file, "word_offsets");
    assert_eq!(
        body.stmts.get(..expected.stmts.len()),
        Some(expected.stmts.as_slice())
    );
}

#[derive(Default)]
struct HostCalls(Vec<String>);

impl<'ast> Visit<'ast> for HostCalls {
    fn visit_expr_call(&mut self, call: &'ast syn::ExprCall) {
        if let Expr::Path(path) = call.func.as_ref() {
            self.0
                .push(path.path.segments.last().unwrap().ident.to_string());
        }
        visit::visit_expr_call(self, call);
    }

    fn visit_expr_method_call(&mut self, call: &'ast syn::ExprMethodCall) {
        self.0.push(call.method.to_string());
        visit::visit_expr_method_call(self, call);
    }
}

#[test]
fn successful_builds_never_wait_or_use_synchronizing_uploads() {
    let file = source();
    for name in ["build", "build_inner", "build_prefix"] {
        let mut calls = HostCalls::default();
        calls.visit_block(&run_method(&file, name));
        for forbidden in [
            "from_host",
            "from_host_in",
            "copy_from_host",
            "copy_from_host_in",
            "memcpy_htod",
            "synchronize",
            "wait",
            "wait_event",
            "to_host_vec",
        ] {
            assert!(
                !calls.0.iter().any(|call| call == forbidden),
                "{name} must enqueue without {forbidden}"
            );
        }
    }
}

#[test]
fn deeper_prefix_upload_is_async_and_owned_by_the_returned_run() {
    let file = source();
    let body = run_method(&file, "build_prefix");
    let mut calls = HostCalls::default();
    calls.visit_block(&body);
    for required in ["memcpy_htod_async", "record_event"] {
        assert_eq!(
            calls
                .0
                .iter()
                .filter(|call| call.as_str() == required)
                .count(),
            1,
            "{required} must occur exactly once"
        );
    }
    let position = |name: &str| calls.0.iter().position(|call| call == name).unwrap();
    assert!(position("memcpy_htod_async") < position("record_event"));
    let uploaded_build = calls
        .0
        .iter()
        .rposition(|call| call == "build_inner")
        .unwrap();
    assert!(position("record_event") < uploaded_build);

    // Assert ownership, not merely that an async-copy symbol occurs in source.
    let field_type = |owner: &str, field: &str| {
        file.items
            .iter()
            .find_map(|item| match item {
                Item::Struct(item) if item.ident == owner => item
                    .fields
                    .iter()
                    .find(|candidate| candidate.ident.as_ref().is_some_and(|name| name == field))
                    .map(|field| field.ty.clone()),
                _ => None,
            })
            .unwrap_or_else(|| panic!("missing {owner}.{field}"))
    };
    assert_eq!(
        field_type("PrefixUpload", "host"),
        syn::parse_str::<syn::Type>("PinnedHostBuffer<PrefixColumn>").unwrap()
    );
    assert_eq!(
        field_type("Run", "_prefix_upload"),
        syn::parse_str::<syn::Type>("Option<PrefixUpload<'a>>").unwrap()
    );

    let pin_position = body
        .stmts
        .iter()
        .position(|statement| {
            let Stmt::Local(local) = statement else {
                return false;
            };
            if !matches!(&local.pat, syn::Pat::Ident(name) if name.ident == "host") {
                return false;
            }
            let mut calls = HostCalls::default();
            calls.visit_local(local);
            calls.0.iter().any(|call| call == "zeroed")
        })
        .expect("pinned host allocation");
    let own_position = body
        .stmts
        .iter()
        .position(|statement| {
            let Stmt::Local(local) = statement else {
                return false;
            };
            if !matches!(&local.pat, syn::Pat::Ident(name)
            if name.ident == "upload" && name.mutability.is_some())
            {
                return false;
            }
            let Some(init) = &local.init else {
                return false;
            };
            let Expr::Struct(value) = init.expr.as_ref() else {
                return false;
            };
            value.path.is_ident("PrefixUpload")
                && value.rest.is_none()
                && value.fields.len() == 3
                && value.fields.iter().all(|field| {
                    let syn::Member::Named(name) = &field.member else {
                        return false;
                    };
                    match name.to_string().as_str() {
                        "host" => path_is(&field.expr, "host"),
                        "ready" => path_is(&field.expr, "None"),
                        "stream" => path_is(&field.expr, "stream"),
                        _ => false,
                    }
                })
        })
        .expect("pinned upload source owner");
    let event: Stmt = syn::parse_str("upload.ready = Some(stream.record_event(None)?);").unwrap();
    let retain: Stmt = syn::parse_str("run._prefix_upload = Some(upload);").unwrap();
    let returned = syn::parse_str::<Block>("{ Ok(run) }")
        .unwrap()
        .stmts
        .remove(0);
    let statement_position = |statement: &Stmt| {
        body.stmts
            .iter()
            .position(|candidate| candidate == statement)
            .unwrap_or_else(|| panic!("missing upload ownership statement: {statement:?}"))
    };
    assert!(pin_position < own_position);
    assert!(own_position < statement_position(&event));
    assert!(statement_position(&event) < statement_position(&retain));
    assert!(statement_position(&retain) < statement_position(&returned));
}

#[test]
fn two_level_prefix_returns_inline_before_metadata_allocation() {
    let file = source();
    let body = run_method(&file, "build_prefix");
    let condition: Expr = syn::parse_str("keys.len() == 2").unwrap();
    let (index, inline) = body
        .stmts
        .iter()
        .enumerate()
        .find_map(|(index, statement)| match statement {
            Stmt::Expr(Expr::If(branch), _) if *branch.cond == condition => {
                Some((index, &branch.then_branch))
            }
            _ => None,
        })
        .expect("two-level inline dispatch");
    let Some(Stmt::Expr(Expr::Return(returned), _)) = inline.stmts.last() else {
        panic!("two-level branch must return before metadata setup");
    };
    let Some(Expr::Call(call)) = returned.expr.as_deref() else {
        panic!("two-level branch must directly return the shared build");
    };
    let Expr::Path(path) = call.func.as_ref() else {
        panic!("named shared build");
    };
    assert_eq!(
        path.path,
        syn::parse_str::<syn::Path>("Self::build_inner").unwrap()
    );

    let mut before_return = HostCalls::default();
    for statement in &body.stmts[..index] {
        before_return.visit_stmt(statement);
    }
    before_return.visit_block(inline);
    for forbidden in ["zeroed", "allocate", "memcpy_htod_async", "record_event"] {
        assert!(
            !before_return.0.iter().any(|call| call == forbidden),
            "two-level prefix must not perform metadata setup: {forbidden}"
        );
    }
}
