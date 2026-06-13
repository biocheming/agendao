//! Form management hooks with validation.
//!
//! This module provides React-like form hooks for managing form state,
//! validation, and field watching using the fiber architecture.
//!
//! # Example
//!
//! ```rust,ignore
//! use reratui_fiber::hooks::{use_form, use_form_context, use_watch};
//!
//! #[component]
//! fn MyForm() -> Element {
//!     let form = use_form(
//!         FormConfig::builder()
//!             .field("email", "")
//!             .field("password", "")
//!             .validator("email", Validator::required("Email is required"))
//!             .validator("email", Validator::email("Invalid email format"))
//!             .on_submit(|values| {
//!                 println!("Form submitted: {:?}", values);
//!             })
//!             .build()
//!     );
//!
//!     rsx! {
//!         <Block>
//!             <FormField field_name={"email"} />
//!         </Block>
//!     }
//! }
//!
//! #[component]
//! fn FormField(field_name: &str) -> Element {
//!     let form = use_form_context();
//!     let value = use_watch(&form, field_name);
//!     
//!     rsx! { <Text text={value} /> }
//! }
//! ```

use regex::Regex;
use std::collections::HashMap;
use std::sync::{Arc, OnceLock};

use super::{use_context, use_context_provider, use_state};
use crate::fiber_tree::with_current_fiber;

// ============================================================================
// Validator
// ============================================================================

/// Validator for form fields.
///
/// Provides common validation rules like required, min/max length, email, etc.
#[derive(Clone)]
pub struct Validator {
    #[allow(clippy::type_complexity)]
    validate_fn: Arc<dyn Fn(&str) -> Option<String> + Send + Sync>,
}

impl Validator {
    /// Create a custom validator with a validation function.
    ///
    /// The function should return `Some(error_message)` if validation fails,
    /// or `None` if validation passes.
    pub fn custom<F>(validate_fn: F) -> Self
    where
        F: Fn(&str) -> Option<String> + Send + Sync + 'static,
    {
        Self {
            validate_fn: Arc::new(validate_fn),
        }
    }

    /// Validate a value, returning an error message if validation fails.
    pub fn validate(&self, value: &str) -> Option<String> {
        (self.validate_fn)(value)
    }

    /// Required field validator.
    pub fn required(message: &'static str) -> Self {
        Self::custom(move |value| {
            if value.trim().is_empty() {
                Some(message.to_string())
            } else {
                None
            }
        })
    }

    /// Minimum length validator.
    pub fn min_length(min: usize, message: &'static str) -> Self {
        Self::custom(move |value| {
            if value.len() < min {
                Some(message.to_string())
            } else {
                None
            }
        })
    }

    /// Maximum length validator.
    pub fn max_length(max: usize, message: &'static str) -> Self {
        Self::custom(move |value| {
            if value.len() > max {
                Some(message.to_string())
            } else {
                None
            }
        })
    }

    /// Email format validator.
    pub fn email(message: &'static str) -> Self {
        static EMAIL_REGEX: OnceLock<Regex> = OnceLock::new();

        Self::custom(move |value| {
            let regex = EMAIL_REGEX.get_or_init(|| {
                Regex::new(r"^[a-zA-Z0-9._%+-]+@[a-zA-Z0-9.-]+\.[a-zA-Z]{2,}$").unwrap()
            });

            if value.is_empty() || regex.is_match(value) {
                None
            } else {
                Some(message.to_string())
            }
        })
    }

    /// URL format validator.
    pub fn url(message: &'static str) -> Self {
        static URL_REGEX: OnceLock<Regex> = OnceLock::new();

        Self::custom(move |value| {
            let regex =
                URL_REGEX.get_or_init(|| Regex::new(r"^https?://[^\s/$.?#].[^\s]*$").unwrap());

            if value.is_empty() || regex.is_match(value) {
                None
            } else {
                Some(message.to_string())
            }
        })
    }

    /// Numeric validator.
    pub fn numeric(message: &'static str) -> Self {
        Self::custom(move |value| {
            if value.is_empty() || value.parse::<f64>().is_ok() {
                None
            } else {
                Some(message.to_string())
            }
        })
    }

