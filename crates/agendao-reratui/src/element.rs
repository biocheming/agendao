//! Element module for the virtual DOM tree.
//!
//! This module provides the `Element` type which represents nodes in the virtual DOM tree.
//! It supports widgets, fiber-based components (Component), text nodes, and fragments.

use ratatui::{
    buffer::Buffer,
    layout::{Constraint, Direction, Layout, Rect},
    widgets::Widget,
};
use std::{any::Any, rc::Rc};

/// A trait for components that can be rendered with fiber management.
///
/// This trait is implemented by `ComponentWrapper` to enable fiber-based
/// components to be used in the Element system.
pub trait RenderableComponent: 'static {
    /// Render the component with proper fiber management.
    fn render_with_fiber(&self, area: Rect, buffer: &mut Buffer);

    /// Clone the wrapper into a boxed trait object.
    fn clone_box(&self) -> Box<dyn RenderableComponent>;

    /// Debug representation for the component.
    fn debug_fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result;
}

impl Clone for Box<dyn RenderableComponent> {
    fn clone(&self) -> Self {
        self.clone_box()
    }
}

impl std::fmt::Debug for Box<dyn RenderableComponent> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.debug_fmt(f)
    }
}

/// Type alias for the render function used by Widget elements.
type RenderFn = Rc<dyn Fn(&dyn Any, Rect, &mut Buffer)>;

/// Represents a virtual node in the virtual DOM tree.
///
/// Elements can be:
/// - `Component`: A fiber-based component with lifecycle management
/// - `Widget`: A ratatui widget
/// - `Text`: A text node
///
/// # Example
///
/// ```rust,ignore
/// use reratui_fiber::element::Element;
/// use ratatui::widgets::Paragraph;
///
/// // Create a widget element
/// let widget_elem = Element::widget(Paragraph::new("Hello"));
///
/// // Create a text element
/// let text_elem = Element::text("Hello, World!");
///
/// // Create a fragment with multiple elements
/// let fragment = Element::fragment(vec![widget_elem, text_elem]);
/// ```
#[derive(Clone)]
pub enum Element {
    /// Represents a fiber-based component (Component) in the virtual DOM tree.
    Component {
        /// The wrapper that handles fiber management.
        wrapper: Box<dyn RenderableComponent>,
        /// The key of the component for reconciliation.
        key: Option<String>,
    },
    /// Represents a primitive widget in the virtual DOM tree.
    Widget {
        /// The widget instance.
        widget: Rc<dyn Any>,
        /// Render function that knows how to render this specific widget.
        render_fn: RenderFn,
        /// The key of the widget for reconciliation.
        key: Option<String>,
    },
    /// Represents a text node in the virtual DOM tree.
    Text(String),
}

impl std::fmt::Debug for Element {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Element::Component { wrapper, key } => f
                .debug_struct("Component")
                .field("wrapper", wrapper)
                .field("key", key)
                .finish(),
            Element::Widget { key, .. } => f
                .debug_struct("Widget")
                .field("key", key)
                .field("widget", &"<opaque>")
                .finish(),
            Element::Text(text) => f.debug_tuple("Text").field(text).finish(),
        }
    }
}

impl PartialEq for Element {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Element::Text(a), Element::Text(b)) => a == b,
            (Element::Component { key: key_a, .. }, Element::Component { key: key_b, .. }) => {
                key_a == key_b
            }
            (Element::Widget { key: key_a, .. }, Element::Widget { key: key_b, .. }) => {
                key_a == key_b
            }
            _ => false,
        }
    }
}

impl Default for Element {
    fn default() -> Self {
        Self::new()
    }
}

impl Element {
    /// Creates a new empty element (placeholder for compatibility).
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// use reratui_fiber::element::Element;
    ///
    /// let elem = Element::new();
    /// ```
    pub fn new() -> Self {
        Element::Text(String::new())
    }

    /// Creates a new widget node.
    ///
    /// The widget must implement `Widget`, `Clone`, and `'static`.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// use reratui_fiber::element::Element;
    /// use ratatui::widgets::Paragraph;
    ///
    /// let elem = Element::widget(Paragraph::new("Hello"));
    /// ```
    pub fn widget<W: Widget + Clone + 'static>(widget: W) -> Self {
        let widget_box = Rc::new(widget.clone());
        let render_fn = Rc::new(move |any: &dyn Any, area: Rect, buffer: &mut Buffer| {
            if let Some(w) = any.downcast_ref::<W>() {
                w.clone().render(area, buffer);
            }
        });

