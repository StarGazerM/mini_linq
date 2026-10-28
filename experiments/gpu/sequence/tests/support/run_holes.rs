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
            structures {
                d host scan = ::gpu_datalog_runtime::DeviceLayout<(i32,),(i32,)> [];
                e host scan = ::gpu_datalog_runtime::DeviceLayout<(i32,),(u32,)> [];
                e host result(joined) = ::gpu_datalog_runtime::DeviceLayout<(i32,),(u32,)> [y];
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
            structures {
                d host scan = ::gpu_datalog_runtime::DeviceLayout<(i32,),(i32,)> [];
                b host scan = ::gpu_datalog_runtime::DeviceLayout<(i32,),()> [];
                b host result(allowed) = ::gpu_datalog_runtime::DeviceLayout<(i32,),()> [x];
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
            structures {
                e host scan = ::gpu_datalog_runtime::DeviceLayout<(i32,),(u32,)> [];
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
            structures {
                e host scan = ::gpu_datalog_runtime::DeviceLayout<(i32,),(u32,)> [];
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
            structures {
                e host scan = ::gpu_datalog_runtime::DeviceLayout<(i64,),(i32,i64)> [];
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
            structures {
                input host scan = ::gpu_datalog_runtime::DeviceLayout<(i64,),(i32,i64,i32,i32)> [];
            }
        },
        _ => panic!("unknown run hole {name}"),
    }
}
