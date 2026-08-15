use agendao_provider::bootstrap::{
    bootstrap_config_from_raw, create_registry_from_bootstrap_config,
};
use std::collections::HashMap;

#[test]
fn test_provider_registry_snapshot() {
    let config = bootstrap_config_from_raw(HashMap::new(), vec![], vec![], None, None);
    let auth_store = HashMap::new();
    let registry = create_registry_from_bootstrap_config(&config, &auth_store);

    // Extract registered provider IDs
    let mut registered_ids: Vec<String> =
        registry.list().iter().map(|p| p.id().to_string()).collect();
    registered_ids.sort();

    assert!(registered_ids.is_empty());
}
