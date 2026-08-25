//! Spawn inventory guard (Phase 0 of the sandbox plan).
//!
//! Reads the checked-in manifest at
//! `crates/agendao-server/tests/fixtures/sandbox-spawn-inventory.tsv` and diffs it against a
//! fresh scan of production sources for direct process-spawn calls
//! (`Command::new`, portable-pty `CommandBuilder::new`, and aliased forms —
//! the scan matches the substring, so `ProcessCommand::new` and
//! `StdCommand::new` are covered).
//!
//! The manifest anchors entries by the trimmed source line, not by line
//! number, so unrelated edits above a spawn call cannot drift the guard.
//! Coverage is exact in both directions:
//!
//! * a production spawn that is not registered fails ("new unregistered
//!   direct spawn");
//! * a registered needle with no matching production spawn line fails
//!   ("stale registration").
//!
//! Additional policy constraints enforced here:
//!
//! * `model_reachable=true` entries must be `boundary`;
//! * `native_allowed` entries must not be model reachable;
//! * `test_only` entries must live below the file's last `#[cfg(test)]`.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

const NEEDLE_SPAWN_MARKERS: [&str; 2] = ["Command::new", "CommandBuilder::new"];
const VALID_CATEGORIES: [&str; 4] = ["A", "B", "C", "D"];
const VALID_TRUST_CLASSES: [&str; 4] = [
    "ModelReachable",
    "UserConfiguredIntegration",
    "HostManagement",
    "TestOnly",
];

#[derive(Debug)]
struct InventoryRow {
    path: String,
    needle: String,
    category: String,
    trust_class: String,
    model_reachable: bool,
    status: String,
}

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("crates/agendao-server manifest must have a repo root two levels up")
        .to_path_buf()
}

fn parse_manifest(raw: &str) -> Vec<InventoryRow> {
    let mut rows = Vec::new();
    for (index, line) in raw.lines().enumerate() {
        let line = line.trim_end_matches('\r');
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let columns: Vec<&str> = line.split('\t').collect();
        assert!(
            columns.len() >= 6,
            "inventory line {} must have at least 6 tab-separated columns (path, needle, category, trust_class, model_reachable, status): {line:?}",
            index + 1,
        );
        let model_reachable = match columns[4] {
            "true" => true,
            "false" => false,
            other => panic!("invalid model_reachable {other:?} on line {}", index + 1),
        };
        rows.push(InventoryRow {
            path: columns[0].to_string(),
            needle: columns[1].to_string(),
            category: columns[2].to_string(),
            trust_class: columns[3].to_string(),
            model_reachable,
            status: columns[5].to_string(),
        });
    }
    rows
}

fn is_scannable(rel: &Path) -> bool {
    let text = rel.to_string_lossy();
    if text.contains("/target/") || text.contains("/tests/") {
        return false;
    }
    let is_crate_source = text.starts_with("crates/") && text.contains("/src/");
    let is_build_script = text.starts_with("crates/") && text.ends_with("/build.rs");
    let is_app_source = text.starts_with("apps/") && text.ends_with(".rs");
    is_crate_source || is_build_script || is_app_source
}

fn collect_rs_files(root: &Path, dir: &Path, out: &mut Vec<PathBuf>) {
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_rs_files(root, &path, out);
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            let rel = path.strip_prefix(root).expect("walked path under root");
            if is_scannable(&rel.to_path_buf()) {
                out.push(rel.to_path_buf());
            }
        }
    }
}

fn is_spawn_line(line: &str) -> bool {
    NEEDLE_SPAWN_MARKERS
        .iter()
        .any(|marker| line.contains(marker))
}

/// Split a file's spawn lines into (production, test) sets using the last
/// `#[cfg(test)]` occurrence as the test-module boundary.
fn scan_file(source: &str) -> (BTreeSet<String>, BTreeSet<String>) {
    let mut test_boundary = usize::MAX;
    for (index, line) in source.lines().enumerate() {
        if line.trim_start().starts_with("#[cfg(test)]") {
            test_boundary = index + 1;
        }
    }
    let mut production = BTreeSet::new();
    let mut test = BTreeSet::new();
    for (index, line) in source.lines().enumerate() {
        if !is_spawn_line(line) {
            continue;
        }
        let trimmed = line.trim();
        if index < test_boundary {
            production.insert(trimmed.to_string());
        } else {
            test.insert(trimmed.to_string());
        }
    }
    (production, test)
}

