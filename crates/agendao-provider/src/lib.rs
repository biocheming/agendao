pub mod artifact;
pub mod auth;
pub mod bootstrap;
pub mod bridge;
pub mod cache;
pub mod catalog;
pub mod custom_fetch;
pub mod descriptor;
pub mod diagnostics;
pub mod driver;
pub mod error_classification;
pub mod error_code;
pub mod error_summary;
pub mod instance;
pub mod message;
pub mod models;
pub mod profile;
pub mod protocol;
pub mod protocol_loader;
pub mod protocol_validator;
pub mod protocols;
pub mod provider;
pub mod registry;
pub mod responses;
pub mod responses_convert;
pub mod retry;
pub mod runtime;
pub mod stream;
pub mod tools;
pub mod transform;
pub mod transport;

pub mod core_surface {
    pub use crate::artifact::*;
    pub use crate::auth::*;
    pub use crate::bootstrap::create_registry_from_env;
    pub use crate::bootstrap::create_registry_from_env_with_auth_store;
    pub use crate::bootstrap::{
        apply_custom_loaders, bootstrap_config_from_raw, create_registry_from_bootstrap_config,
        filter_models_by_status, BootstrapConfig, ConfigModel, ConfigProvider, CustomLoaderResult,
    };
    pub use crate::bridge::{
        bridge_streaming_events, driver_response_to_chat_response, streaming_event_to_stream_events,
    };
    pub use crate::cache::*;
    pub use crate::custom_fetch::*;
    pub use crate::descriptor::*;
    pub use crate::diagnostics::*;
    pub use crate::error_summary::*;
    pub use crate::message::*;
    pub use crate::models::{
        get_model_context_limit, supports_function_calling, supports_vision, ModelCost,
        ModelInfo as ModelsDevInfo, ModelLimit, ModelModalities, ModelsData,
        ProviderInfo as ModelsProviderInfo,
    };
    pub use crate::profile::*;
    pub use crate::protocol::*;
    pub use crate::provider::*;
    pub use crate::retry::{with_retry, with_retry_and_hook, IsRetryable, RetryConfig};
    pub use crate::stream::*;
    pub use crate::tools::*;
    pub use crate::transform::{
        apply_caching, apply_caching_per_part, apply_caching_with_policy, dedup_messages,
        ensure_noop_tool_if_needed, extract_reasoning_from_response, max_output_tokens,
        mime_to_modality, normalize_interleaved_thinking, normalize_messages,
        normalize_messages_for_caching, normalize_messages_with_interleaved_field, options,
        provider_options_map, schema, sdk_key, small_options, temperature_for_model,
        top_k_for_model, top_p_for_model, transform_messages, unsupported_parts, variants,
        Modality, ProviderType, OUTPUT_TOKEN_MAX,
    };
}

#[cfg(feature = "http-transport")]
pub mod http_surface {
    pub use crate::bridge::DriverBasedAdapter;
    pub use crate::catalog::{
        default_catalog_metadata_path, default_catalog_snapshot_path,
        default_model_catalog_authority, load_default_catalog_data_sync, CatalogMetadata,
        CatalogRefreshResult, CatalogRefreshStatus, CatalogSnapshot, ModelCatalogAuthority,
    };
    pub use crate::instance::*;
    pub use crate::protocols::*;
    pub use crate::registry::ModelsRegistry;
    pub use crate::runtime::*;
    pub use crate::transport::*;
}

pub use artifact::*;
pub use auth::*;
pub use bootstrap::create_registry_from_env;
pub use bootstrap::create_registry_from_env_with_auth_store;
pub use bootstrap::{
    apply_custom_loaders, bootstrap_config_from_raw, create_registry_from_bootstrap_config,
    filter_models_by_status, BootstrapConfig, ConfigModel, ConfigProvider, CustomLoaderResult,
};
#[cfg(feature = "http-transport")]
pub use bridge::DriverBasedAdapter;
pub use bridge::{
    bridge_streaming_events, driver_response_to_chat_response, streaming_event_to_stream_events,
};
pub use cache::*;
#[cfg(feature = "http-transport")]
pub use catalog::{
    default_catalog_metadata_path, default_catalog_snapshot_path, default_model_catalog_authority,
    load_default_catalog_data_sync, metadata_path_for_snapshot, CatalogMetadata,
    CatalogRefreshResult, CatalogRefreshStatus, CatalogSnapshot, ModelCatalogAuthority,
};
pub use custom_fetch::*;
pub use descriptor::*;
pub use diagnostics::*;
pub use error_summary::*;
#[cfg(feature = "http-transport")]
pub use instance::*;
pub use message::*;
pub use profile::*;
pub use protocol::*;
pub use protocols::*;
pub use provider::*;
pub use retry::{with_retry, with_retry_and_hook, IsRetryable, RetryConfig};
pub use stream::*;
pub use tools::*;
pub use transform::{
    apply_caching, apply_caching_per_part, apply_caching_with_policy, dedup_messages,
    ensure_noop_tool_if_needed, extract_reasoning_from_response, max_output_tokens,
    mime_to_modality, normalize_interleaved_thinking, normalize_messages,
    normalize_messages_for_caching, normalize_messages_with_interleaved_field, options,
    provider_options_map, schema, sdk_key, small_options, temperature_for_model, top_k_for_model,
    top_p_for_model, transform_messages, unsupported_parts, variants, Modality, ProviderType,
    OUTPUT_TOKEN_MAX,
};
#[cfg(feature = "http-transport")]
pub use transport::*;

pub use models::{
    get_model_context_limit, supports_function_calling, supports_vision, ModelCost,
    ModelInfo as ModelsDevInfo, ModelLimit, ModelModalities, ModelsData,
    ProviderInfo as ModelsProviderInfo,
};
#[cfg(feature = "http-transport")]
pub use registry::ModelsRegistry;
