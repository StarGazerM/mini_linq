realize_physical! {
    j0 = natural_join l with r;
    j1 = natural_join m with r;
    indexes {
        i0 = @r[0];
        i1 = @r[1];
    }
    uses {
        j0.right = i0;
        j1.right = i1;
    }
    physical {
        relations {
            l = ext::slice_rows;
            m = ext::slice_rows;
            r = ext::array_rows;
            j0 = ext::borrowed_row;
            j1 = ext::borrowed_row;
        }
        indexes {
            i0 = ext::hash_buckets;
            i1 = ext::sorted_buckets;
        }
        lowerings {
            j0 = ext::indexed_join;
            j1 = ext::indexed_join;
        }
    }
}
