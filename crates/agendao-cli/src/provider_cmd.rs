use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use agendao_config::{Config, ConfigLoader, ProviderConfig};
use agendao_provider::{
    export_provider_artifact_bundle, import_provider_artifact_bundle,
    ConfigProvider as BootstrapConfigProvider,
};
use agendao_types::{ProviderArtifactBundle, ProviderArtifactImportEnvelope};

use crate::providers::convert_config_providers_for_artifact;

pub(crate) async fn handle_provider_command(
    action: crate::cli::ProviderCommands,
) -> anyhow::Result<()> {
    match action {
        crate::cli::ProviderCommands::Export { output } => export_provider_data(output),
        crate::cli::ProviderCommands::Import { file } => import_provider_data(file),
    }
}

pub(crate) fn export_provider_data(output: Option<PathBuf>) -> anyhow::Result<()> {
    let current_dir = std::env::current_dir()?;
    export_provider_data_from_dir(&current_dir, output)
}

pub(crate) fn import_provider_data(file: String) -> anyhow::Result<()> {
    let current_dir = std::env::current_dir()?;
    import_provider_data_into_dir(&current_dir, Path::new(&file))?;
    Ok(())
}

fn export_provider_data_from_dir(base_dir: &Path, output: Option<PathBuf>) -> anyhow::Result<()> {
    let config = load_project_provider_config(base_dir)?;
    let export = export_provider_artifact_from_config(&config)?;

    let json = serde_json::to_string_pretty(&export)?;
    match output {
        Some(path) => {
            fs::write(&path, json)?;
            println!("Exported provider data to {}", path.display());
        }
        None => {
            println!("{}", json);
        }
    }

    Ok(())
}

fn import_provider_data_into_dir(base_dir: &Path, file: &Path) -> anyhow::Result<usize> {
    let raw = fs::read_to_string(file)?;
    let payload: ProviderArtifactImportEnvelope = serde_json::from_str(&raw)?;
    let imported = import_provider_artifact_into_project_config(base_dir, payload)?;

    println!("Imported {} provider(s) from {}", imported, file.display());
    Ok(imported)
}

fn export_provider_artifact_from_config(config: &Config) -> anyhow::Result<ProviderArtifactBundle> {
    let providers = convert_config_providers_for_artifact(config);
    Ok(export_provider_artifact_bundle(&providers)?)
}

fn load_project_provider_config(base_dir: &Path) -> anyhow::Result<Config> {
    let mut loader = ConfigLoader::new();
    loader.load_project(base_dir)?;
    Ok(loader.into_config())
}

fn import_provider_artifact_into_project_config(
    base_dir: &Path,
    payload: ProviderArtifactImportEnvelope,
) -> anyhow::Result<usize> {
    let imported = import_provider_artifact_bundle(payload)?;
    let imported_count = imported.len();
    let mut config = load_project_provider_config(base_dir)?;

    apply_imported_provider_configs(&mut config, imported)?;
    agendao_config::write_config(base_dir, &config)?;

    Ok(imported_count)
}

fn apply_imported_provider_configs(
    config: &mut Config,
    imported: HashMap<String, BootstrapConfigProvider>,
) -> anyhow::Result<()> {
    let providers = config.provider.get_or_insert_with(HashMap::new);

    for (provider_id, imported_provider) in imported {
        let existing = providers.get(&provider_id);
        ensure_existing_provider_is_import_safe(&provider_id, existing)?;
        let next = imported_provider_to_config(&imported_provider, existing);
        providers.insert(provider_id, next);
    }

    Ok(())
}

