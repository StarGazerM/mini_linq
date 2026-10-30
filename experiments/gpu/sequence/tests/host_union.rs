use cq_lang_sequence::{Program, check_grading};
use quote::{ToTokens, quote};

#[test]
fn host_union_concatenates_published_dispatches_before_optional_canon() {
    for canonical in [false, true] {
        let canon = canonical.then(|| quote!(canon));
        let program: Program = syn::parse2(quote! {
            physical {
                relation R(a:u32) = (&data.r);
                relation S(a:u32) = (&data.s);
                r = pipe rename R@0 {a -> a};
                s = pipe rename S@1 {a -> a};
                answer = let canon union r,s;
                return answer as answer(a);
            }
            structures {
                r host scan = ::cq_gpu_runtime::DeviceColumns<'static,(u32,)> [];
                s host scan = ::cq_gpu_runtime::DeviceColumns<'static,(u32,)> [];
                answer canon = sort using ::cq_gpu_sequence_runtime::canon::PackedCanon;
                answer rows = ::cq_gpu_runtime::DeviceColumns<'static,(u32,)>;
            }
            sequence {
                R = store rows r;
                S = store rows s;
                left_feed = flatten tile 256 R;
                left = store left_feed;
                right_feed = flatten tile 256 S;
                right = store right_feed;
                result = store #canon union [left,right];
                output count;
            }
            graded gpu {
                R at host;
                S at host;
                left_feed at device.block;
                left at host;
                right_feed at device.block;
                right at host;
                result at host;
            }
        }).unwrap();
        let program = cq_gpu_sequence_core::runs::record_representation(&program).unwrap();
        let analysis = check_grading(&program, &cq_gpu_sequence::machine()).unwrap();
        assert_eq!(analysis.dispatches.len(), 2);
        let generated = cq_gpu_sequence::translate(&program, &analysis).unwrap()
            .to_token_stream().to_string();
        assert_eq!(generated.matches("memcpy_dtod_async").count(), 2);
        let union = generated.find("let __cq_store_length_2").unwrap();
        assert!(generated[..union].contains("let (__cq_store_rows_1"));
        assert_eq!(generated.contains("canon . canonicalize"), canonical);
        assert!(generated.contains("__cq_union_pack_2"));
        assert!(!generated.contains("__cq_sequence_dispatch_2"));
    }
}
