//! 金 — Dialog stack: unified authority for all modal dialogs.
//!
//! Only the topmost dialog receives key events and renders.
//! Dialogs are pushed/popped in LIFO order via Signals.

use revue::prelude::*;

/// Kinds of dialogs managed by the stack.
#[derive(Clone, Debug, PartialEq)]
pub enum DialogKind {
    Alert,
    Help,
    ModelSelect,
    SessionList,
    Permission,
    Question,
}

/// Signal-based dialog stack.
#[derive(Clone)]
pub struct DialogStack {
    /// Stack of open dialogs (topmost = last element).
    pub stack: Signal<Vec<DialogKind>>,
}

impl DialogStack {
    pub fn new() -> Self {
        Self { stack: signal(Vec::new()) }
    }

    /// Push a dialog onto the stack.
    pub fn push(&self, kind: DialogKind) {
        self.stack.update(|s| s.push(kind));
    }

    /// Pop the topmost dialog (typically to dismiss it).
    pub fn pop(&self) {
        self.stack.update(|s| { s.pop(); });
    }

    /// Close a specific kind of dialog.
    pub fn close(&self, kind: &DialogKind) {
        self.stack.update(|s| s.retain(|k| k != kind));
    }

    /// Check if any dialog is open.
    pub fn is_open(&self) -> bool {
        !self.stack.get().is_empty()
    }

    /// Get the topmost dialog kind, if any.
    pub fn top(&self) -> Option<DialogKind> {
        self.stack.get().last().cloned()
    }

    /// Clear all dialogs.
    pub fn clear(&self) {
        self.stack.set(Vec::new());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_stack_is_empty() {
        let ds = DialogStack::new();
        assert!(!ds.is_open());
        assert!(ds.top().is_none());
    }

    #[test]
    fn push_and_pop() {
        let ds = DialogStack::new();
        ds.push(DialogKind::Alert);
        assert!(ds.is_open());
        assert_eq!(ds.top(), Some(DialogKind::Alert));
        ds.pop();
        assert!(!ds.is_open());
    }

    #[test]
    fn close_specific_kind() {
        let ds = DialogStack::new();
        ds.push(DialogKind::Alert);
        ds.push(DialogKind::Help);
        ds.close(&DialogKind::Alert);
        assert_eq!(ds.top(), Some(DialogKind::Help));
    }

    #[test]
    fn clear_all() {
        let ds = DialogStack::new();
        ds.push(DialogKind::Alert);
        ds.push(DialogKind::Help);
        ds.clear();
        assert!(!ds.is_open());
    }
}
