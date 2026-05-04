use super::*;

trait DeepMerge {
    fn deep_merge(&mut self, other: Self);
}

impl<T: DeepMerge> DeepMerge for Box<T> {
    fn deep_merge(&mut self, other: Self) {
        self.as_mut().deep_merge(*other);
    }
}

fn merge_option_replace<T>(target: &mut Option<T>, source: Option<T>) {
    if let Some(value) = source {
        *target = Some(value);
    }
}

fn merge_vec_replace_if_non_empty<T>(target: &mut Vec<T>, source: Vec<T>) {
    if !source.is_empty() {
        *target = source;
    }
}

fn merge_option_deep<T: DeepMerge>(target: &mut Option<T>, source: Option<T>) {
    if let Some(source_value) = source {
        if let Some(target_value) = target {
            target_value.deep_merge(source_value);
        } else {
            *target = Some(source_value);
        }
    }
}

fn merge_map_deep_values<T: DeepMerge>(
    target: &mut HashMap<String, T>,
    source: HashMap<String, T>,
) {
    for (key, source_value) in source {
        if let Some(target_value) = target.get_mut(&key) {
            target_value.deep_merge(source_value);
        } else {
            target.insert(key, source_value);
        }
    }
}

fn merge_option_map_deep_values<T: DeepMerge>(
    target: &mut Option<HashMap<String, T>>,
    source: Option<HashMap<String, T>>,
) {
    if let Some(source_map) = source {
        if let Some(target_map) = target {
            merge_map_deep_values(target_map, source_map);
        } else {
            *target = Some(source_map);
        }
    }
}

fn merge_option_map_overwrite_values<T>(
    target: &mut Option<HashMap<String, T>>,
    source: Option<HashMap<String, T>>,
) {
    if let Some(source_map) = source {
        if let Some(target_map) = target {
            merge_map_overwrite_values(target_map, source_map);
        } else {
            *target = Some(source_map);
        }
    }
}

fn merge_map_overwrite_values<T>(target: &mut HashMap<String, T>, source: HashMap<String, T>) {
    for (key, value) in source {
        target.insert(key, value);
    }
}

fn merge_json_value(target: &mut serde_json::Value, source: serde_json::Value) {
    match (target, source) {
        (serde_json::Value::Object(target_map), serde_json::Value::Object(source_map)) => {
            for (key, source_value) in source_map {
                if let Some(target_value) = target_map.get_mut(&key) {
                    merge_json_value(target_value, source_value);
                } else {
                    target_map.insert(key, source_value);
                }
            }
        }
        (target_value, source_value) => *target_value = source_value,
    }
}

fn merge_option_json_map(
    target: &mut Option<HashMap<String, serde_json::Value>>,
    source: Option<HashMap<String, serde_json::Value>>,
) {
    if let Some(source_map) = source {
        if let Some(target_map) = target {
            for (key, source_value) in source_map {
                if let Some(target_value) = target_map.get_mut(&key) {
                    merge_json_value(target_value, source_value);
                } else {
                    target_map.insert(key, source_value);
                }
            }
        } else {
            *target = Some(source_map);
        }
    }
}

fn append_unique_keep_order(target: &mut Vec<String>, source: Vec<String>) {
    for item in source {
        if !target.contains(&item) {
            target.push(item);
        }
    }
}

macro_rules! merge_option_replace_fields {
    ($target:ident, $source:ident, $( $field:ident ),+ $(,)?) => {
        $(
            merge_option_replace(&mut $target.$field, $source.$field);
        )+
    };
}

macro_rules! merge_option_deep_fields {
    ($target:ident, $source:ident, $( $field:ident ),+ $(,)?) => {
        $(
            merge_option_deep(&mut $target.$field, $source.$field);
        )+
    };
}

impl DeepMerge for KeybindsConfig {
    fn deep_merge(&mut self, other: Self) {
        merge_option_replace_fields!(
            self,
            other,
            leader,
            app_exit,
            editor_open,
            theme_list,
            sidebar_toggle,
            scrollbar_toggle,
            username_toggle,
            status_view,
            session_export,
            session_new,
            session_list,
            session_timeline,
            session_fork,
            session_rename,
            session_delete,
            stash_delete,
            model_provider_list,
            model_favorite_toggle,
            session_share,
            session_unshare,
            session_interrupt,
            session_compact,
            messages_page_up,
            messages_page_down,
            messages_line_up,
            messages_line_down,
            messages_half_page_up,
            messages_half_page_down,
            messages_first,
            messages_last,
            messages_next,
            messages_previous,
            messages_last_user,
            messages_copy,
            messages_undo,
            messages_redo,
            messages_toggle_conceal,
            tool_details,
            model_list,
            model_cycle_recent,
            model_cycle_recent_reverse,
            model_cycle_favorite,
            model_cycle_favorite_reverse,
            command_list,
            agent_list,
            agent_cycle,
            agent_cycle_reverse,
            variant_cycle,
            input_clear,
            input_paste,
            input_submit,
            input_newline,
            input_move_left,
            input_move_right,
            input_move_up,
            input_move_down,
            input_select_left,
            input_select_right,
            input_select_up,
            input_select_down,
            input_line_home,
            input_line_end,
            input_select_line_home,
            input_select_line_end,
            input_visual_line_home,
            input_visual_line_end,
            input_select_visual_line_home,
            input_select_visual_line_end,
            input_buffer_home,
            input_buffer_end,
            input_select_buffer_home,
            input_select_buffer_end,
            input_delete_line,
            input_delete_to_line_end,
            input_delete_to_line_start,
            input_backspace,
            input_delete,
            input_undo,
            input_redo,
            input_word_forward,
            input_word_backward,
            input_select_word_forward,
            input_select_word_backward,
            input_delete_word_forward,
            input_delete_word_backward,
            history_previous,
            history_next,
            session_child_cycle,
            session_child_cycle_reverse,
            session_parent,
            terminal_suspend,
            terminal_title_toggle,
            tips_toggle,
            display_thinking,
            submit,
            cancel,
            interrupt,
        );
    }
}

