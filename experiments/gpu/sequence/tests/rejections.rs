use cq_gpu_sequence::machine;
use cq_lang_sequence::{Program, check_grading};

#[test]
fn fold_at_noncollaborative_device_is_rejected() {
    let program: Program = syn::parse_quote! {
        physical {
            relation R(a:i32) = (&data.r);
            r = pipe canon rename R@0 {a -> a};
            return r as answer(a);
        }
        structures {
            r host scan = Reader [];
            r canon = Canon;
        }
        sequence {
            input = store rows r;
            count = fold count input;
        }
        graded gpu { input at host; count at device; }
    };
    let error = check_grading(&program, &machine()).unwrap_err().to_string();
    assert!(error.contains("BE-Collective"), "{error}");
}

#[test]
fn opaque_rust_below_host_is_rejected() {
    let mut program: Program = syn::parse_quote! {
        physical {
            relation R(a:i32) = (&data.r);
            r = pipe rename R@0 {a -> a};
            bound = let canon bind r with n:i32 = (opaque(*a));
            return bound as answer(a,n);
        }
        structures {
            r host scan = Reader [];
            bound canon = Canon;
            bound rows = Rows;
        }
        sequence { result = store map bound rows r; }
        graded gpu { result at device; }
    };
    let error = check_grading(&program, &machine()).unwrap_err().to_string();
    assert!(error.contains("BE-Partial"), "{error}");
    program.graded = Some(syn::parse_quote!(graded gpu { result at host; }));
    assert!(
        check_grading(&program, &machine()).is_ok(),
        "opaque Rust remains legal at host"
    );
}

#[test]
fn scalar_fold_host_code_parses() {
    use quote::{ToTokens, quote};
    for operation in [
        quote!(fold count input),
        quote!(fold sum a tile 1024 items 4 input),
    ] {
        let program: Program = syn::parse2(quote! {
            physical {
                relation R(a:i64) = (&data.r);
                r = pipe canon rename R@0 {a -> a};
                return r as answer(a);
            }
            structures { r host scan = Reader []; r canon = Canon; }
            sequence {
                input = store rows r;
                result = store capacity 1024 #operation;
                output bounded;
            }
            graded gpu { input at host; result at device.block; }
        })
        .unwrap();
        let analysis = check_grading(&program, &machine()).unwrap();
        let generated = cq_gpu_sequence::translate(&program, &analysis)
            .unwrap()
            .to_token_stream()
            .to_string();
        assert!(generated.contains("__cq_fold_accumulator_"));
        assert!(generated.contains("__fold_result"));
        assert!(!generated.contains("__row"));
    }
}
