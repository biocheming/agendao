use clap::{Args, Parser, Subcommand, ValueEnum};
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "agendao")]
#[command(about = "AgenDao", long_about = None)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand)]
pub enum Commands {
    #[command(about = "Run agendao with a message")]
    Run {
        #[command(flatten)]
        args: RunCommandArgs,
    },
    #[command(about = "List available models")]
    Models {
        #[arg(value_name = "PROVIDER")]
        provider: Option<String>,
        #[arg(long, default_value_t = false)]
        refresh: bool,
        #[arg(long, default_value_t = false)]
        verbose: bool,
    },
    #[command(about = "Manage sessions")]
    Session {
        #[command(subcommand)]
        action: SessionCommands,
    },
    #[command(about = "Manage memory authority artifacts")]
    Memory {
        #[command(subcommand)]
        action: MemoryCommands,
    },
    #[command(about = "Manage skill catalog and remote hub", alias = "skills")]
    Skill {
        #[command(subcommand)]
        action: SkillCommands,
    },
    #[command(about = "Manage provider authority artifacts")]
    Provider {
        #[command(subcommand)]
        action: ProviderCommands,
    },
    #[command(about = "Show token usage and cost statistics")]
    Stats {
        #[arg(long)]
        days: Option<i64>,
        #[arg(long)]
        tools: Option<usize>,
        #[arg(long)]
        models: Option<usize>,
        #[arg(long)]
        project: Option<String>,
    },
    #[command(about = "Database tools")]
    Db {
        #[command(subcommand)]
        action: Option<DbCommands>,
        #[arg(value_name = "QUERY")]
        query: Option<String>,
        #[arg(long, default_value = "tsv")]
        format: DbOutputFormat,
    },
    #[command(about = "Show configuration and config validation")]
    Config {
        #[command(subcommand)]
        action: Option<ConfigCommands>,
    },
    #[command(about = "Manage credentials")]
    Auth {
        #[command(subcommand)]
        action: AuthCommands,
    },
    #[command(about = "Manage agents")]
    Agent {
        #[command(subcommand)]
        action: AgentCommands,
    },
    #[command(about = "Debugging and troubleshooting utilities")]
    Debug {
        #[command(subcommand)]
        action: DebugCommands,
    },
    #[command(about = "Manage MCP (Model Context Protocol) servers")]
    Mcp {
        #[arg(long, default_value = "http://127.0.0.1:3000")]
        server: String,
        #[command(subcommand)]
        action: McpCommands,
    },
    #[command(about = "Export session data as JSON")]
    Export {
        #[arg(value_name = "SESSION_ID")]
        session_id: Option<String>,
        #[arg(short, long)]
        output: Option<PathBuf>,
    },
    #[command(about = "Import session data from JSON file or share URL")]
    Import {
        #[arg(value_name = "FILE_OR_URL")]
        file: String,
    },
    #[command(about = "Manage the GitHub agent")]
    Github {
        #[command(subcommand)]
        action: GithubCommands,
    },
    #[command(about = "Fetch and checkout a GitHub PR branch, then run agendao")]
    Pr {
        #[arg(value_name = "NUMBER")]
        number: u32,
    },
    #[command(about = "Submit a mid-run steering message to a session")]
    Steer {
        #[arg(short = 's', long, value_name = "SESSION_ID")]
        session: String,
        #[arg(value_name = "MESSAGE", trailing_var_arg = true)]
        message: Vec<String>,
    },
}

#[derive(Args, Clone, Debug)]
pub struct RunCommandArgs {
    #[arg(value_name = "MESSAGE", trailing_var_arg = true)]
    pub message: Vec<String>,
    #[arg(long)]
    pub command: Option<String>,
    #[arg(short = 'c', long = "continue", default_value_t = false)]
    pub continue_last: bool,
    #[arg(short = 's', long)]
    pub session: Option<String>,
    #[arg(long, default_value_t = false)]
    pub fork: bool,
    #[arg(long)]
    pub share: bool,
    #[arg(short = 'm', long)]
    pub model: Option<String>,
    #[arg(long, conflicts_with = "scheduler_profile")]
    pub agent: Option<String>,
    #[arg(long, conflicts_with = "agent")]
    pub scheduler_profile: Option<String>,
    #[arg(short = 'f', long)]
    pub file: Vec<PathBuf>,
    #[arg(long, default_value = "default")]
    pub format: RunOutputFormat,
    #[arg(long)]
    pub title: Option<String>,
    #[arg(long = "attach", alias = "attach-url")]
    pub attach: Option<String>,
    #[arg(long)]
    pub dir: Option<PathBuf>,
    #[arg(long)]
    pub port: Option<u16>,
    #[arg(long)]
    pub variant: Option<String>,
    #[arg(long, default_value_t = false)]
    pub thinking: bool,
    /// Force Direct (in-process) mode. This is already the default unless
    /// `--socket` or `--attach` / `--attach-url` is provided.
    #[arg(long, default_value_t = false)]
    pub local: bool,
    /// Use the standard local Unix socket instead of Direct mode.
    #[arg(long = "socket", alias = "unix-socket", default_value_t = false)]
    pub socket: bool,
}

