//! Launcher-contract-only test doubles.
//!
//! These types live outside the general fixture module so integration tests
//! that only need a filesystem root do not compile unrelated fake backends.

use std::sync::Mutex;

use agendao_sandbox::{
    BackendChild, BackendExit, BackendProbe, ChildEnvironment, ProcessMode, SandboxBackend,
    SandboxPlan, SpawnSpec, StdioPlan,
};
use async_trait::async_trait;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordedSpawn {
    pub fingerprint: String,
    pub program: String,
    pub process_mode: ProcessMode,
    /// Fully resolved environment the backend was told to apply.
    pub env: Vec<(String, String)>,
}

pub struct FakeBackend {
    pub probe: BackendProbe,
    pub spawns: Mutex<Vec<RecordedSpawn>>,
}

impl FakeBackend {
    pub fn available() -> Self {
        Self {
            probe: BackendProbe::available(),
            spawns: Mutex::new(Vec::new()),
        }
    }

    pub fn unavailable(reason: &str) -> Self {
        Self {
            probe: BackendProbe::unavailable(reason),
            spawns: Mutex::new(Vec::new()),
        }
    }

    pub fn recorded(&self) -> Vec<RecordedSpawn> {
        self.spawns.lock().expect("fake backend spawns").clone()
    }
}

#[async_trait]
impl SandboxBackend for FakeBackend {
    fn name(&self) -> &'static str {
        "fake"
    }

    fn probe(&self) -> BackendProbe {
        self.probe.clone()
    }

    fn supports(&self, _plan: &SandboxPlan) -> bool {
        true
    }

    async fn spawn(
        &self,
        plan: &SandboxPlan,
        spec: &SpawnSpec,
        env: &ChildEnvironment,
        _stdio: &StdioPlan,
        _violation_token: agendao_sandbox::BackendViolationToken,
    ) -> Result<Box<dyn BackendChild>, agendao_sandbox::SandboxExecutionError> {
        self.spawns
            .lock()
            .expect("fake backend spawns")
            .push(RecordedSpawn {
                fingerprint: plan.fingerprint.clone(),
                program: spec.program.clone(),
                process_mode: plan.process.mode,
                env: env.iter().map(|(k, v)| (k.clone(), v.clone())).collect(),
            });
        Ok(Box::new(FakeChild {
            signals: Mutex::new(Vec::new()),
            exit: BackendExit {
                success: true,
                code: Some(0),
                signal: None,
            },
        }))
    }
}

struct FakeChild {
    signals: Mutex<Vec<&'static str>>,
    exit: BackendExit,
}

#[async_trait]
impl BackendChild for FakeChild {
    fn pid(&self) -> Option<u32> {
        Some(4242)
    }

    async fn wait(&mut self) -> Result<BackendExit, agendao_sandbox::SandboxExecutionError> {
        Ok(self.exit)
    }

    async fn signal_term(&mut self) -> Result<(), agendao_sandbox::SandboxExecutionError> {
        self.signals.lock().expect("fake child").push("TERM");
        Ok(())
    }

    async fn signal_kill(&mut self) -> Result<(), agendao_sandbox::SandboxExecutionError> {
        self.signals.lock().expect("fake child").push("KILL");
        Ok(())
    }
}

pub struct SequentialIds(Mutex<std::cell::Cell<u32>>);

impl agendao_sandbox::ExecutionIdMinter for SequentialIds {
    fn mint(&self) -> String {
        let cell = self.0.lock().expect("id minter");
        let next = cell.get() + 1;
        cell.set(next);
        format!("exec-{next:04}")
    }
}

pub fn sequential_minter() -> std::sync::Arc<SequentialIds> {
    std::sync::Arc::new(SequentialIds(Mutex::new(std::cell::Cell::new(0))))
}