impl DeepMerge for TuiConfig {
    fn deep_merge(&mut self, other: Self) {
        merge_option_replace_fields!(
            self,
            other,
            mode,
            sidebar,
            scroll_speed,
            scroll_acceleration,
            diff_style,
        );
    }
}

impl DeepMerge for ServerConfig {
    fn deep_merge(&mut self, other: Self) {
        merge_option_replace_fields!(self, other, port, hostname, mdns, mdns_domain, cors,);
    }
}

impl CommandConfig {
    fn merge_replace_fields(&mut self, other: Self) {
        merge_option_replace_fields!(
            self,
            other,
            name,
            description,
            template,
            model,
            agent,
            subtask,
        );
    }
}

impl DeepMerge for CommandConfig {
    fn deep_merge(&mut self, other: Self) {
        self.merge_replace_fields(other);
    }
}

impl DeepMerge for SkillsConfig {
    fn deep_merge(&mut self, other: Self) {
        merge_vec_replace_if_non_empty(&mut self.paths, other.paths);
        merge_vec_replace_if_non_empty(&mut self.urls, other.urls);
        merge_option_deep(&mut self.hub, other.hub);
    }
}

impl DeepMerge for SkillHubConfig {
    fn deep_merge(&mut self, other: Self) {
        merge_option_replace(
            &mut self.artifact_cache_retention_seconds,
            other.artifact_cache_retention_seconds,
        );
        merge_option_replace(&mut self.fetch_timeout_ms, other.fetch_timeout_ms);
        merge_option_replace(&mut self.max_download_bytes, other.max_download_bytes);
        merge_option_replace(&mut self.max_extract_bytes, other.max_extract_bytes);
    }
}

impl DeepMerge for DocsConfig {
    fn deep_merge(&mut self, other: Self) {
        merge_option_replace(
            &mut self.context_docs_registry_path,
            other.context_docs_registry_path,
        );
    }
}

impl DeepMerge for WatcherConfig {
    fn deep_merge(&mut self, other: Self) {
        merge_vec_replace_if_non_empty(&mut self.ignore, other.ignore);
    }
}

impl AgentConfig {
    fn merge_replace_fields(
        &mut self,
        name: Option<String>,
        model: Option<String>,
        variant: Option<String>,
        temperature: Option<f32>,
        top_p: Option<f32>,
        prompt: Option<String>,
        disable: Option<bool>,
        description: Option<String>,
        mode: Option<AgentMode>,
        hidden: Option<bool>,
        color: Option<String>,
        steps: Option<u32>,
        max_tokens: Option<u64>,
        max_steps: Option<u32>,
    ) {
        merge_option_replace(&mut self.name, name);
        merge_option_replace(&mut self.model, model);
        merge_option_replace(&mut self.variant, variant);
        merge_option_replace(&mut self.temperature, temperature);
        merge_option_replace(&mut self.top_p, top_p);
        merge_option_replace(&mut self.prompt, prompt);
        merge_option_replace(&mut self.disable, disable);
        merge_option_replace(&mut self.description, description);
        merge_option_replace(&mut self.mode, mode);
        merge_option_replace(&mut self.hidden, hidden);
        merge_option_replace(&mut self.color, color);
        merge_option_replace(&mut self.steps, steps);
        merge_option_replace(&mut self.max_tokens, max_tokens);
        merge_option_replace(&mut self.max_steps, max_steps);
    }

    fn merge_json_fields(&mut self, options: Option<HashMap<String, serde_json::Value>>) {
        merge_option_json_map(&mut self.options, options);
    }

    fn merge_nested_fields(&mut self, permission: Option<PermissionConfig>) {
        merge_option_deep(&mut self.permission, permission);
    }