#[derive(Clone, Debug, ValueEnum)]
pub enum RunOutputFormat {
    Default,
    Json,
}

#[derive(Clone, Debug, ValueEnum)]
pub enum SessionListFormat {
    Table,
    Json,
}

#[derive(Clone, Debug, ValueEnum)]
pub enum SessionProvisionFormat {
    Text,
    Json,
}

#[derive(Clone, Debug, ValueEnum)]
pub enum DbOutputFormat {
    Json,
    Tsv,
}

#[derive(Subcommand)]
pub enum DbCommands {
    #[command(about = "Print the database path")]
    Path,
}

#[derive(Subcommand)]
pub enum SessionCommands {
    #[command(about = "List sessions")]
    List {
        #[arg(long = "max-count", short = 'n')]
        max_count: Option<i64>,
        #[arg(long, default_value = "table")]
        format: SessionListFormat,
        #[arg(long)]
        project: Option<String>,
    },
    #[command(about = "Show session info")]
    Show {
        #[arg(required = true)]
        session_id: String,
    },
    #[command(about = "Delete a session")]
    Delete {
        #[arg(required = true)]
        session_id: String,
    },
    #[command(about = "Provision an owner-local external adapter session on the server")]
    ProvisionExternalAdapter {
        #[arg(long)]
        adapter_id: String,
        #[arg(long)]
        actor_id: String,
        #[arg(long)]
        workspace_id: Option<String>,
        #[arg(long)]
        route_policy_id: Option<String>,
        #[arg(long)]
        scheduler_profile: Option<String>,
        #[arg(long)]
        directory: Option<PathBuf>,
        #[arg(long)]
        project_id: Option<String>,
        #[arg(long)]
        title: Option<String>,
        #[arg(long, default_value = "text")]
        format: SessionProvisionFormat,
    },
}

#[derive(Subcommand)]
pub enum MemoryCommands {
    #[command(about = "Export memory authority records as JSON")]
    Export {
        #[arg(short, long)]
        output: Option<PathBuf>,
    },
    #[command(about = "Import memory authority records from JSON file")]
    Import {
        #[arg(value_name = "FILE")]
        file: String,
    },
}

#[derive(Subcommand)]
pub enum SkillCommands {
    #[command(about = "Export workspace-local skill authority artifact as JSON")]
    Export {
        #[arg(short, long)]
        output: Option<PathBuf>,
    },
    #[command(about = "Import workspace-local skill authority artifact from JSON file")]
    Import {
        #[arg(value_name = "FILE")]
        file: String,
    },
    #[command(about = "Remote distribution, lifecycle, and managed source operations")]
    Hub {
        #[command(subcommand)]
        action: SkillHubCommands,
    },
    #[command(about = "Review and manage skill evolution proposals")]
    Proposal {
        #[command(subcommand)]
        action: ProposalCommands,
    },
}

#[derive(Subcommand)]
pub enum ProviderCommands {
    #[command(about = "Export provider authority artifact as JSON")]
    Export {
        #[arg(short, long)]
        output: Option<PathBuf>,
    },
    #[command(about = "Import provider authority artifact from JSON file")]
    Import {
        #[arg(value_name = "FILE")]
        file: String,
    },
}

#[derive(Subcommand)]
pub enum ConfigCommands {
    #[command(about = "Show authority-backed config validation snapshot")]
    Validation {
        #[command(flatten)]
        output: ConfigOutputArgs,
    },
}

#[derive(Args, Clone, Debug)]
pub struct ConfigOutputArgs {
    #[arg(long, default_value = "text")]
    pub format: ConfigOutputFormat,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, ValueEnum)]
