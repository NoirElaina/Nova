use globset::{GlobBuilder, GlobSet, GlobSetBuilder};
use grep_regex::{RegexMatcher, RegexMatcherBuilder};
use grep_searcher::{
    BinaryDetection, Searcher, SearcherBuilder, Sink, SinkContext, SinkContextKind, SinkFinish,
    SinkMatch,
};
use ignore::WalkBuilder;
use serde::Serialize;
use std::collections::BTreeSet;
use std::io;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug)]
pub struct WalkOptions {
    pub include_hidden: bool,
    pub include_ignored: bool,
    pub follow_symlinks: bool,
}

impl Default for WalkOptions {
    fn default() -> Self {
        Self {
            include_hidden: false,
            include_ignored: false,
            follow_symlinks: false,
        }
    }
}

#[derive(Clone, Debug)]
pub struct GlobSearchOptions {
    pub pattern: String,
    pub case_sensitive: bool,
    pub max_results: usize,
    pub walk: WalkOptions,
}

#[derive(Clone, Debug, Serialize)]
pub struct GlobSearchStats {
    pub files_searched: usize,
    pub matches_returned: usize,
    pub truncated: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct GlobSearchResult {
    pub stats: GlobSearchStats,
    pub files: Vec<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TextSearchMode {
    Literal,
    Regex,
}

#[derive(Clone, Debug)]
pub struct TextSearchOptions {
    pub pattern: String,
    pub mode: TextSearchMode,
    pub case_sensitive: bool,
    pub include_globs: Vec<String>,
    pub context_lines: usize,
    pub max_results: usize,
    pub max_output_chars: usize,
    pub walk: WalkOptions,
}

#[derive(Clone, Debug, Serialize)]
pub struct SearchLine {
    pub line_number: u64,
    pub text: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct TextMatch {
    pub path: String,
    pub line_number: u64,
    pub line: String,
    pub before: Vec<SearchLine>,
    pub after: Vec<SearchLine>,
}

#[derive(Clone, Debug, Serialize)]
pub struct TextSearchStats {
    pub files_searched: usize,
    pub files_with_matches: usize,
    pub matches_returned: usize,
    pub truncated: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct TextSearchResult {
    pub stats: TextSearchStats,
    pub matches: Vec<TextMatch>,
}

pub fn find_files(
    workspace_root: &Path,
    search_root: &Path,
    options: &GlobSearchOptions,
) -> Result<GlobSearchResult, String> {
    let matcher = build_glob_set(
        &[options.pattern.as_str()],
        options.case_sensitive,
        "pattern",
    )?;
    let files = collect_files(workspace_root, search_root, None, &options.walk);
    let mut matches = Vec::new();
    let mut truncated = false;

    for file in &files {
        let Some(relative) = relative_path(workspace_root, file) else {
            continue;
        };
        if !matcher.is_match(&relative) {
            continue;
        }
        if matches.len() >= options.max_results {
            truncated = true;
            break;
        }
        matches.push(relative);
    }

    Ok(GlobSearchResult {
        stats: GlobSearchStats {
            files_searched: files.len(),
            matches_returned: matches.len(),
            truncated,
        },
        files: matches,
    })
}

pub fn grep_text(
    workspace_root: &Path,
    target: &Path,
    options: &TextSearchOptions,
) -> Result<TextSearchResult, String> {
    let matcher = build_text_matcher(&options.pattern, &options.mode, options.case_sensitive)?;
    let include_matcher = if options.include_globs.is_empty() {
        None
    } else {
        let patterns = options
            .include_globs
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>();
        Some(build_glob_set(
            &patterns,
            options.case_sensitive,
            "include_globs",
        )?)
    };

    let files = collect_files(
        workspace_root,
        target,
        include_matcher.as_ref(),
        &options.walk,
    );
    let mut matches = Vec::new();
    let mut truncated = false;
    let mut used_chars = 0usize;

    for file in &files {
        let remaining_results = options.max_results.saturating_sub(matches.len());
        let remaining_chars = options.max_output_chars.saturating_sub(used_chars);
        if remaining_results == 0 || remaining_chars == 0 {
            truncated = true;
            break;
        }

        let path =
            relative_path(workspace_root, file).unwrap_or_else(|| file.display().to_string());
        let mut sink = StructuredSearchSink::new(path, remaining_results + 1, remaining_chars);
        let mut builder = SearcherBuilder::new();
        builder
            .line_number(true)
            .before_context(options.context_lines)
            .after_context(options.context_lines)
            .binary_detection(BinaryDetection::quit(b'\x00'));
        let mut searcher = builder.build();

        if searcher.search_path(&matcher, file, &mut sink).is_err() {
            continue;
        }

        let (mut file_matches, file_chars, file_truncated) = sink.finish();
        used_chars = used_chars.saturating_add(file_chars);
        if file_matches.len() > remaining_results {
            file_matches.truncate(remaining_results);
            truncated = true;
        }
        truncated = truncated || file_truncated;
        matches.extend(file_matches);
        if truncated {
            break;
        }
    }

    let files_with_matches = count_files_with_matches(&matches);
    Ok(TextSearchResult {
        stats: TextSearchStats {
            files_searched: files.len(),
            files_with_matches,
            matches_returned: matches.len(),
            truncated,
        },
        matches,
    })
}

fn build_glob_set(patterns: &[&str], case_sensitive: bool, label: &str) -> Result<GlobSet, String> {
    let mut builder = GlobSetBuilder::new();
    for pattern in patterns {
        let trimmed = pattern.trim();
        if trimmed.is_empty() {
            return Err(format!("{} cannot contain an empty glob", label));
        }
        add_glob(&mut builder, trimmed, case_sensitive, label)?;

        // Match ripgrep-style file globs: a bare pattern applies at any depth.
        if !trimmed.contains('/') && !trimmed.contains('\\') {
            add_glob(
                &mut builder,
                &format!("**/{}", trimmed),
                case_sensitive,
                label,
            )?;
        }
    }

    builder
        .build()
        .map_err(|error| format!("Invalid {}: {}", label, error))
}

fn add_glob(
    builder: &mut GlobSetBuilder,
    pattern: &str,
    case_sensitive: bool,
    label: &str,
) -> Result<(), String> {
    let glob = GlobBuilder::new(pattern)
        .literal_separator(true)
        .backslash_escape(false)
        .case_insensitive(!case_sensitive)
        .build()
        .map_err(|error| format!("Invalid {} '{}': {}", label, pattern, error))?;
    builder.add(glob);
    Ok(())
}

fn build_text_matcher(
    pattern: &str,
    mode: &TextSearchMode,
    case_sensitive: bool,
) -> Result<RegexMatcher, String> {
    let effective_pattern = match mode {
        TextSearchMode::Literal => regex::escape(pattern),
        TextSearchMode::Regex => pattern.to_string(),
    };

    RegexMatcherBuilder::new()
        .case_insensitive(!case_sensitive)
        .build(&effective_pattern)
        .map_err(|error| format!("Invalid search pattern: {}", error))
}

fn collect_files(
    workspace_root: &Path,
    target: &Path,
    include_glob: Option<&GlobSet>,
    options: &WalkOptions,
) -> Vec<PathBuf> {
    if target.is_file() {
        return should_search_file(workspace_root, target, include_glob)
            .then(|| target.to_path_buf())
            .into_iter()
            .collect();
    }

    let mut builder = WalkBuilder::new(target);
    builder
        .standard_filters(true)
        .hidden(!options.include_hidden)
        .ignore(!options.include_ignored)
        .git_ignore(!options.include_ignored)
        .git_global(!options.include_ignored)
        .git_exclude(!options.include_ignored)
        .parents(!options.include_ignored)
        .follow_links(options.follow_symlinks);

    let mut files = Vec::new();
    for entry in builder.build().filter_map(Result::ok) {
        let Some(file_type) = entry.file_type() else {
            continue;
        };
        if !file_type.is_file() || !should_search_file(workspace_root, entry.path(), include_glob) {
            continue;
        }
        files.push(entry.path().to_path_buf());
    }

    files.sort();
    files
}

fn should_search_file(workspace_root: &Path, file: &Path, include_glob: Option<&GlobSet>) -> bool {
    let Some(include_glob) = include_glob else {
        return true;
    };
    relative_path(workspace_root, file)
        .map(|relative| include_glob.is_match(relative))
        .unwrap_or(false)
}

fn count_files_with_matches(matches: &[TextMatch]) -> usize {
    let mut files = BTreeSet::<&str>::new();
    for item in matches {
        files.insert(&item.path);
    }
    files.len()
}

fn relative_path(workspace_root: &Path, path: &Path) -> Option<String> {
    path.strip_prefix(workspace_root)
        .ok()
        .map(|relative| relative.to_string_lossy().replace('\\', "/"))
        .filter(|relative| !relative.is_empty())
}

struct StructuredSearchSink {
    path: String,
    matches: Vec<TextMatch>,
    current: Option<TextMatch>,
    before: Vec<SearchLine>,
    used_chars: usize,
    match_limit: usize,
    char_limit: usize,
    truncated: bool,
}

impl StructuredSearchSink {
    fn new(path: String, match_limit: usize, char_limit: usize) -> Self {
        Self {
            path,
            matches: Vec::new(),
            current: None,
            before: Vec::new(),
            used_chars: 0,
            match_limit,
            char_limit,
            truncated: false,
        }
    }

    fn finish(mut self) -> (Vec<TextMatch>, usize, bool) {
        self.flush_current();
        (self.matches, self.used_chars, self.truncated)
    }

    fn flush_current(&mut self) {
        if let Some(current) = self.current.take() {
            self.matches.push(current);
        }
    }

    fn accept_line(&mut self, line: &SearchLine) -> bool {
        let line_cost = line.text.len().saturating_add(32);
        if self.used_chars.saturating_add(line_cost) > self.char_limit {
            self.truncated = true;
            return false;
        }
        self.used_chars += line_cost;
        true
    }

    fn can_accept_match(&self) -> bool {
        self.matches.len() + usize::from(self.current.is_some()) < self.match_limit
            && !self.truncated
    }
}

impl Sink for StructuredSearchSink {
    type Error = io::Error;

    fn matched(&mut self, _searcher: &Searcher, mat: &SinkMatch<'_>) -> Result<bool, Self::Error> {
        if !self.can_accept_match() {
            self.truncated = true;
            return Ok(false);
        }

        let line = sink_line(mat.line_number(), mat.bytes())?;
        if !self.accept_line(&line) {
            return Ok(false);
        }

        self.flush_current();
        self.current = Some(TextMatch {
            path: self.path.clone(),
            line_number: line.line_number,
            line: line.text,
            before: std::mem::take(&mut self.before),
            after: Vec::new(),
        });
        Ok(true)
    }

    fn context(
        &mut self,
        _searcher: &Searcher,
        context: &SinkContext<'_>,
    ) -> Result<bool, Self::Error> {
        let line = sink_line(context.line_number(), context.bytes())?;
        if !self.accept_line(&line) {
            return Ok(false);
        }

        match context.kind() {
            SinkContextKind::Before => {
                if self.current.is_some() {
                    self.flush_current();
                }
                self.before.push(line);
            }
            SinkContextKind::After => {
                if let Some(current) = self.current.as_mut() {
                    current.after.push(line);
                }
            }
            SinkContextKind::Other => {}
        }
        Ok(true)
    }

    fn context_break(&mut self, _searcher: &Searcher) -> Result<bool, Self::Error> {
        self.flush_current();
        self.before.clear();
        Ok(true)
    }

    fn finish(&mut self, _searcher: &Searcher, _finish: &SinkFinish) -> Result<(), Self::Error> {
        self.flush_current();
        Ok(())
    }
}

fn sink_line(line_number: Option<u64>, bytes: &[u8]) -> Result<SearchLine, io::Error> {
    let line_number = line_number
        .ok_or_else(|| io::Error::new(io::ErrorKind::Other, "line numbers not enabled"))?;
    let text = String::from_utf8_lossy(bytes)
        .trim_end_matches(&['\r', '\n'][..])
        .to_string();
    Ok(SearchLine { line_number, text })
}
