fn main() {
    cq_gpu::print! {
        pipeline {};
        physical {
            relation L(a:String); relation R(a:String,b:i64);
            l = pipe rename L@0 {a -> x};
            r = let rename R@1 {a -> x,b -> y};
            p = pipe canon probe first [x] l,r;
            return p as answer(x,y);
        }
        structures {
            l host scan = ::core::iter::Map<_,_> [];
            r host scan = ::core::iter::Map<_,_> [];
            r = ::cq_storage::SortedRun<_,_,::cq_storage::BorrowedI64TupleCodec> [x];
            p canon = ::cq_gpu_runtime::DeviceSet<_>;
        }
    };
}
