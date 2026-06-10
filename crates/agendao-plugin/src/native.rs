//! Native (dylib) plugin loading via `libloading`.
//!
//! Rust plugins compiled as `cdylib` / `dylib` can be loaded at runtime
//! without spawning a separate process.  The plugin must be compiled with
//! the **same Rust compiler version** as agendao — Rust does not guarantee
//! a stable ABI across versions.
//!
//! # Plugin entry point
//!
//! The shared library must export a function named `agendao_plugin_create`
//! with the following signature:
//!
//! ```ignore
//! #[no_mangle]
//! pub fn agendao_plugin_create() -> Box<dyn agendao_plugin::Plugin> {
//!     Box::new(MyPlugin)
//! }
//! ```
//!
//! The convenience macro [`declare_plugin!`] generates this for you:
//!
//! ```ignore
//! agendao_plugin::declare_plugin!(MyPlugin);
//! ```

use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::{Plugin, PluginHookRef, PluginSystem};

/// Symbol name that every native plugin must export.
const ENTRY_SYMBOL: &[u8] = b"agendao_plugin_create";

/// Function signature of the plugin entry point.
///
/// # Safety
/// The function returns ownership of a heap-allocated `Box<dyn Plugin>`.
/// The caller takes ownership and is responsible for dropping it.
type CreateFn = unsafe fn() -> Box<dyn Plugin>;

/// A loaded native plugin together with its library handle.
///
/// The `_library` field **must** outlive `plugin` — dropping it unloads
/// the shared library and invalidates all function pointers.
pub struct NativePluginHandle {
    _library: libloading::Library,
    plugin: Arc<dyn Plugin>,
    path: PathBuf,
    hook_refs: Vec<PluginHookRef>,
}

impl std::fmt::Debug for NativePluginHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NativePluginHandle")
            .field("plugin_name", &self.plugin.name())
            .field("plugin_version", &self.plugin.version())
            .field("path", &self.path)
            .field("hook_refs", &self.hook_refs)
            .finish()
    }
}

impl NativePluginHandle {
    /// Load a native plugin from a shared library.
    ///
    /// # Safety
    ///
    /// The shared library **must**:
    /// - Be compiled with the same Rust compiler version as agendao.
    /// - Export `agendao_plugin_create` returning `Box<dyn Plugin>`.
    /// - Not have been tampered with (arbitrary code execution risk).
    pub unsafe fn load(path: &Path) -> anyhow::Result<Self> {
        let library = libloading::Library::new(path)
            .map_err(|e| anyhow::anyhow!("failed to load native plugin {:?}: {}", path, e))?;

        let create: libloading::Symbol<CreateFn> = library.get(ENTRY_SYMBOL).map_err(|e| {
            anyhow::anyhow!(
                "native plugin {:?} missing `agendao_plugin_create` symbol: {}",
                path,
                e
            )
        })?;

        let plugin: Box<dyn Plugin> = create();
        let plugin: Arc<dyn Plugin> = Arc::from(plugin);
        let hook_refs = plugin.hook_refs();

        tracing::info!(
            plugin_name = plugin.name(),
            plugin_version = plugin.version(),
            path = %path.display(),
            "loaded native plugin"
        );

        Ok(Self {
            _library: library,
            plugin,
            path: path.to_path_buf(),
            hook_refs,
        })
    }

