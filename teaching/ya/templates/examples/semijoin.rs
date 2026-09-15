use mini_linq::planned_relational_plan;

// Enter below the YA scheduler to work on physical semijoin lowering alone.
planned_relational_plan! {
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
