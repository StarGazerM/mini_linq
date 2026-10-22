fn main() {
    cq_gpu::print! {
        pipeline {};
        physical {
            relation L(a:i64); relation R(a:i64);
            l = let rename L@0 {a -> x};
            r = let rename R@1 {a -> x};
            p = pipe canon merge [x] l,r;
            return p as answer(x);
        }
        structures {
            l host scan = ::core::iter::Map<_,_> [];
            r host scan = ::core::iter::Map<_,_> [];
            l = ::cq_storage::SortedRun<_,_,::cq_storage::BorrowedI64TupleCodec> [x];
            r = ::cq_storage::SortedRun<_,_,::cq_storage::BorrowedI64TupleCodec> [x];
            p canon = ::cq_gpu_runtime::DeviceSet<_>;
        }
    };
}
