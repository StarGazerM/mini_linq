fn main() {
    let rows = vec![(1i64,)];
    let _ = cq_gpu::print! {
        arguments {};
        pipeline {};
        physical {
            relation R(a:i64,b:i64) = (&rows);
            r = pipe rename R@0 {a -> x,b -> y};
            p = pipe canon project r keep {x,y};
            return p as answer(x,y);
        }
        structures {
            r host scan = ::core::iter::Map<_,_> [];
            p canon = ::cq_gpu_runtime::DeviceSet<_>;
        }
    };
}