    /// Integer validator.
    pub fn integer(message: &'static str) -> Self {
        Self::custom(move |value| {
            if value.is_empty() || value.parse::<i64>().is_ok() {
                None
            } else {
                Some(message.to_string())
            }
        })
    }

    /// Pattern (regex) validator.
    pub fn pattern(pattern: &'static str, message: &'static str) -> Self {
        Self::custom(move |value| {
            let regex = Regex::new(pattern).unwrap();
            if value.is_empty() || regex.is_match(value) {
                None
            } else {
                Some(message.to_string())
            }
        })
    }

    /// Minimum value validator (for numeric fields).
    pub fn min(min: f64, message: &'static str) -> Self {
        Self::custom(move |value| {
            if let Ok(num) = value.parse::<f64>() {
                if num < min {
                    Some(message.to_string())
                } else {
                    None
                }
            } else {
                None
            }
        })
    }

    /// Maximum value validator (for numeric fields).
    pub fn max(max: f64, message: &'static str) -> Self {
        Self::custom(move |value| {
            if let Ok(num) = value.parse::<f64>() {
                if num > max {
                    Some(message.to_string())
                } else {
                    None
                }
            } else {
                None
            }
        })
    }

    /// Range validator (for numeric fields).
    pub fn range(min: f64, max: f64, message: &'static str) -> Self {
        Self::custom(move |value| {
            if let Ok(num) = value.parse::<f64>() {
                if num < min || num > max {
                    Some(message.to_string())
                } else {
                    None
                }
            } else {
                None
            }
        })
    }

    /// Alphanumeric validator.
    pub fn alphanumeric(message: &'static str) -> Self {
        Self::custom(move |value| {
            if value.is_empty() || value.chars().all(|c| c.is_alphanumeric()) {
                None
            } else {
                Some(message.to_string())
            }
        })
    }

    /// Alpha (letters only) validator.
    pub fn alpha(message: &'static str) -> Self {
        Self::custom(move |value| {
            if value.is_empty() || value.chars().all(|c| c.is_alphabetic()) {
                None
            } else {
                Some(message.to_string())
            }
        })
    }

    /// Matches another value validator.
    pub fn matches(other_value: String, message: &'static str) -> Self {
        Self::custom(move |value| {
            if value == other_value {
                None
            } else {
                Some(message.to_string())
            }
        })
    }
}

// ============================================================================
// Form Configuration
// ============================================================================

/// Configuration for form initialization.
#[derive(Clone)]
pub struct FormConfig {
    /// Initial values for form fields.
    pub(crate) initial_values: HashMap<String, String>,
    /// Validators for each field.
    pub(crate) validators: HashMap<String, Vec<Validator>>,
    /// Callback when form is submitted.
    pub(crate) on_submit: Arc<dyn Fn(HashMap<String, String>) + Send + Sync>,
}

impl FormConfig {
    /// Create a new FormConfig builder.
    pub fn builder() -> FormConfigBuilder {
        FormConfigBuilder::new()
    }
}

/// Builder for creating FormConfig with a fluent API.
pub struct FormConfigBuilder {
    initial_values: HashMap<String, String>,
    validators: HashMap<String, Vec<Validator>>,
    #[allow(clippy::type_complexity)]
    on_submit: Option<Arc<dyn Fn(HashMap<String, String>) + Send + Sync>>,
}

impl FormConfigBuilder {
    /// Create a new form configuration builder.
    pub fn new() -> Self {
        Self {
            initial_values: HashMap::new(),
            validators: HashMap::new(),
            on_submit: None,
        }
    }

    /// Add a field with an initial value.
    pub fn field(mut self, name: impl Into<String>, initial_value: impl Into<String>) -> Self {
        self.initial_values
            .insert(name.into(), initial_value.into());
        self
    }

    /// Add multiple fields at once.
    pub fn fields(mut self, fields: HashMap<String, String>) -> Self {
        self.initial_values.extend(fields);
        self
    }

    /// Add validators for a specific field.
    pub fn validate(mut self, field: impl Into<String>, validators: Vec<Validator>) -> Self {
        self.validators.insert(field.into(), validators);
        self
    }

    /// Add a single validator for a field.
    pub fn validator(mut self, field: impl Into<String>, validator: Validator) -> Self {
        let field = field.into();
        self.validators.entry(field).or_default().push(validator);
        self
    }