pub enum ConfigOutputFormat {
    Text,
    Json,
}

#[derive(Subcommand)]
pub enum ProposalCommands {
    #[command(about = "List skill evolution proposals")]
    List {
        #[arg(long, default_value = "draft")]
        status: String,
    },
    #[command(about = "Show proposal detail")]
    Show {
        #[arg(value_name = "ID")]
        id: String,
    },
    #[command(about = "Approve a proposal (does not apply to SKILL.md)")]
    Approve {
        #[arg(value_name = "ID")]
        id: String,
    },
    #[command(about = "Reject a proposal")]
    Reject {
        #[arg(value_name = "ID")]
        id: String,
    },
}

#[derive(Subcommand)]
pub enum SkillHubCommands {
    #[command(
        about = "Show a combined view of distributions, artifact cache, and lifecycle state"
    )]
    Status {
        #[command(flatten)]
        output: SkillHubOutputArgs,
    },
    #[command(about = "Show managed skill provenance records")]
    Managed {
        #[command(flatten)]
        output: SkillHubOutputArgs,
    },
    #[command(about = "Show canonical skill usage and write ledger inspection")]
    Usage {
        #[command(flatten)]
        output: SkillHubOutputArgs,
    },
    #[command(about = "Show read-only negative entropy diagnostics")]
    NegativeEntropy {
        #[command(flatten)]
        output: SkillHubOutputArgs,
    },
    #[command(about = "Mark current negative-entropy review candidates on workspace-local skills")]
    ReviewCandidatesSync {
        #[arg(long)]
        session_id: String,
        #[command(flatten)]
        output: SkillHubOutputArgs,
    },
    #[command(about = "Show read-only semantic overlap diagnostics")]
    SemanticConflicts {
        #[command(flatten)]
        output: SkillHubOutputArgs,
    },
    #[command(
        about = "Mark redundant workspace-local skills as review candidates from semantic conflict diagnostics"
    )]
    SemanticConflictReviewSync {
        #[arg(long)]
        session_id: String,
        #[command(flatten)]
        output: SkillHubOutputArgs,
    },
    #[command(about = "Set one workspace-local skill vitality state")]
    VitalitySet {
        #[arg(long)]
        session_id: String,
        #[arg(long)]
        skill_name: String,
        #[arg(long, value_enum)]
        state: SkillVitalityStateArg,
        #[arg(long, value_enum)]
        reason_kind: Option<SkillRetirementReasonKindArg>,
        #[arg(long)]
        summary: String,
        #[arg(long)]
        related_skill_name: Option<String>,
        #[command(flatten)]
        output: SkillHubOutputArgs,
    },
    #[command(about = "Show cached skill source indices")]
    Index {
        #[command(flatten)]
        output: SkillHubOutputArgs,
    },
    #[command(about = "Show resolved remote distribution records")]
    Distributions {
        #[command(flatten)]
        output: SkillHubOutputArgs,
    },
    #[command(about = "Show artifact cache entries and fetch failure reasons")]
    ArtifactCache {
        #[command(flatten)]
        output: SkillHubOutputArgs,
    },
    #[command(about = "Show current skill hub artifact policy")]
    Policy {
        #[command(flatten)]
        output: SkillHubOutputArgs,
    },
    #[command(about = "Show managed lifecycle records")]
    Lifecycle {
        #[command(flatten)]
        output: SkillHubOutputArgs,
    },
    #[command(about = "Refresh one source index cache entry")]
    IndexRefresh {
        #[command(flatten)]
        source: SkillSourceArgs,
        #[command(flatten)]
        output: SkillHubOutputArgs,
    },
    #[command(about = "Create a hub sync plan for one source")]
    SyncPlan {
        #[command(flatten)]
        source: SkillSourceArgs,
        #[command(flatten)]
        output: SkillHubOutputArgs,
    },
    #[command(about = "Apply a hub sync plan for one source")]
    SyncApply {
        #[arg(long)]
        session_id: String,
        #[command(flatten)]
        source: SkillSourceArgs,
        #[command(flatten)]
        output: SkillHubOutputArgs,
    },
    #[command(about = "Plan one remote distribution install")]
    InstallPlan {
        #[command(flatten)]
        source: SkillSourceArgs,
        #[arg(long)]
        skill_name: String,
        #[command(flatten)]
        output: SkillHubOutputArgs,
    },
    #[command(about = "Apply one remote distribution install")]
    InstallApply {
        #[arg(long)]
        session_id: String,
        #[command(flatten)]
        source: SkillSourceArgs,
        #[arg(long)]
        skill_name: String,
        #[command(flatten)]
        output: SkillHubOutputArgs,
    },
    #[command(about = "Plan one managed remote update")]
    UpdatePlan {
        #[command(flatten)]
        source: SkillSourceArgs,
        #[arg(long)]
        skill_name: String,
        #[command(flatten)]
        output: SkillHubOutputArgs,
    },
    #[command(about = "Apply one managed remote update")]
    UpdateApply {
        #[arg(long)]
        session_id: String,
        #[command(flatten)]
        source: SkillSourceArgs,
        #[arg(long)]
        skill_name: String,
        #[command(flatten)]
        output: SkillHubOutputArgs,
    },
    #[command(about = "Detach one managed skill from its source while keeping workspace files")]
    Detach {
        #[arg(long)]
        session_id: String,
        #[command(flatten)]
        source: SkillSourceArgs,
        #[arg(long)]
        skill_name: String,
        #[command(flatten)]
        output: SkillHubOutputArgs,
    },
    #[command(about = "Remove one managed skill and delete the workspace copy only when clean")]
    Remove {
        #[arg(long)]
        session_id: String,
        #[command(flatten)]
        source: SkillSourceArgs,
        #[arg(long)]
        skill_name: String,
        #[command(flatten)]
        output: SkillHubOutputArgs,
    },
}