    fn merge_map_fields(&mut self, tools: Option<HashMap<String, bool>>) {
        merge_option_map_overwrite_values(&mut self.tools, tools);
    }
}

impl DeepMerge for AgentConfig {
    fn deep_merge(&mut self, other: Self) {
        let AgentConfig {
            name,
            model,
            variant,
            temperature,
            top_p,
            prompt,
            disable,
            description,
            mode,
            hidden,
            options,
            color,
            steps,
            max_steps,
            max_tokens,
            permission,
            tools,
        } = other;

        self.merge_replace_fields(
            name,
            model,
            variant,
            temperature,
            top_p,
            prompt,
            disable,
            description,
            mode,
            hidden,
            color,
            steps,
            max_tokens,
            max_steps,
        );
        self.merge_json_fields(options);
        self.merge_nested_fields(permission);
        self.merge_map_fields(tools);
    }
}

impl DeepMerge for AgentConfigs {
    fn deep_merge(&mut self, other: Self) {
        merge_map_deep_values(&mut self.entries, other.entries);
    }
}

impl DeepMerge for CompositionConfig {
    fn deep_merge(&mut self, other: Self) {
        merge_option_deep_fields!(self, other, skill_tree,);
    }
}

impl DeepMerge for SkillTreeConfig {
    fn deep_merge(&mut self, other: Self) {
        merge_option_replace_fields!(
            self,
            other,
            enabled,
            root,
            separator,
            token_budget,
            truncation_strategy,
        );
    }
}

impl DeepMerge for ModelModalities {
    fn deep_merge(&mut self, other: Self) {
        merge_option_replace_fields!(self, other, input, output,);
    }
}

impl DeepMerge for ModelCostConfig {
    fn deep_merge(&mut self, other: Self) {
        merge_option_replace_fields!(self, other, input, output, cache_read, cache_write,);
        merge_option_deep_fields!(self, other, context_over_200k,);
    }
}

impl DeepMerge for ModelLimitConfig {
    fn deep_merge(&mut self, other: Self) {
        merge_option_replace_fields!(self, other, context, input, output,);
    }
}

impl DeepMerge for ModelProviderConfig {
    fn deep_merge(&mut self, other: Self) {
        merge_option_replace_fields!(self, other, npm, api,);
    }
}

impl ModelConfig {
    // Scalar capability/identity fields use straightforward overlay semantics.
    fn merge_replace_fields(
        &mut self,
        name: Option<String>,
        model: Option<String>,
        api_key: Option<String>,
        base_url: Option<String>,
        tool_call: Option<bool>,
        reasoning: Option<bool>,
        attachment: Option<bool>,
        temperature: Option<bool>,
        interleaved: Option<ModelInterleavedConfig>,
        family: Option<String>,
        status: Option<String>,
        release_date: Option<String>,
        experimental: Option<bool>,
    ) {
        merge_option_replace(&mut self.name, name);
        merge_option_replace(&mut self.model, model);
        merge_option_replace(&mut self.api_key, api_key);
        merge_option_replace(&mut self.base_url, base_url);
        merge_option_replace(&mut self.tool_call, tool_call);
        merge_option_replace(&mut self.reasoning, reasoning);
        merge_option_replace(&mut self.attachment, attachment);
        merge_option_replace(&mut self.temperature, temperature);
        merge_option_replace(&mut self.interleaved, interleaved);
        merge_option_replace(&mut self.family, family);
        merge_option_replace(&mut self.status, status);
        merge_option_replace(&mut self.release_date, release_date);
        merge_option_replace(&mut self.experimental, experimental);
    }

    // Maps stay grouped so overwrite vs deep-merge policy is visible in one place.
    fn merge_map_fields(
        &mut self,
        variants: Option<HashMap<String, ModelVariantConfig>>,
        options: Option<HashMap<String, serde_json::Value>>,
        headers: Option<HashMap<String, String>>,
    ) {
        merge_option_map_deep_values(&mut self.variants, variants);
        merge_option_json_map(&mut self.options, options);
        merge_option_map_overwrite_values(&mut self.headers, headers);
    }

    // Nested config blocks recurse using each child type's own merge contract.
    fn merge_nested_fields(
        &mut self,
        modalities: Option<ModelModalities>,
        cost: Option<ModelCostConfig>,
        limit: Option<ModelLimitConfig>,
        provider: Option<ModelProviderConfig>,
    ) {
        merge_option_deep(&mut self.modalities, modalities);
        merge_option_deep(&mut self.cost, cost);
        merge_option_deep(&mut self.limit, limit);
        merge_option_deep(&mut self.provider, provider);
    }
}

