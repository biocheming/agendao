use std::collections::HashMap;

use super::*;
use crate::models;
use crate::{Content, ContentPart, Message, Role};

#[test]
fn test_provider_type_uses_explicit_protocol_shapes() {
    assert_eq!(
        ProviderType::from_api_family(crate::ProviderApiFamily::AnthropicMessages),
        ProviderType::Anthropic
    );
    assert_eq!(
        ProviderType::from_api_family(crate::ProviderApiFamily::OpenAiCompatible),
        ProviderType::OpenAI
    );
    assert_eq!(
        ProviderType::from_supported_npm("@ai-sdk/anthropic"),
        Some(ProviderType::Anthropic)
    );
    assert_eq!(
        ProviderType::from_supported_npm("@ai-sdk/openai-compatible"),
        Some(ProviderType::OpenAI)
    );
    assert_eq!(
        ProviderType::from_supported_npm("anthropic-compatible"),
        None
    );
    assert_eq!(ProviderType::from_supported_npm("unknown"), None);
}

#[test]
fn test_caching_support() {
    assert!(ProviderType::Anthropic.supports_caching());
    assert!(!ProviderType::OpenAI.supports_caching());
    assert!(!ProviderType::Other.supports_caching());
}

#[test]
fn test_interleaved_thinking_support() {
    assert!(ProviderType::Anthropic.supports_interleaved_thinking());
    assert!(!ProviderType::OpenAI.supports_interleaved_thinking());
}

#[test]
fn test_apply_caching_anthropic_family() {
    let mut messages = vec![
        Message::system("System prompt"),
        Message::user("Hello"),
        Message::assistant("Hi there"),
    ];

    apply_caching(&mut messages, ProviderType::Anthropic);

    // Anthropic-family providers use message-level providerOptions here.
    assert!(messages[0].provider_options.is_some());
    assert!(messages[2].provider_options.is_some());
}

#[test]
fn test_apply_caching_uses_stable_boundary_before_current_user() {
    let mut messages = vec![
        Message::system("System prompt"),
        Message::user("Hello"),
        Message::assistant("Hi there"),
        Message::user("Follow up"),
    ];

    apply_caching(&mut messages, ProviderType::Anthropic);

    assert!(messages[0].provider_options.is_some());
    assert!(messages[2].provider_options.is_some());
    assert!(messages[3].provider_options.is_none());
}

#[test]
fn test_apply_caching_with_policy_can_disable_markers() {
    let mut messages = vec![
        Message::system("System prompt"),
        Message::assistant("Hi there"),
    ];
    let policy = crate::cache::AnthropicCachePolicy {
        enabled: false,
        ..Default::default()
    };

    apply_caching_with_policy(&mut messages, ProviderType::Anthropic, &policy);

    assert!(messages
        .iter()
        .all(|message| message.provider_options.is_none()));
}

#[test]
fn test_apply_caching_with_policy_preserves_ttl_and_scope_shape() {
    let mut messages = vec![
        Message::system("System prompt"),
        Message::assistant("Hi there"),
    ];
    let policy = crate::cache::AnthropicCachePolicy {
        ttl: crate::cache::AnthropicCacheTtl::OneHour,
        global_scope: true,
        ..Default::default()
    };

    apply_caching_with_policy(&mut messages, ProviderType::Anthropic, &policy);

    let cache_control = messages[0]
        .provider_options
        .as_ref()
        .and_then(|options| options.get("anthropic"))
        .and_then(|value| value.get("cacheControl"))
        .expect("cache control should be present");
    assert_eq!(
        cache_control.get("type").and_then(|value| value.as_str()),
        Some("ephemeral")
    );
    assert_eq!(
        cache_control.get("ttl").and_then(|value| value.as_str()),
        Some("1h")
    );
    assert_eq!(
        cache_control.get("scope").and_then(|value| value.as_str()),
        Some("global")
    );
}

