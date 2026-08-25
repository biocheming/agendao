//! # Terminal Lifecycle & External Editor Handoff
//!
//! ## Architectural & Runtime Constraints Note
//! - **Integration Harness**: `AdaptiveEventReader` provides a validated protocol for `request_pause`,
//!   `resume_reader`, generation tracking, and `InputEventEnvelope` invalidation.
//! - **Production Runtime Boundary**: `revue`'s `App::run` loop hardcodes its internal `EventReader`
//!   and does not expose a custom event source stream. Therefore, in production synchronous `App::run`
//!   handling, `TerminalLifecycle` operates with `reader_control: None` (blocking TUI events naturally
//!   during the handler block), while the `AdaptiveEventReader` remains a fully validated component
//!   for decoupled/custom event loop architectures.

use std::fmt;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use tokio::sync::{mpsc, oneshot};

/// 输入事件代际标记（代际递增后，旧代际的滞留按键将被完全忽略）
pub type InputGeneration = u64;

/// Handoff 挂起失败聚合结构（携带主失败与所有回滚失败，防静默丢失）
#[derive(Debug, Clone)]
pub struct HandoffFailure {
    pub primary: HandoffError,
    pub rollback_errors: Vec<HandoffError>,
    pub state: SuspendedTerminalState,
}

impl fmt::Display for HandoffFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Primary handoff failure: {}", self.primary)?;
        if !self.rollback_errors.is_empty() {
            write!(f, " (Rollback failures: {:?})", self.rollback_errors)?;
        }
        Ok(())
    }
}

impl std::error::Error for HandoffFailure {}

/// Handoff 错误枚举
#[derive(Debug, Clone)]
pub enum HandoffError {
    ReaderPauseTimeout(String),
    ReaderResumeTimeout(String),
    SuspendFailed(String),
    ResumeFailed(String),
    RollbackFailed(String),
    SuspendFailure(Box<HandoffFailure>),
    NoEditorFound,
    EditorParseError(String, String),
    ProcessLaunchError(String),
    DraftIoError(String),
    DraftConflict {
        expected_session: String,
        actual_session: String,
        expected_revision: u64,
        actual_revision: u64,
    },
    NonZeroExit(Option<i32>),
}

impl fmt::Display for HandoffError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ReaderPauseTimeout(s) => {
                write!(f, "EventReader failed to acknowledge pause: {s}")
            }
            Self::ReaderResumeTimeout(s) => {
                write!(f, "EventReader failed to acknowledge resume: {s}")
            }
            Self::SuspendFailed(s) => write!(f, "Terminal suspend step failed: {s}"),
            Self::ResumeFailed(s) => write!(f, "Terminal resume step failed: {s}"),
            Self::RollbackFailed(s) => write!(f, "Terminal rollback failed: {s}"),
            Self::SuspendFailure(fail) => write!(f, "Suspend failure with diagnostics: {fail}"),
            Self::NoEditorFound => {
                write!(f, "No external editor found. Please set $VISUAL or $EDITOR")
            }
            Self::EditorParseError(cmd, err) => {
                write!(f, "Failed to parse editor command '{cmd}': {err}")
            }
            Self::ProcessLaunchError(err) => write!(f, "Failed to launch editor process: {err}"),
            Self::DraftIoError(err) => write!(f, "Temporary draft file I/O error: {err}"),
            Self::DraftConflict {
                expected_session,
                actual_session,
                expected_revision,
                actual_revision,
            } => {
                write!(f, "Draft conflict detected: session({expected_session}->{actual_session}), revision({expected_revision}->{actual_revision})")
            }
            Self::NonZeroExit(code) => write!(f, "Editor exited with non-zero status: {code:?}"),
        }
    }
}

impl std::error::Error for HandoffError {}

/// 终端挂起状态记录（用于严格的部分失败回滚）
#[derive(Default, Debug, Clone)]
pub struct SuspendedTerminalState {
    pub reader_paused: bool,
    pub mouse_disabled: bool,
    pub paste_disabled: bool,
    pub alternate_screen_left: bool,
    pub raw_mode_disabled: bool,
    pub cursor_restored: bool,
}

/// 终端操作后端抽象（支持真实终端与 Mock 测试）
pub trait TerminalBackend {
    fn disable_mouse_capture(&mut self) -> Result<(), io::Error>;
    fn enable_mouse_capture(&mut self) -> Result<(), io::Error>;

    fn disable_bracketed_paste(&mut self) -> Result<(), io::Error>;
    fn enable_bracketed_paste(&mut self) -> Result<(), io::Error>;

    fn leave_alternate_screen(&mut self) -> Result<(), io::Error>;
    fn enter_alternate_screen(&mut self) -> Result<(), io::Error>;

    fn disable_raw_mode(&mut self) -> Result<(), io::Error>;
    fn enable_raw_mode(&mut self) -> Result<(), io::Error>;

