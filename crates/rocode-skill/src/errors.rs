use rocode_types::SkillGuardReport;
use std::path::PathBuf;

#[derive(Debug, thiserror::Error, Clone, PartialEq, Eq)]
pub enum SkillError {
    #[error("Unknown skill: {requested}. Available skills: {available}")]
    UnknownSkill {
        requested: String,
        available: String,
    },

    #[error("invalid skill file path for `{skill}`: {file_path}")]
    InvalidSkillFilePath { skill: String, file_path: String },

    #[error("skill file not found for `{skill}`: {file_path}")]
    SkillFileNotFound { skill: String, file_path: String },

    #[error("workspace skill writes are limited to `.rocode/skills`: {path}")]
    InvalidWriteTarget { path: PathBuf },

    #[error("skill `{name}` is not writable because it is outside the workspace sandbox: {path}")]
    SkillNotWritable { name: String, path: PathBuf },

    #[error("invalid skill name: {name}")]
    InvalidSkillName { name: String },

    #[error("invalid skill description for `{name}`")]
    InvalidSkillDescription { name: String },

    #[error("invalid skill content: {message}")]
    InvalidSkillContent { message: String },

    #[error("invalid skill category path: {category}")]
    InvalidSkillCategory { category: String },

    #[error("invalid skill frontmatter: {message}")]
    InvalidSkillFrontmatter { message: String },

    #[error("skill already exists: {name}")]
    SkillAlreadyExists { name: String },

    #[error(
        "skill guard blocked `{}` with {} violation(s)",
        report.skill_name,
        report.violations.len()
    )]
    GuardBlocked { report: SkillGuardReport },

    #[error("skill write size limit exceeded for `{path}`: {size} bytes > {limit} bytes")]
    SkillWriteSizeExceeded {
        path: String,
        size: usize,
        limit: usize,
    },

    #[error("artifact fetch timed out for `{locator}` after {timeout_ms}ms")]
    ArtifactFetchTimeout { locator: String, timeout_ms: u64 },

    #[error("artifact download size limit exceeded for `{locator}`: {size} bytes > {limit} bytes")]
    ArtifactDownloadSizeExceeded {
        locator: String,
        size: u64,
        limit: u64,
    },

    #[error("artifact extract size limit exceeded for `{path}`: {size} bytes > {limit} bytes")]
    ArtifactExtractSizeExceeded {
        path: PathBuf,
        size: u64,
        limit: u64,
    },

    #[error("artifact checksum mismatch: expected sha256:{expected}, got sha256:{actual}")]
    ArtifactChecksumMismatch { expected: String, actual: String },

    #[error("artifact layout mismatch at `{path}`: {message}")]
    ArtifactLayoutMismatch { path: PathBuf, message: String },

    #[error("failed to read skill path `{path}`: {message}")]
    ReadFailed { path: PathBuf, message: String },

    #[error("failed to write skill path `{path}`: {message}")]
    WriteFailed { path: PathBuf, message: String },

    #[error("skill cache state is unavailable: {resource}")]
    CachePoisoned { resource: &'static str },
}
