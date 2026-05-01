use crate::models::ProviderInfo as ModelsProviderInfo;
use std::collections::HashMap;

use super::provider_factory::options_get_insensitive;
use super::ProviderState;

/// Result of a custom loader operation.
#[derive(Default)]
pub struct CustomLoaderResult {
    pub autoload: bool,
    pub options: HashMap<String, serde_json::Value>,
    pub has_custom_get_model: bool,
    pub models: HashMap<String, crate::models::ModelInfo>,
    pub headers: HashMap<String, String>,
    pub blacklist: Vec<String>,
}

/// Trait for provider-specific model loading customization.
pub trait CustomLoader: Send + Sync {
    fn load(
        &self,
        provider: &ModelsProviderInfo,
        provider_state: Option<&ProviderState>,
    ) -> CustomLoaderResult;
}

struct OpenCodeLoader;

impl CustomLoader for OpenCodeLoader {
    fn load(
        &self,
        provider: &ModelsProviderInfo,
        provider_state: Option<&ProviderState>,
    ) -> CustomLoaderResult {
        let mut result = CustomLoaderResult::default();

        let has_key = provider.env.iter().any(|name| std::env::var(name).is_ok())
            || provider_state
                .and_then(|state| provider_option_string(state, &["apiKey", "api_key", "apikey"]))
                .is_some();

        if !has_key {
            let paid_ids: Vec<String> = provider
                .models
                .iter()
                .filter(|(_, model)| model.cost.as_ref().map(|c| c.input > 0.0).unwrap_or(false))
                .map(|(id, _)| id.clone())
                .collect();
            for id in &paid_ids {
                result.blacklist.push(id.clone());
            }
        }

        let remaining = provider.models.len().saturating_sub(result.blacklist.len());
        result.autoload = remaining > 0;

        if !has_key {
            result.options.insert(
                "apiKey".to_string(),
                serde_json::Value::String("public".to_string()),
            );
        }

        result
    }
}

struct OpenAILoader;

impl CustomLoader for OpenAILoader {
    fn load(
        &self,
        _provider: &ModelsProviderInfo,
        _provider_state: Option<&ProviderState>,
    ) -> CustomLoaderResult {
        let mut result = CustomLoaderResult {
            has_custom_get_model: true,
            ..Default::default()
        };
        result.blacklist.extend(vec![
            "whisper".to_string(),
            "tts".to_string(),
            "dall-e".to_string(),
            "embedding".to_string(),
            "moderation".to_string(),
        ]);
        result
    }
}

struct GitHubCopilotLoader;

impl CustomLoader for GitHubCopilotLoader {
    fn load(
        &self,
        _provider: &ModelsProviderInfo,
        _provider_state: Option<&ProviderState>,
    ) -> CustomLoaderResult {
        CustomLoaderResult {
            has_custom_get_model: true,
            ..Default::default()
        }
    }
}

struct GitHubCopilotEnterpriseLoader;

impl CustomLoader for GitHubCopilotEnterpriseLoader {
    fn load(
        &self,
        _provider: &ModelsProviderInfo,
        _provider_state: Option<&ProviderState>,
    ) -> CustomLoaderResult {
        CustomLoaderResult {
            has_custom_get_model: true,
            ..Default::default()
        }
    }
}

/// Amazon Bedrock custom loader - the most complex loader.
/// Handles region resolution, AWS credential chain, cross-region model prefixing.
pub(super) struct AmazonBedrockLoader;

impl AmazonBedrockLoader {
    fn provider_option_string(state: Option<&ProviderState>, keys: &[&str]) -> Option<String> {
        let state = state?;
        for key in keys {
            let Some(value) = options_get_insensitive(&state.options, key) else {
                continue;
            };
            match value {
                serde_json::Value::String(s) if !s.trim().is_empty() => return Some(s.clone()),
                serde_json::Value::Number(n) => return Some(n.to_string()),
                serde_json::Value::Bool(b) => return Some(b.to_string()),
                _ => {}
            }
        }
        None
    }
}