#[test]
fn test_extract_reasoning() {
    let content = "Hello <thinking>let me think</thinking> World";
    let (reasoning, rest) = extract_reasoning_from_response(content);

    assert_eq!(reasoning, Some("let me think".to_string()));
    assert!(rest.contains("Hello"));
    assert!(rest.contains("World"));
}

fn default_model_info() -> models::ModelInfo {
    models::ModelInfo {
        id: "test-model".to_string(),
        name: "Test Model".to_string(),
        family: None,
        release_date: None,
        attachment: false,
        reasoning: false,
        temperature: false,
        tool_call: false,
        interleaved: None,
        cost: None,
        limit: models::ModelLimit {
            context: 128000,
            input: None,
            output: 8192,
        },
        modalities: None,
        experimental: None,
        status: None,
        options: HashMap::new(),
        headers: None,
        provider: None,
        variants: None,
    }
}

#[test]
fn test_max_output_tokens() {
    let model = models::ModelInfo {
        id: "test".to_string(),
        name: "Test".to_string(),
        limit: models::ModelLimit {
            context: 200000,
            input: None,
            output: 64000,
        },
        ..default_model_info()
    };
    assert_eq!(max_output_tokens(&model), OUTPUT_TOKEN_MAX);
}

#[test]
fn test_max_output_tokens_small_model() {
    let model = models::ModelInfo {
        limit: models::ModelLimit {
            context: 128000,
            input: None,
            output: 4096,
        },
        ..default_model_info()
    };
    assert_eq!(max_output_tokens(&model), 4096);
}

#[test]
fn test_variants_non_reasoning() {
    let model = models::ModelInfo {
        reasoning: false,
        ..default_model_info()
    };
    assert!(variants(&model).is_empty());
}

#[test]
fn test_sdk_key_mapping() {
    assert_eq!(sdk_key("@ai-sdk/anthropic"), Some("anthropic"));
    assert_eq!(sdk_key("@ai-sdk/openai"), Some("openai"));
    assert_eq!(sdk_key("@ai-sdk/openai-compatible"), Some("openai"));
    assert_eq!(sdk_key("anthropic-compatible"), None);
    assert_eq!(sdk_key("openai-compatible"), None);
    assert_eq!(sdk_key("@openrouter/ai-sdk-provider"), None);
    assert_eq!(sdk_key("@ai-sdk/perplexity"), None);
    assert_eq!(sdk_key("unknown-package"), None);
}

#[test]
fn test_normalize_interleaved_thinking_strips_non_last() {
    let mut messages = vec![
        Message {
            role: Role::Assistant,
            content: Content::Parts(vec![
                ContentPart {
                    content_type: "thinking".to_string(),
                    text: Some("thinking...".to_string()),
                    ..Default::default()
                },
                ContentPart {
                    content_type: "text".to_string(),
                    text: Some("response 1".to_string()),
                    ..Default::default()
                },
            ]),
            cache_control: None,
            provider_options: None,
        },
        Message::user("follow up"),
        Message {
            role: Role::Assistant,
            content: Content::Parts(vec![
                ContentPart {
                    content_type: "thinking".to_string(),
                    text: Some("more thinking...".to_string()),
                    ..Default::default()
                },
                ContentPart {
                    content_type: "text".to_string(),
                    text: Some("response 2".to_string()),
                    ..Default::default()
                },
            ]),
            cache_control: None,
            provider_options: None,
        },
    ];

    normalize_interleaved_thinking(&mut messages, &ProviderType::OpenAI, false);

    // First assistant: thinking stripped, text kept
    if let Content::Parts(ref parts) = messages[0].content {
        assert_eq!(parts.len(), 1);
        assert_eq!(parts[0].content_type, "text");
    } else {
        panic!("Expected Parts content");
    }

    // Last assistant: thinking kept
    if let Content::Parts(ref parts) = messages[2].content {
        assert_eq!(parts.len(), 2);
    } else {
        panic!("Expected Parts content");
    }
}