#[derive(Args, Clone, Debug)]
pub struct SkillHubOutputArgs {
    #[arg(long, default_value = "text")]
    pub format: SkillHubOutputFormat,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, ValueEnum)]
pub enum SkillHubOutputFormat {
    Text,
    Json,
}

#[derive(Args, Clone, Debug)]
pub struct SkillSourceArgs {
    #[arg(long)]
    pub source_id: String,
    #[arg(long, value_enum)]
    pub source_kind: SkillSourceKindArg,
    #[arg(long)]
    pub locator: String,
    #[arg(long)]
    pub revision: Option<String>,
}

#[derive(Subcommand)]
pub enum AuthCommands {
    #[command(
        about = "List supported auth providers and current environment status",
        alias = "ls"
    )]
    List,
    #[command(about = "Set credential for current process (non-persistent)")]
    Login {
        #[arg(value_name = "PROVIDER_OR_URL")]
        provider: Option<String>,
        #[arg(long)]
        token: Option<String>,
    },
    #[command(about = "Clear credential from current process")]
    Logout {
        #[arg(value_name = "PROVIDER")]
        provider: Option<String>,
    },
}

#[derive(Subcommand)]
pub enum AgentCommands {
    #[command(about = "List available agents")]
    List,
    #[command(about = "Create an agent markdown file")]
    Create {
        #[arg(value_name = "NAME")]
        name: String,
        #[arg(long)]
        description: String,
        #[arg(long, default_value = "all")]
        mode: AgentFileMode,
        #[arg(long)]
        path: Option<PathBuf>,
        #[arg(long)]
        tools: Option<String>,
        #[arg(short = 'm', long)]
        model: Option<String>,
    },
}

#[derive(Clone, Debug, ValueEnum)]
pub enum AgentFileMode {
    All,
    Primary,
    Subagent,
}

impl AgentFileMode {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::All => "all",
            Self::Primary => "primary",
            Self::Subagent => "subagent",
        }
    }
}

#[derive(Subcommand)]
pub enum DebugCommands {
    #[command(about = "Show important local paths")]
    Paths,
    #[command(about = "Show resolved config in JSON")]
    Config,
    #[command(about = "List all available skills")]
    Skill,
    #[command(about = "Skill catalog debugging utilities")]
    Skills {
        #[command(subcommand)]
        action: DebugSkillsCommands,
    },
    #[command(about = "List all known projects")]
    Scrap,
    #[command(about = "Wait indefinitely (for debugging)")]
    Wait,
    #[command(about = "Snapshot debugging utilities")]
    Snapshot {
        #[command(subcommand)]
        action: DebugSnapshotCommands,
    },
    #[command(about = "File system debugging utilities")]
    File {
        #[command(subcommand)]
        action: DebugFileCommands,
    },
    #[command(about = "Ripgrep debugging utilities")]
    Rg {
        #[command(subcommand)]
        action: DebugRgCommands,
    },
    #[command(about = "LSP debugging utilities")]
    Lsp {
        #[command(subcommand)]
        action: DebugLspCommands,
    },
    #[command(about = "Context docs debugging utilities")]
    Docs {
        #[command(subcommand)]
        action: DebugDocsCommands,
    },
    #[command(about = "Tool repair telemetry debugging utilities")]
    Repair {
        #[command(subcommand)]
        action: DebugRepairCommands,
    },
    #[command(about = "Show agent configuration details")]
    Agent {
        #[arg(value_name = "NAME")]
        name: String,
        #[arg(long)]
        tool: Option<String>,
        #[arg(long)]
        params: Option<String>,
    },
}

