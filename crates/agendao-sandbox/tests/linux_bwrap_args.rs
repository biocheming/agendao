//! bwrap argv construction (Phase 3): pure-function assertions that the
//! mount/namespace/env contract of a plan maps to a reproducible argv —
//! the property that makes "what policy actually ran" auditable from
//! the plan fingerprint. Also covers the seccomp cBPF compiler shape.
#![cfg(target_os = "linux")]

mod support;

use agendao_sandbox::{
    build_bwrap_args, build_plan, derive_profile, BwrapBackend, ChildEnvironment,
    EnvironmentPolicy, FilesystemMode, FilesystemPolicy, NetworkPolicy, PlanContext, PolicyInputs,
    ProcessPolicy, ProfileKind, SandboxBackend, SandboxProfile, SpawnSpec, TrustClass,
};
use agendao_types::SessionPermissionMode;
use support::{cleanup, test_root};

fn inputs() -> PolicyInputs {
    PolicyInputs::baseline(SessionPermissionMode::Default)
}

fn contained_plan(kind: ProfileKind, root: &std::path::Path) -> agendao_sandbox::SandboxPlan {
    let profile = derive_profile(TrustClass::ModelReachable, kind, &inputs()).unwrap();
    build_plan(&profile, kind, root, &PlanContext::new("argv-test")).unwrap()
}

fn env_with(keys: &[(&str, &str)]) -> ChildEnvironment {
    keys.iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
}

#[test]
fn proxy_only_plan_is_explicitly_unsupported_until_proxy_transport_exists() {
    let root = test_root("bwrap_proxy_only_unavailable");
    let mut plan = contained_plan(ProfileKind::WorkspaceWrite, &root);
    plan.network = NetworkPolicy::proxy_only("unix:///run/agendao/proxy.sock");
    assert!(!BwrapBackend::discover().supports(&plan));
    cleanup(&root);
}

#[test]
fn integration_runtime_roots_are_explicit_ro_binds() {
    let root = test_root("bwrap_args_integration_ro");
    let profile = derive_profile(
        TrustClass::UserConfiguredIntegration,
        ProfileKind::Integration,
        &inputs(),
    )
    .unwrap();
    let profile = SandboxProfile {
        filesystem: FilesystemPolicy {
            read_only_roots: vec![std::path::PathBuf::from("/usr")],
            ..profile.filesystem
        },
        ..profile
    };
    let plan = build_plan(
        &profile,
        ProfileKind::Integration,
        &root,
        &PlanContext::new("integration-ro"),
    )
    .unwrap();
    let args = build_bwrap_args(&plan, &SpawnSpec::new("/bin/true"), &env_with(&[]));
    assert!(args.windows(3).any(|w| w == ["--ro-bind", "/usr", "/usr"]));
    cleanup(&root);
}

/// Index of `flag`'s value argument (the entry right after `flag`).
fn value_of<'a>(args: &'a [String], flag: &str) -> &'a str {
    let pos = args
        .iter()
        .position(|a| a == flag)
        .unwrap_or_else(|| panic!("flag {flag} missing from {args:?}"));
    &args[pos + 1]
}

#[test]
fn workspace_write_binds_workspace_and_rebinds_protected_metadata() {
    let root = test_root("bwrap_args");
    std::fs::create_dir_all(root.join(".git")).unwrap();
    let plan = contained_plan(ProfileKind::WorkspaceWrite, &root);

    let args = build_bwrap_args(&plan, &SpawnSpec::new("/bin/true"), &env_with(&[]));

    // Containment core.
    assert!(args.contains(&"--unshare-all".to_string()));
    assert!(args.contains(&"--die-with-parent".to_string()));
    assert!(args.contains(&"--new-session".to_string()));
    assert!(args.contains(&"--cap-drop".to_string()));
    let capdrop = value_of(&args, "--cap-drop");
    assert_eq!(capdrop, "ALL");
    assert!(args.windows(2).any(|w| w == ["--proc", "/proc"]));
    assert!(args.windows(2).any(|w| w == ["--dev", "/dev"]));
    assert!(args.windows(2).any(|w| w == ["--tmpfs", "/tmp"]));

    // Workspace is writable...
    let bind_pos = args
        .windows(3)
        .position(|w| w[0] == "--bind" && w[1] == root.to_str().unwrap())
        .expect("workspace --bind");
    // ...and protected metadata is re-bound read-only *after* it.
    let git = root.join(".git");
    let protected_pos = args
        .windows(3)
        .position(|w| w[0] == "--ro-bind-try" && w[2] == git.to_str().unwrap())
        .expect(".git ro-bind-try override");
    assert!(
        protected_pos > bind_pos,
        "override must stack after the bind"
    );

    // Host dirs are read-only and -try (tolerate symlink-only layouts).
    for dir in ["/usr", "/etc", "/lib", "/bin", "/sbin"] {
        assert!(
            args.windows(3)
                .any(|w| w[0] == "--ro-bind-try" && w[2] == dir),
            "expected ro-bind-try for {dir}"
        );
    }
    cleanup(&root);
}

