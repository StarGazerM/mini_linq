//! Authored Structures only: all Sequence schedules come from the planner.
pub fn fixture(name: &str) -> cq_lang_structures::Program {
    match name {
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
            structures {
                d host scan = ::gpu_datalog_runtime::DeviceLayout<(i32,),(i32,)> [];
                e host scan = ::gpu_datalog_runtime::DeviceLayout<(i32,),(u32,)> [];
                e = ::cq_gpu_sequence_runtime::RangeProvider [y];
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
            structures {
                d host scan = ::gpu_datalog_runtime::DeviceLayout<(i32,),(i32,)> [];
                e host scan = ::gpu_datalog_runtime::DeviceLayout<(i32,),(i32,)> [];
                e = ::cq_gpu_sequence_runtime::RangeProvider [y];
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
            structures {
                d host scan = ::gpu_datalog_runtime::DeviceLayout<(i32,),(i32,)> [];
                left host scan = ::gpu_datalog_runtime::DeviceLayout<(i32,),(u32,)> [];
                right host scan = ::gpu_datalog_runtime::DeviceLayout<(i32,),(u32,)> [];
                left = ::cq_gpu_sequence_runtime::RangeProvider [a];
                right = ::cq_gpu_sequence_runtime::RangeProvider [b];
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
            structures {
                d host scan = ::gpu_datalog_runtime::DeviceLayout<(i32,),(i32,)> [];
                b host scan = ::gpu_datalog_runtime::DeviceLayout<(i32,),()> [];
                b = ::cq_gpu_sequence_runtime::RangeProvider [x];
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
            structures {
                e host scan = ::gpu_datalog_runtime::DeviceLayout<(i32,),(u32,)> [];
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
            structures {
                e host scan = ::gpu_datalog_runtime::DeviceLayout<(i64,),(i64,)> [];
            }
        },
        _ => panic!("unknown run hole {name}"),
    }
}
