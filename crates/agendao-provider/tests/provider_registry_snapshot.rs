use agendao_provider::bootstrap::{
    bootstrap_config_from_raw, create_registry_from_bootstrap_config,
};
use std::collections::HashMap;

#[test]
fn test_provider_registry_snapshot() {
    // Keep this snapshot hermetic: bootstrap scans real provider env vars, so
    // an explicit allowlist prevents local credentials from changing the result.
    let config = bootstrap_config_from_raw(
        HashMap::new(),
        vec![],
        vec!["opencode".to_string()],
        None,
        None,
    );
    let auth_store = HashMap::new();
    let registry = create_registry_from_bootstrap_config(&config, &auth_store);

    // Extract registered provider IDs
    let mut registered_ids: Vec<String> =
        registry.list().iter().map(|p| p.id().to_string()).collect();
    registered_ids.sort();

    // With no config, env, or auth store, only the custom-loader-backed opencode
    // provider autoloads. All other providers require explicit credentials/config.
    assert_eq!(registered_ids, vec!["opencode".to_string()]);
}