#[test]
fn test_normalize_interleaved_thinking_supports_interleaved() {
    let mut messages = vec![Message {
        role: Role::Assistant,
        content: Content::Parts(vec![ContentPart {
            content_type: "thinking".to_string(),
            text: Some("thinking...".to_string()),
            ..Default::default()
        }]),
        cache_control: None,
        provider_options: None,
    }];

    normalize_interleaved_thinking(&mut messages, &ProviderType::Anthropic, true);

    // Nothing stripped when interleaved is supported
    if let Content::Parts(ref parts) = messages[0].content {
        assert_eq!(parts.len(), 1);
        assert_eq!(parts[0].content_type, "thinking");
    } else {
        panic!("Expected Parts content");
    }
}

#[test]
fn test_apply_caching_per_part_anthropic_family() {
    let mut messages = vec![
        Message::system("system prompt"),
        Message::user("hello"),
        Message {
            role: Role::Assistant,
            content: Content::Text("response".to_string()),
            cache_control: None,
            provider_options: None,
        },
        Message::user("follow up"),
    ];

    apply_caching_per_part(&mut messages, &ProviderType::Anthropic);

    // System message should have cache control
    assert!(messages[0].cache_control.is_some());

    // Current user message is dynamic and should not become the cache boundary.
    assert!(messages[3].cache_control.is_none());

    // Previous assistant message is the stable conversation boundary.
    assert!(messages[2].cache_control.is_some());

    // First user message should NOT have cache control
    assert!(messages[1].cache_control.is_none());
}

#[test]
fn test_output_token_max_is_32000() {
    assert_eq!(OUTPUT_TOKEN_MAX, 32_000);
}

#[test]
fn test_variants_anthropic_sdk() {
    let model = models::ModelInfo {
        id: "anthropic/sonnet".to_string(),
        reasoning: true,
        provider: Some(models::ModelProvider {
            npm: Some("@ai-sdk/anthropic".to_string()),
            api: Some("anthropic/sonnet".to_string()),
        }),
        limit: models::ModelLimit {
            context: 200_000,
            input: None,
            output: 64_000,
        },
        ..default_model_info()
    };

    let v = variants(&model);
    assert!(v.contains_key("high"));
    assert!(v.contains_key("max"));
    assert!(v["high"].contains_key("thinking"));
}

#[test]
fn test_options_hashes_openai_prompt_cache_key() {
    use serde_json::json;
    let model = models::ModelInfo {
        id: "gpt-model".to_string(),
        provider: Some(models::ModelProvider {
            npm: Some("@ai-sdk/openai".to_string()),
            api: Some("gpt-5".to_string()),
        }),
        ..default_model_info()
    };
    let opts = HashMap::from([
        ("cacheStage".to_string(), json!("exec")),
        ("cacheRepoHash".to_string(), json!("repo_456")),
    ]);

    let result = options("openai", &model, "session-with-local-detail", &opts);
    let key = result
        .get("promptCacheKey")
        .and_then(|value| value.as_str())
        .expect("prompt cache key should be present");

    assert!(key.starts_with("agendao:"));
    assert!(key.contains(":exec:repo_456"));
    assert!(!key.contains("session-with-local-detail"));
}

#[test]
fn test_options_skips_prompt_cache_key_for_unknown_openai_compatible_provider() {
    let model = models::ModelInfo {
        id: "deepseek-model".to_string(),
        provider: Some(models::ModelProvider {
            npm: Some("@ai-sdk/openai-compatible".to_string()),
            api: Some("deepseek-chat".to_string()),
        }),
        ..default_model_info()
    };

    let result = options("deepseek", &model, "session", &HashMap::new());

    assert!(!result.contains_key("promptCacheKey"));
    assert!(!result.contains_key("prompt_cache_key"));
}

#[test]
fn test_normalize_tool_call_id_anthropic_family_ascii_only() {
    let normalized = normalize_tool_call_id("call:中文/id-1", true);
    assert_eq!(normalized, "call____id-1");
}
