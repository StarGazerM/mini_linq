use cq_lang_sequence::{Program, check_grading};
use quote::quote;

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
            structures { providers { device_columns = ::cq_gpu_runtime::DeviceColumns<_> { reads [], build macro, abi columns, bounds resident_columns }; packed_canon = ::cq_gpu_sequence_runtime::canon::PackedCanon { reads [distinct], build inferred, packed_bits 128 }; }
                r host scan = @device_columns input { keys 0; columns [u32 => u32] } as ::cq_gpu_runtime::DeviceColumns<'static,(u32,)> [];
                s host scan = @device_columns input { keys 0; columns [u32 => u32] } as ::cq_gpu_runtime::DeviceColumns<'static,(u32,)> [];
                answer rows = @packed_canon as ::cq_gpu_sequence_runtime::canon::PackedCanon;
                answer canon = rows;
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
        })
        .unwrap();
        let program = cq_gpu_sequence_core::runs::record_representation(&program).unwrap();
        let analysis = check_grading(&program, &cq_gpu_sequence::machine()).unwrap();
        assert_eq!(analysis.dispatches.len(), 2);
        use cq_lang_sequence::gpu::publication::{self, PublicationStorage};
        for name in ["left", "right"] {
            assert!(matches!(
                analysis.gpu_publications[name].storage,
                PublicationStorage::FlatRows(_)
            ));
            assert!(analysis.gpu_publications[name].union_staging.is_none());
        }
        let record = &analysis.gpu_publications["result"];
        assert!(matches!(record.storage, PublicationStorage::PackedKeys(_)));
        assert_eq!(record.storage.canonical_provider().is_some(), canonical);
        assert_eq!(record.union_staging.as_ref().unwrap().columns, ["a"]);
        publication::check(&program, &analysis, &analysis.gpu_publications).unwrap();
        let mut missing_staging = analysis.clone();
        missing_staging
            .gpu_publications
            .get_mut("result")
            .unwrap()
            .union_staging = None;
        assert!(
            publication::check(
                &program,
                &missing_staging,
                &missing_staging.gpu_publications
            )
            .is_err()
        );
        assert!(cq_gpu_sequence::translate(&program, &missing_staging).is_err());
        let expression = cq_gpu_sequence::translate(&program, &analysis).unwrap();
        struct PublicationCalls(Vec<&'static str>);
        impl<'a> syn::visit::Visit<'a> for PublicationCalls {
            fn visit_expr_call(&mut self, call: &'a syn::ExprCall) {
                if let syn::Expr::Path(path) = call.func.as_ref()
                    && path
                        .path
                        .segments
                        .last()
                        .is_some_and(|segment| segment.ident == "memcpy_dtod_async")
                {
                    self.0.push("copy");
                }
                syn::visit::visit_expr_call(self, call);
            }
            fn visit_expr_method_call(&mut self, call: &'a syn::ExprMethodCall) {
                if call.method == "canonicalize" {
                    self.0.push("canonicalize");
                }
                syn::visit::visit_expr_method_call(self, call);
            }
        }
        let mut calls = PublicationCalls(Vec::new());
        syn::visit::Visit::visit_expr(&mut calls, &expression);
        let mut expected = vec!["copy", "copy"];
        if canonical {
            expected.push("canonicalize");
        }
        assert_eq!(calls.0, expected);
        // A canonical union may not drop its recorded Canon boundary.
        if canonical {
            let mut dropped = analysis.clone();
            let PublicationStorage::PackedKeys(packed) =
                &mut dropped.gpu_publications.get_mut("result").unwrap().storage
            else {
                unreachable!()
            };
            packed.granularity = publication::Granularity::Rows;
            assert!(publication::check(&program, &dropped, &dropped.gpu_publications).is_err());
            assert!(cq_gpu_sequence::translate(&program, &dropped).is_err());
        }
    }
}
