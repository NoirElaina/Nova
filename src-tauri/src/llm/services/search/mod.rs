use globset::{GlobBuilder, GlobSet, GlobSetBuilder};
use grep_regex::{RegexMatcher, RegexMatcherBuilder};
use grep_searcher::{
    BinaryDetection, Searcher, SearcherBuilder, Sink, SinkContext, SinkContextKind, SinkFinish,
    SinkMatch,
};
use ignore::WalkBuilder;
use serde::Serialize;
use std::collections::BTreeSet;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

const MAX_LINE_CHARS: usize = 500;

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

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TextSearchOutputMode {
    Content,
    FilesWithMatches,
    Count,
}

#[derive(Clone, Debug)]
pub struct TextSearchOptions {
    pub pattern: String,
    pub case_sensitive: bool,
    pub glob_patterns: Vec<String>,
    pub type_globs: Vec<String>,
    pub exclude_globs: Vec<String>,
    pub output_mode: TextSearchOutputMode,
    pub before_context: usize,
    pub after_context: usize,
    pub show_line_numbers: bool,
    pub head_limit: Option<usize>,
    pub offset: usize,
    pub multiline: bool,
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
#[serde(rename_all = "camelCase")]
pub struct TextSearchResult {
    pub mode: TextSearchOutputMode,
    pub num_files: usize,
    pub filenames: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub num_lines: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub num_matches: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub applied_limit: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub applied_offset: Option<usize>,
    pub stats: TextSearchStats,
}

pub fn find_files(
    match_root: &Path,
    search_root: &Path,
    options: &GlobSearchOptions,
) -> Result<GlobSearchResult, String> {
    let matcher = build_glob_set(
        &[options.pattern.as_str()],
        options.case_sensitive,
        "pattern",
    )?;
    let files = collect_files(match_root, search_root, Vec::new(), None, &options.walk);
    let mut matches = Vec::new();
    let mut truncated = false;

    for file in &files {
        let Some(relative) = match_path(match_root, file) else {
            continue;
        };
        if !matcher.is_match(&relative) {
            continue;
        }
        if matches.len() >= options.max_results {
            truncated = true;
            break;
        }
        matches.push(output_path(file));
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
    match_root: &Path,
    target: &Path,
    options: &TextSearchOptions,
) -> Result<TextSearchResult, String> {
    let matcher = build_text_matcher(&options.pattern, options.case_sensitive, options.multiline)?;
    let include_matchers = build_include_matchers(options)?;
    let exclude_matcher = build_exclude_matcher(options)?;
    let files = collect_files(
        match_root,
        target,
        include_matchers,
        exclude_matcher.as_ref(),
        &options.walk,
    );

    match options.output_mode {
        TextSearchOutputMode::Content => grep_content(&files, &matcher, options),
        TextSearchOutputMode::FilesWithMatches => {
            grep_files_with_matches(&files, &matcher, options)
        }
        TextSearchOutputMode::Count => grep_count(&files, &matcher, options),
    }
}

fn build_include_matchers(options: &TextSearchOptions) -> Result<Vec<GlobSet>, String> {
    let mut matchers = Vec::new();
    if !options.glob_patterns.is_empty() {
        let patterns = options
            .glob_patterns
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>();
        matchers.push(build_glob_set(&patterns, true, "glob")?);
    }
    if !options.type_globs.is_empty() {
        let patterns = options
            .type_globs
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>();
        matchers.push(build_glob_set(&patterns, false, "type")?);
    }
    Ok(matchers)
}

fn build_exclude_matcher(options: &TextSearchOptions) -> Result<Option<GlobSet>, String> {
    if options.exclude_globs.is_empty() {
        return Ok(None);
    }
    let patterns = options
        .exclude_globs
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>();
    build_glob_set(&patterns, false, "exclude_globs").map(Some)
}

fn grep_content(
    files: &[PathBuf],
    matcher: &RegexMatcher,
    options: &TextSearchOptions,
) -> Result<TextSearchResult, String> {
    let mut lines = Vec::<String>::new();
    let mut files_with_matches = BTreeSet::<String>::new();
    let mut matches_returned = 0usize;
    let mut used_chars = 0usize;
    let mut truncated = false;

    for file in files {
        let remaining_chars = options.max_output_chars.saturating_sub(used_chars);
        if remaining_chars == 0 {
            truncated = true;
            break;
        }

        let path = output_path(file);
        let mut sink = StructuredSearchSink::new(path.clone(), usize::MAX, remaining_chars);
        let mut searcher = searcher_builder(options)
            .before_context(options.before_context)
            .after_context(options.after_context)
            .build();

        if searcher.search_path(matcher, file, &mut sink).is_err() {
            continue;
        }

        let (file_matches, file_chars, file_truncated) = sink.finish();
        if file_matches.is_empty() {
            continue;
        }

        used_chars = used_chars.saturating_add(file_chars);
        files_with_matches.insert(path);
        matches_returned = matches_returned.saturating_add(file_matches.len());
        truncated = truncated || file_truncated;

        for item in &file_matches {
            append_match_lines(&mut lines, item, options.show_line_numbers);
        }
        if truncated {
            break;
        }
    }

    let window = apply_window(lines, options.head_limit, options.offset);
    truncated = truncated || window.truncated;

    let filenames = files_with_matches.into_iter().collect::<Vec<_>>();
    let num_files = filenames.len();

    Ok(TextSearchResult {
        mode: TextSearchOutputMode::Content,
        num_files,
        filenames,
        content: Some(window.items.join("\n")),
        num_lines: Some(window.items.len()),
        num_matches: None,
        applied_limit: window.applied_limit,
        applied_offset: window.applied_offset,
        stats: TextSearchStats {
            files_searched: files.len(),
            files_with_matches: num_files,
            matches_returned,
            truncated,
        },
    })
}

fn grep_files_with_matches(
    files: &[PathBuf],
    matcher: &RegexMatcher,
    options: &TextSearchOptions,
) -> Result<TextSearchResult, String> {
    let mut matches = Vec::<(String, u128)>::new();

    for file in files {
        let mut sink = CountSearchSink::stop_after_first();
        let mut searcher = searcher_builder(options).build();
        if searcher.search_path(matcher, file, &mut sink).is_err() {
            continue;
        }
        if sink.count == 0 {
            continue;
        }

        matches.push((output_path(file), mtime_millis(file)));
    }

    matches.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    let all_filenames = matches
        .into_iter()
        .map(|(path, _)| path)
        .collect::<Vec<_>>();
    let total_matches = all_filenames.len();
    let window = apply_window(all_filenames, options.head_limit, options.offset);

    Ok(TextSearchResult {
        mode: TextSearchOutputMode::FilesWithMatches,
        num_files: window.items.len(),
        filenames: window.items,
        content: None,
        num_lines: None,
        num_matches: None,
        applied_limit: window.applied_limit,
        applied_offset: window.applied_offset,
        stats: TextSearchStats {
            files_searched: files.len(),
            files_with_matches: total_matches,
            matches_returned: total_matches,
            truncated: window.truncated,
        },
    })
}

fn grep_count(
    files: &[PathBuf],
    matcher: &RegexMatcher,
    options: &TextSearchOptions,
) -> Result<TextSearchResult, String> {
    let mut count_lines = Vec::<(String, usize)>::new();
    let mut total_matching_lines = 0usize;

    for file in files {
        let mut sink = CountSearchSink::count_all();
        let mut searcher = searcher_builder(options).build();
        if searcher.search_path(matcher, file, &mut sink).is_err() {
            continue;
        }
        if sink.count == 0 {
            continue;
        }

        total_matching_lines = total_matching_lines.saturating_add(sink.count);
        let path = output_path(file);
        count_lines.push((path, sink.count));
    }

    let all_lines = count_lines
        .iter()
        .map(|(path, count)| format!("{}:{}", path, count))
        .collect::<Vec<_>>();
    let window = apply_window(all_lines, options.head_limit, options.offset);

    let displayed_matches = window
        .items
        .iter()
        .filter_map(|line| line.rsplit_once(':'))
        .filter_map(|(_, count)| count.parse::<usize>().ok())
        .sum::<usize>();

    Ok(TextSearchResult {
        mode: TextSearchOutputMode::Count,
        num_files: window.items.len(),
        filenames: Vec::new(),
        content: Some(window.items.join("\n")),
        num_lines: None,
        num_matches: Some(displayed_matches),
        applied_limit: window.applied_limit,
        applied_offset: window.applied_offset,
        stats: TextSearchStats {
            files_searched: files.len(),
            files_with_matches: count_lines.len(),
            matches_returned: total_matching_lines,
            truncated: window.truncated,
        },
    })
}

fn append_match_lines(lines: &mut Vec<String>, item: &TextMatch, show_line_numbers: bool) {
    for before in &item.before {
        lines.push(format_search_line(
            &item.path,
            before,
            show_line_numbers,
            false,
        ));
    }
    lines.push(format_search_line(
        &item.path,
        &SearchLine {
            line_number: item.line_number,
            text: item.line.clone(),
        },
        show_line_numbers,
        true,
    ));
    for after in &item.after {
        lines.push(format_search_line(
            &item.path,
            after,
            show_line_numbers,
            false,
        ));
    }
}

fn format_search_line(
    path: &str,
    line: &SearchLine,
    show_line_numbers: bool,
    is_match: bool,
) -> String {
    if show_line_numbers {
        let separator = if is_match { ':' } else { '-' };
        format!(
            "{}{}{}{}{}",
            path, separator, line.line_number, separator, line.text
        )
    } else {
        format!("{}:{}", path, line.text)
    }
}

struct Window<T> {
    items: Vec<T>,
    applied_limit: Option<usize>,
    applied_offset: Option<usize>,
    truncated: bool,
}

fn apply_window<T>(items: Vec<T>, head_limit: Option<usize>, offset: usize) -> Window<T> {
    let total = items.len();
    let skipped = offset.min(total);
    let mut iter = items.into_iter().skip(skipped);
    let (items, truncated, applied_limit) = match head_limit {
        Some(limit) => {
            let mut kept = Vec::new();
            for item in iter.by_ref().take(limit) {
                kept.push(item);
            }
            let remaining_after_window = total.saturating_sub(skipped).saturating_sub(kept.len());
            let truncated = remaining_after_window > 0;
            let applied_limit = truncated.then_some(limit);
            (kept, truncated, applied_limit)
        }
        None => (iter.collect(), false, None),
    };

    Window {
        items,
        applied_limit,
        applied_offset: (offset > 0).then_some(offset),
        truncated,
    }
}

fn searcher_builder(options: &TextSearchOptions) -> SearcherBuilder {
    let mut builder = SearcherBuilder::new();
    builder
        .line_number(true)
        .multi_line(options.multiline)
        .binary_detection(BinaryDetection::quit(b'\x00'));
    builder
}

fn build_glob_set(patterns: &[&str], case_sensitive: bool, label: &str) -> Result<GlobSet, String> {
    let mut builder = GlobSetBuilder::new();
    for pattern in patterns {
        let trimmed = pattern.trim();
        if trimmed.is_empty() {
            return Err(format!("{} cannot contain an empty glob", label));
        }
        add_glob(&mut builder, trimmed, case_sensitive, label)?;

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
    case_sensitive: bool,
    multiline: bool,
) -> Result<RegexMatcher, String> {
    RegexMatcherBuilder::new()
        .case_insensitive(!case_sensitive)
        .multi_line(multiline)
        .dot_matches_new_line(multiline)
        .build(pattern)
        .map_err(|error| format!("Invalid search pattern: {}", error))
}

fn collect_files(
    match_root: &Path,
    target: &Path,
    include_globs: Vec<GlobSet>,
    exclude_glob: Option<&GlobSet>,
    options: &WalkOptions,
) -> Vec<PathBuf> {
    if target.is_file() {
        return should_search_file(match_root, target, &include_globs, exclude_glob)
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
        if !file_type.is_file()
            || !should_search_file(match_root, entry.path(), &include_globs, exclude_glob)
        {
            continue;
        }
        files.push(entry.path().to_path_buf());
    }

    files.sort();
    files
}

fn should_search_file(
    match_root: &Path,
    file: &Path,
    include_globs: &[GlobSet],
    exclude_glob: Option<&GlobSet>,
) -> bool {
    let Some(candidate_path) = match_path(match_root, file) else {
        return false;
    };
    if exclude_glob
        .map(|matcher| matcher.is_match(&candidate_path))
        .unwrap_or(false)
    {
        return false;
    }
    include_globs
        .iter()
        .all(|matcher| matcher.is_match(&candidate_path))
}

fn match_path(match_root: &Path, path: &Path) -> Option<String> {
    if let Ok(relative) = path.strip_prefix(match_root) {
        let value = relative.to_string_lossy().replace('\\', "/");
        return (!value.is_empty()).then_some(value);
    }
    Some(path.to_string_lossy().replace('\\', "/"))
}

fn output_path(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

fn mtime_millis(path: &Path) -> u128 {
    fs::metadata(path)
        .and_then(|metadata| metadata.modified())
        .ok()
        .and_then(|mtime| mtime.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|duration| duration.as_millis())
        .unwrap_or(0)
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

struct CountSearchSink {
    count: usize,
    stop_after_first: bool,
}

impl CountSearchSink {
    fn count_all() -> Self {
        Self {
            count: 0,
            stop_after_first: false,
        }
    }

    fn stop_after_first() -> Self {
        Self {
            count: 0,
            stop_after_first: true,
        }
    }
}

impl Sink for CountSearchSink {
    type Error = io::Error;

    fn matched(&mut self, _searcher: &Searcher, _mat: &SinkMatch<'_>) -> Result<bool, Self::Error> {
        self.count = self.count.saturating_add(1);
        Ok(!self.stop_after_first)
    }
}

fn sink_line(line_number: Option<u64>, bytes: &[u8]) -> Result<SearchLine, io::Error> {
    let line_number = line_number
        .ok_or_else(|| io::Error::new(io::ErrorKind::Other, "line numbers not enabled"))?;
    let mut text = String::from_utf8_lossy(bytes)
        .trim_end_matches(&['\r', '\n'][..])
        .to_string();
    if text.chars().count() > MAX_LINE_CHARS {
        text = format!(
            "{}...",
            text.chars().take(MAX_LINE_CHARS).collect::<String>()
        );
    }
    Ok(SearchLine { line_number, text })
}