#[test]
fn read_only_workspace_gets_ro_bind_and_no_protected_overrides() {
    let root = test_root("bwrap_args_ro");
    std::fs::create_dir_all(root.join(".git")).unwrap();
    // A read-only profile derived by policy: Default session + an empty
    // file grant downgrades WorkspaceWrite to ReadOnly (Phase 2 rule).
    let profile = derive_profile(
        TrustClass::ModelReachable,
        ProfileKind::WorkspaceWrite,
        &inputs(),
    )
    .unwrap();
    let profile = SandboxProfile {
        filesystem: FilesystemPolicy {
            mode: FilesystemMode::ReadOnly,
            writable_roots: Vec::new(),
            read_only_roots: Vec::new(),
        },
        ..profile
    };
    let plan = build_plan(
        &profile,
        ProfileKind::WorkspaceWrite,
        &root,
        &PlanContext::new("argv-test-ro"),
    )
    .unwrap();

    let args = build_bwrap_args(&plan, &SpawnSpec::new("/bin/true"), &env_with(&[]));

    assert!(
        args.windows(3)
            .any(|w| w[0] == "--ro-bind" && w[1] == root.to_str().unwrap()),
        "read-only workspace must use --ro-bind"
    );
    let git = root.join(".git");
    assert!(
        !args.iter().any(|a| a == git.to_str().unwrap()),
        "no protected override needed when nothing is writable"
    );
    cleanup(&root);
}

#[test]
fn extra_writable_roots_bind_after_the_workspace() {
    let root = test_root("bwrap_args_extra");
    let extra = root.join("cache");
    std::fs::create_dir_all(&extra).unwrap();
    let profile = derive_profile(
        TrustClass::ModelReachable,
        ProfileKind::WorkspaceWrite,
        &inputs(),
    )
    .unwrap();
    let extra_canon = agendao_sandbox::canonicalize_existing(&extra).unwrap();
    let plan = build_plan(
        &profile,
        ProfileKind::WorkspaceWrite,
        &root,
        &PlanContext {
            execution_id: "argv-test-extra".into(),
            extra_writable_roots: vec![extra_canon],
            extra_read_only_roots: Vec::new(),
            term_grace: None,
            session_origin: None,
        },
    )
    .unwrap();

    let args = build_bwrap_args(&plan, &SpawnSpec::new("/bin/true"), &env_with(&[]));
    assert!(
        args.windows(3)
            .any(|w| w[0] == "--bind" && w[1] == extra.to_str().unwrap()),
        "extra writable root must be rw-bound: {args:?}"
    );
    cleanup(&root);
}

#[test]
fn readonly_profile_never_promotes_workspace_from_a_malformed_writable_root() {
    let root = test_root("bwrap_args_ro_workspace_guard");
    let profile = SandboxProfile {
        trust_class: TrustClass::ModelReachable,
        filesystem: FilesystemPolicy {
            mode: FilesystemMode::ReadOnly,
            // This is an invalid plan input which used to make the entire
            // workspace writable. The backend must fail closed even when a
            // caller constructs such a legacy/malformed plan.
            writable_roots: vec![root.clone()],
            read_only_roots: Vec::new(),
        },
        network: NetworkPolicy::disabled(),
        environment: EnvironmentPolicy::default(),
        process: ProcessPolicy {
            mode: agendao_sandbox::ProcessMode::Contained,
        },
    };
    let plan = build_plan(
        &profile,
        ProfileKind::Check,
        &root,
        &PlanContext::new("argv-test-ro-workspace-guard"),
    )
    .unwrap();

    let args = build_bwrap_args(&plan, &SpawnSpec::new("/bin/true"), &env_with(&[]));
    assert!(
        args.windows(3)
            .any(|w| w[0] == "--ro-bind" && w[1] == root.to_str().unwrap()),
        "read-only mode must win over a matching writable root"
    );
    assert!(
        !args
            .windows(3)
            .any(|w| w[0] == "--bind" && w[1] == root.to_str().unwrap()),
        "the workspace must never receive a writable bind"
    );
    cleanup(&root);
}

