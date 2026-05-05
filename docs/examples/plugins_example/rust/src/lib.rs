use std::future::Future;
use std::pin::Pin;

use rocode_plugin::{Hook, HookContext, HookEvent, HookOutput, Plugin, PluginSystem};

#[derive(Default)]
pub struct NativeDylibDemoPlugin;

impl Plugin for NativeDylibDemoPlugin {
    fn name(&self) -> &str {
        "native-dylib-demo"
    }

    fn version(&self) -> &str {
        "0.1.0"
    }

    fn register_hooks<'a>(
        &'a self,
        system: &'a PluginSystem,
    ) -> Pin<Box<dyn Future<Output = ()> + Send + 'a>> {
        Box::pin(async move {
            // Native dylib plugins use the HookEvent enum directly; this surface
            // is broader than the TS string-hook interface documented elsewhere.
            system
                .register(Hook::new(
                    "native:native-dylib-demo:session-start",
                    HookEvent::SessionStart,
                    |ctx: HookContext| async move {
                        let agent = ctx
                            .get("agent")
                            .cloned()
                            .unwrap_or(serde_json::Value::Null);
                        Ok(HookOutput::with_payload(serde_json::json!({
                            "native_demo_loaded": true,
                            "agent": agent,
                        })))
                    },
                ))
                .await;
        })
    }
}

rocode_plugin::declare_plugin!(NativeDylibDemoPlugin);
