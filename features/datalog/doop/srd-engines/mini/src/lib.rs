#![recursion_limit = "2048"]
#[cfg(not(feature = "phase-timing"))]
srd_doop_shared::srd_doop_engine!(run, pipeline {});
#[cfg(feature = "phase-timing")]
srd_doop_shared::srd_doop_engine!(run, pipeline {
    fork = cq_datalog::passes::fork(timing = phases;);
});
