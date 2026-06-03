use std::io::{self, Write};

use crate::cli::AuthCommands;

pub(crate) const AUTH_ENV_PROVIDERS: &[(&str, &str)] = &[
    ("ethnopic", "ANTHROPIC_API_KEY"),
    ("openai", "OPENAI_API_KEY"),
    ("openrouter", "OPENROUTER_API_KEY"),
    ("google", "GOOGLE_API_KEY"),
    ("azure", "AZURE_OPENAI_API_KEY"),
    ("bedrock", "AWS_ACCESS_KEY_ID"),
    ("mistral", "MISTRAL_API_KEY"),
    ("groq", "GROQ_API_KEY"),
    ("xai", "XAI_API_KEY"),
    ("deepseek", "DEEPSEEK_API_KEY"),
    ("cohere", "COHERE_API_KEY"),
    ("together", "TOGETHER_API_KEY"),
    ("perplexity", "PERPLEXITY_API_KEY"),
    ("cerebras", "CEREBRAS_API_KEY"),
    ("deepinfra", "DEEPINFRA_API_KEY"),
    ("vercel", "VERCEL_API_KEY"),
    ("gitlab", "GITLAB_TOKEN"),
    ("github-copilot", "GITHUB_COPILOT_TOKEN"),
];

pub(crate) fn provider_env_var(provider: &str) -> Option<&'static str> {
    let normalized = provider.trim().to_lowercase();
    AUTH_ENV_PROVIDERS
        .iter()
        .find_map(|(name, env)| (*name == normalized).then_some(*env))
}

pub(crate) async fn handle_auth_command(action: AuthCommands) -> anyhow::Result<()> {
    match action {
        AuthCommands::List => {
            println!("\nCredential providers:");
            for (provider, env_var) in AUTH_ENV_PROVIDERS {
                let status = if std::env::var(env_var).is_ok() {
                    "set"
                } else {
                    "not set"
                };
                println!("  {:<16} {:<24} {}", provider, env_var, status);
            }
            println!();
        }
        AuthCommands::Login { provider, token } => {
            let provider = if let Some(provider) = provider {
                provider
            } else {
                println!("No provider specified. Supported providers:");
                for (p, _) in AUTH_ENV_PROVIDERS {
                    println!("  - {}", p);
                }
                print!("Provider: ");
                io::stdout().flush()?;
                let mut input = String::new();
                io::stdin().read_line(&mut input)?;
                input.trim().to_string()
            };

            if provider.starts_with("http://") || provider.starts_with("https://") {
                anyhow::bail!(
                    "Well-known URL login is not fully wired in Rust CLI yet. Use `agendao auth login <provider> --token ...` for now."
                );
            }

            let Some(env_var) = provider_env_var(&provider) else {
                anyhow::bail!(
                    "Unknown provider: {}. Run `agendao auth list` to see supported providers.",
                    provider
                );
            };

            let value = if let Some(token) = token {
                token
            } else {
                print!("Enter token for {}: ", provider);
                io::stdout().flush()?;
                let mut input = String::new();
                io::stdin().read_line(&mut input)?;
                input.trim().to_string()
            };

            if value.is_empty() {
                anyhow::bail!("Token cannot be empty");
            }

            std::env::set_var(env_var, &value);
            println!(
                "Set {} for current process only. For persistence, export it in your shell profile.",
                env_var
            );
        }
        AuthCommands::Logout { provider } => {
            let provider = if let Some(provider) = provider {
                provider
            } else {
                println!("Specify provider to logout. Currently supported:");
                for (p, _) in AUTH_ENV_PROVIDERS {
                    println!("  - {}", p);
                }
                print!("Provider: ");
                io::stdout().flush()?;
                let mut input = String::new();
                io::stdin().read_line(&mut input)?;
                input.trim().to_string()
            };

            let Some(env_var) = provider_env_var(&provider) else {
                anyhow::bail!(
                    "Unknown provider: {}. Run `agendao auth list` to see supported providers.",
                    provider
                );
            };

            std::env::remove_var(env_var);
            println!(
                "Cleared {} from current process. Also remove it from your shell profile if configured.",
                env_var
            );
        }
    }

    Ok(())
}
