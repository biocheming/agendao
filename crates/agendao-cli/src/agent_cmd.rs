use std::fs;

use agendao_agent::AgentRegistry;
use agendao_config::loader::load_config;

use crate::cli::AgentCommands;

pub(crate) async fn handle_agent_command(action: AgentCommands) -> anyhow::Result<()> {
    match action {
        AgentCommands::List => {
            let cwd = std::env::current_dir()?;
            let config = load_config(&cwd)?;
            let registry = AgentRegistry::from_config(&config);
            println!("\nAvailable agents:\n");
            for agent in registry.list() {
                let description = agent.description.as_deref().unwrap_or("no description");
                println!("  {:<12} {}", agent.name, description);
            }
            println!();
        }
        AgentCommands::Create {
            name,
            description,
            mode,
            path,
            tools,
            model,
        } => {
            let sanitized: String = name
                .chars()
                .map(|c| {
                    if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                        c.to_ascii_lowercase()
                    } else {
                        '_'
                    }
                })
                .collect();

            if sanitized.is_empty() {
                anyhow::bail!("Agent name is empty after sanitization");
            }

            let base = match path {
                Some(path) => path,
                None => std::env::current_dir()?.join(".agendao/agent"),
            };
            fs::create_dir_all(&base)?;

            let file_path = base.join(format!("{}.md", sanitized));
            if file_path.exists() {
                anyhow::bail!("Agent file already exists: {}", file_path.display());
            }

            let yaml_description = description.replace('\n', " ").replace('"', "\\\"");
            let mut frontmatter = format!(
                "---\ndescription: \"{}\"\nmode: {}\n",
                yaml_description,
                mode.as_str(),
            );
            if let Some(model) = model {
                frontmatter.push_str(&format!("model: \"{}\"\n", model));
            }
            if let Some(tools) = tools {
                frontmatter.push_str(&format!("tools: \"{}\"\n", tools));
            }
            frontmatter.push_str("---\n");
            let content = format!(
                "{}\nYou are an AI assistant specialized in: {}.\n",
                frontmatter, description
            );

            fs::write(&file_path, content)?;
            println!("Agent created: {}", file_path.display());
        }
    }

    Ok(())
}