    fn restore_cursor(&mut self) -> Result<(), io::Error>;
    fn apply_cursor_policy(&mut self, show: bool) -> Result<(), io::Error>;
    fn flush(&mut self) -> Result<(), io::Error>;
}

/// 生产环境真实 Crossterm 终端操作后端
pub struct CrosstermTerminalBackend;

impl TerminalBackend for CrosstermTerminalBackend {
    fn disable_mouse_capture(&mut self) -> Result<(), io::Error> {
        crossterm::execute!(io::stdout(), crossterm::event::DisableMouseCapture)
    }
    fn enable_mouse_capture(&mut self) -> Result<(), io::Error> {
        crossterm::execute!(io::stdout(), crossterm::event::EnableMouseCapture)
    }
    fn disable_bracketed_paste(&mut self) -> Result<(), io::Error> {
        crossterm::execute!(io::stdout(), crossterm::event::DisableBracketedPaste)
    }
    fn enable_bracketed_paste(&mut self) -> Result<(), io::Error> {
        crossterm::execute!(io::stdout(), crossterm::event::EnableBracketedPaste)
    }
    fn leave_alternate_screen(&mut self) -> Result<(), io::Error> {
        crossterm::execute!(io::stdout(), crossterm::terminal::LeaveAlternateScreen)
    }
    fn enter_alternate_screen(&mut self) -> Result<(), io::Error> {
        crossterm::execute!(io::stdout(), crossterm::terminal::EnterAlternateScreen)
    }
    fn disable_raw_mode(&mut self) -> Result<(), io::Error> {
        crossterm::terminal::disable_raw_mode()
    }
    fn enable_raw_mode(&mut self) -> Result<(), io::Error> {
        crossterm::terminal::enable_raw_mode()
    }
    fn restore_cursor(&mut self) -> Result<(), io::Error> {
        crossterm::execute!(io::stdout(), crossterm::cursor::Show)
    }
    fn apply_cursor_policy(&mut self, show: bool) -> Result<(), io::Error> {
        if show {
            crossterm::execute!(io::stdout(), crossterm::cursor::Show)
        } else {
            crossterm::execute!(io::stdout(), crossterm::cursor::Hide)
        }
    }
    fn flush(&mut self) -> Result<(), io::Error> {
        io::stdout().flush()
    }
}

/// 带有代际标记的输入事件信封（用于丢弃挂起期间的陈旧输入）
#[derive(Debug, Clone)]
pub struct InputEventEnvelope<E> {
    pub generation: InputGeneration,
    pub event: E,
}

impl<E> InputEventEnvelope<E> {
    pub fn new(generation: InputGeneration, event: E) -> Self {
        Self { generation, event }
    }

    /// 校验事件代际，旧代际事件直接失效丢弃
    pub fn is_valid(&self, current_generation: InputGeneration) -> bool {
        self.generation == current_generation
    }
}

/// 后台 EventReader 暂停与恢复控制句柄
#[derive(Clone)]
pub struct EventReaderControl {
    generation: Arc<AtomicU64>,
    pause_tx: mpsc::Sender<oneshot::Sender<()>>,
    resume_tx: mpsc::Sender<oneshot::Sender<()>>,
}

impl EventReaderControl {
    pub fn new(
        generation: Arc<AtomicU64>,
        pause_tx: mpsc::Sender<oneshot::Sender<()>>,
        resume_tx: mpsc::Sender<oneshot::Sender<()>>,
    ) -> Self {
        Self {
            generation,
            pause_tx,
            resume_tx,
        }
    }

    /// 当前有效代际
    pub fn current_generation(&self) -> InputGeneration {
        self.generation.load(Ordering::SeqCst)
    }

    /// 推进代际（旧代际按键将自动失效）
    pub fn advance_generation(&self) -> InputGeneration {
        self.generation.fetch_add(1, Ordering::SeqCst) + 1
    }

    /// 请求 EventReader 暂停并同步等待确认握手 (Acknowledgement)
    pub async fn request_pause(&self) -> Result<(), HandoffError> {
        let (ack_tx, ack_rx) = oneshot::channel();
        self.pause_tx
            .send(ack_tx)
            .await
            .map_err(|e| HandoffError::ReaderPauseTimeout(e.to_string()))?;

        tokio::time::timeout(std::time::Duration::from_millis(500), ack_rx)
            .await
            .map_err(|_| HandoffError::ReaderPauseTimeout("Acknowledgement timed out".into()))?
            .map_err(|_| HandoffError::ReaderPauseTimeout("Acknowledgement sender dropped".into()))
    }

