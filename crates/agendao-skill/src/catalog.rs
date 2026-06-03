use crate::{LoadedSkill, SkillMeta};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SkillRoot {
    pub path: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SkillFileSignature {
    pub path: PathBuf,
    #[serde(default = "default_true")]
    pub exists: bool,
    #[serde(default = "default_true")]
    pub is_file: bool,
    pub modified_ns: u128,
    pub size: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SkillDirectorySignature {
    pub path: PathBuf,
    #[serde(default = "default_true")]
    pub exists: bool,
    #[serde(default = "default_true")]
    pub is_dir: bool,
    #[serde(default)]
    pub modified_ns: u128,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SkillRootSignature {
    pub root: PathBuf,
    #[serde(default)]
    pub directories: Vec<SkillDirectorySignature>,
    pub files: Vec<SkillFileSignature>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SkillCatalogSnapshot {
    pub roots: Vec<SkillRoot>,
    pub signatures: Vec<SkillRootSignature>,
    pub skills: Vec<SkillMeta>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoredSkillCatalogSnapshot {
    pub schema: String,
    pub version: u32,
    pub snapshot: SkillCatalogSnapshot,
}

#[derive(Debug, Clone)]
pub struct LoadedSkillCacheEntry {
    pub skill: LoadedSkill,
    pub last_access_tick: u64,
}

#[derive(Debug, Clone, Default)]
pub struct SkillCatalogCache {
    pub snapshot: Option<SkillCatalogSnapshot>,
    pub config_revision: u64,
    pub access_tick: u64,
    pub loaded_skills: HashMap<String, LoadedSkillCacheEntry>,
}

const LOADED_SKILL_CACHE_LIMIT: usize = 16;
const SNAPSHOT_FILE_NAME: &str = "skills_prompt_snapshot.json";
pub const SKILL_CATALOG_SNAPSHOT_SCHEMA: &str = "agendao.skill_catalog_snapshot";
pub const SKILL_CATALOG_SNAPSHOT_VERSION: u32 = 1;

impl SkillCatalogCache {
    pub fn set_snapshot(&mut self, snapshot: SkillCatalogSnapshot, config_revision: u64) {
        self.snapshot = Some(snapshot);
        self.config_revision = config_revision;
        self.loaded_skills.clear();
    }

    pub fn clear(&mut self) {
        self.snapshot = None;
        self.loaded_skills.clear();
    }

    pub fn cached_loaded_skill(&mut self, name: &str) -> Option<LoadedSkill> {
        let key = normalize_skill_key(name);
        self.access_tick = self.access_tick.saturating_add(1);
        let entry = self.loaded_skills.get_mut(&key)?;
        entry.last_access_tick = self.access_tick;
        Some(entry.skill.clone())
    }

    pub fn remember_loaded_skill(&mut self, skill: LoadedSkill) {
        self.access_tick = self.access_tick.saturating_add(1);
        let key = normalize_skill_key(&skill.meta.name);
        self.loaded_skills.insert(
            key,
            LoadedSkillCacheEntry {
                skill,
                last_access_tick: self.access_tick,
            },
        );
        self.evict_loaded_skill_overflow();
    }

    fn evict_loaded_skill_overflow(&mut self) {
        while self.loaded_skills.len() > LOADED_SKILL_CACHE_LIMIT {
            let Some(lru_key) = self
                .loaded_skills
                .iter()
                .min_by_key(|(_, entry)| entry.last_access_tick)
                .map(|(key, _)| key.clone())
            else {
                break;
            };
            self.loaded_skills.remove(&lru_key);
        }
    }
}

pub fn load_snapshot_from_disk(base_dir: &Path) -> Option<SkillCatalogSnapshot> {
    let path = snapshot_path(base_dir);
    let bytes = fs::read(path).ok()?;
    parse_snapshot_bytes(&bytes).map(|stored| stored.snapshot)
}

pub fn persist_snapshot_to_disk(
    base_dir: &Path,
    snapshot: &SkillCatalogSnapshot,
) -> std::io::Result<()> {
    let path = snapshot_path(base_dir);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    let temp_path = path.with_extension("json.tmp");
    let stored = StoredSkillCatalogSnapshot {
        schema: SKILL_CATALOG_SNAPSHOT_SCHEMA.to_string(),
        version: SKILL_CATALOG_SNAPSHOT_VERSION,
        snapshot: snapshot.clone(),
    };
    let bytes = serde_json::to_vec_pretty(&stored)
        .map_err(|error| std::io::Error::other(error.to_string()))?;
    fs::write(&temp_path, bytes)?;
    fs::rename(temp_path, path)?;
    Ok(())
}

pub fn snapshot_path(base_dir: &Path) -> PathBuf {
    base_dir
        .join(".agendao")
        .join("cache")
        .join(SNAPSHOT_FILE_NAME)
}

fn normalize_skill_key(name: &str) -> String {
    name.trim().to_ascii_lowercase()
}

const fn default_true() -> bool {
    true
}

fn parse_snapshot_bytes(bytes: &[u8]) -> Option<StoredSkillCatalogSnapshot> {
    let stored = serde_json::from_slice::<StoredSkillCatalogSnapshot>(bytes).ok()?;
    if stored.schema != SKILL_CATALOG_SNAPSHOT_SCHEMA
        || stored.version != SKILL_CATALOG_SNAPSHOT_VERSION
    {
        return None;
    }
    Some(stored)
}