impl DeepMerge for ModelConfig {
    fn deep_merge(&mut self, other: Self) {
        let ModelConfig {
            name,
            model,
            api_key,
            base_url,
            variants,
            tool_call,
            modalities,
            reasoning,
            attachment,
            temperature,
            interleaved,
            options,
            cost,
            limit,
            headers,
            family,
            status,
            release_date,
            experimental,
            provider,
        } = other;

        self.merge_replace_fields(
            name,
            model,
            api_key,
            base_url,
            tool_call,
            reasoning,
            attachment,
            temperature,
            interleaved,
            family,
            status,
            release_date,
            experimental,
        );
        self.merge_map_fields(variants, options, headers);
        self.merge_nested_fields(modalities, cost, limit, provider);
    }
}

impl DeepMerge for ModelVariantConfig {
    fn deep_merge(&mut self, other: Self) {
        merge_option_replace(&mut self.disabled, other.disabled);
        merge_map_overwrite_values(&mut self.extra, other.extra);
    }
}

impl ProviderConfig {
    // Provider identity/auth fields are scalar overlays.
    fn merge_replace_fields(
        &mut self,
        name: Option<String>,
        id: Option<String>,
        api_key: Option<String>,
        base_url: Option<String>,
        npm: Option<String>,
        api_style: Option<String>,
        api_shape: Option<String>,
        transport: Option<String>,
        usage_shape: Option<String>,
        env: Option<Vec<String>>,
    ) {
        merge_option_replace(&mut self.name, name);
        merge_option_replace(&mut self.id, id);
        merge_option_replace(&mut self.api_key, api_key);
        merge_option_replace(&mut self.base_url, base_url);
        merge_option_replace(&mut self.npm, npm);
        merge_option_replace(&mut self.api_style, api_style);
        merge_option_replace(&mut self.api_shape, api_shape);
        merge_option_replace(&mut self.transport, transport);
        merge_option_replace(&mut self.usage_shape, usage_shape);
        merge_option_replace(&mut self.env, env);
    }

    // Provider child model map and option bag use different merge semantics.
    fn merge_map_fields(
        &mut self,
        models: Option<HashMap<String, ModelConfig>>,
        options: Option<HashMap<String, serde_json::Value>>,
    ) {
        merge_option_map_deep_values(&mut self.models, models);
        merge_option_json_map(&mut self.options, options);
    }

    // Allow/deny lists are replace-if-non-empty, not union.
    fn merge_vec_fields(&mut self, whitelist: Vec<String>, blacklist: Vec<String>) {
        merge_vec_replace_if_non_empty(&mut self.whitelist, whitelist);
        merge_vec_replace_if_non_empty(&mut self.blacklist, blacklist);
    }
}

impl DeepMerge for ProviderConfig {
    fn deep_merge(&mut self, other: Self) {
        let ProviderConfig {
            name,
            id,
            api_key,
            base_url,
            models,
            options,
            npm,
            api_style,
            api_shape,
            transport,
            usage_shape,
            quirks,
            env,
            whitelist,
            blacklist,
        } = other;

        self.merge_replace_fields(
            name,
            id,
            api_key,
            base_url,
            npm,
            api_style,
            api_shape,
            transport,
            usage_shape,
            env,
        );
        self.merge_map_fields(models, options);
        merge_vec_replace_if_non_empty(&mut self.quirks, quirks);
        self.merge_vec_fields(whitelist, blacklist);
    }
}

impl McpServer {
    // Endpoint identity and switches overlay directly.
    fn merge_replace_fields(
        &mut self,
        server_type: Option<String>,
        url: Option<String>,
        enabled: Option<bool>,
        timeout: Option<u64>,
        oauth: Option<McpOAuthConfig>,
        client_id: Option<String>,
        authorization_url: Option<String>,
    ) {
        merge_option_replace(&mut self.server_type, server_type);
        merge_option_replace(&mut self.url, url);
        merge_option_replace(&mut self.enabled, enabled);
        merge_option_replace(&mut self.timeout, timeout);
        merge_option_replace(&mut self.oauth, oauth);
        merge_option_replace(&mut self.client_id, client_id);
        merge_option_replace(&mut self.authorization_url, authorization_url);
    }

    // Command/args vectors are replace-if-non-empty, matching existing launch semantics.
    fn merge_vec_fields(&mut self, command: Vec<String>, args: Vec<String>) {
        merge_vec_replace_if_non_empty(&mut self.command, command);
        merge_vec_replace_if_non_empty(&mut self.args, args);
    }

    // Environment/header bags overwrite by key rather than deep-merging values.
    fn merge_map_fields(
        &mut self,
        environment: Option<HashMap<String, String>>,
        headers: Option<HashMap<String, String>>,
        env: Option<HashMap<String, String>>,
    ) {
        merge_option_map_overwrite_values(&mut self.environment, environment);
        merge_option_map_overwrite_values(&mut self.headers, headers);
        merge_option_map_overwrite_values(&mut self.env, env);
    }
}

impl DeepMerge for McpServer {
    fn deep_merge(&mut self, other: Self) {
        let McpServer {
            server_type,
            command,
            environment,
            url,
            enabled,
            timeout,
            headers,
            oauth,
            args,
            env,
            client_id,
            authorization_url,
        } = other;

        self.merge_replace_fields(
            server_type,
            url,
            enabled,
            timeout,
            oauth,
            client_id,
            authorization_url,
        );
        self.merge_vec_fields(command, args);
        self.merge_map_fields(environment, headers, env);
    }
}