    /// Set the submit handler.
    pub fn on_submit<F>(mut self, handler: F) -> Self
    where
        F: Fn(HashMap<String, String>) + Send + Sync + 'static,
    {
        self.on_submit = Some(Arc::new(handler));
        self
    }

    /// Build the final FormConfig.
    ///
    /// # Panics
    ///
    /// Panics if `on_submit` was not set.
    pub fn build(self) -> FormConfig {
        FormConfig {
            initial_values: self.initial_values,
            validators: self.validators,
            on_submit: self.on_submit.expect(
                "on_submit handler must be set. Use build_with_default_submit() for a no-op handler.",
            ),
        }
    }

    /// Build the FormConfig with a default no-op submit handler.
    pub fn build_with_default_submit(self) -> FormConfig {
        FormConfig {
            initial_values: self.initial_values,
            validators: self.validators,
            on_submit: self.on_submit.unwrap_or_else(|| Arc::new(|_| {})),
        }
    }
}

impl Default for FormConfigBuilder {
    fn default() -> Self {
        Self::new()
    }
}

// ============================================================================
// Form State (internal)
// ============================================================================

/// Internal form state stored in fiber hooks.
#[derive(Clone)]
pub struct FormState {
    pub values: HashMap<String, String>,
    pub errors: HashMap<String, String>,
    pub touched: HashMap<String, bool>,
    pub is_submitting: bool,
    pub is_valid: bool,
}

impl FormState {
    fn new(initial_values: HashMap<String, String>) -> Self {
        Self {
            values: initial_values,
            errors: HashMap::new(),
            touched: HashMap::new(),
            is_submitting: false,
            is_valid: true,
        }
    }
}

// ============================================================================
// Form Handle
// ============================================================================

/// Handle for interacting with form state.
///
/// Provides methods to get/set field values, validate fields, and submit the form.
#[derive(Clone)]
pub struct FormHandle {
    pub(crate) fiber_id: crate::fiber::FiberId,
    pub(crate) hook_index: usize,
    pub(crate) validators: HashMap<String, Vec<Validator>>,
    pub(crate) on_submit: Arc<dyn Fn(HashMap<String, String>) + Send + Sync>,
}

impl FormHandle {
    /// Register a field and get its registration info.
    pub fn register(&self, name: &str) -> FieldRegistration {
        FieldRegistration {
            name: name.to_string(),
            value: self.get_value(name).unwrap_or_default(),
            error: self.get_error(name),
            touched: self.is_touched(name),
        }
    }

    /// Get the current value of a field.
    pub fn get_value(&self, name: &str) -> Option<String> {
        self.with_state(|state| state.values.get(name).cloned())
    }

    /// Set the value of a field.
    pub fn set_value(&self, name: &str, value: String) {
        let name = name.to_string();
        let validators = self.validators.clone();

        self.update_state(move |state| {
            state.values.insert(name.clone(), value.clone());

            // Validate field if it has been touched
            if state.touched.get(&name).copied().unwrap_or(false) {
                if let Some(field_validators) = validators.get(&name) {
                    for validator in field_validators {
                        if let Some(error) = validator.validate(&value) {
                            state.errors.insert(name.clone(), error);
                            return;
                        }
                    }
                }
                state.errors.remove(&name);
            }
        });
    }

    /// Get the error message for a field.
    pub fn get_error(&self, name: &str) -> Option<String> {
        self.with_state(|state| state.errors.get(name).cloned())
    }

    /// Set the error for a field.
    pub fn set_error(&self, name: &str, error: Option<String>) {
        let name = name.to_string();
        self.update_state(move |state| {
            if let Some(err) = error {
                state.errors.insert(name, err);
            } else {
                state.errors.remove(&name);
            }
        });
    }

    /// Check if a field has been touched.
    pub fn is_touched(&self, name: &str) -> bool {
        self.with_state(|state| state.touched.get(name).copied().unwrap_or(false))
    }

    /// Mark a field as touched.
    pub fn set_touched(&self, name: &str, is_touched: bool) {
        let name = name.to_string();
        self.update_state(move |state| {
            state.touched.insert(name, is_touched);
        });
    }