fn ensure_existing_provider_is_import_safe(
    provider_id: &str,
    existing: Option<&ProviderConfig>,
) -> anyhow::Result<()> {
    let Some(existing) = existing else {
        return Ok(());
    };

    if existing
        .options
        .as_ref()
        .is_some_and(|options| !options.is_empty())
    {
        anyhow::bail!(
            "provider `{}` has unsupported existing field `options`; provider artifact v1 import cannot merge it safely",
            provider_id
        );
    }
    if existing
        .models
        .as_ref()
        .is_some_and(|models| !models.is_empty())
    {
        anyhow::bail!(
            "provider `{}` has unsupported existing field `models`; provider artifact v1 import cannot merge it safely",
            provider_id
        );
    }
    if !existing.whitelist.is_empty() {
        anyhow::bail!(
            "provider `{}` has unsupported existing field `whitelist`; provider artifact v1 import cannot merge it safely",
            provider_id
        );
    }
    if !existing.blacklist.is_empty() {
        anyhow::bail!(
            "provider `{}` has unsupported existing field `blacklist`; provider artifact v1 import cannot merge it safely",
            provider_id
        );
    }

    Ok(())
}

fn imported_provider_to_config(
    imported: &BootstrapConfigProvider,
    existing: Option<&ProviderConfig>,
) -> ProviderConfig {
    ProviderConfig {
        name: imported.name.clone(),
        id: existing.and_then(|provider| provider.id.clone()),
        api_key: existing.and_then(|provider| provider.api_key.clone()),
        base_url: imported.api.clone(),
        models: None,
        options: None,
        npm: imported.npm.clone(),
        api_style: imported.api_style.clone(),
        api_shape: imported.api_shape.clone(),
        transport: imported.transport.clone(),
        usage_shape: imported.usage_shape.clone(),
        quirks: imported.quirks.clone().unwrap_or_default(),
        env: imported.env.clone(),
        whitelist: Vec::new(),
        blacklist: Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        apply_imported_provider_configs, export_provider_artifact_from_config,
        export_provider_data_from_dir, import_provider_data_into_dir,
    };
    use agendao_config::{Config, ProviderConfig};
    use agendao_types::{
        ProviderArtifactApiFamily, ProviderArtifactApiShape, ProviderArtifactBundle,
        ProviderArtifactCacheFamily, ProviderArtifactEntry, ProviderArtifactProfile,
        ProviderArtifactQuirk, ProviderArtifactTransport, ProviderArtifactUsageShape,
    };
    use std::collections::HashMap;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_workspace(label: &str) -> PathBuf {
        let unique = format!(
            "agendao-cli-provider-{label}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("clock should be valid")
                .as_nanos()
        );
        let path = std::env::temp_dir().join(unique);
        fs::create_dir_all(&path).expect("workspace should be created");
        path
    }

    fn write_config(root: &Path, content: &str) {
        fs::write(root.join("agendao.json"), content).expect("config should be written");
    }

    fn sample_bundle() -> ProviderArtifactBundle {
        ProviderArtifactBundle::new(
            123,
            vec![ProviderArtifactEntry {
                provider_id: "openai".to_string(),
                name: Some("OpenAI".to_string()),
                base_url: Some("https://api.openai.com/v1".to_string()),
                env: vec!["OPENAI_API_KEY".to_string()],
                profile: ProviderArtifactProfile {
                    npm: "@ai-sdk/openai".to_string(),
                    api_family: ProviderArtifactApiFamily::CloseAiCompatible,
                    api_shape: ProviderArtifactApiShape::Responses,
                    transport: ProviderArtifactTransport::Bearer,
                    usage_shape: ProviderArtifactUsageShape::CloseAiCachedTokens,
                    cache_family: ProviderArtifactCacheFamily::CloseAiCompatible,
                    quirks: vec![ProviderArtifactQuirk::NonStreamingSse],
                },
            }],
        )
    }

    #[test]
    fn export_provider_artifact_excludes_provider_api_key_and_secret_injected_options() {
        let config = Config {
            provider: Some(HashMap::from([(
                "openai".to_string(),
                ProviderConfig {
                    api_key: Some("secret-123".to_string()),
                    base_url: Some("https://api.openai.com/v1".to_string()),
                    ..Default::default()
                },
            )])),
            ..Default::default()
        };

        let artifact = export_provider_artifact_from_config(&config).expect("export");
        let json = serde_json::to_string(&artifact).expect("serialize");

        assert!(!json.contains("secret-123"));
        assert!(!json.contains("apiKey"));
        assert_eq!(artifact.providers.len(), 1);
        assert_eq!(artifact.providers[0].provider_id, "openai");
    }

    #[test]
    fn import_provider_artifact_preserves_existing_api_key_and_replaces_core_fields() {
        let mut config = Config {
            provider: Some(HashMap::from([(
                "openai".to_string(),
                ProviderConfig {
                    id: Some("openai".to_string()),
                    api_key: Some("secret-123".to_string()),
                    base_url: Some("https://old.example/v1".to_string()),
                    npm: Some("@ai-sdk/openai-compatible".to_string()),
                    ..Default::default()
                },
            )])),
            ..Default::default()
        };
        let imported = agendao_provider::import_provider_artifact_bundle(
            agendao_types::ProviderArtifactImportEnvelope::Bundle(sample_bundle()),
        )
        .expect("artifact import");

        apply_imported_provider_configs(&mut config, imported).expect("config apply");

        let provider = config
            .provider
            .as_ref()
            .and_then(|providers| providers.get("openai"))
            .expect("provider should exist");
        assert_eq!(provider.api_key.as_deref(), Some("secret-123"));
        assert_eq!(
            provider.base_url.as_deref(),
            Some("https://api.openai.com/v1")
        );
        assert_eq!(provider.npm.as_deref(), Some("@ai-sdk/openai"));
        assert_eq!(provider.api_shape.as_deref(), Some("responses"));
        assert_eq!(provider.quirks, vec!["non-streaming-sse".to_string()]);
    }

    #[test]
    fn import_provider_artifact_rejects_existing_non_core_fields() {
        let mut config = Config {
            provider: Some(HashMap::from([(
                "openai".to_string(),
                ProviderConfig {
                    options: Some(HashMap::from([(
                        "timeoutMs".to_string(),
                        serde_json::json!(10),
                    )])),
                    ..Default::default()
                },
            )])),
            ..Default::default()
        };
        let imported = agendao_provider::import_provider_artifact_bundle(
            agendao_types::ProviderArtifactImportEnvelope::Bundle(sample_bundle()),
        )
        .expect("artifact import");

        let error =
            apply_imported_provider_configs(&mut config, imported).expect_err("should fail");
        assert!(error
            .to_string()
            .contains("unsupported existing field `options`"));
    }

    #[test]
    fn provider_export_and_import_roundtrip_through_cli_helpers() {
        let source = temp_workspace("export-source");
        let target = temp_workspace("export-target");
        let export_path = temp_workspace("export-file").join("providers.json");

        write_config(
            &source,
            r#"{
  "provider": {
    "openai": {
      "api_key": "secret-123",
      "base_url": "https://api.openai.com/v1",
      "npm": "@ai-sdk/openai",
      "apiShape": "responses",
      "apiStyle": "closeai-compatible",
      "transport": "bearer",
      "usageShape": "closeai-cached-tokens",
      "quirks": ["non-streaming-sse"],
      "env": ["OPENAI_API_KEY"]
    }
  }
}"#,
        );
        write_config(
            &target,
            r#"{
  "provider": {
    "openai": {
      "api_key": "keep-me"
    }
  }
}"#,
        );

        export_provider_data_from_dir(&source, Some(export_path.clone())).expect("export");
        let exported = fs::read_to_string(&export_path).expect("export file");
        assert!(exported.contains("\"version\": \"agendao-rust/provider/v1\""));
        assert!(!exported.contains("secret-123"));

        let imported = import_provider_data_into_dir(&target, &export_path).expect("import");
        assert_eq!(imported, 1);

        let config = fs::read_to_string(target.join("agendao.json")).expect("persisted config");
        assert!(config.contains("\"api_key\": \"keep-me\""));
        assert!(config.contains("\"npm\": \"@ai-sdk/openai\""));
        assert!(config.contains("\"api_shape\": \"responses\""));
        assert!(config.contains("\"base_url\": \"https://api.openai.com/v1\""));
    }
}
