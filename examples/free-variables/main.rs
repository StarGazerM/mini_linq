use cq_lang_cq as cq;
use cq_lang_cq::BodyItem;

fn report(position: &str, expression: &syn::Expr) {
    let free = cq::rust::free_variables(expression);
    // Illustrative candidates only; the student's CQ analysis supplies the
    // actual environment at each clause and accounts for aggregate-local names.
    let environment = ["x", "limit", "config", "amount"].map(String::from).into();
    println!(
        "{position}: names={:?}, opaque={}, logical inputs={:?}",
        free.names,
        free.opaque,
        free.dependencies(&environment)
    );
}

fn main() {
    let source: cq::Program = syn::parse_quote! {
        struct Inspect; relation R(value:i32);
        answer(x) :- R(transform(x)),
            if { let threshold = limit; x > threshold },
            let label: String = format!("{x}"),
            !R(offset(x)),
            agg total = (factory(config))(amount) in R(amount);
    };
    for clause in &source.query.body {
        match clause {
            BodyItem::Positive { atom } => {
                for expression in &atom.args {
                    report("atom", expression);
                }
            }
            BodyItem::Filter(filter) => report("if", &filter.condition),
            BodyItem::Let(binding) => {
                report("let initializer", &binding.expression);
                println!("let outputs: {:?}", cq::rust::bindings(&binding.pattern));
            }
            BodyItem::Negation(negation) => {
                for expression in &negation.atom.args {
                    report("negated atom", expression);
                }
            }
            BodyItem::Extension { name, .. } => println!("unexpanded {name}!"),
            BodyItem::Aggregate(aggregate) => {
                if let cq::Aggregator::Expression { expression, .. } = &aggregate.aggregator {
                    report("aggregator", expression);
                }
                for expression in &aggregate.atom.args {
                    report("aggregate input", expression);
                }
                println!(
                    "aggregate outputs: {:?}",
                    cq::rust::bindings(&aggregate.pattern)
                );
            }
        }
    }
}