    /// 请求 EventReader 恢复并同步等待确认握手
    pub async fn resume_reader(&self) -> Result<(), HandoffError> {
        let (ack_tx, ack_rx) = oneshot::channel();
        self.resume_tx
            .send(ack_tx)
            .await
            .map_err(|e| HandoffError::ReaderResumeTimeout(e.to_string()))?;

        tokio::time::timeout(std::time::Duration::from_millis(500), ack_rx)
            .await
            .map_err(|_| {
                HandoffError::ReaderResumeTimeout("Resume acknowledgement timed out".into())
            })?
            .map_err(|_| {
                HandoffError::ReaderResumeTimeout("Resume acknowledgement sender dropped".into())
            })
    }
}

/// 生产级 EventReader 循环适配器（支持可中断 poll 与 generation 派发）
pub struct AdaptiveEventReader {
    generation: Arc<AtomicU64>,
    pause_rx: mpsc::Receiver<oneshot::Sender<()>>,
    resume_rx: mpsc::Receiver<oneshot::Sender<()>>,
}

impl AdaptiveEventReader {
    pub fn new_pair() -> (EventReaderControl, Self) {
        let generation = Arc::new(AtomicU64::new(1));
        let (pause_tx, pause_rx) = mpsc::channel(1);
        let (resume_tx, resume_rx) = mpsc::channel(1);

        let control = EventReaderControl::new(generation.clone(), pause_tx, resume_tx);
        let reader = Self {
            generation,
            pause_rx,
            resume_rx,
        };
        (control, reader)
    }

    /// 真实事件循环辅助：检查暂停并在暂停期间等待恢复信号
    pub async fn check_lifecycle_control(&mut self) {
        // 若收到暂停请求，发送 ACK 并一直等待恢复信号
        if let Ok(ack) = self.pause_rx.try_recv() {
            let _ = ack.send(());
            if let Some(resume_ack) = self.resume_rx.recv().await {
                let _ = resume_ack.send(());
            }
        }
    }

    /// 启动后台事件监听协程：
    /// 1. 在每个 poll/read 周期前检查 pause 请求并发送 ACK；
    /// 2. 在暂停期间等待 resume 信号并应答 ACK；
    /// 3. 为产生的所有事件盖上当时的 generation 代际信封；
    /// 4. 派发给 output_tx 通道。
    pub fn spawn_loop<E, F, Fut>(
        mut self,
        handle: &tokio::runtime::Handle,
        event_producer: F,
        output_tx: tokio::sync::mpsc::Sender<InputEventEnvelope<E>>,
    ) -> tokio::task::JoinHandle<()>
    where
        E: Send + 'static,
        F: Fn() -> Fut + Send + 'static,
        Fut: std::future::Future<Output = Option<E>> + Send + 'static,
    {
        handle.spawn(async move {
            loop {
                let gen = self.current_generation();
                tokio::select! {
                    Some(ack) = self.pause_rx.recv() => {
                        let _ = ack.send(());
                        if let Some(resume_ack) = self.resume_rx.recv().await {
                            let _ = resume_ack.send(());
                        }
                    }
                    maybe_ev = event_producer() => {
                        match maybe_ev {
                            Some(ev) => {
                                let envelope = InputEventEnvelope::new(gen, ev);
                                if output_tx.send(envelope).await.is_err() {
                                    break;
                                }
                            }
                            None => break,
                        }
                    }
                }
            }
        })
    }

    pub fn current_generation(&self) -> InputGeneration {
        self.generation.load(Ordering::SeqCst)
    }
}

/// 外部编辑器解析器
pub struct EditorResolver;

impl EditorResolver {
    /// 优先级：$VISUAL -> $EDITOR -> 系统常用 (nano, vim, vi)
    pub fn resolve_editor() -> Result<(PathBuf, Vec<String>), HandoffError> {
        let raw = std::env::var("VISUAL")
            .ok()
            .filter(|s| !s.trim().is_empty())
            .or_else(|| {
                std::env::var("EDITOR")
                    .ok()
                    .filter(|s| !s.trim().is_empty())
            });

        if let Some(cmd) = raw {
            return Self::parse_editor_command(&cmd);
        }

        // 探测系统自带编辑器
        for fallback in &["nano", "vim", "vi"] {
            if let Some(path) = Self::find_executable_in_path(fallback) {
                return Ok((path, Vec::new()));
            }
        }

        Err(HandoffError::NoEditorFound)
    }

    /// 在 PATH 中查找且必须具备可执行权限
    fn find_executable_in_path(name: &str) -> Option<PathBuf> {
        if let Ok(paths) = std::env::var("PATH") {
            for dir in std::env::split_paths(&paths) {
                let candidate = dir.join(name);
                if candidate.is_file() {
                    #[cfg(unix)]
                    {
                        use std::os::unix::fs::PermissionsExt;
                        if let Ok(meta) = candidate.metadata() {
                            if meta.permissions().mode() & 0o111 != 0 {
                                return Some(candidate);
                            }
                        }
                    }
                    #[cfg(not(unix))]
                    {
                        return Some(candidate);
                    }
                }
            }
        }
        None
    }

