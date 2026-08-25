//! Syscall filter generation for the Linux backend.
//!
//! Depth-of-defense on top of the network *namespace*: a classic-BPF
//! seccomp filter denying the socket family and ptrace even if a future
//! mount or namespace regression opened a path out. The filter is
//! handed to bubblewrap via `--seccomp <fd>` (see `bwrap.rs`; the fd is
//! pinned with a `dup2` in `pre_exec`).
//!
//! Denied calls return `ERRNO(EPERM)` rather than killing the process:
//! a normal tool sees "network is unavailable", which is the honest
//! projection of `NetworkMode::Disabled` inside the sandbox.

/// Syscall numbers we deny, per architecture. Numbers only — the filter
/// itself is arch-agnostic machine code built by `compile`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SeccompFilter {
    denied: Vec<i64>,
}

// x86_64
#[cfg(target_arch = "x86_64")]
const SYS_PTRACE: i64 = 101;
#[cfg(target_arch = "x86_64")]
const NETWORK_SYSCALLS: &[i64] = &[
    41,  // socket
    42,  // connect
    43,  // accept
    44,  // sendto
    45,  // recvfrom
    46,  // sendmsg
    47,  // recvmsg
    48,  // shutdown
    49,  // bind
    50,  // listen
    51,  // getsockname
    52,  // getpeername
    53,  // socketpair
    288, // accept4
];

// aarch64
#[cfg(target_arch = "aarch64")]
const SYS_PTRACE: i64 = 117;
#[cfg(target_arch = "aarch64")]
const NETWORK_SYSCALLS: &[i64] = &[
    198, // socket
    199, // socketpair
    200, // bind
    201, // listen
    202, // accept
    203, // connect
    204, // getsockname
    205, // getpeername
    206, // sendto
    207, // recvfrom
    210, // shutdown
    211, // sendmsg
    212, // recvmsg
    242, // accept4
];

/// `seccomp_data.nr` offset (validated by layout: nr is the first u64).
const SECCOMP_NR_OFFSET: u8 = 0;

/// `SECCOMP_RET_ERRNO | EPERM`
const RET_ERRNO_EPERM: u32 = 0x0005_0000 | 1;
/// `SECCOMP_RET_ALLOW`
const RET_ALLOW: u32 = 0x7fff_0000;

/// One classic-BPF instruction (`struct sock_filter` layout).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SockFilter {
    pub code: u16,
    pub jt: u8,
    pub jf: u8,
    pub k: u32,
}

// BPF operation encodings (linux/filter.h).
const BPF_LD: u16 = 0x00;
const BPF_W: u16 = 0x00;
const BPF_ABS: u16 = 0x20;
const BPF_JMP: u16 = 0x05;
const BPF_JEQ: u16 = 0x10;
const BPF_K: u16 = 0x00;
const BPF_RET: u16 = 0x06;

impl SeccompFilter {
    /// ptrace is always denied, for every contained plan.
    pub fn deny_ptrace_only() -> Self {
        Self {
            denied: vec![SYS_PTRACE],
        }
    }

    /// ptrace + the whole socket family (network namespace backup).
    pub fn deny_network_and_ptrace() -> Self {
        let mut denied = NETWORK_SYSCALLS.to_vec();
        denied.push(SYS_PTRACE);
        Self { denied }
    }

    /// Compile to classic-BPF instructions:
    /// load nr; for each denied nr: if equal jump-forward to the ERRNO
    /// return; fallthrough returns ALLOW.
    pub fn compile(&self) -> Vec<SockFilter> {
        // Layout: [load nr] [jeq nr -> errno] * n [allow] [errno]
        let mut program = Vec::with_capacity(self.denied.len() + 3);
        program.push(SockFilter {
            code: BPF_LD | BPF_W | BPF_ABS,
            jt: 0,
            jf: 0,
            k: SECCOMP_NR_OFFSET as u32,
        });
        // Jump-true lands on the last instruction (ERRNO). Classic BPF
        // jt/jf offsets are relative to the *next* instruction
        // (target = pc + 1 + jt), so the JEQ for denied[index] at
        // program position 1+index needs jt = n - index to reach the
        // ERRNO at position n+2. False falls through to the next check.
        let n = self.denied.len();
        for (index, nr) in self.denied.iter().enumerate() {
            let distance_to_errno = (n - index) as u8;
            program.push(SockFilter {
                code: BPF_JMP | BPF_JEQ | BPF_K,
                jt: distance_to_errno,
                jf: 0,
                k: *nr as u32,
            });
        }
        program.push(SockFilter {
            code: BPF_RET | BPF_K,
            jt: 0,
            jf: 0,
            k: RET_ALLOW,
        });
        program.push(SockFilter {
            code: BPF_RET | BPF_K,
            jt: 0,
            jf: 0,
            k: RET_ERRNO_EPERM,
        });
        program
    }

    /// Serialize into the stream format bwrap's `--seccomp FD` expects:
    /// the raw cBPF instruction sequence as produced by libseccomp's
    /// `seccomp_export_bpf` — no length prefix; bwrap reads the fd to
    /// EOF and divides by `sizeof(struct sock_filter)` (8).
    pub fn to_bpf_bytes(&self) -> Vec<u8> {
        let program = self.compile();
        let mut bytes = Vec::with_capacity(program.len() * 8);
        for ins in &program {
            bytes.extend_from_slice(&ins.code.to_ne_bytes());
            bytes.extend_from_slice(&[ins.jt, ins.jf]);
            bytes.extend_from_slice(&ins.k.to_ne_bytes());
        }
        bytes
    }
}

/// Denied syscall numbers (for tests and projections).
impl SeccompFilter {
    pub fn denied_syscalls(&self) -> &[i64] {
        &self.denied
    }
}