#[derive(Subcommand)]
pub enum DebugSkillsCommands {
    #[command(about = "List the resolved skill catalog")]
    List {
        #[arg(long)]
        session_id: Option<String>,
    },
    #[command(about = "Show raw detail for one resolved skill")]
    View {
        #[arg(value_name = "NAME")]
        name: String,
        #[arg(long)]
        session_id: Option<String>,
    },
    #[command(about = "Show managed skill provenance records")]
    Managed,
    #[command(about = "Show cached skill source indices")]
    Index,
    #[command(about = "Show resolved remote distribution records")]
    Distributions,
    #[command(about = "Show artifact cache entries and fetch failure reasons")]
    ArtifactCache,
    #[command(about = "Show managed lifecycle records")]
    Lifecycle,
    #[command(about = "Refresh one source index cache entry")]
    IndexRefresh {
        #[arg(long)]
        source_id: String,
        #[arg(long, value_enum)]
        source_kind: SkillSourceKindArg,
        #[arg(long)]
        locator: String,
        #[arg(long)]
        revision: Option<String>,
    },
    #[command(about = "Show recent skill governance audit events")]
    Audit,
    #[command(about = "Show unified skill governance timeline")]
    Timeline {
        #[arg(long)]
        skill_name: Option<String>,
        #[arg(long)]
        source_id: Option<String>,
        #[arg(long)]
        limit: Option<usize>,
    },
    #[command(about = "Run guard scan for one resolved skill or one source")]
    Guard {
        #[arg(long)]
        name: Option<String>,
        #[arg(long)]
        source_id: Option<String>,
        #[arg(long, value_enum)]
        source_kind: Option<SkillSourceKindArg>,
        #[arg(long)]
        locator: Option<String>,
        #[arg(long)]
        revision: Option<String>,
    },
    #[command(about = "Create a hub sync plan for one source")]
    SyncPlan {
        #[arg(long)]
        source_id: String,
        #[arg(long, value_enum)]
        source_kind: SkillSourceKindArg,
        #[arg(long)]
        locator: String,
        #[arg(long)]
        revision: Option<String>,
    },
    #[command(about = "Apply a hub sync plan for one source")]
    SyncApply {
        #[arg(long)]
        session_id: String,
        #[arg(long)]
        source_id: String,
        #[arg(long, value_enum)]
        source_kind: SkillSourceKindArg,
        #[arg(long)]
        locator: String,
        #[arg(long)]
        revision: Option<String>,
    },
    #[command(about = "Plan one remote distribution install")]
    InstallPlan {
        #[arg(long)]
        source_id: String,
        #[arg(long, value_enum)]
        source_kind: SkillSourceKindArg,
        #[arg(long)]
        locator: String,
        #[arg(long)]
        skill_name: String,
        #[arg(long)]
        revision: Option<String>,
    },
    #[command(about = "Apply one remote distribution install")]
    InstallApply {
        #[arg(long)]
        session_id: String,
        #[arg(long)]
        source_id: String,
        #[arg(long, value_enum)]
        source_kind: SkillSourceKindArg,
        #[arg(long)]
        locator: String,
        #[arg(long)]
        skill_name: String,
        #[arg(long)]
        revision: Option<String>,
    },
    #[command(about = "Plan one managed remote update")]
    UpdatePlan {
        #[arg(long)]
        source_id: String,
        #[arg(long, value_enum)]
        source_kind: SkillSourceKindArg,
        #[arg(long)]
        locator: String,
        #[arg(long)]
        skill_name: String,
        #[arg(long)]
        revision: Option<String>,
    },
    #[command(about = "Apply one managed remote update")]
    UpdateApply {
        #[arg(long)]
        session_id: String,
        #[arg(long)]
        source_id: String,
        #[arg(long, value_enum)]
        source_kind: SkillSourceKindArg,
        #[arg(long)]
        locator: String,
        #[arg(long)]
        skill_name: String,
        #[arg(long)]
        revision: Option<String>,
    },
    #[command(about = "Detach one managed skill from its source while keeping workspace files")]
    Detach {
        #[arg(long)]
        session_id: String,
        #[arg(long)]
        source_id: String,
        #[arg(long, value_enum)]
        source_kind: SkillSourceKindArg,
        #[arg(long)]
        locator: String,
        #[arg(long)]
        skill_name: String,
        #[arg(long)]
        revision: Option<String>,
    },
    #[command(about = "Remove one managed skill and delete the workspace copy only when clean")]
    Remove {
        #[arg(long)]
        session_id: String,
        #[arg(long)]
        source_id: String,
        #[arg(long, value_enum)]
        source_kind: SkillSourceKindArg,
        #[arg(long)]
        locator: String,
        #[arg(long)]
        skill_name: String,
        #[arg(long)]
        revision: Option<String>,
    },
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, ValueEnum)]
pub enum SkillSourceKindArg {
    Bundled,
    LocalPath,
    Git,
    Archive,
    Registry,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, ValueEnum)]