    /// 解析 argv（简单稳健的引号参数切分）
    pub fn parse_editor_command(command_str: &str) -> Result<(PathBuf, Vec<String>), HandoffError> {
        let words = Self::split_words(command_str)?;
        if words.is_empty() {
            return Err(HandoffError::NoEditorFound);
        }

        let program_name = &words[0];
        let program_path = if Path::new(program_name).is_absolute() || program_name.contains('/') {
            PathBuf::from(program_name)
        } else {
            Self::find_executable_in_path(program_name).ok_or_else(|| {
                HandoffError::EditorParseError(
                    command_str.to_string(),
                    "Executable not found in PATH or not executable".into(),
                )
            })?
        };

        let args = words[1..].to_vec();
        Ok((program_path, args))
    }

    fn split_words(s: &str) -> Result<Vec<String>, HandoffError> {
        let mut words = Vec::new();
        let mut current = String::new();
        let mut in_single = false;
        let mut in_double = false;
        let mut escaped = false;

        for c in s.chars() {
            if in_single {
                if c == '\'' {
                    in_single = false;
                } else {
                    current.push(c);
                }
                continue;
            }

            if escaped {
                current.push(c);
                escaped = false;
                continue;
            }

            match c {
                '\\' => escaped = true,
                '\'' if !in_double => in_single = true,
                '"' if !in_single => in_double = !in_double,
                ' ' | '\t' if !in_double => {
                    if !current.is_empty() {
                        words.push(current);
                        current = String::new();
                    }
                }
                _ => current.push(c),
            }
        }

        if in_single || in_double {
            return Err(HandoffError::EditorParseError(
                s.to_string(),
                "Unmatched quote in command".into(),
            ));
        }

        if !current.is_empty() {
            words.push(current);
        }

        Ok(words)
    }
}

/// 临时草稿文件管理器（0600 原子创建与退出清理）
pub struct DraftFileManager {
    temp_path: PathBuf,
}

impl DraftFileManager {
    pub fn create_temp_draft(initial_content: &str) -> Result<Self, HandoffError> {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let filename = format!("agendao_draft_{}_{}.md", std::process::id(), now);
        let temp_path = std::env::temp_dir().join(filename);

        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            let mut file = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(&temp_path)
                .map_err(|e| {
                    HandoffError::DraftIoError(format!("Failed to create 0600 temp file: {e}"))
                })?;
            file.write_all(initial_content.as_bytes()).map_err(|e| {
                HandoffError::DraftIoError(format!("Failed to write initial content: {e}"))
            })?;
        }
        #[cfg(not(unix))]
        {
            let mut file = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&temp_path)
                .map_err(|e| {
                    HandoffError::DraftIoError(format!("Failed to create temp file: {e}"))
                })?;
            file.write_all(initial_content.as_bytes()).map_err(|e| {
                HandoffError::DraftIoError(format!("Failed to write initial content: {e}"))
            })?;
        }

        Ok(Self { temp_path })
    }

    pub fn path(&self) -> &Path {
        &self.temp_path
    }

    pub fn read_content(&self) -> Result<String, HandoffError> {
        std::fs::read_to_string(&self.temp_path)
            .map_err(|e| HandoffError::DraftIoError(format!("Failed to read draft file: {e}")))
    }
}

impl Drop for DraftFileManager {
    fn drop(&mut self) {
        if self.temp_path.exists() {
            let _ = std::fs::remove_file(&self.temp_path);
        }
    }
}

/// 草稿事务目标（防止 Session 切换或草稿并发修改造成串号覆盖）
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DraftTarget {
    pub session_id: String,
    pub draft_revision: u64,
}

/// Handoff 执行结果
#[derive(Debug, PartialEq, Eq)]
pub enum HandoffOutcome {
    Success {
        new_content: String,
        requires_full_redraw: bool,
    },
    ConflictRetained {
        stash_content: String,
        error_message: String,
    },
    Cancelled,
}

/// 终端生命周期管理器 (Terminal Lifecycle Authority)
pub struct TerminalLifecycle<B: TerminalBackend> {
    backend: B,
    reader_control: Option<EventReaderControl>,
}

impl<B: TerminalBackend> TerminalLifecycle<B> {
    pub fn new(backend: B, reader_control: Option<EventReaderControl>) -> Self {
        Self {
            backend,
            reader_control,
        }
    }