impl DeepMerge for McpServerConfig {
    fn deep_merge(&mut self, other: Self) {
        match other {
            McpServerConfig::Enabled { enabled } => match self {
                McpServerConfig::Enabled {
                    enabled: target_enabled,
                } => *target_enabled = enabled,
                McpServerConfig::Full(target_server) => target_server.enabled = Some(enabled),
            },
            McpServerConfig::Full(mut source_server) => match self {
                McpServerConfig::Full(target_server) => target_server.deep_merge(source_server),
                McpServerConfig::Enabled { enabled } => {
                    if source_server.enabled.is_none() {
                        source_server.enabled = Some(*enabled);
                    }
                    *self = McpServerConfig::Full(source_server);
                }
            },
        }
    }
}

impl FormatterEntry {
    fn merge_replace_fields(&mut self, disabled: Option<bool>) {
        merge_option_replace(&mut self.disabled, disabled);
    }

    fn merge_vec_fields(&mut self, command: Vec<String>, extensions: Vec<String>) {
        merge_vec_replace_if_non_empty(&mut self.command, command);
        merge_vec_replace_if_non_empty(&mut self.extensions, extensions);
    }

    fn merge_map_fields(&mut self, environment: Option<HashMap<String, String>>) {
        merge_option_map_overwrite_values(&mut self.environment, environment);
    }
}

impl DeepMerge for FormatterEntry {
    fn deep_merge(&mut self, other: Self) {
        let FormatterEntry {
            disabled,
            command,
            environment,
            extensions,
        } = other;

        self.merge_replace_fields(disabled);
        self.merge_vec_fields(command, extensions);
        self.merge_map_fields(environment);
    }
}

impl DeepMerge for FormatterConfig {
    fn deep_merge(&mut self, other: Self) {
        match other {
            FormatterConfig::Disabled(value) => *self = FormatterConfig::Disabled(value),
            FormatterConfig::Enabled(source_map) => match self {
                FormatterConfig::Disabled(_) => *self = FormatterConfig::Enabled(source_map),
                FormatterConfig::Enabled(target_map) => {
                    merge_map_deep_values(target_map, source_map);
                }
            },
        }
    }
}

impl LspServerConfig {
    fn merge_replace_fields(&mut self, disabled: Option<bool>) {
        merge_option_replace(&mut self.disabled, disabled);
    }

    fn merge_vec_fields(&mut self, command: Vec<String>, extensions: Vec<String>) {
        merge_vec_replace_if_non_empty(&mut self.command, command);
        merge_vec_replace_if_non_empty(&mut self.extensions, extensions);
    }

    fn merge_map_fields(
        &mut self,
        env: Option<HashMap<String, String>>,
        initialization: Option<HashMap<String, serde_json::Value>>,
    ) {
        merge_option_map_overwrite_values(&mut self.env, env);
        merge_option_json_map(&mut self.initialization, initialization);
    }
}

impl DeepMerge for LspServerConfig {
    fn deep_merge(&mut self, other: Self) {
        let LspServerConfig {
            command,
            extensions,
            disabled,
            env,
            initialization,
        } = other;

        self.merge_replace_fields(disabled);
        self.merge_vec_fields(command, extensions);
        self.merge_map_fields(env, initialization);
    }
}

impl DeepMerge for LspConfig {
    fn deep_merge(&mut self, other: Self) {
        match other {
            LspConfig::Disabled(value) => *self = LspConfig::Disabled(value),
            LspConfig::Enabled(source_map) => match self {
                LspConfig::Disabled(_) => *self = LspConfig::Enabled(source_map),
                LspConfig::Enabled(target_map) => {
                    merge_map_deep_values(target_map, source_map);
                }
            },
        }
    }
}

impl DeepMerge for PermissionConfig {
    fn deep_merge(&mut self, other: Self) {
        merge_map_overwrite_values(&mut self.rules, other.rules);
    }
}

impl DeepMerge for EnterpriseConfig {
    fn deep_merge(&mut self, other: Self) {
        merge_option_replace_fields!(self, other, url, managed_config_dir,);
    }
}

impl DeepMerge for CompactionConfig {
    fn deep_merge(&mut self, other: Self) {
        merge_option_replace_fields!(self, other, auto, prune, reserved,);
    }
}

impl ExperimentalConfig {
    fn merge_replace_fields(
        &mut self,
        disable_paste_summary: Option<bool>,
        batch_tool: Option<bool>,
        open_telemetry: Option<bool>,
        continue_loop_on_deny: Option<bool>,
        mcp_timeout: Option<u64>,
    ) {
        merge_option_replace(&mut self.disable_paste_summary, disable_paste_summary);
        merge_option_replace(&mut self.batch_tool, batch_tool);
        merge_option_replace(&mut self.open_telemetry, open_telemetry);
        merge_option_replace(&mut self.continue_loop_on_deny, continue_loop_on_deny);
        merge_option_replace(&mut self.mcp_timeout, mcp_timeout);
    }

