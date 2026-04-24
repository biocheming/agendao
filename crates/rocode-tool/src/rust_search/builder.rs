use std::path::{Path, PathBuf};

use super::filter::FilterType;
use super::search::SearchOptions;
use super::Search;

fn replace_tilde_with_home_dir(path: impl AsRef<Path>) -> PathBuf {
    let path = path.as_ref();
    if path.starts_with("~") {
        if let Some(home_dir) = dirs::home_dir() {
            return home_dir.join(path.strip_prefix("~").unwrap());
        }
    }
    path.to_path_buf()
}

/// Builder for a [`Search`] instance, allowing for more complex searches.
pub struct SearchBuilder {
    search_location: PathBuf,
    more_locations: Option<Vec<PathBuf>>,
    search_input: Option<String>,
    file_ext: Option<String>,
    depth: Option<usize>,
    limit: Option<usize>,
    strict: bool,
    ignore_case: bool,
    hidden: bool,
    filters: Vec<FilterType>,
}

impl SearchBuilder {
    pub fn build(&self) -> Search {
        Search::new(SearchOptions {
            search_location: self.search_location.clone(),
            more_locations: self.more_locations.clone(),
            search_input: self.search_input.clone(),
            file_ext: self.file_ext.clone(),
            depth: self.depth,
            limit: self.limit,
            strict: self.strict,
            ignore_case: self.ignore_case,
            with_hidden: self.hidden,
            filters: self.filters.clone(),
        })
    }

    pub fn location(mut self, location: impl AsRef<Path>) -> Self {
        self.search_location = replace_tilde_with_home_dir(location);
        self
    }

    pub fn search_input(mut self, input: impl Into<String>) -> Self {
        self.search_input = Some(input.into());
        self
    }

    pub fn ext(mut self, ext: impl Into<String>) -> Self {
        let ext: String = ext.into();
        self.file_ext = Some(
            ext.strip_prefix('.')
                .map_or_else(|| ext.clone(), str::to_owned),
        );
        self
    }

    pub fn filter(mut self, filter: FilterType) -> Self {
        self.filters.push(filter);
        self
    }

    pub const fn depth(mut self, depth: usize) -> Self {
        self.depth = Some(depth);
        self
    }

    pub const fn limit(mut self, limit: usize) -> Self {
        self.limit = Some(limit);
        self
    }

    pub const fn strict(mut self) -> Self {
        self.strict = true;
        self
    }

    pub const fn ignore_case(mut self) -> Self {
        self.ignore_case = true;
        self
    }

    pub const fn hidden(mut self) -> Self {
        self.hidden = true;
        self
    }

    pub fn more_locations(mut self, more_locations: Vec<impl AsRef<Path>>) -> Self {
        self.more_locations = Some(
            more_locations
                .into_iter()
                .map(replace_tilde_with_home_dir)
                .collect(),
        );
        self
    }
}

impl Default for SearchBuilder {
    fn default() -> Self {
        Self {
            search_location: std::env::current_dir().unwrap_or_else(|error| {
                tracing::warn!(%error, "failed to resolve current directory for SearchBuilder default, falling back to `.`");
                PathBuf::from(".")
            }),
            more_locations: None,
            search_input: None,
            file_ext: None,
            depth: None,
            limit: None,
            strict: false,
            ignore_case: false,
            hidden: false,
            filters: vec![],
        }
    }
}
