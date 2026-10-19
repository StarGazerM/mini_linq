fn main() {
    cq_gpu::print! {
        pipeline {};
        physical {
            relation L(a:i64);
            l = pipe rename L@0 {a -> x};
            g = pipe guard l if (opaque(x));
            p = pipe canon project g keep {x};
            return p as answer(x);
        }
        structures {
            l host scan = ::core::iter::Map<_,_> [];
            p canon = ::cq_gpu_runtime::DeviceSet<_>;
        }
    };
}