    fn merge_vec_fields(&mut self, primary_tools: Vec<String>) {
        merge_vec_replace_if_non_empty(&mut self.primary_tools, primary_tools);
    }
}

impl DeepMerge for ExperimentalConfig {
    fn deep_merge(&mut self, other: Self) {
        let ExperimentalConfig {
            disable_paste_summary,
            batch_tool,
            open_telemetry,
            primary_tools,
            continue_loop_on_deny,
            mcp_timeout,
        } = other;

        self.merge_replace_fields(
            disable_paste_summary,
            batch_tool,
            open_telemetry,
            continue_loop_on_deny,
            mcp_timeout,
        );
        self.merge_vec_fields(primary_tools);
    }
}

impl WebSearchConfig {
    fn merge_replace_fields(
        &mut self,
        base_url: Option<String>,
        endpoint: Option<String>,
        method: Option<String>,
        default_search_type: Option<String>,
        default_num_results: Option<usize>,
    ) {
        merge_option_replace(&mut self.base_url, base_url);
        merge_option_replace(&mut self.endpoint, endpoint);
        merge_option_replace(&mut self.method, method);
        merge_option_replace(&mut self.default_search_type, default_search_type);
        merge_option_replace(&mut self.default_num_results, default_num_results);
    }

    fn merge_map_fields(&mut self, options: Option<HashMap<String, serde_json::Value>>) {
        merge_option_map_overwrite_values(&mut self.options, options);
    }
}

impl DeepMerge for WebSearchConfig {
    fn deep_merge(&mut self, other: Self) {
        let WebSearchConfig {
            base_url,
            endpoint,
            method,
            default_search_type,
            default_num_results,
            options,
        } = other;

        self.merge_replace_fields(
            base_url,
            endpoint,
            method,
            default_search_type,
            default_num_results,
        );
        self.merge_map_fields(options);
    }
}

impl DeepMerge for MultimodalConfig {
    fn deep_merge(&mut self, other: Self) {
        merge_option_deep_fields!(self, other, voice, limits, policy,);
    }
}

impl DeepMerge for MultimodalLimitsConfig {
    fn deep_merge(&mut self, other: Self) {
        merge_option_replace(&mut self.max_input_bytes, other.max_input_bytes);
        merge_option_replace(
            &mut self.max_attachments_per_prompt,
            other.max_attachments_per_prompt,
        );
    }
}

impl DeepMerge for MultimodalAttachmentPolicyConfig {
    fn deep_merge(&mut self, other: Self) {
        merge_option_replace_fields!(
            self,
            other,
            allow_audio_input,
            allow_image_input,
            allow_file_input,
        );
    }
}

impl VoiceConfig {
    fn merge_replace_fields(
        &mut self,
        duration_seconds: Option<u64>,
        attach_audio: Option<bool>,
        mime: Option<String>,
        language: Option<String>,
    ) {
        merge_option_replace(&mut self.duration_seconds, duration_seconds);
        merge_option_replace(&mut self.attach_audio, attach_audio);
        merge_option_replace(&mut self.mime, mime);
        merge_option_replace(&mut self.language, language);
    }

    fn merge_nested_fields(
        &mut self,
        record: Option<VoiceCommandConfig>,
        transcribe: Option<VoiceCommandConfig>,
    ) {
        merge_option_deep(&mut self.record, record);
        merge_option_deep(&mut self.transcribe, transcribe);
    }
}

impl DeepMerge for VoiceConfig {
    fn deep_merge(&mut self, other: Self) {
        let VoiceConfig {
            duration_seconds,
            attach_audio,
            mime,
            language,
            record,
            transcribe,
        } = other;

        self.merge_replace_fields(duration_seconds, attach_audio, mime, language);
        self.merge_nested_fields(record, transcribe);
    }
}

impl DeepMerge for VoiceCommandConfig {
    fn deep_merge(&mut self, other: Self) {
        merge_vec_replace_if_non_empty(&mut self.command, other.command);
        merge_map_overwrite_values(&mut self.env, other.env);
    }
}

impl DeepMerge for ExternalAdapterConfig {
    fn deep_merge(&mut self, other: Self) {
        merge_map_deep_values(&mut self.adapters, other.adapters);
        merge_option_deep(&mut self.replay, other.replay);
    }
}

impl DeepMerge for ExternalAdapterEntryConfig {
    fn deep_merge(&mut self, other: Self) {
        merge_option_replace_fields!(
            self,
            other,
            enabled,
            source,
            secret_ref,
            default_workspace,
            route_policy_id,
            allow_session_run,
        );
        merge_vec_replace_if_non_empty(&mut self.allowed_workspaces, other.allowed_workspaces);
    }
}

