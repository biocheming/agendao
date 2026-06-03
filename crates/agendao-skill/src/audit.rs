use crate::SkillError;
use agendao_types::SkillAuditEvent;
use std::fs::{self, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::Path;

pub(crate) const DEFAULT_AUDIT_TAIL_LIMIT: usize = 128;

pub(crate) fn load_audit_events(
    path: &Path,
    max_tail: usize,
) -> Result<Vec<SkillAuditEvent>, SkillError> {
    if !path.exists() {
        return Ok(Vec::new());
    }

    let file = fs::File::open(path).map_err(|error| SkillError::ReadFailed {
        path: path.to_path_buf(),
        message: error.to_string(),
    })?;
    let reader = BufReader::new(file);
    let mut events = Vec::new();
    for line in reader.lines() {
        let line = line.map_err(|error| SkillError::ReadFailed {
            path: path.to_path_buf(),
            message: error.to_string(),
        })?;
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let event = serde_json::from_str::<SkillAuditEvent>(trimmed).map_err(|error| {
            SkillError::ReadFailed {
                path: path.to_path_buf(),
                message: error.to_string(),
            }
        })?;
        events.push(event);
        if events.len() > max_tail {
            events.remove(0);
        }
    }
    Ok(events)
}

pub(crate) fn append_audit_event(path: &Path, event: &SkillAuditEvent) -> Result<(), SkillError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| SkillError::WriteFailed {
            path: parent.to_path_buf(),
            message: error.to_string(),
        })?;
    }

    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|error| SkillError::WriteFailed {
            path: path.to_path_buf(),
            message: error.to_string(),
        })?;
    let line = serde_json::to_string(event).map_err(|error| SkillError::WriteFailed {
        path: path.to_path_buf(),
        message: error.to_string(),
    })?;
    writeln!(file, "{line}").map_err(|error| SkillError::WriteFailed {
        path: path.to_path_buf(),
        message: error.to_string(),
    })
}