#[test]
fn environment_is_cleared_then_set_and_program_is_last() {
    let root = test_root("bwrap_args_env");
    let plan = contained_plan(ProfileKind::WorkspaceWrite, &root);
    let spec = SpawnSpec::new("/bin/sh").with_args(vec!["-c".into(), "exit 0".into()]);
    let env = env_with(&[
        ("PATH", "/usr/bin:/bin"),
        ("AGENDAO_SANDBOX_EXECUTION_ID", "e-1"),
    ]);

    let args = build_bwrap_args(&plan, &spec, &env);

    let clear = args.iter().position(|a| a == "--clearenv").unwrap();
    let first_setenv = args.iter().position(|a| a == "--setenv").unwrap();
    assert!(clear < first_setenv, "clearenv must precede setenv");
    // BTreeMap iteration is lexicographic: AGENDAO_* sorts before PATH.
    // Deterministic order is part of the reproducible-argv contract.
    assert_eq!(value_of(&args, "--setenv"), "AGENDAO_SANDBOX_EXECUTION_ID");
    assert!(args
        .windows(2)
        .any(|w| w[0] == "--setenv" && w[1] == "PATH"));
    // chdir defaults to the workspace when spec has no cwd.
    assert_eq!(value_of(&args, "--chdir"), root.to_str().unwrap());
    // The program and its args are the tail, after every option.
    assert_eq!(args.last().unwrap(), "exit 0");
    assert_eq!(args[args.len() - 2], "-c");
    assert_eq!(args[args.len() - 3], "/bin/sh");
    cleanup(&root);
}

#[test]
fn spec_cwd_is_respected_inside_the_sandbox() {
    let root = test_root("bwrap_args_cwd");
    let inner = root.join("inner");
    std::fs::create_dir_all(&inner).unwrap();
    let plan = contained_plan(ProfileKind::WorkspaceWrite, &root);
    let spec = SpawnSpec::new("/bin/pwd").with_cwd(&inner);

    let args = build_bwrap_args(&plan, &spec, &env_with(&[]));
    assert_eq!(value_of(&args, "--chdir"), inner.to_str().unwrap());
    cleanup(&root);
}

#[test]
fn unrestricted_filesystem_plan_is_not_supported() {
    let root = test_root("bwrap_args_unrestricted");
    let profile = SandboxProfile {
        trust_class: TrustClass::HostManagement,
        filesystem: FilesystemPolicy {
            mode: FilesystemMode::Unrestricted,
            writable_roots: Vec::new(),
            read_only_roots: Vec::new(),
        },
        network: NetworkPolicy::disabled(),
        environment: EnvironmentPolicy::default(),
        process: ProcessPolicy {
            mode: agendao_sandbox::ProcessMode::Contained,
        },
    };
    let plan = build_plan(
        &profile,
        ProfileKind::WorkspaceWrite,
        &root,
        &PlanContext::new("argv-test-unrestricted"),
    )
    .unwrap();

    let backend = BwrapBackend::discover();
    assert!(!backend.supports(&plan));
    cleanup(&root);
}

// --- seccomp compiler shape -------------------------------------------

use agendao_sandbox::platform::linux::seccomp::{SeccompFilter, SockFilter};

#[test]
fn seccomp_filter_compiles_to_valid_jump_distances() {
    let filter = SeccompFilter::deny_network_and_ptrace();
    let denied = filter.denied_syscalls();
    let program = filter.compile();
    let n = denied.len();

    // [load nr] [jeq × n] [allow] [errno]
    assert_eq!(program.len(), n + 2 + 1);
    assert!(
        matches!(
            program[0],
            SockFilter {
                code: 0x0020,
                k: 0,
                ..
            }
        ),
        "first instruction loads seccomp_data.nr"
    );
    for (i, nr) in denied.iter().enumerate() {
        let ins = &program[i + 1];
        assert_eq!(ins.code, 0x0015, "JEQ|K encoding at {i}");
        assert_eq!(ins.k, *nr as u32);
        // Classic BPF jumps are relative to the NEXT instruction:
        // target = (i+1) + 1 + jt must equal the ERRNO at n+2.
        assert_eq!(i + 2 + ins.jt as usize, n + 2, "jt at {i}");
        assert_eq!(ins.jf, 0);
    }
    assert_eq!(program[n + 1].k, 0x7fff_0000, "fallthrough returns ALLOW");
    assert_eq!(program[n + 2].k, 0x0005_0001, "deny returns ERRNO|EPERM");
}

#[test]
fn seccomp_stream_bytes_have_no_length_prefix() {
    let filter = SeccompFilter::deny_ptrace_only();
    let program = filter.compile();
    let bytes = filter.to_bpf_bytes();
    assert_eq!(bytes.len(), program.len() * 8);
    // bwrap reads the fd to EOF and divides by sizeof(sock_filter)=8;
    // a leading length would corrupt the first instruction.
    assert_eq!(&bytes[..2], &0x0020u16.to_ne_bytes());
    // x86_64: ptrace is syscall 101 — pinned for the deny set content.
    #[cfg(target_arch = "x86_64")]
    assert!(filter.denied_syscalls().contains(&101));
}