#[test]
fn spawn_inventory_matches_source_scan_and_policy_constraints() {
    let root = repo_root();
    let manifest_path =
        root.join("crates/agendao-server/tests/fixtures/sandbox-spawn-inventory.tsv");
    let raw = fs::read_to_string(&manifest_path)
        .unwrap_or_else(|err| panic!("read {}: {err}", manifest_path.display()));
    let rows = parse_manifest(&raw);

    // Policy constraints on declared rows.
    for row in &rows {
        assert!(
            VALID_CATEGORIES.contains(&row.category.as_str()),
            "invalid category {:?} for {}",
            row.category,
            row.path
        );
        assert!(
            VALID_TRUST_CLASSES.contains(&row.trust_class.as_str()),
            "invalid trust_class {:?} for {}",
            row.trust_class,
            row.path
        );
        if row.model_reachable {
            assert!(
                row.status == "boundary",
                "model-reachable spawn {} ({}) must be boundary, got {:?}",
                row.path,
                row.needle,
                row.status
            );
        }
        if row.status == "native_allowed" {
            assert!(
                !row.model_reachable,
                "native_allowed spawn {} ({}) must not be model reachable",
                row.path, row.needle
            );
        }
    }

    // Scan production sources.
    let mut sources = Vec::new();
    collect_rs_files(&root, &root, &mut sources);

    let mut scanned: BTreeMap<String, (BTreeSet<String>, BTreeSet<String>)> = BTreeMap::new();
    for rel in &sources {
        let Ok(source) = fs::read_to_string(root.join(rel)) else {
            continue;
        };
        scanned.insert(rel.to_string_lossy().into_owned(), scan_file(&source));
    }

    // Registered rows grouped per file, split by scope.
    let mut registered_production: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    let mut registered_test: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for row in &rows {
        let bucket = if row.status == "test_only" {
            &mut registered_test
        } else {
            &mut registered_production
        };
        bucket
            .entry(row.path.clone())
            .or_default()
            .insert(row.needle.clone());
    }

    let mut failures = Vec::new();

    // Direction 1: every scanned production spawn must be registered.
    for (path, (production, _)) in &scanned {
        let registered = registered_production.get(path).cloned().unwrap_or_default();
        for line in production {
            if !registered.contains(line) {
                failures.push(format!(
                    "unregistered production spawn in {path}:\n    {line}\n  register it in crates/agendao-server/tests/fixtures/sandbox-spawn-inventory.tsv with category/trust_class/status"
                ));
            }
        }
    }

    // Direction 2: every registered production needle must exist in scan.
    for (path, registered) in &registered_production {
        match scanned.get(path) {
            Some((production, _)) => {
                for needle in registered {
                    if !production.contains(needle) {
                        failures.push(format!(
                            "stale registration in {path}:\n    {needle}\n  the spawn call no longer exists; update or migrate the inventory row"
                        ));
                    }
                }
            }
            None => failures.push(format!(
                "registered file {path} is not scanned (moved or deleted?); update the inventory"
            )),
        }
    }

    // Direction 3: test_only rows must point at test-scope spawn lines.
    for (path, registered) in &registered_test {
        match scanned.get(path) {
            Some((_, test)) => {
                for needle in registered {
                    if !test.contains(needle) {
                        failures.push(format!(
                            "test_only registration in {path} does not match a cfg(test) spawn:\n    {needle}"
                        ));
                    }
                }
            }
            None => failures.push(format!("test_only file {path} not scanned")),
        }
    }

    assert!(
        failures.is_empty(),
        "sandbox spawn inventory drift ({} problem(s)):\n{}",
        failures.len(),
        failures.join("\n")
    );
}