pub enum SkillVitalityStateArg {
    Active,
    ReviewCandidate,
    Retired,
    Archived,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, ValueEnum)]
pub enum SkillRetirementReasonKindArg {
    NegativeEntropy,
    SemanticConflict,
    ManualOverride,
    Restored,
}

#[derive(Subcommand)]
pub enum DebugSnapshotCommands {
    #[command(about = "Track current snapshot state")]
    Track,
    #[command(about = "Show patch for a snapshot hash")]
    Patch {
        #[arg(value_name = "HASH")]
        hash: String,
    },
    #[command(about = "Show diff for a snapshot hash")]
    Diff {
        #[arg(value_name = "HASH")]
        hash: String,
    },
}

#[derive(Subcommand)]
pub enum DebugFileCommands {
    #[command(about = "Search files by query")]
    Search {
        #[arg(value_name = "QUERY")]
        query: String,
    },
    #[command(about = "Read file contents as JSON")]
    Read {
        #[arg(value_name = "PATH")]
        path: String,
    },
    #[command(about = "Show file status information")]
    Status,
    #[command(about = "List files in a directory")]
    List {
        #[arg(value_name = "PATH")]
        path: String,
    },
    #[command(about = "Show directory tree")]
    Tree {
        #[arg(value_name = "DIR")]
        dir: Option<PathBuf>,
    },
}

#[derive(Subcommand)]
pub enum DebugRgCommands {
    #[command(about = "Show file tree using ripgrep")]
    Tree {
        #[arg(long)]
        limit: Option<usize>,
    },
    #[command(about = "List files using ripgrep")]
    Files {
        #[arg(long)]
        query: Option<String>,
        #[arg(long)]
        glob: Option<String>,
        #[arg(long)]
        limit: Option<usize>,
    },
    #[command(about = "Search file contents using ripgrep")]
    Search {
        #[arg(value_name = "PATTERN")]
        pattern: String,
        #[arg(long)]
        glob: Vec<String>,
        #[arg(long)]
        limit: Option<usize>,
    },
}

#[derive(Subcommand)]
pub enum DebugLspCommands {
    #[command(about = "Get diagnostics for a file")]
    Diagnostics {
        #[arg(value_name = "FILE")]
        file: String,
    },
    #[command(about = "Search workspace symbols")]
    Symbols {
        #[arg(value_name = "QUERY")]
        query: String,
    },
    #[command(about = "Get symbols from a document")]
    DocumentSymbols {
        #[arg(value_name = "URI")]
        uri: String,
    },
}

#[derive(Subcommand)]
pub enum DebugDocsCommands {
    #[command(about = "Validate context docs registry or index files")]
    Validate {
        #[arg(long, value_name = "PATH", conflicts_with = "index")]
        registry: Option<PathBuf>,
        #[arg(long, value_name = "PATH", conflicts_with = "registry")]
        index: Option<PathBuf>,
    },
}

