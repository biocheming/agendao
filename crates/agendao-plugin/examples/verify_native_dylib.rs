use std::fs;
use std::path::{Path, PathBuf};

use agendao_config::Config;
use agendao_plugin::{HookContext, HookEvent};
use serde_json::Value;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let config_path = args
        .next()
        .map(PathBuf::from)
        .ok_or("usage: verify_native_dylib <path-to-agendao.json>")?;

    let config_raw = fs::read_to_string(&config_path)?;
    let config: Config = serde_json::from_str(&config_raw)?;
    let base_dir = config_path
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."));

    let native_paths: Vec<PathBuf> = config
        .plugin
        .iter()
        .filter_map(|(_name, cfg)| {
            if !cfg.is_native() {
                return None;
            }
            let raw = cfg.dylib_path()?;
            let path = PathBuf::from(raw);
            if path.is_absolute() {
                Some(path)
            } else {
                Some(base_dir.join(path))
            }
        })
        .collect();

    if native_paths.is_empty() {
        return Err("no type=dylib plugins found in config".into());
    }

    let hook_system = agendao_plugin::global();
    let errors = agendao_plugin::load_native_plugins(&native_paths, hook_system.clone()).await;
    if !errors.is_empty() {
        return Err(format!("failed to load native plugins: {}", errors[0]).into());
    }

    let results = hook_system
        .trigger(
            HookContext::new(HookEvent::SessionStart)
                .with_data("agent", serde_json::json!("native-dylib-e2e")),
        )
        .await;

    let matched = results.into_iter().any(|result| match result {
        Ok(output) => output
            .payload
            .is_some_and(|payload| payload.get("native_demo_loaded") == Some(&Value::Bool(true))),
        Err(_) => false,
    });

    if !matched {
        return Err("native plugin loaded but did not emit expected hook payload".into());
    }

    println!("native dylib plugin loaded and hook executed successfully");
    Ok(())
}
