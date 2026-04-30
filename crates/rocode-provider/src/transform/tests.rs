use std::collections::HashMap;

use super::*;
use crate::models;
use crate::{Content, ContentPart, Message, Role};

#[test]
fn test_provider_type_detection() {
    assert!(matches!(
        ProviderType::from_provider_id("anthropic"),
        ProviderType::Ethnopic
    ));
    assert!(matches!(
        ProviderType::from_provider_id("openrouter"),
        ProviderType::OpenRouter
    ));
    assert!(matches!(
        ProviderType::from_provider_id("bedrock"),
        ProviderType::Bedrock
    ));
    assert!(matches!(
        ProviderType::from_provider_id("openai"),
        ProviderType::OpenAI
    ));
    assert!(matches!(
        ProviderType::from_provider_id("unknown"),
        ProviderType::Other
    ));
}

#[test]
fn test_caching_support() {
    assert!(ProviderType::Ethnopic.supports_caching());
    assert!(ProviderType::OpenRouter.supports_caching());
    assert!(!ProviderType::Other.supports_caching());
}

#[test]
fn test_interleaved_thinking_support() {
    assert!(ProviderType::Ethnopic.supports_interleaved_thinking());
    assert!(ProviderType::OpenRouter.supports_interleaved_thinking());
    assert!(!ProviderType::OpenAI.supports_interleaved_thinking());
}

#[test]
fn test_apply_caching_ethnopic_family() {
    let mut messages = vec![
        Message::system("System prompt"),
        Message::user("Hello"),
        Message::assistant("Hi there"),
    ];

    apply_caching(&mut messages, ProviderType::Ethnopic);

    // Messages-family providers use message-level providerOptions here.
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

    apply_caching(&mut messages, ProviderType::Ethnopic);

    assert!(messages[0].provider_options.is_some());
    assert!(messages[2].provider_options.is_some());
    assert!(messages[3].provider_options.is_none());
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
    assert_eq!(sdk_key("@ai-sdk/anthropic"), Some("ethnopic"));
    assert_eq!(sdk_key("ethnopic-compatible"), Some("ethnopic"));
    assert_eq!(sdk_key("@ai-sdk/openai"), Some("openai"));
    assert_eq!(sdk_key("closeai-compatible"), Some("openai"));
    assert_eq!(sdk_key("openai-compatible"), Some("openai"));
    assert_eq!(sdk_key("@ai-sdk/google"), Some("google"));
    assert_eq!(sdk_key("@ai-sdk/google-vertex"), Some("google"));
    assert_eq!(sdk_key("@ai-sdk/amazon-bedrock"), Some("bedrock"));
    assert_eq!(sdk_key("@openrouter/ai-sdk-provider"), Some("openrouter"));
    assert_eq!(sdk_key("@ai-sdk/perplexity"), Some("perplexity"));
    assert_eq!(sdk_key("unknown-package"), None);
}

#[test]
fn test_ethnopic_compatible_npm_detection() {
    assert!(super::normalize::is_ethnopic_compatible_npm(
        "@ai-sdk/anthropic"
    ));
    assert!(super::normalize::is_ethnopic_compatible_npm(
        "@ai-sdk/google-vertex/anthropic"
    ));
    assert!(super::normalize::is_ethnopic_compatible_npm(
        "ethnopic-compatible"
    ));
    assert!(!super::normalize::is_ethnopic_compatible_npm(
        "@ai-sdk/openai-compatible"
    ));
}

#[test]
fn test_ethnopic_compatible_hint_detection() {
    assert!(super::normalize::is_ethnopic_compatible_hint("anthropic"));
    assert!(super::normalize::is_ethnopic_compatible_hint(
        "ethnopic-compatible"
    ));
    assert!(super::normalize::is_ethnopic_compatible_hint(
        "ethnopic-model"
    ));
    assert!(!super::normalize::is_ethnopic_compatible_hint(
        "openai-compatible"
    ));
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

    normalize_interleaved_thinking(&mut messages, &ProviderType::Ethnopic, true);

    // Nothing stripped when interleaved is supported
    if let Content::Parts(ref parts) = messages[0].content {
        assert_eq!(parts.len(), 1);
        assert_eq!(parts[0].content_type, "thinking");
    } else {
        panic!("Expected Parts content");
    }
}

