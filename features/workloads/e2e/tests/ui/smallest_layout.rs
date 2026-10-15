// Selection normally supplies r = Trie [b][c]. This hand-authored Structures
// program omits that retained candidate index, while keeping the host-side
// candidate/membership decisions. The public printer must reject the missing
// two-level layout, not silently fall back to an ordinary full-key index.
fn main() {
    let L = [(1_i32, 2_i32)];
    let R = [(2_i32, 3_i32)];
    let S = [(1_i32, 3_i32)];
    let _ = mini_linq::passes::print_pull! {
        pipeline {};
        physical {
            relation L(a:i32,b:i32);
            relation R(b:i32,c:i32);
            relation S(a:i32,c:i32);
            l = pipe rename L@0 {a -> a,b -> b};
            r0 = pipe rename R@0 {b -> b,c -> c};
            r = let project r0 keep {b,c};
            s = pipe rename S@0 {a -> a,c -> c};
            candidate = pipe probe smallest [b] l,r;
            checked = pipe canon semi eager [a,c] candidate,s;
            return checked as answer(a,b,c);
        }
        structures {
            l host scan = ::core::iter::Map<_, _> [];
            r0 host scan = ::core::iter::Map<_, _> [];
            r rows = ::std::vec::Vec<_>;
            s host scan = ::core::iter::Map<_, _> [];
            s host checked = ::cq_storage::dispatch::KeySet<_> [a,c];
            s host candidates = ::cq_storage::HashIndex<'_, _, _> [a];
            s host membership = ::cq_storage::dispatch::KeySet<_> [a,c];
            checked canon = ::cq_storage::HashSet<_>;
        }
    };
}
