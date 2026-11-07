//! Authored Structures only: all Sequence schedules come from the planner.
pub struct StructuresStack {
    pub physical: cq_lang_structures::physical::Program,
    pub structures: cq_lang_structures::Program,
}

impl syn::parse::Parse for StructuresStack {
    fn parse(input: syn::parse::ParseStream<'_>) -> syn::Result<Self> {
        let physical = input.parse()?;
        let structures = cq_lang_structures::Program::parse(input, &physical)?;
        Ok(Self { physical, structures })
    }
}

pub fn fixture(name: &str) -> StructuresStack {
    match name {
        "native_local_word" => {
            let mut program = fixture("native_linear");
            for entry in &mut program.structures.structures.entries {
                let provider = entry.structure.provider_mut().expect("native fixture provider");
                let types = provider.input.as_ref().expect("native fixture schema")
                    .columns.iter().map(|column| &column.storage);
                provider.ty = syn::parse_quote!(
                    ::gpu_datalog_runtime::RadixLayout<(#(#types,)*), (), LocalWord, u64, ()>);
                provider.radix.as_mut().expect("native fixture advice").word = syn::parse_quote!(LocalWord);
            }
            program
        },
        "linear" => syn::parse_quote! {
            physical {
                relation Delta(x:i32,y:i32) = (&delta);
                relation Edge(y:i32,z:u32) = (&edge);
                consumer canonical;
                d = pipe rename Delta@0 {x -> x,y -> y};
                e = let rename Edge@1 {y -> y,z -> z};
                joined = pipe probe first [y] d,e;
                result = pipe project joined keep {x,z};
                return result as answer(x,z);
            }
            structures { providers { radix_relation = ::gpu_datalog_runtime::RadixLayout<_, (), _, u64, ()> { reads [exact, range, membership, resident, distinct], build typed, abi radix_relation }; range_provider = ::cq_gpu_sequence_runtime::RangeProvider { reads [exact, range, membership], build inferred, abi range, contiguous_range }; }
                d host scan = @radix_relation input { keys 2; columns [i32 => i32, i32 => i32] } radix { word u64; offset u64; ratio 4; radix_bits 0; merge_tile 32 } as ::gpu_datalog_runtime::RadixLayout<(i32,i32),(),u64,u64,()> [];
                e host scan = @radix_relation input { keys 2; columns [i32 => i32, u32 => u32] } radix { word u64; offset u64; ratio 4; radix_bits 0; merge_tile 32 } as ::gpu_datalog_runtime::RadixLayout<(i32,u32),(),u64,u64,()> [];
                e = @range_provider as ::cq_gpu_sequence_runtime::RangeProvider [y];
            }
        },
        "native_linear" => syn::parse_quote! {
            physical {
                relation Delta(x:i32,y:i32) = (&delta);
                relation Edge(y:i32,z:u32) = (&edge);
                consumer canonical;
                d = pipe rename Delta@0 {x -> x,y -> y};
                e = pipe rename Edge@1 {y -> y,z -> z};
                joined = pipe probe first [y] d,e;
                // Match Datalog's owned scalar head projections.
                owned_x = pipe bind joined with out_x:i32 = (*x);
                owned_z = pipe bind owned_x with out_z:u32 = (*z);
                result = pipe project owned_z keep {out_x,out_z};
                return result as answer(out_x,out_z);
            }
            structures { providers { radix_relation = ::gpu_datalog_runtime::RadixLayout<_, (), _, u64, ()> { reads [exact, range, membership, resident, distinct], build typed, abi radix_relation }; }
                d host scan = @radix_relation input { keys 2; columns [i32 => i32, i32 => i32] } radix { word u64; offset u64; ratio 4; radix_bits 0; merge_tile 32 } as ::gpu_datalog_runtime::RadixLayout<(i32,i32),(),u64,u64,()> [];
                e host scan = @radix_relation input { keys 2; columns [i32 => i32, u32 => u32] } radix { word u64; offset u64; ratio 4; radix_bits 0; merge_tile 32 } as ::gpu_datalog_runtime::RadixLayout<(i32,u32),(),u64,u64,()> [];
                e host result(joined) = @radix_relation input { keys 2; columns [i32 => i32, u32 => u32] } radix { word u64; offset u64; ratio 4; radix_bits 0; merge_tile 32 } as ::gpu_datalog_runtime::RadixLayout<(i32,u32),(),u64,u64,()> [y];
            }
        },
        "nonlinear" => syn::parse_quote! {
            physical {
                relation Delta(x:i32,y:i32) = (&delta);
                relation Full(y:i32,z:i32) = (&full);
                consumer canonical;
                d = pipe rename Delta@0 {x -> x,y -> y};
                e = let rename Full@1 {y -> y,z -> z};
                joined = pipe probe first [y] d,e;
                result = pipe project joined keep {x,z};
                return result as answer(x,z);
            }
            structures { providers { radix_relation = ::gpu_datalog_runtime::RadixLayout<_, (), _, u64, ()> { reads [exact, range, membership, resident, distinct], build typed, abi radix_relation }; range_provider = ::cq_gpu_sequence_runtime::RangeProvider { reads [exact, range, membership], build inferred, abi range, contiguous_range }; }
                d host scan = @radix_relation input { keys 2; columns [i32 => i32, i32 => i32] } radix { word u64; offset u64; ratio 4; radix_bits 0; merge_tile 32 } as ::gpu_datalog_runtime::RadixLayout<(i32,i32),(),u64,u64,()> [];
                e host scan = @radix_relation input { keys 2; columns [i32 => i32, i32 => i32] } radix { word u64; offset u64; ratio 4; radix_bits 0; merge_tile 32 } as ::gpu_datalog_runtime::RadixLayout<(i32,i32),(),u64,u64,()> [];
                e = @range_provider as ::cq_gpu_sequence_runtime::RangeProvider [y];
            }
        },
        "double" => syn::parse_quote! {
            physical {
                relation Delta(a:i32,b:i32) = (&delta);
                relation Edge(a:i32,x:u32) = (&edge);
                consumer canonical;
                d = pipe rename Delta@0 {a -> a,b -> b};
                left = let rename Edge@1 {a -> a,x -> x};
                right = let rename Edge@2 {a -> b,x -> y};
                first = pipe probe first [a] d,left;
                second = pipe probe first [b] first,right;
                result = pipe project second keep {x,y};
                return result as answer(x,y);
            }
            structures { providers { radix_relation = ::gpu_datalog_runtime::RadixLayout<_, (), _, u64, ()> { reads [exact, range, membership, resident, distinct], build typed, abi radix_relation }; range_provider = ::cq_gpu_sequence_runtime::RangeProvider { reads [exact, range, membership], build inferred, abi range, contiguous_range }; }
                d host scan = @radix_relation input { keys 2; columns [i32 => i32, i32 => i32] } radix { word u64; offset u64; ratio 4; radix_bits 0; merge_tile 32 } as ::gpu_datalog_runtime::RadixLayout<(i32,i32),(),u64,u64,()> [];
                left host scan = @radix_relation input { keys 2; columns [i32 => i32, u32 => u32] } radix { word u64; offset u64; ratio 4; radix_bits 0; merge_tile 32 } as ::gpu_datalog_runtime::RadixLayout<(i32,u32),(),u64,u64,()> [];
                right host scan = @radix_relation input { keys 2; columns [i32 => i32, u32 => u32] } radix { word u64; offset u64; ratio 4; radix_bits 0; merge_tile 32 } as ::gpu_datalog_runtime::RadixLayout<(i32,u32),(),u64,u64,()> [];
                left = @range_provider as ::cq_gpu_sequence_runtime::RangeProvider [a];
                right = @range_provider as ::cq_gpu_sequence_runtime::RangeProvider [b];
            }
        },
        "anti" => syn::parse_quote! {
            physical {
                relation Delta(x:i32,y:i32) = (&delta);
                relation Blocked(x:i32) = (&blocked);
                consumer canonical;
                d = pipe rename Delta@0 {x -> x,y -> y};
                b = let rename Blocked@1 {x -> x};
                allowed = pipe anti eager [x] d,b;
                result = pipe project allowed keep {x,y};
                return result as answer(x,y);
            }
            structures { providers { radix_relation = ::gpu_datalog_runtime::RadixLayout<_, (), _, u64, ()> { reads [exact, range, membership, resident, distinct], build typed, abi radix_relation }; range_provider = ::cq_gpu_sequence_runtime::RangeProvider { reads [exact, range, membership], build inferred, abi range, contiguous_range }; }
                d host scan = @radix_relation input { keys 2; columns [i32 => i32, i32 => i32] } radix { word u64; offset u64; ratio 4; radix_bits 0; merge_tile 32 } as ::gpu_datalog_runtime::RadixLayout<(i32,i32),(),u64,u64,()> [];
                b host scan = @radix_relation input { keys 1; columns [i32 => i32] } radix { word u64; offset u64; ratio 4; radix_bits 0; merge_tile 32 } as ::gpu_datalog_runtime::RadixLayout<(i32,),(),u64,u64,()> [];
                b = @range_provider as ::cq_gpu_sequence_runtime::RangeProvider [x];
            }
        },
        "native_anti" => syn::parse_quote! {
            physical {
                relation Delta(x:i32,y:i32) = (&delta);
                relation Blocked(x:i32) = (&blocked);
                consumer canonical;
                d = pipe rename Delta@0 {x -> x,y -> y};
                b = pipe rename Blocked@1 {x -> x};
                allowed = pipe anti eager [x] d,b;
                result = pipe project allowed keep {x,y};
                return result as answer(x,y);
            }
            structures { providers { radix_relation = ::gpu_datalog_runtime::RadixLayout<_, (), _, u64, ()> { reads [exact, range, membership, resident, distinct], build typed, abi radix_relation }; }
                d host scan = @radix_relation input { keys 2; columns [i32 => i32, i32 => i32] } radix { word u64; offset u64; ratio 4; radix_bits 0; merge_tile 32 } as ::gpu_datalog_runtime::RadixLayout<(i32,i32),(),u64,u64,()> [];
                b host scan = @radix_relation input { keys 1; columns [i32 => i32] } radix { word u64; offset u64; ratio 4; radix_bits 0; merge_tile 32 } as ::gpu_datalog_runtime::RadixLayout<(i32,),(),u64,u64,()> [];
                b host result(allowed) = @radix_relation input { keys 1; columns [i32 => i32] } radix { word u64; offset u64; ratio 4; radix_bits 0; merge_tile 32 } as ::gpu_datalog_runtime::RadixLayout<(i32,),(),u64,u64,()> [x];
            }
        },
        "scan" => syn::parse_quote! {
            physical {
                relation Edge(x:i32,y:u32) = (&edge);
                consumer canonical;
                e = pipe rename Edge@0 {x -> x,y -> y};
                result = pipe project e keep {x,y};
                return result as answer(x,y);
            }
            structures { providers { radix_relation = ::gpu_datalog_runtime::RadixLayout<_, (), _, u64, ()> { reads [exact, range, membership, resident, distinct], build typed, abi radix_relation }; }
                e host scan = @radix_relation input { keys 2; columns [i32 => i32, u32 => u32] } radix { word u64; offset u64; ratio 4; radix_bits 0; merge_tile 32 } as ::gpu_datalog_runtime::RadixLayout<(i32,u32),(),u64,u64,()> [];
            }
        },
        "wide_scan" => syn::parse_quote! {
            physical {
                relation Edge(x:i64,y:i64) = (&edge);
                consumer canonical;
                e = pipe rename Edge@0 {x -> x,y -> y};
                result = pipe project e keep {x,y};
                return result as answer(x,y);
            }
            structures { providers { radix_relation = ::gpu_datalog_runtime::RadixLayout<_, (), _, u64, ()> { reads [exact, range, membership, resident, distinct], build typed, abi radix_relation }; }
                e host scan = @radix_relation input { keys 2; columns [i64 => i64, i64 => i64] } radix { word u128; offset u64; ratio 4; radix_bits 0; merge_tile 32 } as ::gpu_datalog_runtime::RadixLayout<(i64,i64),(),u128,u64,()> [];
            }
        },
        "capture" => syn::parse_quote! {
            physical {
                relation Edge(x:i32,y:u32) = (&edge);
                consumer canonical;
                e = pipe rename Edge@0 {x -> x,y -> y};
                selected = pipe guard e if (*x == host_key() && allow());
                bound = pipe bind selected with z:u32 = (host_value());
                result = pipe project bound keep {x,z};
                return result as answer(x,z);
            }
            structures { providers { radix_relation = ::gpu_datalog_runtime::RadixLayout<_, (), _, u64, ()> { reads [exact, range, membership, resident, distinct], build typed, abi radix_relation }; }
                e host scan = @radix_relation input { keys 2; columns [i32 => i32, u32 => u32] } radix { word u64; offset u64; ratio 4; radix_bits 0; merge_tile 32 } as ::gpu_datalog_runtime::RadixLayout<(i32,u32),(),u64,u64,()> [];
            }
        },
        "constant_atom" => syn::parse_quote! {
            physical {
                relation Edge(x:i32,y:u32) = (&edge);
                consumer canonical;
                e = let rename Edge@0 {y -> y} constants {x = -7};
                result = pipe project e keep {y};
                return result as answer(y);
            }
            structures { providers { radix_relation = ::gpu_datalog_runtime::RadixLayout<_, (), _, u64, ()> { reads [exact, range, membership, resident, distinct], build typed, abi radix_relation }; }
                e host scan = @radix_relation input { keys 2; columns [i32 => i32, u32 => u32] } radix { word u64; offset u64; ratio 4; radix_bits 0; merge_tile 32 } as ::gpu_datalog_runtime::RadixLayout<(i32,u32),(),u64,u64,()> [];
            }
        },
        "wide3_scan" => syn::parse_quote! {
            physical {
                relation Edge(x:i64,y:i32,z:i64) = (&edge);
                consumer canonical;
                e = pipe rename Edge@0 {x -> x,y -> y,z -> z};
                result = pipe project e keep {x,y,z};
                return result as answer(x,y,z);
            }
            structures { providers { radix_relation = ::gpu_datalog_runtime::RadixLayout<_, (), _, u64, ()> { reads [exact, range, membership, resident, distinct], build typed, abi radix_relation }; }
                e host scan = @radix_relation input { keys 3; columns [i64 => i64, i32 => i32, i64 => i64] } radix { word [u64;3]; offset u64; ratio 4; radix_bits 0; merge_tile 32 } as ::gpu_datalog_runtime::RadixLayout<(i64,i32,i64),(),[u64;3],u64,()> [];
            }
        },
        "wide4_scan" => syn::parse_quote! {
            physical {
                relation Edge(a:i64,b:i32,c:i64,d:i32,e:i32) = (&edge);
                consumer canonical;
                input = pipe rename Edge@0 {a -> a,b -> b,c -> c,d -> d,e -> e};
                result = pipe project input keep {a,b,c,d,e};
                return result as answer(a,b,c,d,e);
            }
            structures { providers { radix_relation = ::gpu_datalog_runtime::RadixLayout<_, (), _, u64, ()> { reads [exact, range, membership, resident, distinct], build typed, abi radix_relation }; }
                input host scan = @radix_relation input { keys 5; columns [i64 => i64, i32 => i32, i64 => i64, i32 => i32, i32 => i32] } radix { word [u64;4]; offset u64; ratio 4; radix_bits 0; merge_tile 32 } as ::gpu_datalog_runtime::RadixLayout<(i64,i32,i64,i32,i32),(),[u64;4],u64,()> [];
            }
        },
        _ => panic!("unknown run hole {name}"),
    }
}
