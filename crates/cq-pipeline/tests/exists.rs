#![cfg(feature = "exists")]

use cq_ir::iterator_pipeline;
use quote::ToTokens;

#[test]
fn generated_existence_stops_at_a_witness_without_payload_trait_bounds() {
    let source: iterator_pipeline::Pipeline = syn::parse_quote! {
        iter0 = scan payload in (source) yield (payload,);
        iter1 = filter iter0 as (payload,) if (payload.0 == 2);
        iter2 = project iter1 as (payload,) yield (payload);
        iter3 = distinct iter2;
        return iter3;
    };
    let exists = cq_pipeline::exists::compile(&source).unwrap();
    let unit: iterator_pipeline::Pipeline = syn::parse_quote! {
        iter0 = unit yield ();
        iter1 = distinct iter0;
        return iter1;
    };
    let unit_exists = cq_pipeline::exists::compile(&unit).unwrap();
    // Compile and run the actual emitted Rust without a proc-macro facade or
    // another compiler crate. Payload intentionally has no Clone/Eq/Hash impl.
    let fixture = format!(
        r#"
        struct Payload(i32);
        fn exists(source: &mut dyn Iterator<Item = Payload>) -> bool {{ {exists} }}
        fn main() {{
            let visits = std::cell::RefCell::new(Vec::new());
            let mut source = [Payload(1), Payload(2)].into_iter()
                .chain(std::iter::once_with(|| panic!("existence consumed past its witness")))
                .inspect(|payload| visits.borrow_mut().push(payload.0));
            assert!(exists(&mut source));
            assert_eq!(*visits.borrow(), [1, 2]);
            drop(source);
            visits.borrow_mut().clear();
            let mut missing = [Payload(3), Payload(1)].into_iter()
                .inspect(|payload| visits.borrow_mut().push(payload.0));
            assert!(!exists(&mut missing));
            assert_eq!(*visits.borrow(), [3, 1]);
            assert!(!exists(&mut std::iter::empty()));
            assert!({unit_exists});
        }}
        "#,
        exists = exists.to_token_stream(),
        unit_exists = unit_exists.to_token_stream(),
    );
    let directory =
        std::env::temp_dir().join(format!("mini-linq-pipeline-exists-{}", std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
    let path = directory.join("main.rs");
    let executable = directory.join("exists");
    std::fs::write(&path, fixture).unwrap();
    let compilation =
        std::process::Command::new(std::env::var_os("RUSTC").unwrap_or_else(|| "rustc".into()))
            .arg("--edition=2024")
            .arg(&path)
            .arg("-o")
            .arg(&executable)
            .output()
            .unwrap();
    assert!(
        compilation.status.success(),
        "{}",
        String::from_utf8_lossy(&compilation.stderr)
    );
    let execution = std::process::Command::new(&executable).output().unwrap();
    std::fs::remove_dir_all(directory).unwrap();
    assert!(
        execution.status.success(),
        "{}",
        String::from_utf8_lossy(&execution.stderr)
    );
}
