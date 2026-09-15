mod wiring {
    pub use ::mini_linq::{
        collect_access, collect_pipeline, materialized as index_requirements,
        result_indexes as relational_plan,
    };
}
use wiring as mini_linq;

// Enter below the YA scheduler to work on physical semijoin lowering alone.
mini_linq::relational_plan! {
    struct Supported;
    relation Items(key:i32, payload:String); relation Keys(key:i32);
    answer(v) :- Items(k,v), Keys(k);
    relational {
        items = rename Items {key -> k, payload -> v};
        keys = rename Keys {key -> k};
        kept = semijoin items with keys;
        p = project kept keep {v};
        output p as answer(v);
    }
}

fn main() {
    let answer = Supported::run([(1, "yes".into()), (2, "no".into())], [(1,)]);
    assert_eq!(answer, vec![("yes".to_owned(),)]);
    println!("{answer:?}");
}