    /// Validate a specific field.
    pub fn validate_field(&self, name: &str, value: &str) -> bool {
        if let Some(validators) = self.validators.get(name) {
            for validator in validators {
                if let Some(error) = validator.validate(value) {
                    self.set_error(name, Some(error));
                    return false;
                }
            }
        }
        self.set_error(name, None);
        true
    }

    /// Validate all fields in the form.
    pub fn validate_all(&self) -> bool {
        let values = self.get_values();
        let mut all_valid = true;

        for (name, value) in values.iter() {
            if !self.validate_field(name, value) {
                all_valid = false;
            }
        }

        self.update_state(move |state| {
            state.is_valid = all_valid;
        });

        all_valid
    }

    /// Reset the form to initial values.
    pub fn reset(&self, initial_values: HashMap<String, String>) {
        self.update_state(move |state| {
            state.values = initial_values;
            state.errors = HashMap::new();
            state.touched = HashMap::new();
            state.is_submitting = false;
            state.is_valid = true;
        });
    }

    /// Submit the form.
    pub fn submit(&self) {
        // Mark all fields as touched
        let values = self.get_values();
        let mut touched = HashMap::new();
        for name in values.keys() {
            touched.insert(name.clone(), true);
        }

        self.update_state(move |state| {
            state.touched = touched;
        });

        // Validate all fields
        if self.validate_all() {
            self.update_state(|state| {
                state.is_submitting = true;
            });

            let values = self.get_values();
            (self.on_submit)(values);

            self.update_state(|state| {
                state.is_submitting = false;
            });
        }
    }

    /// Check if the form is currently submitting.
    pub fn is_submitting(&self) -> bool {
        self.with_state(|state| state.is_submitting)
    }

    /// Check if the form is valid.
    pub fn is_valid(&self) -> bool {
        self.with_state(|state| state.is_valid)
    }

    /// Get all form values.
    pub fn get_values(&self) -> HashMap<String, String> {
        self.with_state(|state| state.values.clone())
    }

    /// Get all errors.
    pub fn get_errors(&self) -> HashMap<String, String> {
        self.with_state(|state| state.errors.clone())
    }

    /// Check if the form has any errors.
    pub fn has_errors(&self) -> bool {
        self.with_state(|state| !state.errors.is_empty())
    }

    /// Check if any field is dirty (modified from initial value).
    pub fn is_dirty(&self) -> bool {
        self.with_state(|state| !state.touched.is_empty())
    }

    // Internal helper to read state
    fn with_state<R, F: FnOnce(&FormState) -> R>(&self, f: F) -> R {
        crate::fiber_tree::with_fiber_tree(|tree| {
            let fiber = tree.get(self.fiber_id).expect("Fiber not found");
            let state = fiber
                .get_hook::<FormState>(self.hook_index)
                .expect("Form state not found");
            f(&state)
        })
        .expect("with_state must be called within a fiber context")
    }

    // Internal helper to update state
    fn update_state<F: FnOnce(&mut FormState) + Send + 'static>(&self, f: F) {
        use crate::scheduler::batch::{StateUpdate, StateUpdateKind, queue_update};

        queue_update(
            self.fiber_id,
            StateUpdate {
                hook_index: self.hook_index,
                update: StateUpdateKind::Updater(Box::new(move |any| {
                    let mut state = any
                        .downcast_ref::<FormState>()
                        .expect("Form state type mismatch")
                        .clone();
                    f(&mut state);
                    Box::new(state)
                })),
            },
        );
    }
}

// ============================================================================
// Field Registration
// ============================================================================

/// Field registration information.
#[derive(Debug, Clone)]
pub struct FieldRegistration {
    /// Field name.
    pub name: String,
    /// Current field value.
    pub value: String,
    /// Current error message, if any.
    pub error: Option<String>,
    /// Whether the field has been touched.
    pub touched: bool,
}

impl FieldRegistration {
    /// Check if the field has an error.
    pub fn has_error(&self) -> bool {
        self.error.is_some()
    }

    /// Get the error message.
    pub fn error_message(&self) -> Option<&str> {
        self.error.as_deref()
    }
}

// ============================================================================
// Form Hooks
// ============================================================================