impl DeepMerge for ExternalAdapterReplayConfig {
    fn deep_merge(&mut self, other: Self) {
        merge_option_replace_fields!(self, other, retention_seconds, nonce_window_seconds,);
    }
}

impl DeepMerge for UiPreferencesConfig {
    fn deep_merge(&mut self, other: Self) {
        merge_option_replace_fields!(
            self,
            other,
            theme,
            web_theme,
            web_mode,
            show_header,
            show_scrollbar,
            tips_hidden,
            show_timestamps,
            show_thinking,
            show_tool_details,
            message_density,
            semantic_highlight,
        );
        merge_vec_replace_if_non_empty(&mut self.recent_models, other.recent_models);
    }
}

impl Config {
    // Top-level scalar selectors and toggles overwrite directly.
    fn merge_replace_fields(
        &mut self,
        schema: Option<String>,
        theme: Option<String>,
        log_level: Option<String>,
        scheduler_path: Option<String>,
        task_category_path: Option<String>,
        snapshot: Option<bool>,
        share: Option<ShareMode>,
        autoshare: Option<bool>,
        autoupdate: Option<AutoUpdateMode>,
        model: Option<String>,
        small_model: Option<String>,
        default_agent: Option<String>,
        username: Option<String>,
        layout: Option<LayoutMode>,
    ) {
        merge_option_replace(&mut self.schema, schema);
        merge_option_replace(&mut self.theme, theme);
        merge_option_replace(&mut self.log_level, log_level);
        merge_option_replace(&mut self.scheduler_path, scheduler_path);
        merge_option_replace(&mut self.task_category_path, task_category_path);
        merge_option_replace(&mut self.snapshot, snapshot);
        merge_option_replace(&mut self.share, share);
        merge_option_replace(&mut self.autoshare, autoshare);
        merge_option_replace(&mut self.autoupdate, autoupdate);
        merge_option_replace(&mut self.model, model);
        merge_option_replace(&mut self.small_model, small_model);
        merge_option_replace(&mut self.default_agent, default_agent);
        merge_option_replace(&mut self.username, username);
        merge_option_replace(&mut self.layout, layout);
    }

    // Nested config authorities recurse through their own merge contracts.
    fn merge_nested_fields(
        &mut self,
        keybinds: Option<KeybindsConfig>,
        tui: Option<TuiConfig>,
        server: Option<ServerConfig>,
        skills: Option<SkillsConfig>,
        docs: Option<DocsConfig>,
        watcher: Option<WatcherConfig>,
        mode: Option<AgentConfigs>,
        agent: Option<AgentConfigs>,
        composition: Option<CompositionConfig>,
        formatter: Option<FormatterConfig>,
        lsp: Option<LspConfig>,
        ui_preferences: Option<UiPreferencesConfig>,
        permission: Option<PermissionConfig>,
        web_search: Option<WebSearchConfig>,
        multimodal: Option<MultimodalConfig>,
        external_adapter: Option<ExternalAdapterConfig>,
        voice: Option<VoiceConfig>,
        enterprise: Option<EnterpriseConfig>,
        compaction: Option<CompactionConfig>,
        experimental: Option<ExperimentalConfig>,
    ) {
        merge_option_deep(&mut self.keybinds, keybinds);
        merge_option_deep(&mut self.tui, tui);
        merge_option_deep(&mut self.server, server);
        merge_option_deep(&mut self.skills, skills);
        merge_option_deep(&mut self.docs, docs);
        merge_option_deep(&mut self.watcher, watcher);
        merge_option_deep(&mut self.mode, mode);
        merge_option_deep(&mut self.agent, agent);
        merge_option_deep(&mut self.composition, composition);
        merge_option_deep(&mut self.formatter, formatter);
        merge_option_deep(&mut self.lsp, lsp);
        merge_option_deep(&mut self.ui_preferences, ui_preferences);
        merge_option_deep(&mut self.permission, permission);
        merge_option_deep(&mut self.web_search, web_search);
        merge_option_deep(&mut self.multimodal, multimodal);
        merge_option_deep(&mut self.external_adapter, external_adapter);
        merge_option_deep(&mut self.voice, voice);
        merge_option_deep(&mut self.enterprise, enterprise);
        merge_option_deep(&mut self.compaction, compaction);
        merge_option_deep(&mut self.experimental, experimental);
    }

