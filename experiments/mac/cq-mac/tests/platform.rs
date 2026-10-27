#![cfg(not(target_os = "macos"))]

use cq_mac::{MacContext, MacError, MaintainedIndex, MaintenancePolicy};

#[test]
fn unsupported_targets_never_create_a_cpu_fallback() {
    assert!(matches!(
        MacContext::new(),
        Err(MacError::UnsupportedPlatform)
    ));
    for policy in [MaintenancePolicy::Flat, MaintenancePolicy::BinaryCarry] {
        let owner = MaintainedIndex::new(policy);
        let snapshot = owner.snapshot();
        assert!(snapshot.is_empty());
        assert_eq!(snapshot.lookup(&u32::MAX).len(), 0);
        assert!(!snapshot.contains(&0));
    }
}