impl CustomLoader for AmazonBedrockLoader {
    fn load(
        &self,
        _provider: &ModelsProviderInfo,
        provider_state: Option<&ProviderState>,
    ) -> CustomLoaderResult {
        let mut result = CustomLoaderResult::default();

        let region = Self::provider_option_string(provider_state, &["region"])
            .or_else(|| std::env::var("AWS_REGION").ok())
            .unwrap_or_else(|| "us-east-1".to_string());
        let profile = Self::provider_option_string(provider_state, &["profile"])
            .or_else(|| std::env::var("AWS_PROFILE").ok());
        let endpoint = Self::provider_option_string(
            provider_state,
            &["endpoint", "endpointUrl", "endpointURL"],
        );

        let aws_access_key_id = Self::provider_option_string(provider_state, &["accessKeyId"])
            .or_else(|| std::env::var("AWS_ACCESS_KEY_ID").ok());
        let aws_secret_access_key =
            Self::provider_option_string(provider_state, &["secretAccessKey"])
                .or_else(|| std::env::var("AWS_SECRET_ACCESS_KEY").ok());
        let aws_bearer_token =
            Self::provider_option_string(provider_state, &["awsBearerTokenBedrock", "bearerToken"])
                .or_else(|| std::env::var("AWS_BEARER_TOKEN_BEDROCK").ok());
        let aws_web_identity_token_file =
            Self::provider_option_string(provider_state, &["webIdentityTokenFile"])
                .or_else(|| std::env::var("AWS_WEB_IDENTITY_TOKEN_FILE").ok());
        let container_creds = std::env::var("AWS_CONTAINER_CREDENTIALS_RELATIVE_URI").is_ok()
            || std::env::var("AWS_CONTAINER_CREDENTIALS_FULL_URI").is_ok();

        if profile.is_none()
            && aws_access_key_id.is_none()
            && aws_secret_access_key.is_none()
            && aws_bearer_token.is_none()
            && aws_web_identity_token_file.is_none()
            && !container_creds
        {
            result.autoload = false;
            return result;
        }

        result.autoload = true;
        result
            .options
            .insert("region".to_string(), serde_json::Value::String(region));
        if let Some(profile) = profile {
            result
                .options
                .insert("profile".to_string(), serde_json::Value::String(profile));
        }
        if let Some(endpoint) = endpoint {
            result
                .options
                .insert("endpoint".to_string(), serde_json::Value::String(endpoint));
        }
        result.has_custom_get_model = true;

        result
    }
}

struct OpenRouterLoader;

impl CustomLoader for OpenRouterLoader {
    fn load(
        &self,
        _provider: &ModelsProviderInfo,
        _provider_state: Option<&ProviderState>,
    ) -> CustomLoaderResult {
        let mut result = CustomLoaderResult::default();
        result.headers.insert(
            "HTTP-Referer".to_string(),
            "https://opencode.ai/".to_string(),
        );
        result
            .headers
            .insert("X-Title".to_string(), "opencode".to_string());
        result
    }
}

struct ZenMuxLoader;

impl CustomLoader for ZenMuxLoader {
    fn load(
        &self,
        _provider: &ModelsProviderInfo,
        _provider_state: Option<&ProviderState>,
    ) -> CustomLoaderResult {
        let mut result = CustomLoaderResult::default();
        result.headers.insert(
            "HTTP-Referer".to_string(),
            "https://opencode.ai/".to_string(),
        );
        result
            .headers
            .insert("X-Title".to_string(), "opencode".to_string());
        result
    }
}

struct VercelLoader;

impl CustomLoader for VercelLoader {
    fn load(
        &self,
        _provider: &ModelsProviderInfo,
        _provider_state: Option<&ProviderState>,
    ) -> CustomLoaderResult {
        let mut result = CustomLoaderResult::default();
        result.headers.insert(
            "http-referer".to_string(),
            "https://opencode.ai/".to_string(),
        );
        result
            .headers
            .insert("x-title".to_string(), "opencode".to_string());
        result
    }
}

struct GoogleVertexLoader;

impl CustomLoader for GoogleVertexLoader {
    fn load(
        &self,
        _provider: &ModelsProviderInfo,
        _provider_state: Option<&ProviderState>,
    ) -> CustomLoaderResult {
        let mut result = CustomLoaderResult::default();

        let project = std::env::var("GOOGLE_CLOUD_PROJECT")
            .or_else(|_| std::env::var("GCP_PROJECT"))
            .or_else(|_| std::env::var("GCLOUD_PROJECT"))
            .ok();
        let location = std::env::var("GOOGLE_CLOUD_LOCATION")
            .or_else(|_| std::env::var("VERTEX_LOCATION"))
            .unwrap_or_else(|_| "us-east5".to_string());

        if let Some(ref project) = project {
            result.autoload = true;
            result.options.insert(
                "project".to_string(),
                serde_json::Value::String(project.clone()),
            );
            result
                .options
                .insert("location".to_string(), serde_json::Value::String(location));
            result.has_custom_get_model = true;
        }

        result
    }
}