    // Keep map semantics centralized: some maps deep-merge children, others overwrite values.
    fn merge_map_fields(
        &mut self,
        command: Option<HashMap<String, CommandConfig>>,
        provider: Option<HashMap<String, ProviderConfig>>,
        mcp: Option<HashMap<String, McpServerConfig>>,
        skill_paths: HashMap<String, String>,
        tools: Option<HashMap<String, bool>>,
        env: Option<HashMap<String, String>>,
        plugin_paths: HashMap<String, String>,
        plugin: HashMap<String, PluginConfig>,
    ) {
        merge_option_map_deep_values(&mut self.command, command);
        merge_option_map_deep_values(&mut self.provider, provider);
        merge_option_map_deep_values(&mut self.mcp, mcp);
        merge_map_overwrite_values(&mut self.skill_paths, skill_paths);
        merge_option_map_overwrite_values(&mut self.tools, tools);
        merge_option_map_overwrite_values(&mut self.env, env);
        merge_map_overwrite_values(&mut self.plugin_paths, plugin_paths);
        merge_map_overwrite_values(&mut self.plugin, plugin);
    }

    // Sequence-style fields intentionally stay separate because they are not all simple replace.
    fn merge_sequence_fields(
        &mut self,
        instructions: Vec<String>,
        disabled_providers: Vec<String>,
        enabled_providers: Vec<String>,
    ) {
        append_unique_keep_order(&mut self.instructions, instructions);
        merge_vec_replace_if_non_empty(&mut self.disabled_providers, disabled_providers);
        merge_vec_replace_if_non_empty(&mut self.enabled_providers, enabled_providers);
    }

    pub fn merge(&mut self, other: Config) {
        let Config {
            schema,
            theme,
            keybinds,
            log_level,
            tui,
            server,
            command,
            skills,
            docs,
            scheduler_path,
            task_category_path,
            skill_paths,
            watcher,
            plugin,
            plugin_paths,
            snapshot,
            share,
            autoshare,
            autoupdate,
            disabled_providers,
            enabled_providers,
            model,
            small_model,
            default_agent,
            username,
            mode,
            agent,
            composition,
            provider,
            mcp,
            formatter,
            lsp,
            instructions,
            layout,
            ui_preferences,
            permission,
            tools,
            web_search,
            multimodal,
            external_adapter,
            voice,
            enterprise,
            compaction,
            experimental,
            env,
        } = other;

        self.merge_replace_fields(
            schema,
            theme,
            log_level,
            scheduler_path,
            task_category_path,
            snapshot,
            share,
            autoshare,
            autoupdate,
            model,
            small_model,
            default_agent,
            username,
            layout,
        );
        self.merge_nested_fields(
            keybinds,
            tui,
            server,
            skills,
            docs,
            watcher,
            mode,
            agent,
            composition,
            formatter,
            lsp,
            ui_preferences,
            permission,
            web_search,
            multimodal,
            external_adapter,
            voice,
            enterprise,
            compaction,
            experimental,
        );
        self.merge_map_fields(
            command,
            provider,
            mcp,
            skill_paths,
            tools,
            env,
            plugin_paths,
            plugin,
        );
        self.merge_sequence_fields(instructions, disabled_providers, enabled_providers);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_merge_deep_merges_ui_preferences() {
        let mut config = Config {
            ui_preferences: Some(UiPreferencesConfig {
                theme: Some("opencode@dark".to_string()),
                web_theme: Some("midnight".to_string()),
                show_header: Some(true),
                recent_models: vec![UiRecentModelConfig {
                    provider: "openai".to_string(),
                    model: "gpt-5".to_string(),
                }],
                ..Default::default()
            }),
            ..Default::default()
        };

        config.merge(Config {
            ui_preferences: Some(UiPreferencesConfig {
                web_mode: Some("agent:atlas".to_string()),
                show_header: Some(false),
                show_thinking: Some(true),
                ..Default::default()
            }),
            ..Default::default()
        });

        let ui = config.ui_preferences.expect("ui preferences");
        assert_eq!(ui.theme.as_deref(), Some("opencode@dark"));
        assert_eq!(ui.web_theme.as_deref(), Some("midnight"));
        assert_eq!(ui.web_mode.as_deref(), Some("agent:atlas"));
        assert_eq!(ui.show_header, Some(false));
        assert_eq!(ui.show_thinking, Some(true));
        assert_eq!(
            ui.recent_models,
            vec![UiRecentModelConfig {
                provider: "openai".to_string(),
                model: "gpt-5".to_string(),
            }]
        );
    }

    #[test]
    fn config_merge_replaces_recent_models_when_supplied() {
        let mut config = Config {
            ui_preferences: Some(UiPreferencesConfig {
                recent_models: vec![UiRecentModelConfig {
                    provider: "openai".to_string(),
                    model: "gpt-5".to_string(),
                }],
                ..Default::default()
            }),
            ..Default::default()
        };

        config.merge(Config {
            ui_preferences: Some(UiPreferencesConfig {
                recent_models: vec![UiRecentModelConfig {
                    provider: "ethnopic".to_string(),
                    model: "test-model-large".to_string(),
                }],
                ..Default::default()
            }),
            ..Default::default()
        });

        let ui = config.ui_preferences.expect("ui preferences");
        assert_eq!(
            ui.recent_models,
            vec![UiRecentModelConfig {
                provider: "ethnopic".to_string(),
                model: "test-model-large".to_string(),
            }]
        );
    }
}