    /// 执行受控挂起 (Suspend) 并返回聚合失败状态（不丢弃任何回滚错误）
    pub async fn suspend(&mut self) -> Result<SuspendedTerminalState, HandoffFailure> {
        let mut state = SuspendedTerminalState::default();

        // 1. 暂停 reader 并等待握手确认
        if let Some(ref reader) = self.reader_control {
            if let Err(e) = reader.request_pause().await {
                let r_errs = self.rollback_suspend(&mut state).await;
                return Err(HandoffFailure {
                    primary: e,
                    rollback_errors: r_errs,
                    state,
                });
            }
            state.reader_paused = true;
        }

        // 2. 禁用鼠标捕获
        if let Err(e) = self.backend.disable_mouse_capture() {
            let r_errs = self.rollback_suspend(&mut state).await;
            return Err(HandoffFailure {
                primary: HandoffError::SuspendFailed(e.to_string()),
                rollback_errors: r_errs,
                state,
            });
        }
        state.mouse_disabled = true;

        // 3. 禁用括号粘贴 (Bracketed Paste)
        if let Err(e) = self.backend.disable_bracketed_paste() {
            let r_errs = self.rollback_suspend(&mut state).await;
            return Err(HandoffFailure {
                primary: HandoffError::SuspendFailed(e.to_string()),
                rollback_errors: r_errs,
                state,
            });
        }
        state.paste_disabled = true;

        // 4. 离开 Alternate Screen
        if let Err(e) = self.backend.leave_alternate_screen() {
            let r_errs = self.rollback_suspend(&mut state).await;
            return Err(HandoffFailure {
                primary: HandoffError::SuspendFailed(e.to_string()),
                rollback_errors: r_errs,
                state,
            });
        }
        state.alternate_screen_left = true;

        // 5. 禁用 Raw Mode
        if let Err(e) = self.backend.disable_raw_mode() {
            let r_errs = self.rollback_suspend(&mut state).await;
            return Err(HandoffFailure {
                primary: HandoffError::SuspendFailed(e.to_string()),
                rollback_errors: r_errs,
                state,
            });
        }
        state.raw_mode_disabled = true;

        // 6. 恢复标准光标
        if let Err(e) = self.backend.restore_cursor() {
            let r_errs = self.rollback_suspend(&mut state).await;
            return Err(HandoffFailure {
                primary: HandoffError::SuspendFailed(e.to_string()),
                rollback_errors: r_errs,
                state,
            });
        }
        state.cursor_restored = true;

        let _ = self.backend.flush();
        Ok(state)
    }

    /// 部分失败异步回滚 (Rollback)
    pub async fn rollback_suspend(
        &mut self,
        state: &mut SuspendedTerminalState,
    ) -> Vec<HandoffError> {
        let mut errs = Vec::new();
        if state.raw_mode_disabled {
            if let Err(e) = self.backend.enable_raw_mode() {
                errs.push(HandoffError::RollbackFailed(format!(
                    "enable_raw_mode: {e}"
                )));
            } else {
                state.raw_mode_disabled = false;
            }
        }
        if state.alternate_screen_left {
            if let Err(e) = self.backend.enter_alternate_screen() {
                errs.push(HandoffError::RollbackFailed(format!(
                    "enter_alternate_screen: {e}"
                )));
            } else {
                state.alternate_screen_left = false;
            }
        }
        if state.paste_disabled {
            if let Err(e) = self.backend.enable_bracketed_paste() {
                errs.push(HandoffError::RollbackFailed(format!(
                    "enable_bracketed_paste: {e}"
                )));
            } else {
                state.paste_disabled = false;
            }
        }
        if state.mouse_disabled {
            if let Err(e) = self.backend.enable_mouse_capture() {
                errs.push(HandoffError::RollbackFailed(format!(
                    "enable_mouse_capture: {e}"
                )));
            } else {
                state.mouse_disabled = false;
            }
        }
        if state.reader_paused {
            if let Some(ref reader) = self.reader_control {
                reader.advance_generation();
                if let Err(e) = reader.resume_reader().await {
                    errs.push(e);
                } else {
                    state.reader_paused = false;
                }
            }
        }
        errs
    }

    /// 执行逆序受控恢复 (Resume)
    pub async fn resume(&mut self, state: &mut SuspendedTerminalState) -> Result<(), HandoffError> {
        let mut errs = Vec::new();

        // 1. 恢复 raw mode
        if state.raw_mode_disabled {
            if let Err(e) = self.backend.enable_raw_mode() {
                errs.push(format!("enable_raw_mode failed: {e}"));
            } else {
                state.raw_mode_disabled = false;
            }
        }

        // 2. 进入 alternate screen
        if state.alternate_screen_left {
            if let Err(e) = self.backend.enter_alternate_screen() {
                errs.push(format!("enter_alternate_screen failed: {e}"));
            } else {
                state.alternate_screen_left = false;
            }
        }

        // 3. 启用括号粘贴
        if state.paste_disabled {
            if let Err(e) = self.backend.enable_bracketed_paste() {
                errs.push(format!("enable_bracketed_paste failed: {e}"));
            } else {
                state.paste_disabled = false;
            }
        }

        // 4. 启用鼠标捕获
        if state.mouse_disabled {
            if let Err(e) = self.backend.enable_mouse_capture() {
                errs.push(format!("enable_mouse_capture failed: {e}"));
            } else {
                state.mouse_disabled = false;
            }
        }

        // 5. 恢复 TUI 光标策略
        if let Err(e) = self.backend.apply_cursor_policy(true) {
            errs.push(format!("apply_cursor_policy failed: {e}"));
        }
        if let Err(e) = self.backend.flush() {
            errs.push(format!("flush failed: {e}"));
        }

        // 6. 推进输入代际（使编辑期间的旧按键作废），最后恢复 reader
        if state.reader_paused {
            if let Some(ref reader) = self.reader_control {
                reader.advance_generation();
                if let Err(e) = reader.resume_reader().await {
                    errs.push(format!("resume_reader failed: {e}"));
                } else {
                    state.reader_paused = false;
                }
            }
        }

        if !errs.is_empty() {
            Err(HandoffError::ResumeFailed(errs.join("; ")))
        } else {
            Ok(())
        }
    }