#[derive(Subcommand)]
pub enum DebugRepairCommands {
    #[command(about = "Show cached repair summary for one session")]
    Summary {
        #[arg(value_name = "SESSION_ID")]
        session_id: String,
    },
    #[command(about = "Query repair events for one session or across all sessions")]
    Query {
        #[arg(long)]
        session_id: Option<String>,
        #[arg(long)]
        provider_id: Option<String>,
        #[arg(long)]
        model_id: Option<String>,
        #[arg(long)]
        tool_name: Option<String>,
        #[arg(long)]
        repair_kind: Option<String>,
        #[arg(long)]
        layer: Option<String>,
        #[arg(long, default_value_t = false)]
        strict_only: bool,
        #[arg(long, default_value_t = false)]
        include_samples: bool,
        #[arg(long)]
        limit: Option<usize>,
    },
}

#[derive(Subcommand)]
pub enum McpCommands {
    #[command(about = "List MCP servers and status", alias = "ls")]
    List,
    #[command(about = "Add an MCP server to runtime")]
    Add {
        #[arg(value_name = "NAME")]
        name: String,
        #[arg(long)]
        url: Option<String>,
        #[arg(long)]
        command: Option<String>,
        #[arg(long = "arg")]
        args: Vec<String>,
        #[arg(long, default_value_t = true)]
        enabled: bool,
        #[arg(long)]
        timeout: Option<u64>,
    },
    #[command(about = "Connect MCP server")]
    Connect {
        #[arg(value_name = "NAME")]
        name: String,
    },
    #[command(about = "Disconnect MCP server")]
    Disconnect {
        #[arg(value_name = "NAME")]
        name: String,
    },
    #[command(about = "MCP OAuth operations")]
    Auth {
        #[command(subcommand)]
        action: Option<McpAuthCommands>,
        #[arg(value_name = "NAME")]
        name: Option<String>,
        #[arg(long)]
        code: Option<String>,
        #[arg(long, default_value_t = false)]
        authenticate: bool,
    },
    #[command(about = "Remove MCP OAuth credentials")]
    Logout {
        #[arg(value_name = "NAME")]
        name: Option<String>,
    },
    #[command(about = "Debug OAuth connection for an MCP server")]
    Debug {
        #[arg(value_name = "NAME")]
        name: String,
    },
}

#[derive(Subcommand)]
pub enum McpAuthCommands {
    #[command(about = "List OAuth-capable MCP servers and status", alias = "ls")]
    List,
}

#[derive(Subcommand)]
pub enum GithubCommands {
    #[command(about = "Check GitHub CLI installation and auth status")]
    Status,
    #[command(about = "Install the GitHub agent in this repository")]
    Install,
    #[command(about = "Run the GitHub agent (CI mode)")]
    Run {
        #[arg(long)]
        event: Option<String>,
        #[arg(long)]
        token: Option<String>,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_command_keeps_default_show_behavior_without_subcommand() {
        let cli = Cli::parse_from(["agendao", "config"]);
        match cli.command {
            Commands::Config { action: None } => {}
            _ => panic!("expected bare config command"),
        }
    }

    #[test]
    fn config_validation_command_parses_output_format() {
        let cli = Cli::parse_from(["agendao", "config", "validation", "--format", "json"]);
        match cli.command {
            Commands::Config {
                action: Some(ConfigCommands::Validation { output }),
            } => {
                assert_eq!(output.format, ConfigOutputFormat::Json);
            }
            _ => panic!("expected config validation command"),
        }
    }

    #[test]
    fn steer_command_parses_session_and_message() {
        let cli = Cli::parse_from([
            "agendao",
            "steer",
            "--session",
            "sess_abc123",
            "stop after current tool",
        ]);
        match cli.command {
            Commands::Steer { session, message } => {
                assert_eq!(session, "sess_abc123");
                assert_eq!(message.join(" "), "stop after current tool");
            }
            _ => panic!("expected steer command"),
        }
    }

    #[test]
    fn steer_command_parses_multi_word_message() {
        let cli = Cli::parse_from([
            "agendao",
            "steer",
            "-s",
            "sess_xyz",
            "please",
            "switch",
            "to",
            "the",
            "structured_family",
            "matcher",
        ]);
        match cli.command {
            Commands::Steer { session, message } => {
                assert_eq!(session, "sess_xyz");
                assert_eq!(
                    message.join(" "),
                    "please switch to the structured_family matcher"
                );
            }
            _ => panic!("expected steer command"),
        }
    }
}
