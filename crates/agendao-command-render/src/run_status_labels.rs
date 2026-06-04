#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RunStatusLabels {
    pub slug: &'static str,
    pub title: &'static str,
    pub badge: &'static str,
}

pub fn canonical_run_status_labels(status: &str) -> RunStatusLabels {
    match status.trim().to_ascii_lowercase().as_str() {
        "running" => RunStatusLabels {
            slug: "running",
            title: "Running",
            badge: "RUNNING",
        },
        "awaiting_permission" => RunStatusLabels {
            slug: "awaiting_permission",
            title: "Waiting for permission",
            badge: "AWAITING PERMISSION",
        },
        "awaiting_user" | "waiting_on_user" => RunStatusLabels {
            slug: "awaiting_user",
            title: "Waiting for user input",
            badge: "AWAITING USER",
        },
        "waiting_on_tool" => RunStatusLabels {
            slug: "waiting_on_tool",
            title: "Waiting on tool",
            badge: "WAITING ON TOOL",
        },
        "complete" | "completed" => RunStatusLabels {
            slug: "complete",
            title: "Run complete",
            badge: "COMPLETE",
        },
        "idle" => RunStatusLabels {
            slug: "idle",
            title: "Session idle",
            badge: "IDLE",
        },
        "error" | "failed" => RunStatusLabels {
            slug: "error",
            title: "Run failed",
            badge: "ERROR",
        },
        "compacting" => RunStatusLabels {
            slug: "compacting",
            title: "Compacting",
            badge: "COMPACTING",
        },
        "cancelling" => RunStatusLabels {
            slug: "cancelling",
            title: "Cancelling",
            badge: "CANCELLING",
        },
        "blocked" => RunStatusLabels {
            slug: "blocked",
            title: "Blocked",
            badge: "BLOCKED",
        },
        "sleeping" => RunStatusLabels {
            slug: "sleeping",
            title: "Sleeping",
            badge: "SLEEPING",
        },
        "retrying" => RunStatusLabels {
            slug: "retrying",
            title: "Retrying",
            badge: "RETRYING",
        },
        "reconnecting" => RunStatusLabels {
            slug: "reconnecting",
            title: "Reconnecting stream",
            badge: "RECONNECTING",
        },
        "ready" | "" => RunStatusLabels {
            slug: "ready",
            title: "Session ready",
            badge: "READY",
        },
        _ => RunStatusLabels {
            slug: "status",
            title: "Session status",
            badge: "STATUS",
        },
    }
}

pub fn canonical_run_status_title(status: &str) -> &'static str {
    canonical_run_status_labels(status).title
}

pub fn canonical_run_status_badge(status: &str) -> &'static str {
    canonical_run_status_labels(status).badge
}

#[cfg(test)]
mod tests {
    use super::{
        canonical_run_status_badge, canonical_run_status_labels, canonical_run_status_title,
    };

    #[test]
    fn canonical_titles_cover_tail_status_contract() {
        assert_eq!(canonical_run_status_title("running"), "Running");
        assert_eq!(
            canonical_run_status_title("awaiting_permission"),
            "Waiting for permission"
        );
        assert_eq!(
            canonical_run_status_title("awaiting_user"),
            "Waiting for user input"
        );
        assert_eq!(canonical_run_status_title("complete"), "Run complete");
        assert_eq!(canonical_run_status_title("idle"), "Session idle");
        assert_eq!(canonical_run_status_title("error"), "Run failed");
    }

    #[test]
    fn canonical_badges_stay_stable_for_tui_header_usage() {
        assert_eq!(canonical_run_status_badge("running"), "RUNNING");
        assert_eq!(
            canonical_run_status_badge("awaiting_permission"),
            "AWAITING PERMISSION"
        );
        assert_eq!(canonical_run_status_badge("awaiting_user"), "AWAITING USER");
    }

    #[test]
    fn waiting_on_user_normalizes_to_awaiting_user() {
        let labels = canonical_run_status_labels("waiting_on_user");
        assert_eq!(labels.slug, "awaiting_user");
        assert_eq!(labels.title, "Waiting for user input");
    }
}