#[test]
fn test_apply_caching_per_part_ethnopic_family() {
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

    apply_caching_per_part(&mut messages, &ProviderType::Ethnopic);

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
fn test_transform_messages_applies_caching_for_vertex_ethnopic_family() {
    let mut messages = vec![
        Message::system("System prompt"),
        Message::user("Hello"),
        Message::assistant("Hi there"),
    ];

    transform_messages(
        &mut messages,
        ProviderType::Ethnopic,
        "kimi-k2p5",
        &[],
        "@ai-sdk/google-vertex/anthropic",
        "dashscope-compatible",
    );

    assert!(messages[0].provider_options.is_some());
    assert!(messages[2].provider_options.is_some());
}

#[test]
fn test_output_token_max_is_32000() {
    assert_eq!(OUTPUT_TOKEN_MAX, 32_000);
}

#[test]
fn test_schema_gemini_sanitization() {
    use serde_json::json;
    let model = models::ModelInfo {
        id: "google".to_string(),
        provider: Some(models::ModelProvider {
            npm: Some("@ai-sdk/google".to_string()),
            api: Some("gemini-2.0-flash".to_string()),
        }),
        ..default_model_info()
    };

    let input = json!({
        "type": "object",
        "properties": {
            "status": {
                "type": "integer",
                "enum": [1, 2, 3]
            }
        },
        "required": ["status", "nonexistent"]
    });

    let result = schema(&model, input);

    // Integer enum should be converted to string enum
    let status = &result["properties"]["status"];
    assert_eq!(status["type"], "string");
    assert_eq!(status["enum"], serde_json::json!(["1", "2", "3"]));

    // Required should be filtered to only existing properties
    assert_eq!(result["required"], serde_json::json!(["status"]));
}

#[test]
fn test_schema_gemini_array_items() {
    use serde_json::json;
    let model = models::ModelInfo {
        id: "google".to_string(),
        provider: Some(models::ModelProvider {
            npm: Some("@ai-sdk/google".to_string()),
            api: Some("gemini-2.0-flash".to_string()),
        }),
        ..default_model_info()
    };

    let input = json!({
        "type": "array"
    });

    let result = schema(&model, input);
    // Empty array should get items with type string
    assert_eq!(result["items"]["type"], "string");
}

#[test]
fn test_variants_sap_ethnopic_family() {
    let model = models::ModelInfo {
        id: "sap-model".to_string(),
        reasoning: true,
        provider: Some(models::ModelProvider {
            npm: Some("@mymediset/sap-ai-provider".to_string()),
            api: Some("ethnopic/test-model-large".to_string()),
        }),
        ..default_model_info()
    };

    let v = variants(&model);
    assert!(v.contains_key("high"));
    assert!(v.contains_key("max"));
    let high = &v["high"];
    assert!(high.contains_key("thinking"));
}

#[test]
fn test_variants_ethnopic_compatible_alias() {
    let model = models::ModelInfo {
        id: "ethnopic-compatible/sonnet".to_string(),
        reasoning: true,
        provider: Some(models::ModelProvider {
            npm: Some("ethnopic-compatible".to_string()),
            api: Some("ethnopic-compatible/sonnet".to_string()),
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
fn test_variants_sap_non_ethnopic_family() {
    let model = models::ModelInfo {
        id: "sap-model".to_string(),
        reasoning: true,
        provider: Some(models::ModelProvider {
            npm: Some("@jerome-benoit/sap-ai-provider-v2".to_string()),
            api: Some("openai/gpt-4o".to_string()),
        }),
        ..default_model_info()
    };

    let v = variants(&model);
    assert!(v.contains_key("low"));
    assert!(v.contains_key("medium"));
    assert!(v.contains_key("high"));
    assert!(!v.contains_key("max"));
}

#[test]
fn test_variants_venice() {
    let model = models::ModelInfo {
        id: "venice-model".to_string(),
        reasoning: true,
        provider: Some(models::ModelProvider {
            npm: Some("venice-ai-sdk-provider".to_string()),
            api: Some("some-model".to_string()),
        }),
        ..default_model_info()
    };

    let v = variants(&model);
    assert!(v.contains_key("low"));
    assert!(v.contains_key("medium"));
    assert!(v.contains_key("high"));
}

#[test]
fn test_provider_options_map_gateway() {
    use serde_json::json;
    let model = models::ModelInfo {
        id: "gateway-model".to_string(),
        provider: Some(models::ModelProvider {
            npm: Some("@ai-sdk/gateway".to_string()),
            api: Some("ethnopic/test-model-large".to_string()),
        }),
        ..default_model_info()
    };

    let mut opts = HashMap::new();
    opts.insert("gateway".to_string(), json!({"caching": "auto"}));
    opts.insert("thinking".to_string(), json!({"type": "enabled"}));

    let result = provider_options_map(&model, opts);
    assert!(result.contains_key("gateway"));
    assert!(result.contains_key("ethnopic"));
}

#[test]
fn test_provider_options_map_gateway_amazon() {
    use serde_json::json;
    let model = models::ModelInfo {
        id: "gateway-model".to_string(),
        provider: Some(models::ModelProvider {
            npm: Some("@ai-sdk/gateway".to_string()),
            api: Some("amazon/nova-2-lite".to_string()),
        }),
        ..default_model_info()
    };

    let mut opts = HashMap::new();
    opts.insert("reasoningEffort".to_string(), json!("high"));

    let result = provider_options_map(&model, opts);
    // amazon -> bedrock via SLUG_OVERRIDES
    assert!(result.contains_key("bedrock"));
}

#[test]
fn test_provider_options_map_gateway_ethnopic_alias() {
    use serde_json::json;
    let model = models::ModelInfo {
        id: "gateway-model".to_string(),
        provider: Some(models::ModelProvider {
            npm: Some("@ai-sdk/gateway".to_string()),
            api: Some("ethnopic-compatible/sonnet".to_string()),
        }),
        ..default_model_info()
    };

    let mut opts = HashMap::new();
    opts.insert("gateway".to_string(), json!({"caching": "auto"}));
    opts.insert("thinking".to_string(), json!({"type": "enabled"}));

    let result = provider_options_map(&model, opts);
    assert!(result.contains_key("gateway"));
    assert!(result.contains_key("ethnopic"));
}

#[test]
fn test_provider_options_map_gateway_closeai_alias() {
    use serde_json::json;
    let model = models::ModelInfo {
        id: "gateway-model".to_string(),
        provider: Some(models::ModelProvider {
            npm: Some("@ai-sdk/gateway".to_string()),
            api: Some("closeai-compatible/gpt-5".to_string()),
        }),
        ..default_model_info()
    };

    let mut opts = HashMap::new();
    opts.insert("reasoningEffort".to_string(), json!("high"));

    let result = provider_options_map(&model, opts);
    assert!(result.contains_key("openai"));
}

#[test]
fn test_provider_options_map_openrouter() {
    use serde_json::json;
    let model = models::ModelInfo {
        id: "openrouter-model".to_string(),
        provider: Some(models::ModelProvider {
            npm: Some("@openrouter/ai-sdk-provider".to_string()),
            api: Some("openai/gpt-5".to_string()),
        }),
        ..default_model_info()
    };

    let mut opts = HashMap::new();
    opts.insert("reasoning".to_string(), json!({"effort": "high"}));
    opts.insert("usage".to_string(), json!({"include": true}));

    let result = provider_options_map(&model, opts);
    assert!(result.contains_key("openrouter"));
}

#[test]
fn test_provider_options_map_perplexity() {
    use serde_json::json;
    let model = models::ModelInfo {
        id: "perplexity-model".to_string(),
        provider: Some(models::ModelProvider {
            npm: Some("@ai-sdk/perplexity".to_string()),
            api: Some("sonar".to_string()),
        }),
        ..default_model_info()
    };

    let mut opts = HashMap::new();
    opts.insert("return_images".to_string(), json!(true));
    opts.insert("search_recency_filter".to_string(), json!("month"));

    let result = provider_options_map(&model, opts);
    assert!(result.contains_key("perplexity"));
}

#[test]
fn test_normalize_tool_call_id_messages_family_ascii_only() {
    let normalized = normalize_tool_call_id("call:中文/id-1", true);
    assert_eq!(normalized, "call____id-1");
}

#[test]
fn test_normalize_tool_call_id_mistral_is_nine_ascii_alnum() {
    let normalized = normalize_tool_call_id_mistral("call-中文-ABC_123456789xyz");
    assert_eq!(normalized, "callABC12");
    assert_eq!(normalized.len(), 9);
    assert!(normalized.chars().all(|c| c.is_ascii_alphanumeric()));
}
