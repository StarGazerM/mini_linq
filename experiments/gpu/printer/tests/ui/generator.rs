fn main() {
    cq_gpu::print! {
        pipeline {};
        physical {
            relation L(a:i64);
            l = pipe rename L@0 {a -> x};
            g = pipe bind l with for y:i64 = ([x, x + 1]);
            p = pipe canon project g keep {y};
            return p as answer(y);
        }
        structures {
            l host scan = ::core::iter::Map<_,_> [];
            p canon = ::cq_gpu_runtime::DeviceSet<_>;
        }
    };
}