    pub fn plugin(&self) -> &Arc<dyn Plugin> {
        &self.plugin
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

/// Manages loading and tracking of native (dylib) plugins.
///
/// # Lifecycle (AgenDao §9, P3.2)
///
/// | Phase      | Method        | What happens                          |
/// |------------|---------------|---------------------------------------|
/// | Load       | `load()`      | dylib loaded → `register_hooks()`     |
/// | Live       | `list()`      | query loaded plugins                  |
/// | Shutdown   | `shutdown()`  | hook remove → handles dropped         |
/// | Drop       | (implicit)    | remaining handles dropped (fallback)  |
pub struct NativePluginLoader {
    handles: Vec<NativePluginHandle>,
}

impl NativePluginLoader {
    pub fn new() -> Self {
        Self {
            handles: Vec::new(),
        }
    }

    /// Load a native plugin and register its hooks with the plugin system.
    ///
    /// # Safety
    ///
    /// See [`NativePluginHandle::load`] for safety requirements.
    pub async fn load(&mut self, path: &Path, system: &PluginSystem) -> anyhow::Result<()> {
        // Avoid loading the same library twice.
        let canonical = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
        if self
            .handles
            .iter()
            .any(|h| std::fs::canonicalize(&h.path).unwrap_or_else(|_| h.path.clone()) == canonical)
        {
            tracing::debug!(path = %path.display(), "native plugin already loaded, skipping");
            return Ok(());
        }

        let handle = unsafe { NativePluginHandle::load(path)? };
        handle.plugin.register_hooks(system).await;
        self.handles.push(handle);
        Ok(())
    }

    /// Load multiple native plugins from a list of paths.
    pub async fn load_all(
        &mut self,
        paths: &[PathBuf],
        system: &PluginSystem,
    ) -> Vec<anyhow::Error> {
        let mut errors = Vec::new();
        for path in paths {
            if let Err(e) = self.load(path, system).await {
                tracing::warn!(path = %path.display(), error = %e, "failed to load native plugin");
                errors.push(e);
            }
        }
        errors
    }

    /// Shut down all native plugins: remove registered hooks, then unload dylibs.
    ///
    /// After `shutdown()`, the loader is empty.
    pub async fn shutdown(&mut self, system: &PluginSystem) {
        for handle in self.handles.drain(..) {
            for hook_ref in &handle.hook_refs {
                let removed = system.remove(&hook_ref.event, &hook_ref.name).await;
                if !removed {
                    tracing::debug!(
                        plugin = handle.plugin.name(),
                        hook_name = %hook_ref.name,
                        event = ?hook_ref.event,
                        "native plugin hook already absent during shutdown"
                    );
                }
            }
            drop(handle); // drops NativePluginHandle → dylib unloaded
        }
    }

    /// List all loaded native plugins as (name, version, path).
    pub fn list(&self) -> Vec<(&str, &str, &Path)> {
        self.handles
            .iter()
            .map(|h| (h.plugin.name(), h.plugin.version(), h.path()))
            .collect()
    }

    /// Number of loaded native plugins.
    pub fn count(&self) -> usize {
        self.handles.len()
    }
}

impl Default for NativePluginLoader {
    fn default() -> Self {
        Self::new()
    }
}

/// Convenience macro for plugin authors to declare the entry point.
///
/// Usage:
/// ```ignore
/// use agendao_plugin::Plugin;
///
/// struct MyPlugin;
///
/// impl Plugin for MyPlugin {
///     fn name(&self) -> &str { "my-plugin" }
///     fn version(&self) -> &str { "0.1.0" }
///     fn hook_refs(&self) -> Vec<agendao_plugin::PluginHookRef> {
///         vec![agendao_plugin::PluginHookRef::new(
///             agendao_plugin::HookEvent::SessionStart,
///             "native:my-plugin:session-start",
///         )]
///     }
///     fn register_hooks(&self, system: &agendao_plugin::PluginSystem)
///         -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send + '_>>
///     {
///         Box::pin(async move { /* register hooks */ })
///     }
/// }
///
/// agendao_plugin::declare_plugin!(MyPlugin);
/// ```
#[macro_export]
macro_rules! declare_plugin {
    ($plugin_type:ty) => {
        #[no_mangle]
        pub fn agendao_plugin_create() -> Box<dyn $crate::Plugin> {
            Box::new(<$plugin_type>::default())
        }
    };
    ($constructor:expr) => {
        #[no_mangle]
        pub fn agendao_plugin_create() -> Box<dyn $crate::Plugin> {
            Box::new($constructor)
        }
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_plugin_loader_starts_empty() {
        let loader = NativePluginLoader::new();
        assert_eq!(loader.count(), 0);
        assert!(loader.list().is_empty());
    }

    #[test]
    fn load_nonexistent_library_fails() {
        let result = unsafe { NativePluginHandle::load(Path::new("/nonexistent/libfoo.so")) };
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("failed to load native plugin"));
    }

    #[tokio::test]
    async fn shutdown_clears_loader() {
        let mut loader = NativePluginLoader::new();
        let system = PluginSystem::new();
        assert_eq!(loader.count(), 0);
        // shutdown on empty loader is a no-op.
        loader.shutdown(&system).await;
        assert_eq!(loader.count(), 0);
        assert!(loader.list().is_empty());
    }
}
