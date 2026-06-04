use serde::{Deserialize, Serialize};
use std::collections::HashMap;

pub const MODELS_DEV_URL: &str = "https://models.dev";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelCost {
    pub input: f64,
    pub output: f64,
    #[serde(default)]
    pub cache_read: Option<f64>,
    #[serde(default)]
    pub cache_write: Option<f64>,
    #[serde(default)]
    pub context_over_200k: Option<Box<ModelCost>>,
}

impl ModelCost {
    /// Compute the cost in dollars for the given token counts.
    /// Prices are per million tokens.
    pub fn compute(
        &self,
        input_tokens: u64,
        output_tokens: u64,
        cache_read_tokens: u64,
        cache_write_tokens: u64,
    ) -> f64 {
        let input_cost = self.input * (input_tokens as f64) / 1_000_000.0;
        let output_cost = self.output * (output_tokens as f64) / 1_000_000.0;
        let cache_read_cost =
            self.cache_read.unwrap_or(self.input) * (cache_read_tokens as f64) / 1_000_000.0;
        let cache_write_cost =
            self.cache_write.unwrap_or(self.input) * (cache_write_tokens as f64) / 1_000_000.0;
        input_cost + output_cost + cache_read_cost + cache_write_cost
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelLimit {
    pub context: u64,
    #[serde(default)]
    pub input: Option<u64>,
    pub output: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelModalities {
    pub input: Vec<String>,
    pub output: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelProvider {
    #[serde(default)]
    pub npm: Option<String>,
    #[serde(default)]
    pub api: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ModelInterleaved {
    Bool(bool),
    Field { field: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ModelExperimental {
    Bool(bool),
    Details(serde_json::Value),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelInfo {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub family: Option<String>,
    #[serde(default)]
    pub release_date: Option<String>,
    #[serde(default)]
    pub attachment: bool,
    #[serde(default)]
    pub reasoning: bool,
    #[serde(default)]
    pub temperature: bool,
    #[serde(default)]
    pub tool_call: bool,
    #[serde(default)]
    pub interleaved: Option<ModelInterleaved>,
    #[serde(default)]
    pub cost: Option<ModelCost>,
    pub limit: ModelLimit,
    #[serde(default)]
    pub modalities: Option<ModelModalities>,
    #[serde(default)]
    pub experimental: Option<ModelExperimental>,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub options: HashMap<String, serde_json::Value>,
    #[serde(default)]
    pub headers: Option<HashMap<String, String>>,
    #[serde(default)]
    pub provider: Option<ModelProvider>,
    #[serde(default)]
    pub variants: Option<HashMap<String, HashMap<String, serde_json::Value>>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderInfo {
    #[serde(default)]
    pub api: Option<String>,
    pub name: String,
    pub env: Vec<String>,
    pub id: String,
    #[serde(default)]
    pub npm: Option<String>,
    pub models: HashMap<String, ModelInfo>,
}

pub type ModelsData = HashMap<String, ProviderInfo>;

pub fn default_model_limits() -> (u64, u64) {
    (4096, 128000)
}

pub fn get_model_context_limit(model_id: &str) -> u64 {
    let lower = model_id.to_lowercase();

    if lower.contains("gpt-4") || lower.contains("gpt-4") {
        if lower.contains("32k") {
            return 32768;
        }
        if lower.contains("128k") || lower.contains("turbo") {
            return 128000;
        }
        return 8192;
    }

    if lower.contains("gemini") {
        if lower.contains("pro") || lower.contains("ultra") {
            return 1000000;
        }
        return 32000;
    }

    if lower.contains("llama") {
        return 128000;
    }

    128000
}

pub fn supports_vision(model_id: &str) -> bool {
    let lower = model_id.to_lowercase();

    lower.contains("vision")
        || lower.contains("gpt-4")
        || lower.contains("gemini")
        || lower.contains("qwen-vl")
}

pub fn supports_function_calling(model_id: &str) -> bool {
    let lower = model_id.to_lowercase();

    !lower.contains("embedding") && !lower.contains("whisper") && !lower.contains("tts")
}

#[cfg(test)]
mod tests {
    use super::{ModelExperimental, ModelsData};

    #[test]
    fn parses_models_dev_experimental_bool_and_object() {
        let raw = r#"
        {
          "openai": {
            "id": "openai",
            "name": "OpenAI",
            "env": ["OPENAI_API_KEY"],
            "models": {
              "gpt-stable": {
                "id": "gpt-stable",
                "name": "GPT Stable",
                "attachment": false,
                "reasoning": false,
                "tool_call": true,
                "temperature": true,
                "experimental": true,
                "limit": { "context": 128000, "output": 8192 }
              },
              "gpt-fast": {
                "id": "gpt-fast",
                "name": "GPT Fast",
                "attachment": false,
                "reasoning": false,
                "tool_call": true,
                "temperature": true,
                "experimental": {
                  "modes": {
                    "fast": {
                      "provider": {
                        "body": {
                          "service_tier": "priority"
                        }
                      }
                    }
                  }
                },
                "limit": { "context": 128000, "output": 8192 }
              }
            }
          }
        }
        "#;

        let parsed =
            serde_json::from_str::<ModelsData>(raw).expect("models.dev payload should parse");
        let provider = parsed.get("openai").expect("provider should exist");
        assert!(matches!(
            provider
                .models
                .get("gpt-stable")
                .and_then(|model| model.experimental.as_ref()),
            Some(ModelExperimental::Bool(true))
        ));
        assert!(matches!(
            provider
                .models
                .get("gpt-fast")
                .and_then(|model| model.experimental.as_ref()),
            Some(ModelExperimental::Details(_))
        ));
    }
}