        Element::Widget {
            widget: widget_box,
            render_fn,
            key: None,
        }
    }

    /// Creates a new Component node from a RenderableComponent.
    ///
    /// For internal use. Prefer `Element::component()` for simpler API.
    pub fn component_from_wrapper(wrapper: Box<dyn RenderableComponent>) -> Self {
        Element::Component { wrapper, key: None }
    }

    /// Creates a new component element directly from a Component implementation.
    ///
    /// This is the simplest way to create component elements - just pass your component!
    /// No Clone derive required!
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// use reratui_fiber::{Element, Component};
    /// use ratatui::{buffer::Buffer, layout::Rect};
    ///
    /// struct Counter;
    ///
    /// impl Component for Counter {
    ///     fn render(&self, area: Rect, buffer: &mut Buffer) {
    ///         let (count, set_count) = use_state(|| 0);
    ///         // render logic...
    ///     }
    /// }
    ///
    /// // Simple usage - no wrapper, no ID, no Clone needed!
    /// let elem = Element::component(Counter);
    /// ```
    pub fn component<C: crate::Component>(component: C) -> Self {
        let wrapper = crate::component::ComponentWrapper::new(component);
        Element::Component {
            wrapper: Box::new(wrapper),
            key: None,
        }
    }

    /// Creates a new text node.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// use reratui_fiber::element::Element;
    ///
    /// let elem = Element::text("Hello, World!");
    /// ```
    pub fn text<S: Into<String>>(text: S) -> Self {
        Element::Text(text.into())
    }

    /// Creates a fragment containing multiple elements.
    ///
    /// This creates a container that can hold and render multiple child elements.
    /// The children are laid out vertically with equal space distribution.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// use reratui_fiber::element::Element;
    /// use ratatui::widgets::Paragraph;
    ///
    /// let fragment = Element::fragment(vec![
    ///     Element::widget(Paragraph::new("First")),
    ///     Element::widget(Paragraph::new("Second")),
    /// ]);
    /// ```
    pub fn fragment(elements: Vec<Element>) -> Self {
        if elements.is_empty() {
            Element::text("")
        } else if elements.len() == 1 {
            elements.into_iter().next().unwrap()
        } else {
            // Create a fragment container that holds all elements
            let fragment_wrapper = FragmentWrapper::new(elements);

            Element::Widget {
                widget: Rc::new(fragment_wrapper),
                render_fn: Rc::new(|widget, area, buffer| {
                    if let Some(fragment) = widget.downcast_ref::<FragmentWrapper>() {
                        fragment.clone().render_fragment(area, buffer);
                    }
                }),
                key: None,
            }
        }
    }

    /// Sets the key for this node.
    ///
    /// Keys are used during reconciliation to identify elements that have moved
    /// or been reordered in a list.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// use reratui_fiber::element::Element;
    /// use ratatui::widgets::Paragraph;
    ///
    /// let elem = Element::widget(Paragraph::new("Item"))
    ///     .with_key("item-1");
    /// ```
    pub fn with_key<S: Into<String>>(mut self, key: S) -> Self {
        match &mut self {
            Element::Component { key: k, .. } => *k = Some(key.into()),
            Element::Widget { key: k, .. } => *k = Some(key.into()),
            Element::Text(_) => {} // Text nodes don't have keys
        }
        self
    }

    /// Returns the key of this element, if any.
    pub fn key(&self) -> Option<&str> {
        match self {
            Element::Component { key, .. } => key.as_deref(),
            Element::Widget { key, .. } => key.as_deref(),
            Element::Text(_) => None,
        }
    }

    /// Renders this node to the buffer.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// use reratui_fiber::element::Element;
    /// use ratatui::{buffer::Buffer, layout::Rect, widgets::Paragraph};
    ///
    /// let elem = Element::widget(Paragraph::new("Hello"));
    /// let area = Rect::new(0, 0, 80, 24);
    /// let mut buffer = Buffer::empty(area);
    ///
    /// elem.render(area, &mut buffer);
    /// ```
    pub fn render(&self, area: Rect, buffer: &mut Buffer) {
        match self {
            Element::Component { wrapper, .. } => {
                // Render with fiber management
                wrapper.render_with_fiber(area, buffer);
            }
            Element::Widget {
                widget, render_fn, ..
            } => {
                render_fn(widget.as_ref(), area, buffer);
            }
            Element::Text(_) => {
                // Text nodes are usually rendered as part of a widget
                // They don't render directly to the buffer
            }
        }
    }

    /// Returns true if this element is a Component variant.
    pub fn is_component(&self) -> bool {
        matches!(self, Element::Component { .. })
    }

    /// Returns true if this element is a Widget variant.
    pub fn is_widget(&self) -> bool {
        matches!(self, Element::Widget { .. })
    }

    /// Returns true if this element is a Text variant.
    pub fn is_text(&self) -> bool {
        matches!(self, Element::Text(_))
    }

    /// Returns the text content if this is a Text element.
    pub fn as_text(&self) -> Option<&str> {
        match self {
            Element::Text(s) => Some(s),
            _ => None,
        }
    }
}