struct GoogleVertexEthnopicLoader;

impl CustomLoader for GoogleVertexEthnopicLoader {
    fn load(
        &self,
        _provider: &ModelsProviderInfo,
        _provider_state: Option<&ProviderState>,
    ) -> CustomLoaderResult {
        let mut result = CustomLoaderResult::default();

        let project = std::env::var("GOOGLE_CLOUD_PROJECT")
            .or_else(|_| std::env::var("GCP_PROJECT"))
            .or_else(|_| std::env::var("GCLOUD_PROJECT"))
            .ok();
        let location = std::env::var("GOOGLE_CLOUD_LOCATION")
            .or_else(|_| std::env::var("VERTEX_LOCATION"))
            .unwrap_or_else(|_| "global".to_string());

        if let Some(ref project) = project {
            result.autoload = true;
            result.options.insert(
                "project".to_string(),
                serde_json::Value::String(project.clone()),
            );
            result
                .options
                .insert("location".to_string(), serde_json::Value::String(location));
            result.has_custom_get_model = true;
        }

        result
    }
}

struct SapAiCoreLoader;

impl CustomLoader for SapAiCoreLoader {
    fn load(
        &self,
        _provider: &ModelsProviderInfo,
        _provider_state: Option<&ProviderState>,
    ) -> CustomLoaderResult {
        let mut result = CustomLoaderResult::default();

        let env_service_key = std::env::var("AICORE_SERVICE_KEY").ok();
        result.autoload = env_service_key.is_some();

        if env_service_key.is_some() {
            if let Ok(deployment_id) = std::env::var("AICORE_DEPLOYMENT_ID") {
                result.options.insert(
                    "deploymentId".to_string(),
                    serde_json::Value::String(deployment_id),
                );
            }
            if let Ok(resource_group) = std::env::var("AICORE_RESOURCE_GROUP") {
                result.options.insert(
                    "resourceGroup".to_string(),
                    serde_json::Value::String(resource_group),
                );
            }
        }
        result.has_custom_get_model = true;

        result
    }
}

struct GitLabLoader;

impl CustomLoader for GitLabLoader {
    fn load(
        &self,
        _provider: &ModelsProviderInfo,
        _provider_state: Option<&ProviderState>,
    ) -> CustomLoaderResult {
        let mut result = CustomLoaderResult::default();

        let instance_url = std::env::var("GITLAB_INSTANCE_URL")
            .unwrap_or_else(|_| "https://gitlab.com".to_string());
        let api_key = std::env::var("GITLAB_TOKEN").ok();

        result.autoload = api_key.is_some();
        result.options.insert(
            "instanceUrl".to_string(),
            serde_json::Value::String(instance_url),
        );
        if let Some(key) = api_key {
            result
                .options
                .insert("apiKey".to_string(), serde_json::Value::String(key));
        }

        let user_agent = format!(
            "opencode/0.1.0 gitlab-ai-provider/0.1.0 ({} {}; {})",
            std::env::consts::OS,
            "unknown",
            std::env::consts::ARCH,
        );
        let mut ai_gateway_headers = HashMap::new();
        ai_gateway_headers.insert(
            "User-Agent".to_string(),
            serde_json::Value::String(user_agent),
        );
        result.options.insert(
            "aiGatewayHeaders".to_string(),
            serde_json::to_value(ai_gateway_headers).unwrap_or_default(),
        );

        let mut feature_flags = HashMap::new();
        feature_flags.insert(
            "duo_agent_platform_agentic_chat".to_string(),
            serde_json::Value::Bool(true),
        );
        feature_flags.insert(
            "duo_agent_platform".to_string(),
            serde_json::Value::Bool(true),
        );
        result.options.insert(
            "featureFlags".to_string(),
            serde_json::to_value(feature_flags).unwrap_or_default(),
        );

        result.has_custom_get_model = true;
        result
    }
}

struct CloudflareWorkersAiLoader;

