#![recursion_limit = "2048"]

/// Global-handle codec used only at loading and logical R/P boundaries.
/// GPU relation fields use independently recorded dense component codes.
pub struct DoopSymbols;

impl gpu_datalog_runtime::SymbolCodec<doop_shared::IStr> for DoopSymbols {
    fn code(value: &doop_shared::IStr) -> u32 {
        value.code()
    }

    // DECODE ?: scalar DOOP handle inversion requires an explicit logical consumer.
    fn decode(code: u32) -> doop_shared::IStr {
        // DECODE ?: global handle restoration inherits the scalar codec caller boundary.
        doop_shared::IStr::from_code(code)
    }

    fn decode_many(codes: Vec<u32>) -> Vec<doop_shared::IStr> {
        // DECODE P: bulk DOOP output restores handles using one dictionary bounds snapshot.
        doop_shared::IStr::from_codes(codes)
    }
}

/// Fully loaded facts and immutable runtime dictionaries, prepared before query timing.
pub struct Prepared {
    facts: doop_shared::Facts,
    domains: std::sync::Arc<gpu_datalog_runtime::DomainCatalog>,
}

pub fn load(directory: &std::path::Path) -> gpu_datalog_runtime::Result<Prepared> {
    use doop_shared::{IStr, interner::sym};
    let (schema, types, required) = doop_shared::doop_query!(cq_gpu_datalog::domain_schema;
        symbols(IStr => [0] crate::DoopSymbols);
    );
    let (facts, mut memberships) = doop_shared::Facts::load_with_domains(directory, schema, types)?;
    for (component, handle) in required() {
        let Some(doop_shared::DomainValues::Symbols(values)) = memberships.get_mut(component as usize) else {
            return Err("creating symbol head has no dictionary component".into());
        };
        if let Err(position) = values.binary_search(&handle) {
            values.insert(position, handle);
        }
    }
    let inputs = memberships.into_iter().map(|values| match values {
        doop_shared::DomainValues::Symbols(values) => gpu_datalog_runtime::DomainInput::Dictionary(values),
        doop_shared::DomainValues::Integers(bounds) => gpu_datalog_runtime::DomainInput::Signed(
            bounds.map(|(minimum, maximum)| (i64::from(minimum), i64::from(maximum)))),
    }).collect();
    let domains = std::sync::Arc::new(gpu_datalog_runtime::DomainCatalog::new(
        gpu_datalog_runtime::thread_pool()?, inputs)?);
    eprintln!("domain_relation\trelation\tbefore_bits\tcarrier_bytes\tused_bits\tcompact_lane_bytes");
    for &(relation, components) in schema {
        let before_bits = components.len() * 32;
        let bits = domains.total_bits(components)?;
        eprintln!("domain_relation\t{relation}\t{before_bits}\t{}\t{bits}\t{}",
            before_bits.div_ceil(64).max(1) * 8, bits.div_ceil(64).max(1) * 8);
    }
    Ok(Prepared { facts, domains })
}

pub fn run(prepared: Prepared) -> doop_shared::Results {
    run_encoded(prepared.facts, prepared.domains)
}

doop_shared::doop_engine!(run_encoded, context __cq_gpu_domains: std::sync::Arc<gpu_datalog_runtime::DomainCatalog>, pipeline {
    incremental = ::cq_datalog::passes::incremental(driver(delta_first););
    storage_rewrites = [::cq_gpu_datalog::domain_storage(symbols(IStr => [0] crate::DoopSymbols))];
    cq = pipeline {
        surface = [::cq_gpu_datalog::encode_domains];
        physical = [::cq_gpu_sequence::capture_constants];
        structures = [::cq_gpu_datalog::bind_inputs, ::cq_gpu_sequence::bind_runs, ::cq_gpu_sequence::select_catalog];
        sequence = ::mini_linq::passes::sequence;
        // Each arena hole reuses only its last validated count as a capacity hint;
        // root tiles are sized at launch to fill the device (GPU-RootSpread).
        // With relational heads (c1b75718) spread measured 0.953x [0.923, 0.960]
        // of no spread on Cassandra (P6b).
        decompose = [
            ::cq_gpu_sequence::decompose,
            ::cq_sequence_rewrites::output(reserve_previous),
            ::cq_sequence_rewrites::roots(spread),
        ];
        stage = ::cq_gpu_sequence::stage;
        grade = ::cq_gpu_sequence::machine_grade;
        print = ::cq_gpu_datalog::translate_domain_arena;
    };
    terminals = [::cq_gpu_datalog::translate_domain_arena => batch];
    // DL-WaveLanes: four streams measured 0.723x [0.702, 0.765] of one on Cassandra.
    round = [::cq_gpu_datalog::bind_round(binding = 6), ::cq_gpu_datalog::lanes(streams = 4)];
    print = ::cq_gpu_datalog::print_rounds(__cq_gpu_domains);
});
