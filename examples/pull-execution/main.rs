//! Execute an already-lowered physical access plan through the supplied
//! `pull!` stage. This example starts below the three student compiler passes.

#![allow(clippy::clone_on_copy)] // Preserve the shape required by non-Copy schemas.

use mini_linq::pull;

struct TriangleProgram;

struct TriangleStorage {
    rows0: Vec<(i32, i32)>,
    index0: std::collections::HashMap<(i32,), Vec<(i32,)>>,
    index1: std::collections::HashSet<(i32, i32)>,
}

impl TriangleProgram {
    fn build(
        input0: impl IntoIterator<Item = (i32, i32)>,
        input1: impl IntoIterator<Item = (i32, i32)>,
        input2: impl IntoIterator<Item = (i32, i32)>,
    ) -> TriangleStorage {
        let relation0: std::collections::HashSet<(i32, i32)> = input0.into_iter().collect();
        let relation1: std::collections::HashSet<(i32, i32)> = input1.into_iter().collect();
        let relation2: std::collections::HashSet<(i32, i32)> = input2.into_iter().collect();

        let rows0 = relation0.into_iter().collect();
        let mut index0: std::collections::HashMap<(i32,), Vec<(i32,)>> =
            std::collections::HashMap::new();
        for (y, z) in relation1 {
            index0.entry((y,)).or_default().push((z,));
        }
        let index1: std::collections::HashSet<(i32, i32)> = relation2;

        TriangleStorage {
            rows0,
            index0,
            index1,
        }
    }

    fn run(
        input0: impl IntoIterator<Item = (i32, i32)>,
        input1: impl IntoIterator<Item = (i32, i32)>,
        input2: impl IntoIterator<Item = (i32, i32)>,
    ) -> Vec<(i32, i32, i32)> {
        Self::build(input0, input1, input2).materialize()
    }
}

impl TriangleStorage {
    fn query(&self) -> impl Iterator<Item = (i32, i32, i32)> + '_ {
        // The first expansion exposes Scan, Join, Filter, Project, and Distinct
        // with their intermediate bindings; the next lowers them to one result
        // iterator. Nothing is scanned until its caller requests a row.
        pull! {
            triangle(x, y, z) => (((*x).clone(), (*y).clone(), (*z).clone())) :-
            R(x, y) => for (x, y) in (self.rows0.iter()),
            S(y, z) => for (z,) in (
                self.index0
                    .get(&((*y).clone(),))
                    .into_iter()
                    .flatten()
            ),
            T(z, x) => if (self.index1.contains(&((*z).clone(), (*x).clone()))).
        }
    }

    fn materialize(&self) -> Vec<(i32, i32, i32)> {
        let mut result = self.query().collect::<Vec<_>>();
        result.sort_unstable();
        result
    }
}

fn main() {
    let storage =
        TriangleProgram::build([(1, 2), (1, 2), (2, 3)], [(2, 3), (3, 4)], [(3, 1), (4, 2)]);

    println!("lazy first row: {:?}", storage.query().next());
    println!("materialized: {:?}", storage.materialize());
    println!(
        "run convenience: {:?}",
        TriangleProgram::run([(1, 2)], [(2, 3)], [(3, 1)])
    );
}