impl CustomLoader for CloudflareWorkersAiLoader {
    fn load(
        &self,
        _provider: &ModelsProviderInfo,
        _provider_state: Option<&ProviderState>,
    ) -> CustomLoaderResult {
        let mut result = CustomLoaderResult::default();

        let account_id = std::env::var("CLOUDFLARE_ACCOUNT_ID").ok();
        if account_id.is_none() {
            result.autoload = false;
            return result;
        }
        let account_id = account_id.unwrap();

        let api_key = std::env::var("CLOUDFLARE_API_KEY").ok();
        result.autoload = api_key.is_some();

        if let Some(key) = api_key {
            result
                .options
                .insert("apiKey".to_string(), serde_json::Value::String(key));
        }
        result.options.insert(
            "baseURL".to_string(),
            serde_json::Value::String(format!(
                "https://api.cloudflare.com/client/v4/accounts/{}/ai/v1",
                account_id
            )),
        );
        result.has_custom_get_model = true;

        result
    }
}

struct CloudflareAiGatewayLoader;

impl CustomLoader for CloudflareAiGatewayLoader {
    fn load(
        &self,
        _provider: &ModelsProviderInfo,
        _provider_state: Option<&ProviderState>,
    ) -> CustomLoaderResult {
        let mut result = CustomLoaderResult::default();

        let account_id = std::env::var("CLOUDFLARE_ACCOUNT_ID").ok();
        let gateway = std::env::var("CLOUDFLARE_GATEWAY_ID").ok();

        if account_id.is_none() || gateway.is_none() {
            result.autoload = false;
            return result;
        }

        let api_token = std::env::var("CLOUDFLARE_API_TOKEN")
            .or_else(|_| std::env::var("CF_AIG_TOKEN"))
            .ok();

        result.autoload = api_token.is_some();

        if let Some(ref token) = api_token {
            result.options.insert(
                "apiKey".to_string(),
                serde_json::Value::String(token.clone()),
            );
        }
        if let Some(ref account_id) = account_id {
            result.options.insert(
                "accountId".to_string(),
                serde_json::Value::String(account_id.clone()),
            );
        }
        if let Some(ref gateway) = gateway {
            result.options.insert(
                "gateway".to_string(),
                serde_json::Value::String(gateway.clone()),
            );
        }
        result.has_custom_get_model = true;

        result
    }
}

struct CerebrasLoader;

impl CustomLoader for CerebrasLoader {
    fn load(
        &self,
        _provider: &ModelsProviderInfo,
        _provider_state: Option<&ProviderState>,
    ) -> CustomLoaderResult {
        let mut result = CustomLoaderResult::default();
        result.headers.insert(
            "X-Cerebras-3rd-Party-Integration".to_string(),
            "opencode".to_string(),
        );
        result
    }
}

pub(super) fn get_custom_loader(provider_id: &str) -> Option<Box<dyn CustomLoader>> {
    match provider_id {
        "opencode" => Some(Box::new(OpenCodeLoader)),
        "openai" => Some(Box::new(OpenAILoader)),
        "github-copilot" => Some(Box::new(GitHubCopilotLoader)),
        "github-copilot-enterprise" => Some(Box::new(GitHubCopilotEnterpriseLoader)),
        "amazon-bedrock" => Some(Box::new(AmazonBedrockLoader)),
        "openrouter" => Some(Box::new(OpenRouterLoader)),
        "zenmux" => Some(Box::new(ZenMuxLoader)),
        "vercel" => Some(Box::new(VercelLoader)),
        "google-vertex" => Some(Box::new(GoogleVertexLoader)),
        "google-vertex-ethnopic" => Some(Box::new(GoogleVertexEthnopicLoader)),
        "sap-ai-core" => Some(Box::new(SapAiCoreLoader)),
        "gitlab" => Some(Box::new(GitLabLoader)),
        "cloudflare-workers-ai" => Some(Box::new(CloudflareWorkersAiLoader)),
        "cloudflare-ai-gateway" => Some(Box::new(CloudflareAiGatewayLoader)),
        "cerebras" => Some(Box::new(CerebrasLoader)),
        _ => None,
    }
}

fn provider_option_string(provider: &ProviderState, keys: &[&str]) -> Option<String> {
    for key in keys {
        let Some(value) = options_get_insensitive(&provider.options, key) else {
            continue;
        };
        match value {
            serde_json::Value::String(s) if !s.trim().is_empty() => return Some(s.clone()),
            serde_json::Value::Number(n) => return Some(n.to_string()),
            serde_json::Value::Bool(b) => return Some(b.to_string()),
            _ => {}
        }
    }
    None
}
