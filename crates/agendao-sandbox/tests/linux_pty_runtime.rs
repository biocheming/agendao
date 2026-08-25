//! Real-bwrap PTY contract (Phase 4): interactive launches go through
//! `start_pty` — the slave is stdio + controlling terminal, the master
//! streams back to the host, the private interactive HOME replaces the
//! host's, and the event ladder matches the piped path exactly. Skips
//! (loudly) on hosts without a usable bwrap.
#![cfg(target_os = "linux")]

mod support;

use std::sync::Arc;
use std::time::Duration;

use agendao_sandbox::{
    BackendRegistry, BwrapBackend, EventLog, NativeBackend, PolicyInputs, PrepareOptions,
    ProfileKind, PtyDimensions, SandboxBackend, SandboxEvent, SandboxExecutionRequest,
    SandboxLauncher, SpawnSpec, TrustClass, INTERACTIVE_PRIVATE_HOME,
};
use agendao_types::SessionPermissionMode;
use support::{cleanup, test_root};

fn bwrap_available() -> bool {
    BwrapBackend::discover().probe().available
}

fn launcher() -> (SandboxLauncher, Arc<EventLog>) {
    let log = Arc::new(EventLog::default());
    let registry = BackendRegistry::native_only(Arc::new(NativeBackend::new()))
        .with_platform_backend(Arc::new(BwrapBackend::discover()));
    (SandboxLauncher::new(registry, log.clone()), log)
}

fn interactive_request(
    root: &std::path::Path,
    program: &str,
    script: &str,
) -> SandboxExecutionRequest {
    SandboxExecutionRequest::new(
        TrustClass::ModelReachable,
        ProfileKind::InteractiveShell,
        SpawnSpec::new(program).with_args(vec!["-c".into(), script.into()]),
        root,
    )
}

/// Read the master until the session ends or a deadline; returns
/// everything seen. When the last slave fd closes, a pty master read
/// reports EOF on Linux as EIO — that is the normal end of stream, not
/// an error.
async fn drain_master(reader: std::fs::File) -> std::io::Result<String> {
    use tokio::io::AsyncReadExt;
    let mut read_file = tokio::fs::File::from_std(reader);
    let mut buf = Vec::new();
    let mut chunk = [0u8; 4096];
    loop {
        let read = tokio::time::timeout(Duration::from_secs(10), read_file.read(&mut chunk)).await;
        let n = match read {
            Err(_elapsed) => {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::TimedOut,
                    "pty read timeout",
                ))
            }
            Ok(Err(err)) if err.kind() == std::io::ErrorKind::NotConnected => break,
            Ok(Err(err)) if err.raw_os_error() == Some(libc_io_eio()) => break,
            Ok(Err(err)) => return Err(err),
            Ok(Ok(n)) => n,
        };
        if n == 0 {
            break;
        }
        buf.extend_from_slice(&chunk[..n]);
        if buf.len() > 1 << 20 {
            break;
        }
    }
    Ok(String::from_utf8_lossy(&buf).into_owned())
}

/// EIO constant without depending on libc from the test.
fn libc_io_eio() -> i32 {
    5
}

#[tokio::test]
async fn interactive_shell_reports_the_private_home_over_the_pty() {
    if !bwrap_available() {
        eprintln!("skipping: bwrap not usable on this host");
        return;
    }
    let root = test_root("bwrap_pty_home");
    let (launcher, log) = launcher();
    let prepared = launcher
        .prepare(
            interactive_request(&root, "/bin/sh", "printf %s \"$HOME\""),
            &PolicyInputs::baseline(SessionPermissionMode::Default),
            &PrepareOptions::default(),
        )
        .unwrap();
    let (mut handle, master) = prepared
        .start_pty(PtyDimensions { rows: 24, cols: 80 })
        .await
        .unwrap();

    let reader = master.try_clone_reader().unwrap();
    let drain = tokio::task::spawn_blocking(move || {
        tokio::runtime::Handle::current().block_on(drain_master(reader))
    });
    // The master read task drives the runtime; wait for the child first.
    let exit = handle.wait().await.unwrap();
    let output = drain.await.unwrap().unwrap();
    assert_eq!(exit.code, Some(0), "output: {output:?}");
    assert_eq!(
        output, INTERACTIVE_PRIVATE_HOME,
        "the interactive HOME is the sandbox-private path, not the host's"
    );
    let events = log.snapshot();
    assert_eq!(events.len(), 3, "prepared -> started -> exited, pty parity");
    cleanup(&root);
}

