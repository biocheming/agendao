use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, OnceLock};

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ToolAccessKey {
    Read {
        path: String,
        offset: usize,
        limit: usize,
    },
    Search {
        pattern: String,
        path: String,
        glob: Option<String>,
        ignore_case: bool,
        hidden: bool,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ReadHistoryEntry {
    pub path: String,
    pub offset: usize,
    pub limit: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToolAccessOutcome {
    Fresh { consecutive: u32 },
    Warn { consecutive: u32 },
    Block { consecutive: u32 },
}

impl ToolAccessOutcome {
    pub fn consecutive(&self) -> u32 {
        match self {
            Self::Fresh { consecutive }
            | Self::Warn { consecutive }
            | Self::Block { consecutive } => *consecutive,
        }
    }
}

#[derive(Debug, Default)]
struct SessionToolAccessState {
    last_key: Option<ToolAccessKey>,
    consecutive: u32,
    read_history: HashSet<ReadHistoryEntry>,
}

type SharedTracker = Arc<Mutex<HashMap<String, SessionToolAccessState>>>;

static TOOL_ACCESS_TRACKER: OnceLock<SharedTracker> = OnceLock::new();

fn tracker() -> SharedTracker {
    TOOL_ACCESS_TRACKER
        .get_or_init(|| Arc::new(Mutex::new(HashMap::new())))
        .clone()
}

pub fn record_tool_access(session_id: &str, key: ToolAccessKey) -> ToolAccessOutcome {
    let tracker = tracker();
    let mut guard = tracker.lock().expect("tool access tracker lock poisoned");
    let session = guard.entry(session_id.to_string()).or_default();

    if let ToolAccessKey::Read {
        path,
        offset,
        limit,
    } = &key
    {
        session.read_history.insert(ReadHistoryEntry {
            path: path.clone(),
            offset: *offset,
            limit: *limit,
        });
    }

    if session.last_key.as_ref() == Some(&key) {
        session.consecutive += 1;
    } else {
        session.last_key = Some(key);
        session.consecutive = 1;
    }

    match session.consecutive {
        4.. => ToolAccessOutcome::Block {
            consecutive: session.consecutive,
        },
        3 => ToolAccessOutcome::Warn {
            consecutive: session.consecutive,
        },
        _ => ToolAccessOutcome::Fresh {
            consecutive: session.consecutive,
        },
    }
}

pub fn notify_other_tool_call(session_id: &str) {
    let tracker = tracker();
    let mut guard = tracker.lock().expect("tool access tracker lock poisoned");
    if let Some(session) = guard.get_mut(session_id) {
        session.last_key = None;
        session.consecutive = 0;
    }
}

pub fn clear_tool_access_tracker(session_id: &str) {
    let tracker = tracker();
    let mut guard = tracker.lock().expect("tool access tracker lock poisoned");
    guard.remove(session_id);
}

pub fn read_history(session_id: &str) -> Vec<ReadHistoryEntry> {
    let tracker = tracker();
    let guard = tracker.lock().expect("tool access tracker lock poisoned");
    let mut entries = guard
        .get(session_id)
        .map(|session| session.read_history.iter().cloned().collect::<Vec<_>>())
        .unwrap_or_default();
    entries.sort_by(|a, b| {
        a.path
            .cmp(&b.path)
            .then(a.offset.cmp(&b.offset))
            .then(a.limit.cmp(&b.limit))
    });
    entries
}

pub fn read_warning_message(consecutive: u32) -> String {
    format!(
        "You have read this exact file region {consecutive} times consecutively. The content has not changed since your last read. Use the information you already have."
    )
}

pub fn read_block_message(consecutive: u32) -> String {
    format!(
        "BLOCKED: You have read this exact file region {consecutive} times in a row. The content has not changed. You already have this information. STOP re-reading and proceed with your task."
    )
}

pub fn search_warning_message(consecutive: u32) -> String {
    format!(
        "You have run this exact search {consecutive} times consecutively. The results have not changed. Use the information you already have."
    )
}

pub fn search_block_message(consecutive: u32) -> String {
    format!(
        "BLOCKED: You have run this exact search {consecutive} times in a row. The results have not changed. You already have this information. STOP re-searching and proceed with your task."
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repeated_read_warns_on_third_and_blocks_on_fourth() {
        let session_id = "tool-access-read-thresholds";
        clear_tool_access_tracker(session_id);
        let key = ToolAccessKey::Read {
            path: "/tmp/demo.txt".to_string(),
            offset: 1,
            limit: 2000,
        };

        assert_eq!(
            record_tool_access(session_id, key.clone()),
            ToolAccessOutcome::Fresh { consecutive: 1 }
        );
        assert_eq!(
            record_tool_access(session_id, key.clone()),
            ToolAccessOutcome::Fresh { consecutive: 2 }
        );
        assert_eq!(
            record_tool_access(session_id, key.clone()),
            ToolAccessOutcome::Warn { consecutive: 3 }
        );
        assert_eq!(
            record_tool_access(session_id, key),
            ToolAccessOutcome::Block { consecutive: 4 }
        );
        clear_tool_access_tracker(session_id);
    }

    #[test]
    fn other_tool_resets_consecutive_but_preserves_read_history() {
        let session_id = "tool-access-reset";
        clear_tool_access_tracker(session_id);
        let key = ToolAccessKey::Read {
            path: "/tmp/demo.txt".to_string(),
            offset: 1,
            limit: 50,
        };

        record_tool_access(session_id, key.clone());
        record_tool_access(session_id, key.clone());
        notify_other_tool_call(session_id);

        assert_eq!(
            record_tool_access(session_id, key),
            ToolAccessOutcome::Fresh { consecutive: 1 }
        );

        let history = read_history(session_id);
        assert_eq!(history.len(), 1);
        assert_eq!(history[0].path, "/tmp/demo.txt");
        assert_eq!(history[0].offset, 1);
        assert_eq!(history[0].limit, 50);
        clear_tool_access_tracker(session_id);
    }

    #[test]
    fn sessions_are_isolated() {
        let first = "tool-access-session-a";
        let second = "tool-access-session-b";
        clear_tool_access_tracker(first);
        clear_tool_access_tracker(second);
        let key = ToolAccessKey::Search {
            pattern: "TODO".to_string(),
            path: ".".to_string(),
            glob: Some("*.rs".to_string()),
            ignore_case: false,
            hidden: false,
        };

        record_tool_access(first, key.clone());
        record_tool_access(first, key.clone());

        assert_eq!(
            record_tool_access(second, key),
            ToolAccessOutcome::Fresh { consecutive: 1 }
        );
        clear_tool_access_tracker(first);
        clear_tool_access_tracker(second);
    }
}