/// Form hook for managing form state, validation, and submission.
///
/// Automatically provides the form to child components via context,
/// allowing them to access it using `use_form_context()`.
///
/// # Example
///
/// ```rust,ignore
/// use reratui_fiber::hooks::{use_form, FormConfig, Validator};
///
/// #[component]
/// fn MyForm() -> Element {
///     let form = use_form(
///         FormConfig::builder()
///             .field("email", "")
///             .field("password", "")
///             .validator("email", Validator::required("Email is required"))
///             .validator("email", Validator::email("Invalid email format"))
///             .on_submit(|values| {
///                 println!("Form submitted: {:?}", values);
///             })
///             .build()
///     );
///
///     let email_reg = form.register("email");
///
///     rsx! {
///         <Block>
///             <FormField field_name={"email"} />
///         </Block>
///     }
/// }
/// ```
pub fn use_form(config: FormConfig) -> FormHandle {
    let (fiber_id, hook_index) = with_current_fiber(|fiber| {
        let hook_index = fiber.next_hook_index();

        // Initialize form state if not already present
        if fiber.get_hook::<FormState>(hook_index).is_none() {
            fiber.set_hook(hook_index, FormState::new(config.initial_values.clone()));
        }

        (fiber.id, hook_index)
    })
    .expect("use_form must be called within a component render context");

    let handle = FormHandle {
        fiber_id,
        hook_index,
        validators: config.validators,
        on_submit: config.on_submit,
    };

    // Provide form to child components via context
    use_context_provider(|| handle.clone());

    handle
}

/// Retrieves the form context from a parent component.
///
/// This hook allows child components to access the form state without
/// having to pass it through props.
///
/// # Panics
///
/// Panics if called outside of a component that has a `use_form()` ancestor.
///
/// # Example
///
/// ```rust,ignore
/// #[component]
/// fn FormField(field_name: &str) -> Element {
///     let form = use_form_context();
///     let registration = form.register(field_name);
///     
///     rsx! {
///         <Block>
///             <Paragraph>{registration.value}</Paragraph>
///         </Block>
///     }
/// }
/// ```
pub fn use_form_context() -> FormHandle {
    use_context::<FormHandle>()
}

/// Try to retrieve the form context, returning None if not available.
pub fn try_use_form_context() -> Option<FormHandle> {
    super::try_use_context::<FormHandle>()
}

// ============================================================================
// Watch Hooks
// ============================================================================

/// Watch a single field value and re-render when it changes.
///
/// # Example
///
/// ```rust,ignore
/// #[component]
/// fn MyComponent() -> Element {
///     let form = use_form_context();
///     let email = use_watch(&form, "email");
///     
///     rsx! {
///         <Paragraph>{format!("Email: {}", email)}</Paragraph>
///     }
/// }
/// ```
pub fn use_watch(form: &FormHandle, field_name: &str) -> String {
    // Get the current value from the form's fiber state directly within the current fiber context
    let current_value = with_current_fiber(|fiber| {
        fiber
            .get_hook::<FormState>(form.hook_index)
            .map(|state| state.values.get(field_name).cloned().unwrap_or_default())
            .unwrap_or_default()
    })
    .unwrap_or_default();

    let (value, set_value) = use_state(|| current_value.clone());

    // If the form value differs from our tracked value, update it
    if current_value != value {
        set_value.set(current_value.clone());
    }

    current_value
}

/// Watch multiple field values and re-render when any of them change.
///
/// # Example
///
/// ```rust,ignore
/// #[component]
/// fn MyComponent() -> Element {
///     let form = use_form_context();
///     let values = use_watch_multiple(&form, &["email", "username"]);
///     
///     rsx! {
///         <Paragraph>{format!("Email: {}, Username: {}",
///             values.get("email").unwrap_or(&String::new()),
///             values.get("username").unwrap_or(&String::new())
///         )}</Paragraph>
///     }
/// }
/// ```
pub fn use_watch_multiple(form: &FormHandle, field_names: &[&str]) -> HashMap<String, String> {
    // Get current values from the form's fiber state directly
    let current_values: HashMap<String, String> = with_current_fiber(|fiber| {
        fiber
            .get_hook::<FormState>(form.hook_index)
            .map(|state| {
                field_names
                    .iter()
                    .map(|name| {
                        (
                            name.to_string(),
                            state.values.get(*name).cloned().unwrap_or_default(),
                        )
                    })
                    .collect()
            })
            .unwrap_or_default()
    })
    .unwrap_or_default();

    let (values, set_values) = use_state(|| current_values.clone());

    // Check if any values have changed
    if current_values != values {
        set_values.set(current_values.clone());
    }

    current_values
}

