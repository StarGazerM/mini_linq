fn main() {
    cq_gpu::print! {
        pipeline {};
        physical {
            relation L(a:i64); relation R(a:i64);
            l = pipe rename L@0 {a -> x};
            r = pipe rename R@1 {a -> x};
            p = pipe canon union l,r;
            return p as answer(x);
        }
        structures {
            l host scan = ::core::iter::Map<_,_> [];
            r host scan = ::core::iter::Map<_,_> [];
            p canon = ::cq_gpu_runtime::DeviceSet<_>;
        }
    };
}