/// Internal wrapper for fragment elements.
///
/// This wrapper holds multiple child elements and renders them in a vertical layout.
#[derive(Clone)]
struct FragmentWrapper {
    children: Vec<Element>,
}

impl FragmentWrapper {
    fn new(children: Vec<Element>) -> Self {
        Self { children }
    }

    fn render_fragment(self, area: Rect, buffer: &mut Buffer) {
        if self.children.is_empty() {
            return;
        }

        // Create constraints for each child
        let constraints: Vec<Constraint> = (0..self.children.len())
            .map(|_| Constraint::Min(0))
            .collect();

        let layout = Layout::default()
            .direction(Direction::Vertical)
            .constraints(constraints);

        let chunks = layout.split(area);

        // Render each child in its corresponding chunk
        for (i, child) in self.children.into_iter().enumerate() {
            if i < chunks.len() {
                child.render(chunks[i], buffer);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::widgets::Paragraph;

    #[test]
    fn test_element_new_creates_empty_text() {
        let elem = Element::new();
        assert!(elem.is_text());
        assert_eq!(elem.as_text(), Some(""));
    }

    #[test]
    fn test_element_default_creates_empty_text() {
        let elem = Element::default();
        assert!(elem.is_text());
        assert_eq!(elem.as_text(), Some(""));
    }

    #[test]
    fn test_element_text_creates_text_node() {
        let elem = Element::text("Hello, World!");
        assert!(elem.is_text());
        assert_eq!(elem.as_text(), Some("Hello, World!"));
    }

    #[test]
    fn test_element_widget_creates_widget_node() {
        let elem = Element::widget(Paragraph::new("Test"));
        assert!(elem.is_widget());
        assert!(!elem.is_text());
        assert!(!elem.is_component());
    }

    #[test]
    fn test_element_with_key_sets_key_on_widget() {
        let elem = Element::widget(Paragraph::new("Test")).with_key("my-key");
        assert_eq!(elem.key(), Some("my-key"));
    }

    #[test]
    fn test_element_with_key_does_nothing_on_text() {
        let elem = Element::text("Hello").with_key("my-key");
        assert_eq!(elem.key(), None);
    }

    #[test]
    fn test_element_fragment_empty_returns_empty_text() {
        let elem = Element::fragment(vec![]);
        assert!(elem.is_text());
        assert_eq!(elem.as_text(), Some(""));
    }

    #[test]
    fn test_element_fragment_single_returns_element() {
        let elem = Element::fragment(vec![Element::text("Single")]);
        assert!(elem.is_text());
        assert_eq!(elem.as_text(), Some("Single"));
    }

    #[test]
    fn test_element_fragment_multiple_creates_widget() {
        let elem = Element::fragment(vec![Element::text("First"), Element::text("Second")]);
        assert!(elem.is_widget());
    }

    #[test]
    fn test_element_widget_renders() {
        let elem = Element::widget(Paragraph::new("Hello"));
        let area = Rect::new(0, 0, 10, 1);
        let mut buffer = Buffer::empty(area);

        elem.render(area, &mut buffer);

        // Check that "Hello" was rendered
        let content: String = buffer.content().iter().map(|cell| cell.symbol()).collect();
        assert!(content.contains("Hello"));
    }

    #[test]
    fn test_element_text_render_is_noop() {
        let elem = Element::text("Hello");
        let area = Rect::new(0, 0, 10, 1);
        let mut buffer = Buffer::empty(area);

        // Text nodes don't render directly
        elem.render(area, &mut buffer);

        // Buffer should be empty (spaces)
        let content: String = buffer.content().iter().map(|cell| cell.symbol()).collect();
        assert!(content.trim().is_empty());
    }

    #[test]
    fn test_element_fragment_renders_children() {
        let elem = Element::fragment(vec![
            Element::widget(Paragraph::new("First")),
            Element::widget(Paragraph::new("Second")),
        ]);
        let area = Rect::new(0, 0, 10, 2);
        let mut buffer = Buffer::empty(area);

        elem.render(area, &mut buffer);

        let content: String = buffer.content().iter().map(|cell| cell.symbol()).collect();
        assert!(content.contains("First"));
        assert!(content.contains("Second"));
    }
}

#[cfg(test)]
mod property_tests {
    use super::*;
    use proptest::prelude::*;
    use ratatui::widgets::Paragraph;

    // **Property 4: Element Construction Correctness**
    // **Validates: Requirements 7.1, 7.2, 7.3, 7.4, 7.5, 7.6**
    //
    // For any Element constructed via `widget()`, `component()`, `text()`, or `fragment()`:
    // - The Element SHALL be the correct variant
    // - `with_key()` SHALL set the key on the element
    // - `render()` SHALL delegate to the appropriate render method

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(100))]

        /// Property: Element::text creates Text variant with correct content
        #[test]
        fn prop_element_text_creates_text_variant(text in ".*") {
            let elem = Element::text(text.clone());

            prop_assert!(elem.is_text(), "Element::text should create Text variant");
            prop_assert_eq!(elem.as_text(), Some(text.as_str()), "Text content should match");
            prop_assert!(!elem.is_widget(), "Text element should not be widget");
            prop_assert!(!elem.is_component(), "Text element should not be component");
        }

        /// Property: Element::widget creates Widget variant
        #[test]
        fn prop_element_widget_creates_widget_variant(text in "[a-zA-Z0-9 ]{0,50}") {
            let elem = Element::widget(Paragraph::new(text));

            prop_assert!(elem.is_widget(), "Element::widget should create Widget variant");
            prop_assert!(!elem.is_text(), "Widget element should not be text");
            prop_assert!(!elem.is_component(), "Widget element should not be component");
        }

        /// Property: with_key sets key on Widget elements
        #[test]
        fn prop_with_key_sets_key_on_widget(
            text in "[a-zA-Z0-9 ]{0,50}",
            key in "[a-zA-Z0-9_-]{1,20}"
        ) {
            let elem = Element::widget(Paragraph::new(text)).with_key(key.clone());

            prop_assert_eq!(elem.key(), Some(key.as_str()), "Key should be set on widget");
        }

        /// Property: with_key does not set key on Text elements (text nodes don't have keys)
        #[test]
        fn prop_with_key_noop_on_text(
            text in ".*",
            key in "[a-zA-Z0-9_-]{1,20}"
        ) {
            let elem = Element::text(text).with_key(key);

            prop_assert_eq!(elem.key(), None, "Text elements should not have keys");
        }

        /// Property: Element::fragment with empty vec returns empty text
        #[test]
        fn prop_fragment_empty_returns_empty_text(_dummy in 0..1i32) {
            let elem = Element::fragment(vec![]);

            prop_assert!(elem.is_text(), "Empty fragment should be Text variant");
            prop_assert_eq!(elem.as_text(), Some(""), "Empty fragment should have empty text");
        }

        /// Property: Element::fragment with single element returns that element
        #[test]
        fn prop_fragment_single_returns_element(text in ".*") {
            let original = Element::text(text.clone());
            let elem = Element::fragment(vec![Element::text(text.clone())]);

            prop_assert!(elem.is_text(), "Single-element fragment should return the element");
            prop_assert_eq!(elem.as_text(), original.as_text(), "Content should match");
        }

        /// Property: Element::fragment with multiple elements creates Widget
        #[test]
        fn prop_fragment_multiple_creates_widget(
            texts in prop::collection::vec("[a-zA-Z0-9 ]{0,20}", 2..5)
        ) {
            let elements: Vec<Element> = texts.iter()
                .map(|t| Element::text(t.clone()))
                .collect();

            let elem = Element::fragment(elements);

            prop_assert!(elem.is_widget(), "Multi-element fragment should be Widget variant");
        }

        /// Property: Element::new creates empty Text element
        #[test]
        fn prop_element_new_creates_empty_text(_dummy in 0..1i32) {
            let elem = Element::new();

            prop_assert!(elem.is_text(), "Element::new should create Text variant");
            prop_assert_eq!(elem.as_text(), Some(""), "Element::new should have empty text");
        }

        /// Property: Element::default creates empty Text element
        #[test]
        fn prop_element_default_creates_empty_text(_dummy in 0..1i32) {
            let elem = Element::default();

            prop_assert!(elem.is_text(), "Element::default should create Text variant");
            prop_assert_eq!(elem.as_text(), Some(""), "Element::default should have empty text");
        }

        /// Property: Widget elements render correctly
        #[test]
        fn prop_widget_renders_content(
            text in "[a-zA-Z]{1,10}",
            width in 10u16..50,
            height in 1u16..5
        ) {
            let elem = Element::widget(Paragraph::new(text.clone()));
            let area = Rect::new(0, 0, width, height);
            let mut buffer = Buffer::empty(area);

            elem.render(area, &mut buffer);

            // Check that the text was rendered
            let content: String = buffer
                .content()
                .iter()
                .map(|cell| cell.symbol())
                .collect();

            prop_assert!(
                content.contains(&text),
                "Widget should render its content. Expected '{}' in '{}'",
                text,
                content
            );
        }

        /// Property: Text elements render as no-op (don't modify buffer)
        #[test]
        fn prop_text_render_is_noop(
            text in ".*",
            width in 10u16..50,
            height in 1u16..5
        ) {
            let elem = Element::text(text);
            let area = Rect::new(0, 0, width, height);
            let mut buffer = Buffer::empty(area);

            // Get initial buffer state
            let initial_content: String = buffer
                .content()
                .iter()
                .map(|cell| cell.symbol())
                .collect();

            elem.render(area, &mut buffer);

            // Get final buffer state
            let final_content: String = buffer
                .content()
                .iter()
                .map(|cell| cell.symbol())
                .collect();

            prop_assert_eq!(
                initial_content,
                final_content,
                "Text element render should not modify buffer"
            );
        }

        /// Property: Fragment renders all children
        #[test]
        fn prop_fragment_renders_children(
            texts in prop::collection::vec("[a-zA-Z]{1,5}", 2..4)
        ) {
            let elements: Vec<Element> = texts.iter()
                .map(|t| Element::widget(Paragraph::new(t.clone())))
                .collect();

            let elem = Element::fragment(elements);
            let height = (texts.len() as u16) * 2; // Give enough height for each child
            let area = Rect::new(0, 0, 20, height);
            let mut buffer = Buffer::empty(area);

            elem.render(area, &mut buffer);

            let content: String = buffer
                .content()
                .iter()
                .map(|cell| cell.symbol())
                .collect();

            // Each text should appear in the rendered output
            for text in &texts {
                prop_assert!(
                    content.contains(text),
                    "Fragment should render child '{}'. Content: '{}'",
                    text,
                    content
                );
            }
        }

        /// Property: with_key preserves element variant
        #[test]
        fn prop_with_key_preserves_variant(
            text in "[a-zA-Z0-9 ]{0,20}",
            key in "[a-zA-Z0-9_-]{1,20}"
        ) {
            // Test with widget
            let widget_elem = Element::widget(Paragraph::new(text.clone()));
            let widget_with_key = widget_elem.clone().with_key(key.clone());
            prop_assert!(widget_with_key.is_widget(), "with_key should preserve Widget variant");

            // Test with text (key is ignored but variant preserved)
            let text_elem = Element::text(text);
            let text_with_key = text_elem.clone().with_key(key);
            prop_assert!(text_with_key.is_text(), "with_key should preserve Text variant");
        }
    }
}