    /// 执行完整外部编辑器 Handoff 事务（保留完整 HandoffFailure 诊断）
    pub async fn execute_editor_handoff<F>(
        &mut self,
        target: DraftTarget,
        initial_content: &str,
        current_state_provider: F,
    ) -> Result<HandoffOutcome, HandoffError>
    where
        F: FnOnce() -> (String, u64),
    {
        let (editor_path, editor_args) = EditorResolver::resolve_editor()?;
        let draft_manager = DraftFileManager::create_temp_draft(initial_content)?;

        let mut suspended = self
            .suspend()
            .await
            .map_err(|f| HandoffError::SuspendFailure(Box::new(f)))?;

        // 启动外部编辑器进程
        let mut cmd = std::process::Command::new(&editor_path);
        cmd.args(&editor_args);
        cmd.arg(draft_manager.path());

        let status_res = cmd.status();

        // 无条件恢复终端
        let resume_res = self.resume(&mut suspended).await;
        if let Err(e) = resume_res {
            return Err(e);
        }

        let status = status_res.map_err(|e| HandoffError::ProcessLaunchError(e.to_string()))?;
        if !status.success() {
            return Ok(HandoffOutcome::Cancelled);
        }

        let edited_content = draft_manager.read_content()?;

        // 外部进程退出且恢复终端后，重新动态读取最新的权威状态 (Session ID 与 Revision)
        let (current_session_id, current_revision) = current_state_provider();

        // 校验目标 Session 与草稿版本号是否发生冲突
        if current_session_id != target.session_id || current_revision != target.draft_revision {
            return Ok(HandoffOutcome::ConflictRetained {
                stash_content: edited_content,
                error_message: format!(
                    "Draft conflict: session ({}) or revision ({}) modified during external edit",
                    target.session_id, target.draft_revision
                ),
            });
        }

        Ok(HandoffOutcome::Success {
            new_content: edited_content,
            requires_full_redraw: true,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Mutex, OnceLock};

    struct EditorEnvGuard {
        _guard: std::sync::MutexGuard<'static, ()>,
        orig_visual: Option<String>,
        orig_editor: Option<String>,
    }

    impl Drop for EditorEnvGuard {
        fn drop(&mut self) {
            match &self.orig_visual {
                Some(v) => std::env::set_var("VISUAL", v),
                None => std::env::remove_var("VISUAL"),
            }
            match &self.orig_editor {
                Some(e) => std::env::set_var("EDITOR", e),
                None => std::env::remove_var("EDITOR"),
            }
        }
    }

    // `VISUAL`/`EDITOR` are process-global. Serialize the tests that mutate
    // them and restore the original env on drop so the full suite cannot race or pollute.
    fn editor_env_lock() -> EditorEnvGuard {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        let guard = LOCK.get_or_init(|| Mutex::new(())).lock().unwrap();
        EditorEnvGuard {
            _guard: guard,
            orig_visual: std::env::var("VISUAL").ok(),
            orig_editor: std::env::var("EDITOR").ok(),
        }
    }

    #[derive(Default)]
    struct MockTerminalBackend {
        pub raw_mode: bool,
        pub alternate_screen: bool,
        pub mouse_captured: bool,
        pub bracketed_paste: bool,
        pub cursor_policy_applied: bool,
        pub fail_at_step: Option<&'static str>,
    }

    impl TerminalBackend for MockTerminalBackend {
        fn disable_mouse_capture(&mut self) -> Result<(), io::Error> {
            if self.fail_at_step == Some("disable_mouse_capture") {
                return Err(io::Error::new(io::ErrorKind::Other, "mock error"));
            }
            self.mouse_captured = false;
            Ok(())
        }
        fn enable_mouse_capture(&mut self) -> Result<(), io::Error> {
            self.mouse_captured = true;
            Ok(())
        }
        fn disable_bracketed_paste(&mut self) -> Result<(), io::Error> {
            if self.fail_at_step == Some("disable_bracketed_paste") {
                return Err(io::Error::new(io::ErrorKind::Other, "mock error"));
            }
            self.bracketed_paste = false;
            Ok(())
        }
        fn enable_bracketed_paste(&mut self) -> Result<(), io::Error> {
            self.bracketed_paste = true;
            Ok(())
        }
        fn leave_alternate_screen(&mut self) -> Result<(), io::Error> {
            self.alternate_screen = false;
            Ok(())
        }
        fn enter_alternate_screen(&mut self) -> Result<(), io::Error> {
            self.alternate_screen = true;
            Ok(())
        }
        fn disable_raw_mode(&mut self) -> Result<(), io::Error> {
            self.raw_mode = false;
            Ok(())
        }
        fn enable_raw_mode(&mut self) -> Result<(), io::Error> {
            self.raw_mode = true;
            Ok(())
        }
        fn restore_cursor(&mut self) -> Result<(), io::Error> {
            Ok(())
        }
        fn apply_cursor_policy(&mut self, _show: bool) -> Result<(), io::Error> {
            self.cursor_policy_applied = true;
            Ok(())
        }
        fn flush(&mut self) -> Result<(), io::Error> {
            Ok(())
        }
    }

    #[tokio::test]
    async fn test_suspend_resume_and_partial_failure_rollback() {
        // 1. 成功路径
        let backend = MockTerminalBackend {
            raw_mode: true,
            alternate_screen: true,
            mouse_captured: true,
            bracketed_paste: true,
            cursor_policy_applied: false,
            fail_at_step: None,
        };
        let mut lifecycle = TerminalLifecycle::new(backend, None);

        let mut state = lifecycle.suspend().await.unwrap();
        assert!(state.raw_mode_disabled);
        assert!(state.alternate_screen_left);

        lifecycle.resume(&mut state).await.unwrap();
        assert!(lifecycle.backend.raw_mode);
        assert!(lifecycle.backend.alternate_screen);
        assert!(lifecycle.backend.cursor_policy_applied);

        // 2. 部分挂起失败回滚
        let failing_backend = MockTerminalBackend {
            raw_mode: true,
            alternate_screen: true,
            mouse_captured: true,
            bracketed_paste: true,
            cursor_policy_applied: false,
            fail_at_step: Some("disable_bracketed_paste"),
        };
        let mut fail_lifecycle = TerminalLifecycle::new(failing_backend, None);

        let err = fail_lifecycle.suspend().await;
        assert!(err.is_err());
        let failure = err.unwrap_err();
        assert!(matches!(failure.primary, HandoffError::SuspendFailed(..)));
        // 验证回滚：之前禁用的 mouse capture 被重新恢复
        assert!(fail_lifecycle.backend.mouse_captured);
    }

    #[tokio::test]
    async fn test_adaptive_event_reader_lifecycle_and_generation_dispatch() {
        let (control, mut reader) = AdaptiveEventReader::new_pair();

        assert_eq!(reader.current_generation(), 1);

        // 异步任务模拟应用层请求 pause
        let ctrl_clone = control.clone();
        let pause_handle = tokio::spawn(async move {
            ctrl_clone.request_pause().await.unwrap();
        });

        // 模拟真实 EventReader 循环触发 check_lifecycle_control
        // 这里检测到 pause 请求并发送 ACK，然后等待 resume
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;

        let ctrl_resume = control.clone();
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            ctrl_resume.advance_generation();
            ctrl_resume.resume_reader().await.unwrap();
        });

        reader.check_lifecycle_control().await;
        pause_handle.await.unwrap();

        // 验证代际已成功推进为 2
        assert_eq!(reader.current_generation(), 2);
    }

