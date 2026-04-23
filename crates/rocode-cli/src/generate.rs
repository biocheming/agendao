use std::time::Duration;

use rocode_config::loader::load_config;
use rocode_provider::ModelsRegistry;

use crate::providers::setup_providers;
use crate::util::format_tokens;

pub(crate) async fn list_models(
    provider_filter: Option<String>,
    refresh: bool,
    verbose: bool,
) -> anyhow::Result<()> {
    if refresh {
        let registry = ModelsRegistry::default();
        match tokio::time::timeout(Duration::from_secs(15), registry.refresh()).await {
            Ok(_) => eprintln!("Refreshed models.dev cache."),
            Err(_) => eprintln!("Warning: timed out refreshing models.dev cache."),
        }
    }

    let current_dir = std::env::current_dir()?;
    let config = load_config(&current_dir)?;
    let registry = setup_providers(&config).await?;

    println!("\n╔══════════════════════════════════════════╗");
    println!("║         Available Models                  ║");
    println!("╚══════════════════════════════════════════╝\n");

    let providers = registry.list();

    if providers.is_empty() {
        println!("No providers configured. Set API keys to enable providers:");
        println!("  - ANTHROPIC_API_KEY");
        println!("  - OPENAI_API_KEY");
        println!("  - OPENROUTER_API_KEY");
        println!("  - GOOGLE_API_KEY");
        println!("  - MISTRAL_API_KEY");
        println!("  - GROQ_API_KEY");
        println!("  - XAI_API_KEY");
        println!("  - DEEPSEEK_API_KEY");
        println!("  - COHERE_API_KEY");
        println!("  - TOGETHER_API_KEY");
        println!("  - PERPLEXITY_API_KEY");
        println!("  - CEREBRAS_API_KEY");
        println!("  - GOOGLE_VERTEX_API_KEY + GOOGLE_VERTEX_PROJECT_ID + GOOGLE_VERTEX_LOCATION");
        println!("  - AZURE_OPENAI_API_KEY + AZURE_OPENAI_ENDPOINT");
        println!("  - AWS_ACCESS_KEY_ID + AWS_SECRET_ACCESS_KEY + AWS_REGION");
        return Ok(());
    }

    for provider in providers {
        if let Some(ref filter) = provider_filter {
            if !provider.id().contains(filter.to_lowercase().as_str()) {
                continue;
            }
        }

        println!("Provider: {} ({})", provider.name(), provider.id());
        println!("{}", "─".repeat(50));

        let models = provider.models();
        for model in models {
            println!("  {}", model.id);
            println!(
                "    Context: {} tokens | Output: {} tokens",
                format_tokens(model.context_window),
                format_tokens(model.max_output_tokens)
            );
            if model.supports_vision || model.supports_tools {
                let mut caps = Vec::new();
                if model.supports_vision {
                    caps.push("vision");
                }
                if model.supports_tools {
                    caps.push("tools");
                }
                println!("    Capabilities: {}", caps.join(", "));
            }
            if verbose {
                println!(
                    "    Details: name={} vision={} tools={}",
                    model.name, model.supports_vision, model.supports_tools
                );
            }
            println!();
        }
    }

    Ok(())
}
