//! Production file-tool path authority probes.

use agendao_tool::apply_patch::ApplyPatchTool;
use agendao_tool::edit::EditTool;
use agendao_tool::multiedit::MultiEditTool;
use agendao_tool::read::ReadTool;
use agendao_tool::write::WriteTool;
use agendao_tool::{Tool, ToolContext, ToolError};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

fn fixture_root(name: &str) -> PathBuf {
    let configured =
        PathBuf::from(std::env::var("CARGO_TARGET_DIR").expect("CARGO_TARGET_DIR=../target"));
    let target = if configured.is_absolute() {
        configured
    } else {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(|path| path.parent())
            .expect("workspace root")
            .join(configured)
    };
    let root = target.join("agendao-tool-tests").join(name);
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    root
}

fn denied_context(workspace: &Path) -> ToolContext {
    let mut context = ToolContext::new(
        "session".into(),
        "message".into(),
        workspace.display().to_string(),
    );
    context.project_root = workspace.display().to_string();
    context = context.with_ask(|request| async move {
        Err(ToolError::PermissionDenied(format!(
            "denied {}",
            request.permission
        )))
    });
    context
}

#[cfg(unix)]
#[tokio::test]
async fn all_file_tools_reject_external_symlink_targets_before_mutation() {
    use std::os::unix::fs::symlink;

    let base = fixture_root("external-symlink-denied");
    let workspace = base.join("workspace");
    let outside = base.join("outside");
    std::fs::create_dir_all(&workspace).unwrap();
    std::fs::create_dir_all(&outside).unwrap();
    let outside_file = outside.join("secret.txt");
    std::fs::write(&outside_file, "original").unwrap();
    symlink(&outside_file, workspace.join("link.txt")).unwrap();

    let read = ReadTool::with_directory(&workspace);
    assert!(matches!(
        read.execute(
            serde_json::json!({"file_path":"link.txt"}),
            denied_context(&workspace)
        )
        .await,
        Err(ToolError::PermissionDenied(_))
    ));

    let write = WriteTool::new();
    assert!(matches!(
        write
            .execute(
                serde_json::json!({"file_path":"link.txt","content":"write"}),
                denied_context(&workspace)
            )
            .await,
        Err(ToolError::PermissionDenied(_))
    ));

    let edit = EditTool::new();
    assert!(matches!(
        edit.execute(serde_json::json!({"file_path":workspace.join("link.txt"),"old_string":"original","new_string":"edit"}), denied_context(&workspace)).await,
        Err(ToolError::PermissionDenied(_))
    ));

    let multi = MultiEditTool;
    assert!(matches!(
        multi.execute(serde_json::json!({"edits":[{"file_path":"link.txt","edits":[{"old_string":"original","new_string":"multi"}]}]}), denied_context(&workspace)).await,
        Err(ToolError::PermissionDenied(_))
    ));

    let patch = ApplyPatchTool;
    let patch_text = "diff --git a/link/new.txt b/link/new.txt\nnew file mode 100644\n--- /dev/null\n+++ b/link/new.txt\n@@ -0,0 +1 @@\n+blocked\n";
    let patch_result = patch
        .execute(
            serde_json::json!({"patchText":patch_text}),
            denied_context(&workspace),
        )
        .await;
    assert!(
        matches!(patch_result, Err(ToolError::PermissionDenied(_))),
        "{patch_result:?}"
    );

    assert_eq!(std::fs::read_to_string(&outside_file).unwrap(), "original");
    assert!(!outside.join("new.txt").exists());
}

#[tokio::test]
async fn internal_relative_write_and_edit_use_the_workspace_path() {
    let base = fixture_root("internal-relative");
    let workspace = base.join("workspace");
    std::fs::create_dir_all(&workspace).unwrap();

    let write = WriteTool::new();
    write
        .execute(
            serde_json::json!({"file_path":"new.txt","content":"hello"}),
            ToolContext::new("s".into(), "m".into(), workspace.display().to_string()),
        )
        .await
        .unwrap();
    assert_eq!(
        std::fs::read_to_string(workspace.join("new.txt")).unwrap(),
        "hello"
    );

    let edit = EditTool::new();
    edit.execute(
        serde_json::json!({"file_path":"new.txt","old_string":"hello","new_string":"world"}),
        ToolContext::new("s".into(), "m".into(), workspace.display().to_string()),
    )
    .await
    .unwrap();
    assert_eq!(
        std::fs::read_to_string(workspace.join("new.txt")).unwrap(),
        "world"
    );
}

#[cfg(unix)]
#[tokio::test]
async fn apply_patch_move_authorizes_external_destination_separately() {
    use std::os::unix::fs::symlink;

    let base = fixture_root("move-external-destination");
    let workspace = base.join("workspace");
    let outside = base.join("outside");
    std::fs::create_dir_all(&workspace).unwrap();
    std::fs::create_dir_all(&outside).unwrap();
    let source = workspace.join("source.txt");
    std::fs::write(&source, "original\n").unwrap();
    symlink(&outside, workspace.join("link")).unwrap();

    let requests = Arc::new(AtomicUsize::new(0));
    let requests_for_callback = requests.clone();
    let context = ToolContext::new(
        "session".into(),
        "message".into(),
        workspace.display().to_string(),
    )
    .with_ask(move |request| {
        let requests = requests_for_callback.clone();
        async move {
            if request.permission == "external_directory" {
                requests.fetch_add(1, Ordering::Relaxed);
                return Err(ToolError::PermissionDenied(
                    "external destination denied".into(),
                ));
            }
            Ok(())
        }
    });

    let patch = ApplyPatchTool;
    let patch_text = "diff --git a/source.txt b/link/moved.txt\nsimilarity index 100%\nrename from source.txt\nrename to link/moved.txt\n--- a/source.txt\n+++ b/link/moved.txt\n@@ -1 +1 @@\n-original\n+changed\n";
    let result = patch
        .execute(serde_json::json!({"patchText": patch_text}), context)
        .await;
    assert!(
        matches!(result, Err(ToolError::PermissionDenied(_))),
        "{result:?}"
    );
    assert_eq!(requests.load(Ordering::Relaxed), 1);
    assert_eq!(std::fs::read_to_string(&source).unwrap(), "original\n");
    assert!(!outside.join("moved.txt").exists());
}