    #[test]
    fn test_editor_resolver_and_shell_words_parsing() {
        let (path, args) = EditorResolver::parse_editor_command("ls -la /tmp").unwrap();
        assert!(path.to_str().unwrap().contains("ls"));
        assert_eq!(args, vec!["-la", "/tmp"]);

        let (_path2, args2) = EditorResolver::parse_editor_command("ls 'foo bar'").unwrap();
        assert_eq!(args2, vec!["foo bar"]);

        let err = EditorResolver::parse_editor_command("nonexistent_binary_xyz_123");
        assert!(matches!(err, Err(HandoffError::EditorParseError(..))));
    }

    #[test]
    fn test_draft_file_manager_creation_and_cleanup() {
        let draft = DraftFileManager::create_temp_draft("Initial text").unwrap();
        let path = draft.path().to_path_buf();
        assert!(path.exists());
        assert_eq!(draft.read_content().unwrap(), "Initial text");

        drop(draft);
        assert!(!path.exists());
    }

    #[tokio::test]
    async fn test_handoff_transaction_draft_conflict_and_failure_propagation() {
        let _editor_env_guard = editor_env_lock();
        let backend = MockTerminalBackend::default();
        let mut lifecycle = TerminalLifecycle::new(backend, None);

        let target = DraftTarget {
            session_id: "sess_1".into(),
            draft_revision: 1,
        };

        // 模拟 Session 在编辑期间切换 (sess_1 -> sess_2)
        std::env::set_var("VISUAL", "true");
        std::env::set_var("EDITOR", "true"); // 成功退出的空命令
        let outcome = lifecycle
            .execute_editor_handoff(target.clone(), "Original prompt", || ("sess_2".into(), 1))
            .await
            .unwrap();

        match outcome {
            HandoffOutcome::ConflictRetained {
                stash_content,
                error_message,
            } => {
                assert!(error_message.contains("Draft conflict"));
                assert_eq!(stash_content, "Original prompt");
            }
            _ => panic!("Expected ConflictRetained due to session switch"),
        }

        // 模拟同 Session 下草稿版本发生冲突 (revision 1 -> 2)
        let outcome2 = lifecycle
            .execute_editor_handoff(target, "Original prompt", || ("sess_1".into(), 2))
            .await
            .unwrap();

        assert!(matches!(outcome2, HandoffOutcome::ConflictRetained { .. }));
    }