/// Watch all form values and re-render when any value changes.
///
/// # Example
///
/// ```rust,ignore
/// #[component]
/// fn FormDebugger() -> Element {
///     let form = use_form_context();
///     let all_values = use_watch_all(&form);
///     
///     rsx! {
///         <Block title={"Form Values"}>
///             {all_values.iter().map(|(key, value)| {
///                 rsx! {
///                     <Paragraph>{format!("{}: {}", key, value)}</Paragraph>
///                 }
///             })}
///         </Block>
///     }
/// }
/// ```
pub fn use_watch_all(form: &FormHandle) -> HashMap<String, String> {
    // Get current values from the form's fiber state directly
    let current_values: HashMap<String, String> = with_current_fiber(|fiber| {
        fiber
            .get_hook::<FormState>(form.hook_index)
            .map(|state| state.values.clone())
            .unwrap_or_default()
    })
    .unwrap_or_default();

    let (values, set_values) = use_state(|| current_values.clone());

    if current_values != values {
        set_values.set(current_values.clone());
    }

    current_values
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context_stack::clear_context_stack;
    use crate::fiber::FiberId;
    use crate::fiber_tree::{FiberTree, clear_fiber_tree, set_fiber_tree};

    fn setup_test_fiber() -> FiberId {
        let mut tree = FiberTree::new();
        let fiber_id = tree.mount(None, None);
        tree.begin_render(fiber_id);
        set_fiber_tree(tree);
        fiber_id
    }

    fn cleanup_test() {
        clear_fiber_tree();
        clear_context_stack();
        crate::scheduler::batch::clear_state_batch();
    }

    fn apply_batch_and_rerender(fiber_id: FiberId) {
        crate::fiber_tree::with_fiber_tree_mut(|tree| {
            tree.end_render();
            crate::scheduler::batch::with_state_batch_mut(|batch| {
                batch.end_batch(tree);
            });
            tree.begin_render(fiber_id);
        });
    }

    // ========================================================================
    // Validator Tests
    // ========================================================================

    #[test]
    fn test_validator_required() {
        let validator = Validator::required("Field is required");

        assert!(validator.validate("").is_some());
        assert!(validator.validate("   ").is_some());
        assert!(validator.validate("value").is_none());
    }

    #[test]
    fn test_validator_min_length() {
        let validator = Validator::min_length(5, "Must be at least 5 characters");

        assert!(validator.validate("abc").is_some());
        assert!(validator.validate("abcde").is_none());
        assert!(validator.validate("abcdef").is_none());
    }

    #[test]
    fn test_validator_max_length() {
        let validator = Validator::max_length(5, "Must be at most 5 characters");

        assert!(validator.validate("abcdef").is_some());
        assert!(validator.validate("abcde").is_none());
        assert!(validator.validate("abc").is_none());
    }

    #[test]
    fn test_validator_email() {
        let validator = Validator::email("Invalid email");

        assert!(validator.validate("invalid").is_some());
        assert!(validator.validate("test@").is_some());
        assert!(validator.validate("test@example.com").is_none());
        assert!(validator.validate("user.name+tag@example.co.uk").is_none());
        assert!(validator.validate("").is_none()); // Empty is valid (use required for that)
    }

    #[test]
    fn test_validator_numeric() {
        let validator = Validator::numeric("Must be a number");

        assert!(validator.validate("abc").is_some());
        assert!(validator.validate("123").is_none());
        assert!(validator.validate("123.45").is_none());
        assert!(validator.validate("-123.45").is_none());
    }

    #[test]
    fn test_validator_range() {
        let validator = Validator::range(0.0, 100.0, "Must be between 0 and 100");

        assert!(validator.validate("-1").is_some());
        assert!(validator.validate("101").is_some());
        assert!(validator.validate("50").is_none());
        assert!(validator.validate("0").is_none());
        assert!(validator.validate("100").is_none());
    }

    #[test]
    fn test_validator_custom() {
        let validator = Validator::custom(|value| {
            if value.starts_with("test") {
                None
            } else {
                Some("Must start with 'test'".to_string())
            }
        });

        assert!(validator.validate("hello").is_some());
        assert!(validator.validate("test123").is_none());
    }

    // ========================================================================
    // FormConfig Tests
    // ========================================================================

    #[test]
    fn test_form_config_builder() {
        let config = FormConfig::builder()
            .field("email", "test@example.com")
            .field("name", "John")
            .validator("email", Validator::required("Email required"))
            .on_submit(|_| {})
            .build();

        assert_eq!(
            config.initial_values.get("email"),
            Some(&"test@example.com".to_string())
        );
        assert_eq!(config.initial_values.get("name"), Some(&"John".to_string()));
        assert!(config.validators.contains_key("email"));
    }

    #[test]
    fn test_form_config_builder_with_default_submit() {
        let config = FormConfig::builder()
            .field("email", "")
            .build_with_default_submit();

        assert!(config.initial_values.contains_key("email"));
    }

    #[test]
    #[should_panic(expected = "on_submit handler must be set")]
    fn test_form_config_builder_panics_without_submit() {
        let _ = FormConfig::builder().field("email", "").build();
    }

    // ========================================================================
    // use_form Tests
    // ========================================================================

    #[test]
    fn test_use_form_initializes_state() {
        let _fiber_id = setup_test_fiber();

        let form = use_form(
            FormConfig::builder()
                .field("email", "test@example.com")
                .field("name", "John")
                .on_submit(|_| {})
                .build(),
        );

        assert_eq!(
            form.get_value("email"),
            Some("test@example.com".to_string())
        );
        assert_eq!(form.get_value("name"), Some("John".to_string()));
        assert!(!form.is_submitting());
        assert!(form.is_valid());

        cleanup_test();
    }

    #[test]
    fn test_use_form_set_value() {
        let fiber_id = setup_test_fiber();

        let form = use_form(
            FormConfig::builder()
                .field("email", "")
                .on_submit(|_| {})
                .build(),
        );

        form.set_value("email", "new@example.com".to_string());

        // Apply batch and re-render
        apply_batch_and_rerender(fiber_id);

        // Re-create form handle to get updated state
        let form = use_form(
            FormConfig::builder()
                .field("email", "")
                .on_submit(|_| {})
                .build(),
        );

        assert_eq!(form.get_value("email"), Some("new@example.com".to_string()));

        cleanup_test();
    }

    #[test]
    fn test_use_form_touched_state() {
        let fiber_id = setup_test_fiber();

        let form = use_form(
            FormConfig::builder()
                .field("email", "")
                .on_submit(|_| {})
                .build(),
        );

        assert!(!form.is_touched("email"));

        form.set_touched("email", true);
        apply_batch_and_rerender(fiber_id);

        let form = use_form(
            FormConfig::builder()
                .field("email", "")
                .on_submit(|_| {})
                .build(),
        );

        assert!(form.is_touched("email"));

        cleanup_test();
    }

    #[test]
    fn test_use_form_validation() {
        let _fiber_id = setup_test_fiber();

        let form = use_form(
            FormConfig::builder()
                .field("email", "")
                .validator("email", Validator::required("Email is required"))
                .on_submit(|_| {})
                .build(),
        );

        // Validate empty field
        let is_valid = form.validate_field("email", "");
        assert!(!is_valid);

        // Validate non-empty field
        let is_valid = form.validate_field("email", "test@example.com");
        assert!(is_valid);

        cleanup_test();
    }

    #[test]
    fn test_use_form_register() {
        let _fiber_id = setup_test_fiber();

        let form = use_form(
            FormConfig::builder()
                .field("email", "test@example.com")
                .on_submit(|_| {})
                .build(),
        );

        let registration = form.register("email");

        assert_eq!(registration.name, "email");
        assert_eq!(registration.value, "test@example.com");
        assert!(registration.error.is_none());
        assert!(!registration.touched);

        cleanup_test();
    }

    #[test]
    fn test_use_form_reset() {
        let fiber_id = setup_test_fiber();

        let form = use_form(
            FormConfig::builder()
                .field("email", "initial@example.com")
                .on_submit(|_| {})
                .build(),
        );

        form.set_value("email", "changed@example.com".to_string());
        form.set_touched("email", true);
        apply_batch_and_rerender(fiber_id);

        let form = use_form(
            FormConfig::builder()
                .field("email", "initial@example.com")
                .on_submit(|_| {})
                .build(),
        );

        // Reset to new initial values
        let mut new_initial = HashMap::new();
        new_initial.insert("email".to_string(), "reset@example.com".to_string());
        form.reset(new_initial);

        apply_batch_and_rerender(fiber_id);

        let form = use_form(
            FormConfig::builder()
                .field("email", "initial@example.com")
                .on_submit(|_| {})
                .build(),
        );

        assert_eq!(
            form.get_value("email"),
            Some("reset@example.com".to_string())
        );
        assert!(!form.is_touched("email"));

        cleanup_test();
    }

    #[test]
    fn test_use_form_get_all_values() {
        let _fiber_id = setup_test_fiber();

        let form = use_form(
            FormConfig::builder()
                .field("email", "test@example.com")
                .field("name", "John")
                .on_submit(|_| {})
                .build(),
        );

        let values = form.get_values();

        assert_eq!(values.len(), 2);
        assert_eq!(values.get("email"), Some(&"test@example.com".to_string()));
        assert_eq!(values.get("name"), Some(&"John".to_string()));

        cleanup_test();
    }

    // ========================================================================
    // use_form_context Tests
    // ========================================================================

    #[test]
    fn test_use_form_context() {
        let _fiber_id = setup_test_fiber();

        // Create form (which provides context)
        let _form = use_form(
            FormConfig::builder()
                .field("email", "context@example.com")
                .on_submit(|_| {})
                .build(),
        );

        // Get form from context
        let form_from_context = use_form_context();

        assert_eq!(
            form_from_context.get_value("email"),
            Some("context@example.com".to_string())
        );

        cleanup_test();
    }

    #[test]
    fn test_try_use_form_context_returns_none() {
        cleanup_test(); // Ensure clean state

        let result = try_use_form_context();
        assert!(result.is_none());
    }

    // ========================================================================
    // use_watch Tests
    // ========================================================================

    #[test]
    fn test_use_watch() {
        let _fiber_id = setup_test_fiber();

        let form = use_form(
            FormConfig::builder()
                .field("email", "watch@example.com")
                .on_submit(|_| {})
                .build(),
        );

        let value = use_watch(&form, "email");

        assert_eq!(value, "watch@example.com");

        cleanup_test();
    }

    #[test]
    fn test_use_watch_multiple() {
        let _fiber_id = setup_test_fiber();

        let form = use_form(
            FormConfig::builder()
                .field("email", "test@example.com")
                .field("name", "John")
                .on_submit(|_| {})
                .build(),
        );

        let values = use_watch_multiple(&form, &["email", "name"]);

        assert_eq!(values.get("email"), Some(&"test@example.com".to_string()));
        assert_eq!(values.get("name"), Some(&"John".to_string()));

        cleanup_test();
    }

    #[test]
    fn test_use_watch_all() {
        let _fiber_id = setup_test_fiber();

        let form = use_form(
            FormConfig::builder()
                .field("email", "test@example.com")
                .field("name", "John")
                .field("age", "30")
                .on_submit(|_| {})
                .build(),
        );

        let values = use_watch_all(&form);

        assert_eq!(values.len(), 3);
        assert_eq!(values.get("email"), Some(&"test@example.com".to_string()));
        assert_eq!(values.get("name"), Some(&"John".to_string()));
        assert_eq!(values.get("age"), Some(&"30".to_string()));

        cleanup_test();
    }

    // ========================================================================
    // FieldRegistration Tests
    // ========================================================================

    #[test]
    fn test_field_registration_has_error() {
        let reg_with_error = FieldRegistration {
            name: "email".to_string(),
            value: "".to_string(),
            error: Some("Required".to_string()),
            touched: true,
        };

        let reg_without_error = FieldRegistration {
            name: "email".to_string(),
            value: "test@example.com".to_string(),
            error: None,
            touched: true,
        };

        assert!(reg_with_error.has_error());
        assert_eq!(reg_with_error.error_message(), Some("Required"));

        assert!(!reg_without_error.has_error());
        assert_eq!(reg_without_error.error_message(), None);
    }
}
