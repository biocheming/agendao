use clap::{Args, Parser, Subcommand, ValueEnum};
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "rocode")]
#[command(about = "ROCode - A Rusted OpenCode Version", long_about = None)]
pub(crate) struct Cli {
    #[command(subcommand)]
    pub(crate) command: Commands,
}

#[derive(Subcommand)]
pub(crate) enum Commands {
    #[command(about = "Run rocode with a message")]
    Run {
        #[arg(value_name = "MESSAGE", trailing_var_arg = true)]
        message: Vec<String>,
        #[arg(long)]
        command: Option<String>,
        #[arg(short = 'c', long = "continue", default_value_t = false)]
        continue_last: bool,
        #[arg(short = 's', long)]
        session: Option<String>,
        #[arg(long, default_value_t = false)]
        fork: bool,
        #[arg(long)]
        share: bool,
        #[arg(short = 'm', long)]
        model: Option<String>,
        #[arg(long, conflicts_with = "scheduler_profile")]
        agent: Option<String>,
        #[arg(long, conflicts_with = "agent")]
        scheduler_profile: Option<String>,
        #[arg(short = 'f', long)]
        file: Vec<PathBuf>,
        #[arg(long, default_value = "default")]
        format: RunOutputFormat,
        #[arg(long)]
        title: Option<String>,
        #[arg(long)]
        attach: Option<String>,
        #[arg(long)]
        dir: Option<PathBuf>,
        #[arg(long)]
        port: Option<u16>,
        #[arg(long)]
        variant: Option<String>,
        #[arg(long, default_value_t = false)]
        thinking: bool,
        #[arg(long = "interactive-mode", default_value = "rich")]
        interactive_mode: InteractiveCliMode,
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
    #[command(about = "Show configuration")]
    Config,
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
    #[command(about = "Fetch and checkout a GitHub PR branch, then run rocode")]
    Pr {
        #[arg(value_name = "NUMBER")]
        number: u32,
    },
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, ValueEnum)]
pub(crate) enum InteractiveCliMode {
    Rich,
    Compact,
}

#[derive(Clone, Debug, ValueEnum)]
pub(crate) enum RunOutputFormat {
    Default,
    Json,
}

#[derive(Clone, Debug, ValueEnum)]
pub(crate) enum SessionListFormat {
    Table,
    Json,
}

#[derive(Clone, Debug, ValueEnum)]
pub(crate) enum DbOutputFormat {
    Json,
    Tsv,
}

#[derive(Subcommand)]
pub(crate) enum DbCommands {
    #[command(about = "Print the database path")]
    Path,
}

#[derive(Subcommand)]
pub(crate) enum SessionCommands {
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
}

#[derive(Subcommand)]
pub(crate) enum MemoryCommands {
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
pub(crate) enum SkillCommands {
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
pub(crate) enum ProposalCommands {
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
pub(crate) enum SkillHubCommands {
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
pub(crate) struct SkillHubOutputArgs {
    #[arg(long, default_value = "text")]
    pub format: SkillHubOutputFormat,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, ValueEnum)]
pub(crate) enum SkillHubOutputFormat {
    Text,
    Json,
}

#[derive(Args, Clone, Debug)]
pub(crate) struct SkillSourceArgs {
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
pub(crate) enum AuthCommands {
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
pub(crate) enum AgentCommands {
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
pub(crate) enum AgentFileMode {
    All,
    Primary,
    Subagent,
}

impl AgentFileMode {
    pub(crate) fn as_str(&self) -> &'static str {
        match self {
            Self::All => "all",
            Self::Primary => "primary",
            Self::Subagent => "subagent",
        }
    }
}

#[derive(Subcommand)]
pub(crate) enum DebugCommands {
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
pub(crate) enum DebugSkillsCommands {
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
pub(crate) enum SkillSourceKindArg {
    Bundled,
    LocalPath,
    Git,
    Archive,
    Registry,
}

#[derive(Subcommand)]
pub(crate) enum DebugSnapshotCommands {
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
pub(crate) enum DebugFileCommands {
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
pub(crate) enum DebugRgCommands {
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
pub(crate) enum DebugLspCommands {
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
pub(crate) enum DebugDocsCommands {
    #[command(about = "Validate context docs registry or index files")]
    Validate {
        #[arg(long, value_name = "PATH", conflicts_with = "index")]
        registry: Option<PathBuf>,
        #[arg(long, value_name = "PATH", conflicts_with = "registry")]
        index: Option<PathBuf>,
    },
}

#[derive(Subcommand)]
pub(crate) enum McpCommands {
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
pub(crate) enum McpAuthCommands {
    #[command(about = "List OAuth-capable MCP servers and status", alias = "ls")]
    List,
}

#[derive(Subcommand)]
pub(crate) enum GithubCommands {
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