    #[tokio::test]
    async fn test_pause_timeout_when_no_reader_consumes() {
        let (pause_tx, _pause_rx_dropped) = tokio::sync::mpsc::channel(1);
        let (resume_tx, _resume_rx) = tokio::sync::mpsc::channel(1);
        let control = EventReaderControl::new(Arc::new(AtomicU64::new(1)), pause_tx, resume_tx);

        // 丢弃接收端后请求 pause，必须明确报错为 ReaderPauseTimeout
        drop(_pause_rx_dropped);
        let err = control.request_pause().await;
        assert!(matches!(err, Err(HandoffError::ReaderPauseTimeout(..))));
    }

    #[tokio::test]
    async fn test_generation_invalidation_pipeline() {
        let (control, reader) = AdaptiveEventReader::new_pair();
        let (tx, mut rx) = tokio::sync::mpsc::channel(10);
        let handle = tokio::runtime::Handle::current();

        // 启动后台事件监听协程
        let _task = reader.spawn_loop(
            &handle,
            || async {
                tokio::time::sleep(std::time::Duration::from_millis(5)).await;
                Some("key_event")
            },
            tx,
        );

        // 1. 读取第一代事件
        let first_ev = rx.recv().await.unwrap();
        assert_eq!(first_ev.generation, 1);
        assert!(first_ev.is_valid(control.current_generation()));

        // 2. 模拟编辑过程：pause -> advance_generation -> resume
        control.request_pause().await.unwrap();
        control.advance_generation();
        assert_eq!(control.current_generation(), 2);

        // 此时第一代事件在派发校验时必须失效
        assert!(!first_ev.is_valid(control.current_generation()));

        control.resume_reader().await.unwrap();

        // 3. 读取恢复后的第二代事件
        let second_ev = rx.recv().await.unwrap();
        assert_eq!(second_ev.generation, 2);
        assert!(second_ev.is_valid(control.current_generation()));
    }

    #[tokio::test]
    async fn test_handoff_draft_passthrough_and_consecutive_runs() {
        let _editor_env_guard = editor_env_lock();
        let (control, reader) = AdaptiveEventReader::new_pair();
        let (tx, _rx) = tokio::sync::mpsc::channel(10);
        let handle = tokio::runtime::Handle::current();

        let _task = reader.spawn_loop(
            &handle,
            || async {
                tokio::time::sleep(std::time::Duration::from_millis(50)).await;
                Some("tick")
            },
            tx,
        );

        let backend = MockTerminalBackend::default();
        let mut lifecycle = TerminalLifecycle::new(backend, Some(control.clone()));

        // 1. 验证草稿内容正确传入编辑器临时文件并回填
        std::env::set_var("VISUAL", "true");
        std::env::set_var("EDITOR", "true");
        let target1 = DraftTarget {
            session_id: "s1".into(),
            draft_revision: 1,
        };
        let outcome1 = lifecycle
            .execute_editor_handoff(target1, "Draft content 1", || ("s1".into(), 1))
            .await
            .unwrap();

        match outcome1 {
            HandoffOutcome::Success {
                new_content,
                requires_full_redraw,
            } => {
                assert_eq!(new_content, "Draft content 1");
                assert!(requires_full_redraw);
            }
            _ => panic!("Expected Success"),
        }

        // 2. 验证编辑器非零退出时返回 Cancelled 并保持草稿
        std::env::set_var("VISUAL", "false");
        std::env::set_var("EDITOR", "false"); // false 退出码为 1
        let target2 = DraftTarget {
            session_id: "s1".into(),
            draft_revision: 2,
        };
        let outcome2 = lifecycle
            .execute_editor_handoff(target2, "Draft content 2", || ("s1".into(), 2))
            .await
            .unwrap();

        assert_eq!(outcome2, HandoffOutcome::Cancelled);

        // 3. 验证连续两次 handoff 终端状态无残留且代际正确连续递增
        assert_eq!(control.current_generation(), 3); // 初始1 + 两次handoff自增2次 = 3
    }
}