#[tokio::test]
async fn interactive_shell_cannot_read_host_home_dotfiles() {
    if !bwrap_available() {
        eprintln!("skipping: bwrap not usable on this host");
        return;
    }
    let root = test_root("bwrap_pty_isolation");
    let (launcher, _log) = launcher();
    let prepared = launcher
        .prepare(
            interactive_request(&root, "/bin/sh", "ls ~ | wc -l; ls /home 2>&1 | head -1"),
            &PolicyInputs::baseline(SessionPermissionMode::Default),
            &PrepareOptions::default(),
        )
        .unwrap();
    let (mut handle, master) = prepared
        .start_pty(PtyDimensions { rows: 24, cols: 80 })
        .await
        .unwrap();
    let reader = master.try_clone_reader().unwrap();
    let drain = tokio::task::spawn_blocking(move || {
        tokio::runtime::Handle::current().block_on(drain_master(reader))
    });
    let exit = handle.wait().await.unwrap();
    let output = drain.await.unwrap().unwrap();
    assert_eq!(exit.code, Some(0), "output: {output:?}");
    let first_line = output.lines().next().unwrap_or_default().trim();
    assert_eq!(first_line, "0", "private HOME starts empty: {output:?}");
    assert!(
        !output.contains("/home/"),
        "the host /home tree is not mounted at all: {output:?}"
    );
    cleanup(&root);
}

#[tokio::test]
async fn pty_writes_reach_the_session_and_echo_comes_back() {
    if !bwrap_available() {
        eprintln!("skipping: bwrap not usable on this host");
        return;
    }
    let root = test_root("bwrap_pty_write");
    let (launcher, _log) = launcher();
    // cat echoes its stdin through the pty until EOF; the write side
    // proves master->slave and the read side proves slave->master.
    let prepared = launcher
        .prepare(
            interactive_request(
                &root,
                "/bin/sh",
                "read line; printf got:%s \"$line\"; exit 3",
            ),
            &PolicyInputs::baseline(SessionPermissionMode::Default),
            &PrepareOptions::default(),
        )
        .unwrap();
    let (mut handle, master) = prepared
        .start_pty(PtyDimensions { rows: 24, cols: 80 })
        .await
        .unwrap();

    let reader = master.try_clone_reader().unwrap();
    let drain = tokio::task::spawn_blocking(move || {
        tokio::runtime::Handle::current().block_on(drain_master(reader))
    });
    tokio::time::sleep(Duration::from_millis(200)).await;
    let mut writer = master.try_clone_writer().unwrap();
    use std::io::Write as _;
    writer.write_all(b"hello-pty\n").unwrap();
    writer.flush().unwrap();

    let exit = handle.wait().await.unwrap();
    let output = drain.await.unwrap().unwrap();
    assert_eq!(exit.code, Some(3), "output: {output:?}");
    assert!(
        output.contains("got:hello-pty"),
        "the write reached the session and echoed back: {output:?}"
    );
    cleanup(&root);
}

#[tokio::test]
async fn cancel_runs_the_termination_ladder_on_a_pty_session() {
    if !bwrap_available() {
        eprintln!("skipping: bwrap not usable on this host");
        return;
    }
    let root = test_root("bwrap_pty_cancel");
    let (launcher, log) = launcher();
    let prepared = launcher
        .prepare(
            interactive_request(&root, "/bin/sh", "trap '' TERM; sleep 30; echo never"),
            &PolicyInputs::baseline(SessionPermissionMode::Default),
            &PrepareOptions {
                term_grace: Some(Duration::from_millis(500)),
                ..Default::default()
            },
        )
        .unwrap();
    let (mut handle, _master) = prepared
        .start_pty(PtyDimensions { rows: 24, cols: 80 })
        .await
        .unwrap();
    let exit = handle.cancel().await.unwrap();
    assert!(!exit.success);
    let events = log.snapshot();
    assert_eq!(events.len(), 3);
    match &events[2] {
        SandboxEvent::Exited { status, .. } => {
            assert_ne!(status.cleanup, agendao_sandbox::CleanupStatus::NaturalExit);
        }
        _ => panic!("expected Exited last"),
    }
    cleanup(&root);
}

#[tokio::test]
async fn native_backend_pty_launch_streamlines_the_same_way() {
    // The native channel's pty path shares attach_slave_stdio, so a
    // yolo interactive request runs unsandboxed but still on the pty
    // with the same event ladder.
    let root = test_root("native_pty_home");
    let log = Arc::new(EventLog::default());
    let registry = BackendRegistry::native_only(Arc::new(NativeBackend::new()));
    let launcher = SandboxLauncher::new(registry, log.clone());
    let prepared = launcher
        .prepare(
            SandboxExecutionRequest::new(
                TrustClass::ModelReachable,
                ProfileKind::Native,
                SpawnSpec::new("/bin/sh")
                    .with_args(vec!["-c".into(), "printf %s \"$HOME\"".into()]),
                &root,
            ),
            &PolicyInputs::baseline(SessionPermissionMode::UnsandboxedYolo),
            &PrepareOptions::default(),
        )
        .unwrap();
    let (mut handle, master) = prepared
        .start_pty(PtyDimensions { rows: 24, cols: 80 })
        .await
        .unwrap();
    let reader = master.try_clone_reader().unwrap();
    let drain = tokio::task::spawn_blocking(move || {
        tokio::runtime::Handle::current().block_on(drain_master(reader))
    });
    let exit = handle.wait().await.unwrap();
    let output = drain.await.unwrap().unwrap();
    assert_eq!(exit.code, Some(0), "output: {output:?}");
    assert_eq!(
        output,
        std::env::var("HOME").unwrap_or_default(),
        "native interactive keeps the host HOME (no isolation by design)"
    );
    assert_eq!(log.snapshot().len(), 3);
    cleanup(&root);
}
