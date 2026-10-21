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
