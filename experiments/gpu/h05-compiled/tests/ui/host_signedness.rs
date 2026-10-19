fn main() {
    let rows = vec![(1u64,)];
    let _ = cq_gpu::print! {
        arguments {};
        pipeline {};
        physical {
            relation R(a:i64) = (&rows);
            r = pipe rename R@0 {a -> x};
            p = pipe canon project r keep {x};
            return p as answer(x);
        }
        structures {
            r host scan = ::core::iter::Map<_,_> [];
            p canon = ::cq_gpu_runtime::DeviceSet<_>;
        }
    };
}
