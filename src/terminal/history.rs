//! Built-in CLI command history search with zero-latency FZF matching engine.
//!
//! Provides the exact same search power and syntax as `fzf --scheme=history`:
//! - Multi-word AND matching (terms separated by space)
//! - Exact substring match: `'pattern` or `"pattern"`
//! - Prefix exact match: `^pattern`
//! - Suffix exact match: `pattern$`
//! - Inverse / negation: `!pattern`, `!'exact`, `!^prefix`, `!suffix$`
//! - Fuzzy subsequence match: `pattern`
//! - Smart-case sensitivity (case-insensitive unless an uppercase char is used)
//! - Toggle sort modes: Relevance (best score) vs Recency (chronological)
//! - Real-time in-session command recording (searches commands run seconds ago)
//! - Deletion of history entries (`Shift+Delete` / `Ctrl+D`)
//!
//! Zero shell plugins required; works natively in any shell (zsh, bash, fish, sh).

use std::collections::{HashSet, VecDeque};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, mpsc::{self, Receiver, SyncSender}};

/// Maximum number of unique history commands to keep in memory.
const MAX_HISTORY_ITEMS: usize = 10_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortMode {
    Relevance,
    Recency,
}

impl SortMode {
    pub fn label(self) -> &'static str {
        match self {
            SortMode::Relevance => "Relevance",
            SortMode::Recency => "Recent",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FzfTerm {
    /// Exact substring match: `'pattern`
    Exact(String),
    /// Prefix match: `^pattern`
    Prefix(String),
    /// Suffix match: `pattern$`
    Suffix(String),
    /// Inverse match: `!pattern` (does not contain pattern)
    Inverse(String),
    /// Inverse prefix match: `!^pattern` (does not start with pattern)
    InversePrefix(String),
    /// Inverse suffix match: `!pattern$` (does not end with pattern)
    InverseSuffix(String),
    /// Fuzzy subsequence match: `pattern`
    Fuzzy(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MatchResult {
    /// Index into `HistoryStore::items`.
    pub item_index: usize,
    /// Sorting score (higher = better match).
    pub score: i32,
    /// Character indices in the command string that matched positive query terms.
    pub match_indices: Vec<usize>,
}

/// Commands loaded from the shell history files plus those run in this session.
/// Headless: it holds no popup state, so it can be filtered and edited without a UI.
#[derive(Debug, Default)]
pub struct HistoryStore {
    /// Unique commands, newest first.
    pub items: VecDeque<String>,
    pub session_commands: VecDeque<String>,
    worker: Option<HistoryWorker>,
    revision: u64,
    error: Option<&'static str>,
}

impl HistoryStore {
    pub fn new() -> Self {
        Self::default()
    }

    #[cfg(test)]
    pub(crate) fn isolated() -> Self {
        Self { worker: Some(HistoryWorker::start_with(Arc::new(|| {}), Vec::new, Vec::new).unwrap()), ..Self::default() }
    }

    /// Record a newly executed command from the active shell session.
    pub fn add_session_command(&mut self, cmd: &str) {
        let trimmed = cmd.trim();
        if trimmed.is_empty() {
            return;
        }
        // Remove prior duplicate in session commands if present
        if let Some(pos) = self.session_commands.iter().position(|c| c == trimmed) {
            self.session_commands.remove(pos);
        }
        self.session_commands.push_front(trimmed.to_string());
        if self.session_commands.len() > 500 {
            self.session_commands.pop_back();
        }
        // Insert into items if not present or move to top in O(1) when new
        if let Some(pos) = self.items.iter().position(|c| c == trimmed) {
            if let Some(existing) = self.items.remove(pos) {
                self.items.push_front(existing);
            }
        } else {
            self.items.push_front(trimmed.to_string());
            if self.items.len() > MAX_HISTORY_ITEMS {
                self.items.pop_back();
            }
        }
    }

    /// Set the completion notifier without giving the terminal layer UI dependencies.
    pub fn set_wake(&mut self, wake: Arc<dyn Fn() + Send + Sync>) -> std::io::Result<()> {
        self.worker = Some(HistoryWorker::start(wake)?);
        Ok(())
    }

    fn enqueue(&mut self, request: HistoryRequest) -> bool {
        if self.worker.is_none() {
            match HistoryWorker::start(Arc::new(|| {})) {
                Ok(worker) => self.worker = Some(worker),
                Err(_) => { self.error = Some("Unable to start the history worker."); return false; }
            }
        }
        let worker = self.worker.as_ref().unwrap();
        match worker.requests.try_send(HistoryOperation { request, results: worker.reply.clone(), wake: worker.wake.clone() }) {
            Ok(()) => true,
            Err(mpsc::TrySendError::Full(_)) => {
                self.error = Some("History is busy. Try again after pending work finishes.");
                false
            }
            Err(mpsc::TrySendError::Disconnected(_)) => {
                self.error = Some("The history worker is unavailable.");
                false
            }
        }
    }

    /// Preserve the selected entry unless its deletion was accepted by the worker.
    pub fn remove(&mut self, item_index: usize) -> Option<String> {
        let command = self.items.get(item_index)?.clone();
        if !self.enqueue(HistoryRequest::Delete(command.clone())) { return None; }
        self.revision = self.revision.wrapping_add(1);
        self.items.remove(item_index);
        self.session_commands.retain(|c| c != &command);
        Some(command)
    }

    /// Discovery, metadata and file reads all execute on the ordered worker.
    pub fn reload_if_changed(&mut self) {
        self.reload();
    }

    pub fn reload(&mut self) {
        let revision = self.revision.wrapping_add(1);
        if self.enqueue(HistoryRequest::Reload(revision)) { self.revision = revision; }
    }

    pub fn take_error(&mut self) -> Option<&'static str> { self.error.take() }

    /// Apply only current snapshots, retaining commands recorded since the request.
    pub fn poll(&mut self) -> bool {
        let mut changed = false;
        while let Some(result) = self.worker.as_ref().and_then(|w| w.results.try_recv().ok()) {
            match result {
                HistoryResult::Reload(revision, Ok(snapshot)) if revision == self.revision => {
                    if snapshot.read_failed { self.error = Some("Some shell history files could not be read."); }
                    let mut seen = HashSet::new();
                    self.items = self.session_commands.iter().cloned().chain(snapshot.items)
                        .filter(|c| seen.insert(c.clone())).take(MAX_HISTORY_ITEMS).collect();
                    changed = true;
                }
                HistoryResult::Reload(_, Err(())) => self.error = Some("Unable to read shell history."),
                HistoryResult::Delete(Err(())) => self.error = Some("Unable to save the history deletion. Reopen history to refresh."),
                _ => {}
            }
        }
        changed
    }

    /// Items matching `query` under the FZF rules, ordered by `sort`. An empty
    /// query matches everything in history order.
    pub fn filter(&self, query: &str, sort: SortMode) -> Vec<MatchResult> {
        let q = query.trim();
        if q.is_empty() {
            return (0..self.items.len())
                .map(|i| MatchResult {
                    item_index: i,
                    score: 0,
                    match_indices: Vec::new(),
                })
                .collect();
        }

        let terms = parse_fzf_query(q);
        let prepared: Vec<PreparedTerm> = terms.iter().map(FzfTerm::prepare).collect();
        let mut matches = Vec::new();

        for (idx, cmd) in self.items.iter().enumerate() {
            if let Some(m) = match_prepared_terms(&prepared, cmd, idx) {
                matches.push(m);
            }
        }

        if sort == SortMode::Relevance {
            matches.sort_by_key(|m| -m.score);
        } else {
            // Recency: original history order (newest first)
            matches.sort_by_key(|m| m.item_index);
        }
        matches
    }
}

const HISTORY_QUEUE_CAPACITY: usize = 16;

enum HistoryRequest { Reload(u64), Delete(String) }
enum HistoryResult { Reload(u64, Result<HistorySnapshot, ()>), Delete(Result<(), ()>) }
#[derive(Clone)]
struct HistorySnapshot {
    items: VecDeque<String>,
    read_failed: bool,
}
struct HistoryCache {
    fingerprint: Vec<(PathBuf, Option<(std::time::SystemTime, u64)>)>,
    snapshot: HistorySnapshot,
}
struct HistoryOperation {
    request: HistoryRequest,
    results: SyncSender<HistoryResult>,
    wake: Arc<dyn Fn() + Send + Sync>,
}
struct HistoryWorker {
    requests: SyncSender<HistoryOperation>,
    results: Receiver<HistoryResult>,
    reply: SyncSender<HistoryResult>,
    wake: Arc<dyn Fn() + Send + Sync>,
}
impl std::fmt::Debug for HistoryWorker {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { f.write_str("HistoryWorker") }
}
impl HistoryWorker {
    fn start(wake: Arc<dyn Fn() + Send + Sync>) -> std::io::Result<Self> {
        // All windows share one writer: independently renaming the same history
        // file can otherwise undo another window's accepted deletion.
        static REQUESTS: std::sync::OnceLock<std::io::Result<SyncSender<HistoryOperation>>> = std::sync::OnceLock::new();
        let requests = REQUESTS.get_or_init(|| Self::spawn_service(candidate_history_files, active_shell_history_files))
            .as_ref().map_err(|error| std::io::Error::new(error.kind(), "history worker unavailable"))?.clone();
        Ok(Self::client(requests, wake))
    }

    fn client(requests: SyncSender<HistoryOperation>, wake: Arc<dyn Fn() + Send + Sync>) -> Self {
        let (reply, results) = mpsc::sync_channel(HISTORY_QUEUE_CAPACITY);
        Self { requests, results, reply, wake }
    }

    #[cfg(test)]
    fn start_with(wake: Arc<dyn Fn() + Send + Sync>, discover: impl Fn() -> Vec<PathBuf> + Send + 'static, deletion_files: impl Fn() -> Vec<PathBuf> + Send + 'static) -> std::io::Result<Self> {
        Ok(Self::client(Self::spawn_service(discover, deletion_files)?, wake))
    }

    fn spawn_service(
        discover: impl Fn() -> Vec<PathBuf> + Send + 'static,
        deletion_files: impl Fn() -> Vec<PathBuf> + Send + 'static,
    ) -> std::io::Result<SyncSender<HistoryOperation>> {
        Self::spawn_service_using(discover, deletion_files, |work| std::thread::Builder::new().name("litty-history".into()).stack_size(128 * 1024).spawn(work).map(|_| ()))
    }

    fn spawn_service_using(
        discover: impl Fn() -> Vec<PathBuf> + Send + 'static,
        deletion_files: impl Fn() -> Vec<PathBuf> + Send + 'static,
        spawn: impl FnOnce(Box<dyn FnOnce() + Send>) -> std::io::Result<()>,
    ) -> std::io::Result<SyncSender<HistoryOperation>> {
        let (requests, rx) = mpsc::sync_channel::<HistoryOperation>(HISTORY_QUEUE_CAPACITY);
        spawn(Box::new(move || {
            let mut cache = None;
            while let Ok(operation) = rx.recv() {
                let result = match operation.request {
                    HistoryRequest::Reload(revision) => HistoryResult::Reload(revision, load_history(discover(), &mut cache).map_err(|_| ())),
                    HistoryRequest::Delete(command) => {
                        cache = None;
                        let result = deletion_files().iter().try_for_each(|path| remove_command_from_file(path, &command).map(|_| ()));
                        HistoryResult::Delete(result.map_err(|_| ()))
                    }
                };
                if operation.results.send(result).is_ok() { (operation.wake)(); }
            }
        }))?;
        Ok(requests)
    }
}

fn load_history(files: Vec<PathBuf>, cache: &mut Option<HistoryCache>) -> std::io::Result<HistorySnapshot> {
    let mut fingerprint = Vec::with_capacity(files.len());
    let mut first_error = None;
    for file in files {
        let metadata = match std::fs::metadata(&file).and_then(|metadata| metadata.modified().map(|mtime| (mtime, metadata.len()))) {
            Ok(metadata) => Some(metadata),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(error) => { first_error.get_or_insert(error); continue; }
        };
        fingerprint.push((file, metadata));
    }
    if first_error.is_none()
        && let Some(previous) = cache.as_ref().filter(|previous| previous.fingerprint == fingerprint) {
            return Ok(previous.snapshot.clone());
        }
    let mut snapshot = HistorySnapshot { items: VecDeque::new(), read_failed: false };
    let mut seen = HashSet::new();
    for (file, metadata) in &fingerprint {
        if metadata.is_some() && snapshot.items.len() < MAX_HISTORY_ITEMS
            && let Err(error) = parse_history_file_dedup(file, &mut seen, &mut snapshot.items, MAX_HISTORY_ITEMS) {
                first_error.get_or_insert(error);
            }
    }
    if let Some(error) = first_error {
        // Optional unreadable files must not hide successfully loaded history.
        // Retry partial snapshots next time instead of caching them as clean.
        *cache = None;
        if snapshot.items.is_empty() { return Err(error); }
        snapshot.read_failed = true;
    } else {
        *cache = Some(HistoryCache { fingerprint, snapshot: snapshot.clone() });
    }
    Ok(snapshot)
}

/// Candidate history file locations ordered by priority.
fn candidate_history_files() -> Vec<PathBuf> {
    let mut files = Vec::new();

    // 1. $HISTFILE if set and non-empty
    if let Ok(histfile) = std::env::var("HISTFILE")
        && !histfile.trim().is_empty() {
            files.push(PathBuf::from(histfile));
        }

    let home = std::env::var("HOME").ok().map(PathBuf::from);
    let shell = std::env::var("SHELL").unwrap_or_default();

    if let Some(ref h) = home {
        // 2. Shell-specific primary history file
        if shell.ends_with("zsh") {
            files.push(h.join(".zsh_history"));
            files.push(h.join(".bash_history"));
        } else if shell.ends_with("fish") {
            files.push(h.join(".local/share/fish/fish_history"));
            files.push(h.join(".zsh_history"));
            files.push(h.join(".bash_history"));
        } else {
            files.push(h.join(".bash_history"));
            files.push(h.join(".zsh_history"));
        }

        // 3. XDG standard paths
        if let Ok(xdg_data) = std::env::var("XDG_DATA_HOME") {
            files.push(PathBuf::from(xdg_data).join("fish/fish_history"));
        } else {
            files.push(h.join(".local/share/fish/fish_history"));
        }
    }

    // Deduplicate candidate paths
    let mut unique = Vec::new();
    let mut seen = HashSet::new();
    for f in files {
        if seen.insert(f.clone()) {
            unique.push(f);
        }
    }
    unique
}

/// One logical history entry: its exact on-disk byte range plus the decoded command.
#[derive(Debug)]
struct RawEntry {
    /// Byte range in the file, including the trailing newline(s).
    span: std::ops::Range<usize>,
    /// Decoded command, or `None` for blank lines and bash timestamp comments.
    command: Option<String>,
    /// Bash `HISTTIMEFORMAT` comment line (`#1729835432`) that belongs to the next entry.
    is_timestamp: bool,
}

fn is_fish_history(path: &Path) -> bool {
    path.to_string_lossy().contains("fish_history")
}

/// Zsh history files are "metafied": bytes `0x83..` are escaped as `0x83, byte ^ 0x20`.
fn is_zsh_history(path: &Path) -> bool {
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    if name.contains("zsh") || name.contains("zhistory") {
        return true;
    }
    let shell_is_zsh = std::env::var("SHELL").is_ok_and(|s| s.ends_with("zsh"));
    shell_is_zsh && std::env::var_os("HISTFILE").is_some_and(|h| Path::new(&h) == path)
}

/// Reverse zsh's `metafy()`: `0x83, b` decodes to `b ^ 0x20`.
fn unmetafy(bytes: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(bytes.len());
    let mut it = bytes.iter();
    while let Some(&b) = it.next() {
        if b == 0x83
            && let Some(&next) = it.next() {
                out.push(next ^ 0x20);
                continue;
            }
        out.push(b);
    }
    out
}

fn decode_history_bytes(bytes: &[u8], zsh: bool) -> String {
    if zsh && bytes.contains(&0x83)
        && let Ok(s) = String::from_utf8(unmetafy(bytes)) {
            return s;
        }
    String::from_utf8_lossy(bytes).into_owned()
}

/// Split a bash / zsh history file into entries. A line ending in `\` continues
/// onto the next line (zsh multi-line commands); the backslash is dropped.
fn split_line_entries(bytes: &[u8], zsh: bool) -> Vec<RawEntry> {
    let mut entries = Vec::new();
    let mut start = 0;
    let mut pos = 0;
    let mut content: Vec<u8> = Vec::new();
    while pos < bytes.len() {
        let line_end = bytes[pos..].iter().position(|&b| b == b'\n').map_or(bytes.len(), |i| pos + i);
        let next = (line_end + 1).min(bytes.len());
        let line = &bytes[pos..line_end];
        if !content.is_empty() {
            content.push(b'\n');
        }
        if let Some(stripped) = line.strip_suffix(b"\\")
            && next < bytes.len() {
                content.extend_from_slice(stripped);
                pos = next;
                continue;
            }
        content.extend_from_slice(line);
        entries.push(line_entry(start..next, &content, zsh));
        content.clear();
        start = next;
        pos = next;
    }
    entries
}

fn line_entry(span: std::ops::Range<usize>, content: &[u8], zsh: bool) -> RawEntry {
    let text = decode_history_bytes(content, zsh);
    let line = text.trim();
    if line.starts_with('#') && line.chars().skip(1).all(|c| c.is_ascii_digit()) {
        return RawEntry { span, command: None, is_timestamp: true };
    }
    // Zsh extended format: `: <timestamp>:<duration>;<command>`
    let cmd = match line.strip_prefix(": ").and_then(|_| line.find(';')) {
        Some(pos) => line[pos + 1..].trim(),
        None => line,
    };
    let command = (!cmd.is_empty()).then(|| cmd.to_string());
    RawEntry { span, command, is_timestamp: false }
}

/// Reverse fish's history escaping in one pass: `\\` is a backslash and `\n`
/// a newline. Two `replace` calls would read the `\n` inside `\\n` first.
fn unescape_fish(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('n') => out.push('\n'),
            Some('\\') => out.push('\\'),
            Some(other) => {
                out.push('\\');
                out.push(other);
            }
            None => out.push('\\'),
        }
    }
    out
}

/// Split a fish history file into `- cmd: <command>` blocks (each block runs until
/// the next `- cmd:` line and includes its `when:` / `paths:` lines).
fn split_fish_entries(bytes: &[u8]) -> Vec<RawEntry> {
    let mut entries: Vec<RawEntry> = Vec::new();
    let mut pos = 0;
    while pos < bytes.len() {
        let line_end = bytes[pos..].iter().position(|&b| b == b'\n').map_or(bytes.len(), |i| pos + i);
        let next = (line_end + 1).min(bytes.len());
        let line = String::from_utf8_lossy(&bytes[pos..line_end]);
        if let Some(rest) = line.trim().strip_prefix("- cmd:") {
            let cmd = unescape_fish(rest.trim());
            let command = (!cmd.is_empty()).then_some(cmd);
            entries.push(RawEntry { span: pos..next, command, is_timestamp: false });
        } else if let Some(last) = entries.last_mut() {
            last.span.end = next;
        } else {
            // Preamble before the first entry.
            entries.push(RawEntry { span: pos..next, command: None, is_timestamp: false });
        }
        pos = next;
    }
    entries
}

fn split_history_entries(path: &Path, bytes: &[u8]) -> Vec<RawEntry> {
    if is_fish_history(path) {
        split_fish_entries(bytes)
    } else {
        split_line_entries(bytes, is_zsh_history(path))
    }
}

fn decode_history_cow(bytes: &[u8], zsh: bool) -> std::borrow::Cow<'_, str> {
    if zsh && bytes.contains(&0x83)
        && let Ok(s) = String::from_utf8(unmetafy(bytes)) {
            return std::borrow::Cow::Owned(s);
        }
    String::from_utf8_lossy(bytes)
}

fn extract_line_command(text: &str) -> Option<&str> {
    let line = text.trim();
    if line.starts_with('#') && line.chars().skip(1).all(|c| c.is_ascii_digit()) {
        return None;
    }
    let cmd = match line.strip_prefix(": ").and_then(|_| line.find(';')) {
        Some(pos) => line[pos + 1..].trim(),
        None => line,
    };
    (!cmd.is_empty()).then_some(cmd)
}

/// Parse shell history file into `out`, deduplicating against `seen` *before* allocating
/// owned `String`s for duplicate lines.
fn parse_history_file_dedup(
    path: &Path,
    seen: &mut HashSet<Box<str>>,
    out: &mut VecDeque<String>,
    max_items: usize,
) -> std::io::Result<()> {
    let bytes = std::fs::read(path)?;
    if is_fish_history(path) {
        for entry in split_fish_entries(&bytes).into_iter().rev() {
            if let Some(cmd) = entry.command
                && !seen.contains(cmd.as_str()) {
                    seen.insert(cmd.as_str().into());
                    out.push_back(cmd);
                    if out.len() >= max_items {
                        break;
                    }
                }
        }
        return Ok(());
    }

    let zsh = is_zsh_history(path);
    // Collect logical line byte ranges (or joined multi-line ranges) in forward order,
    // then iterate in reverse (newest first) and only allocate String when not in `seen`.
    enum LineSlice<'a> {
        Borrowed(&'a [u8]),
        Joined(Vec<u8>),
    }
    impl<'a> LineSlice<'a> {
        #[inline]
        fn as_bytes(&self) -> &[u8] {
            match self {
                LineSlice::Borrowed(b) => b,
                LineSlice::Joined(v) => v.as_slice(),
            }
        }
    }

    let mut raw_lines: Vec<LineSlice<'_>> = Vec::new();
    let mut pos = 0;
    let mut content: Vec<u8> = Vec::new();
    while pos < bytes.len() {
        let line_end = bytes[pos..].iter().position(|&b| b == b'\n').map_or(bytes.len(), |i| pos + i);
        let next = (line_end + 1).min(bytes.len());
        let line = &bytes[pos..line_end];
        if !content.is_empty() {
            content.push(b'\n');
        }
        if let Some(stripped) = line.strip_suffix(b"\\")
            && next < bytes.len() {
                content.extend_from_slice(stripped);
                pos = next;
                continue;
            }
        if content.is_empty() {
            raw_lines.push(LineSlice::Borrowed(line));
        } else {
            content.extend_from_slice(line);
            raw_lines.push(LineSlice::Joined(std::mem::take(&mut content)));
        }
        pos = next;
    }

    for raw in raw_lines.into_iter().rev() {
        let decoded = decode_history_cow(raw.as_bytes(), zsh);
        if let Some(cmd) = extract_line_command(&decoded)
            && !seen.contains(cmd) {
                let s = cmd.to_string();
                seen.insert(s.as_str().into());
                out.push_back(s);
                if out.len() >= max_items {
                    break;
                }
            }
    }
    Ok(())
}

/// Parse shell history file into commands (appends in reverse-chronological order).
#[cfg(test)]
fn parse_history_file(path: &Path, out: &mut Vec<String>) {
    let Ok(bytes) = std::fs::read(path) else { return };
    // File bottom has the most recent commands.
    out.extend(split_history_entries(path, &bytes).into_iter().rev().filter_map(|e| e.command));
}

/// Return `bytes` with every entry equal to `cmd` removed (each along with a bash
/// timestamp comment directly preceding it), or `None` if nothing matched.
/// The popup lists a command once, so deleting it must leave no copy behind.
/// All other entries are kept byte-for-byte, so metafied zsh data is never re-encoded.
fn strip_command_entries(path: &Path, bytes: &[u8], cmd: &str) -> Option<Vec<u8>> {
    let entries = split_history_entries(path, bytes);
    let mut kept: Vec<&RawEntry> = Vec::with_capacity(entries.len());
    let mut removed = false;
    for entry in &entries {
        if entry.command.as_deref() == Some(cmd) {
            if kept.last().is_some_and(|e| e.is_timestamp) {
                kept.pop();
            }
            removed = true;
            continue;
        }
        kept.push(entry);
    }
    if !removed {
        return None;
    }
    let mut out = Vec::with_capacity(bytes.len());
    for entry in kept {
        out.extend_from_slice(&bytes[entry.span.clone()]);
    }
    Some(out)
}

/// Remove `cmd` from one history file: write the result to a private temp file in
/// the same directory and atomically `rename` it over the original. If the shell
/// appends to the file meanwhile, start over from the newer content.
fn remove_command_from_file(path: &Path, cmd: &str) -> std::io::Result<bool> {
    use std::os::unix::fs::OpenOptionsExt;

    // Rewrite the real file, not a symlink pointing at it (e.g. dotfiles repos).
    // The format is still detected from the configured path (`HISTFILE`, `.zsh_history`).
    let format_path = path;
    let path = std::fs::canonicalize(path)?;
    let file_name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    static TMP_COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let count = TMP_COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let tmp = path.with_file_name(format!(".{file_name}.litty-{}-{count}-{nanos}.tmp", std::process::id()));

    for _ in 0..3 {
        let before = std::fs::metadata(&path)?;
        let original = std::fs::read(&path)?;
        let Some(updated) = strip_command_entries(format_path, &original, cmd) else {
            return Ok(false);
        };

        let _ = std::fs::remove_file(&tmp);
        let mut file = std::fs::OpenOptions::new().write(true).create_new(true).mode(0o600).open(&tmp)?;
        let written = file
            .write_all(&updated)
            .and_then(|_| file.sync_all())
            .and_then(|_| std::fs::set_permissions(&tmp, before.permissions()));
        drop(file);
        if let Err(e) = written {
            let _ = std::fs::remove_file(&tmp);
            return Err(e);
        }

        let after = std::fs::metadata(&path)?;
        if after.len() != before.len() || after.modified().ok() != before.modified().ok() {
            let _ = std::fs::remove_file(&tmp);
            continue;
        }
        if let Err(e) = std::fs::rename(&tmp, &path) {
            let _ = std::fs::remove_file(&tmp);
            return Err(e);
        }
        return Ok(true);
    }
    Ok(false)
}

/// Resolve the history file(s) owned by the current active shell.
fn active_shell_history_files() -> Vec<PathBuf> {
    if let Ok(histfile) = std::env::var("HISTFILE") {
        let p = PathBuf::from(histfile.trim());
        if !histfile.trim().is_empty() && p.exists() {
            return vec![p];
        }
    }
    let Some(h) = std::env::var("HOME").ok().map(PathBuf::from) else { return candidate_history_files(); };
    let shell = std::env::var("SHELL").unwrap_or_default();
    let mut files = Vec::new();
    if shell.ends_with("zsh") {
        let p = h.join(".zsh_history");
        if p.exists() {
            files.push(p);
        }
    } else if shell.ends_with("fish") {
        let p = h.join(".local/share/fish/fish_history");
        if p.exists() {
            files.push(p);
        }
        if let Ok(xdg) = std::env::var("XDG_DATA_HOME") {
            let p2 = PathBuf::from(xdg).join("fish/fish_history");
            if p2.exists() && !files.contains(&p2) {
                files.push(p2);
            }
        }
    } else if shell.ends_with("bash") {
        let p = h.join(".bash_history");
        if p.exists() {
            files.push(p);
        }
    }

    if files.is_empty()
        && let Some(f) = candidate_history_files().into_iter().find(|p| p.exists()) {
            files.push(f);
        }
    files
}

/// Parse an FZF query string into individual match terms.
///
/// Supports:
/// - `'pattern` or `"pattern"`: exact substring match
/// - `^pattern`: prefix match
/// - `pattern$`: suffix match
/// - `!pattern`: inverse match (must not contain)
/// - `!^pattern`: inverse prefix
/// - `!pattern$`: inverse suffix
/// - `pattern`: fuzzy subsequence match
pub fn parse_fzf_query(query: &str) -> Vec<FzfTerm> {
    let mut terms = Vec::new();
    for raw in query.split_whitespace() {
        if raw.is_empty() {
            continue;
        }
        if let Some(rest) = raw.strip_prefix('!') {
            if rest.starts_with('\'') || rest.starts_with('"') {
                let pat = rest[1..].trim_end_matches('"');
                if !pat.is_empty() {
                    terms.push(FzfTerm::Inverse(pat.to_string()));
                }
            } else if let Some(pat) = rest.strip_prefix('^') {
                if !pat.is_empty() {
                    terms.push(FzfTerm::InversePrefix(pat.to_string()));
                }
            } else if rest.ends_with('$') && rest.len() > 1 {
                let pat = &rest[..rest.len() - 1];
                if !pat.is_empty() {
                    terms.push(FzfTerm::InverseSuffix(pat.to_string()));
                }
            } else if !rest.is_empty() {
                terms.push(FzfTerm::Inverse(rest.to_string()));
            }
        } else if raw.starts_with('\'') || (raw.starts_with('"') && raw.ends_with('"') && raw.len() > 1) {
            let pat = raw.trim_matches('\'').trim_matches('"');
            if !pat.is_empty() {
                terms.push(FzfTerm::Exact(pat.to_string()));
            }
        } else if let Some(pat) = raw.strip_prefix('^') {
            if !pat.is_empty() {
                terms.push(FzfTerm::Prefix(pat.to_string()));
            }
        } else if raw.ends_with('$') && raw.len() > 1 {
            let pat = &raw[..raw.len() - 1];
            if !pat.is_empty() {
                terms.push(FzfTerm::Suffix(pat.to_string()));
            }
        } else {
            terms.push(FzfTerm::Fuzzy(raw.to_string()));
        }
    }
    terms
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TermKind {
    Exact,
    Prefix,
    Suffix,
    Inverse,
    InversePrefix,
    InverseSuffix,
    Fuzzy,
}

#[derive(Debug, Clone)]
pub struct PreparedTerm {
    pub kind: TermKind,
    pub pattern: String,
    pub pattern_lower: String,
    pub pattern_chars: Vec<char>,
    pub pattern_bytes: Vec<u8>,
    pub pattern_bytes_lower: Vec<u8>,
    pub is_smart_case: bool,
    pub is_ascii: bool,
}

impl FzfTerm {
    pub fn prepare(&self) -> PreparedTerm {
        let (kind, p) = match self {
            FzfTerm::Exact(s) => (TermKind::Exact, s),
            FzfTerm::Prefix(s) => (TermKind::Prefix, s),
            FzfTerm::Suffix(s) => (TermKind::Suffix, s),
            FzfTerm::Inverse(s) => (TermKind::Inverse, s),
            FzfTerm::InversePrefix(s) => (TermKind::InversePrefix, s),
            FzfTerm::InverseSuffix(s) => (TermKind::InverseSuffix, s),
            FzfTerm::Fuzzy(s) => (TermKind::Fuzzy, s),
        };
        let is_smart_case = p.chars().any(|c| c.is_uppercase());
        let pattern_lower = p.to_lowercase();
        let pattern_chars: Vec<char> = if is_smart_case {
            p.chars().collect()
        } else {
            pattern_lower.chars().collect()
        };
        let is_ascii = p.is_ascii();
        let pattern_bytes = p.as_bytes().to_vec();
        let pattern_bytes_lower = pattern_lower.as_bytes().to_vec();

        PreparedTerm {
            kind,
            pattern: p.clone(),
            pattern_lower,
            pattern_chars,
            pattern_bytes,
            pattern_bytes_lower,
            is_smart_case,
            is_ascii,
        }
    }
}

/// Match command text against pre-compiled FZF terms with smart-case and scoring.
pub fn match_prepared_terms(terms: &[PreparedTerm], text: &str, recency_rank: usize) -> Option<MatchResult> {
    if terms.is_empty() {
        return Some(MatchResult {
            item_index: recency_rank,
            score: 0,
            match_indices: Vec::new(),
        });
    }

    let is_text_ascii = text.is_ascii();
    let all_terms_ascii = terms.iter().all(|t| t.is_ascii);

    if is_text_ascii && all_terms_ascii {
        match_ascii_terms(terms, text, recency_rank)
    } else {
        match_unicode_terms(terms, text, recency_rank)
    }
}

/// Fast-path zero-allocation matcher for ASCII shell commands and search queries.
fn match_ascii_terms(terms: &[PreparedTerm], text: &str, recency_rank: usize) -> Option<MatchResult> {
    let t_bytes = text.as_bytes();
    let t_len = t_bytes.len();

    let mut scratch = [0u8; 256];
    let lower_storage: Vec<u8>;
    let t_lower: &[u8] = if t_len <= scratch.len() {
        for (dst, &b) in scratch.iter_mut().zip(t_bytes) {
            *dst = b.to_ascii_lowercase();
        }
        &scratch[..t_len]
    } else {
        lower_storage = t_bytes.iter().map(|&b| b.to_ascii_lowercase()).collect();
        &lower_storage
    };

    let mut total_score = 0i32;
    let mut all_match_indices = Vec::new();

    for term in terms {
        let p_pat = if term.is_smart_case {
            &term.pattern_bytes
        } else {
            &term.pattern_bytes_lower
        };
        let target = if term.is_smart_case { t_bytes } else { t_lower };
        let p_len = p_pat.len();

        match term.kind {
            TermKind::Inverse => {
                if target.windows(p_len).any(|w| w == p_pat.as_slice()) {
                    return None;
                }
            }
            TermKind::InversePrefix => {
                if target.starts_with(p_pat) {
                    return None;
                }
            }
            TermKind::InverseSuffix => {
                if target.ends_with(p_pat) {
                    return None;
                }
            }
            TermKind::Prefix => {
                if target.starts_with(p_pat) {
                    all_match_indices.extend(0..p_len);
                    total_score += 3000;
                } else {
                    return None;
                }
            }
            TermKind::Suffix => {
                if target.ends_with(p_pat) {
                    let start = t_len.saturating_sub(p_len);
                    all_match_indices.extend(start..t_len);
                    total_score += 2500;
                } else {
                    return None;
                }
            }
            TermKind::Exact => {
                let byte_pos = target.windows(p_len).position(|w| w == p_pat.as_slice())?;
                all_match_indices.extend(byte_pos..byte_pos + p_len);
                total_score += 2000 - (byte_pos as i32 * 3);
                if byte_pos == 0 {
                    total_score += 500;
                } else {
                    let prev = t_bytes[byte_pos - 1];
                    if prev == b' ' || prev == b'-' || prev == b'_' || prev == b'/' || prev == b'.' {
                        total_score += 300;
                    }
                }
            }
            TermKind::Fuzzy => {
                if let Some(byte_pos) = target.windows(p_len).position(|w| w == p_pat.as_slice()) {
                    all_match_indices.extend(byte_pos..byte_pos + p_len);
                    total_score += 1500 - (byte_pos as i32 * 2);
                    if byte_pos == 0 {
                        total_score += 400;
                    } else {
                        let prev = t_bytes[byte_pos - 1];
                        if prev == b' ' || prev == b'-' || prev == b'_' || prev == b'/' || prev == b'.' {
                            total_score += 250;
                        }
                    }
                } else {
                    let mut p_idx = 0;
                    let mut last_match: Option<usize> = None;
                    let mut term_score = 600;
                    let start_len = all_match_indices.len();

                    for (t_idx, &b) in target.iter().enumerate() {
                        if p_idx < p_len && b == p_pat[p_idx] {
                            all_match_indices.push(t_idx);
                            if let Some(last) = last_match
                                && last + 1 == t_idx {
                                    term_score += 50;
                                }
                            if t_idx == 0 {
                                term_score += 100;
                            } else {
                                let prev = t_bytes[t_idx - 1];
                                if prev == b' ' || prev == b'-' || prev == b'_' || prev == b'/' || prev == b'.' {
                                    term_score += 80;
                                }
                            }
                            last_match = Some(t_idx);
                            p_idx += 1;
                        }
                    }
                    if p_idx == p_len {
                        total_score += term_score;
                    } else {
                        all_match_indices.truncate(start_len);
                        return None;
                    }
                }
            }
        }
    }

    all_match_indices.sort_unstable();
    all_match_indices.dedup();

    total_score -= (recency_rank as i32).min(500);

    Some(MatchResult {
        item_index: recency_rank,
        score: total_score,
        match_indices: all_match_indices,
    })
}

/// Unicode fallback matcher for multilingual commands and search queries.
fn match_unicode_terms(terms: &[PreparedTerm], text: &str, recency_rank: usize) -> Option<MatchResult> {
    let t_lower: Vec<char> = text.to_lowercase().chars().collect();
    let t_str_lower = text.to_lowercase();
    let t_chars: Vec<char> = text.chars().collect();

    let mut total_score = 0i32;
    let mut all_match_indices = Vec::new();

    for term in terms {
        let is_smart_case = term.is_smart_case;
        let p_pat = if is_smart_case { &term.pattern } else { &term.pattern_lower };
        let target = if is_smart_case { text } else { &t_str_lower };
        let p_chars = &term.pattern_chars;

        match term.kind {
            TermKind::Inverse => {
                if target.contains(p_pat.as_str()) {
                    return None;
                }
            }
            TermKind::InversePrefix => {
                if target.starts_with(p_pat.as_str()) {
                    return None;
                }
            }
            TermKind::InverseSuffix => {
                if target.ends_with(p_pat.as_str()) {
                    return None;
                }
            }
            TermKind::Prefix => {
                if target.starts_with(p_pat.as_str()) {
                    let p_len = p_chars.len();
                    all_match_indices.extend(0..p_len);
                    total_score += 3000;
                } else {
                    return None;
                }
            }
            TermKind::Suffix => {
                if target.ends_with(p_pat.as_str()) {
                    let p_len = p_chars.len();
                    let start = t_chars.len().saturating_sub(p_len);
                    all_match_indices.extend(start..t_chars.len());
                    total_score += 2500;
                } else {
                    return None;
                }
            }
            TermKind::Exact => {
                let byte_pos = target.find(p_pat.as_str())?;
                let char_pos = target[..byte_pos].chars().count();
                let p_len = p_chars.len();
                all_match_indices.extend(char_pos..char_pos + p_len);
                total_score += 2000 - (char_pos as i32 * 3);
                if char_pos == 0 {
                    total_score += 500;
                } else {
                    let prev = t_chars[char_pos - 1];
                    if prev == ' ' || prev == '-' || prev == '_' || prev == '/' || prev == '.' {
                        total_score += 300;
                    }
                }
            }
            TermKind::Fuzzy => {
                if let Some(byte_pos) = target.find(p_pat.as_str()) {
                    let char_pos = target[..byte_pos].chars().count();
                    let p_len = p_chars.len();
                    all_match_indices.extend(char_pos..char_pos + p_len);
                    total_score += 1500 - (char_pos as i32 * 2);
                    if char_pos == 0 {
                        total_score += 400;
                    } else {
                        let prev = t_chars[char_pos - 1];
                        if prev == ' ' || prev == '-' || prev == '_' || prev == '/' || prev == '.' {
                            total_score += 250;
                        }
                    }
                } else {
                    let mut p_idx = 0;
                    let mut last_match: Option<usize> = None;
                    let mut term_score = 600;
                    let search_target: &[char] = if is_smart_case { &t_chars } else { &t_lower };
                    let start_len = all_match_indices.len();

                    for (t_idx, &tch) in search_target.iter().enumerate() {
                        if p_idx < p_chars.len() && tch == p_chars[p_idx] {
                            all_match_indices.push(t_idx);
                            if let Some(last) = last_match
                                && last + 1 == t_idx {
                                    term_score += 50;
                                }
                            if t_idx == 0 {
                                term_score += 100;
                            } else {
                                let prev = t_chars[t_idx - 1];
                                if prev == ' ' || prev == '-' || prev == '_' || prev == '/' || prev == '.' {
                                    term_score += 80;
                                }
                            }
                            last_match = Some(t_idx);
                            p_idx += 1;
                        }
                    }
                    if p_idx == p_chars.len() {
                        total_score += term_score;
                    } else {
                        all_match_indices.truncate(start_len);
                        return None;
                    }
                }
            }
        }
    }

    all_match_indices.sort_unstable();
    all_match_indices.dedup();

    total_score -= (recency_rank as i32).min(500);

    Some(MatchResult {
        item_index: recency_rank,
        score: total_score,
        match_indices: all_match_indices,
    })
}

/// Match command text against parsed FZF terms with smart-case and scoring.
#[allow(dead_code)]
pub fn match_fzf_terms(terms: &[FzfTerm], text: &str, recency_rank: usize) -> Option<MatchResult> {
    if terms.is_empty() {
        return Some(MatchResult {
            item_index: recency_rank,
            score: 0,
            match_indices: Vec::new(),
        });
    }
    let prepared: Vec<PreparedTerm> = terms.iter().map(FzfTerm::prepare).collect();
    match_prepared_terms(&prepared, text, recency_rank)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reload_does_not_perform_filesystem_work_on_the_caller() {
        let mut store = HistoryStore::new();
        store.reload_if_changed();
        assert!(store.items.is_empty(), "reload must enqueue work instead of discovering/statting history on the UI thread");
    }

    #[test]
    fn blocked_discovery_never_blocks_ui_work_and_completion_wakes_the_consumer() {
        let path = temp_history(".bash_history", b"disk command\n");
        let read_path = path.clone();
        let (entered_tx, entered_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let (wake_tx, wake_rx) = mpsc::channel();
        let mut store = HistoryStore::new();
        store.worker = Some(HistoryWorker::start_with(Arc::new(move || { let _ = wake_tx.send(()); }), move || {
            entered_tx.send(()).unwrap();
            release_rx.recv().unwrap();
            vec![read_path.clone()]
        }, Vec::new).unwrap());
        store.reload_if_changed();
        entered_rx.recv().unwrap();
        store.add_session_command("UI remains responsive");
        assert_eq!(store.items[0], "UI remains responsive");
        release_tx.send(()).unwrap();
        wake_rx.recv().unwrap();
        assert!(store.poll());
        assert_eq!(store.items, ["UI remains responsive", "disk command"]);
        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }

    #[test]
    fn closing_a_client_with_a_full_reply_queue_does_not_strand_the_service() {
        let requests = HistoryWorker::spawn_service(Vec::new, Vec::new).unwrap();
        let first = HistoryWorker::client(requests.clone(), Arc::new(|| {}));
        for revision in 0..=HISTORY_QUEUE_CAPACITY {
            requests.send(HistoryOperation { request: HistoryRequest::Reload(revision as u64), results: first.reply.clone(), wake: first.wake.clone() }).unwrap();
        }
        drop(first);
        let second = HistoryWorker::client(requests.clone(), Arc::new(|| {}));
        requests.send(HistoryOperation { request: HistoryRequest::Reload(99), results: second.reply.clone(), wake: second.wake.clone() }).unwrap();
        assert!(matches!(second.results.recv_timeout(std::time::Duration::from_secs(3)).unwrap(), HistoryResult::Reload(99, Ok(_))));
    }

    #[test]
    fn ordered_worker_keeps_both_deletions_and_discovers_off_the_caller() {
        let path = temp_history(".bash_history", b"first\nsecond\nkeep\n");
        let read_path = path.clone();
        let delete_path = path.clone();
        let caller = std::thread::current().id();
        let (wake_tx, wake_rx) = mpsc::channel();
        let requests = HistoryWorker::spawn_service(move || {
            assert_ne!(std::thread::current().id(), caller);
            vec![read_path.clone()]
        }, move || vec![delete_path.clone()]).unwrap();
        let mut first = HistoryStore::new();
        let mut second = HistoryStore::new();
        let notify: Arc<dyn Fn() + Send + Sync> = Arc::new(move || { let _ = wake_tx.send(()); });
        first.worker = Some(HistoryWorker::client(requests.clone(), notify.clone()));
        second.worker = Some(HistoryWorker::client(requests, notify));
        first.items = VecDeque::from(["first".into(), "second".into(), "keep".into()]);
        second.items = first.items.clone();
        first.reload(); // This snapshot must not resurrect the next deletion.
        assert_eq!(first.remove(0).as_deref(), Some("first"));
        assert_eq!(second.remove(1).as_deref(), Some("second"));
        first.reload();
        first.add_session_command("new session");
        for _ in 0..4 { wake_rx.recv().unwrap(); }
        assert!(first.poll());
        second.poll();
        assert_eq!(first.items, ["new session", "keep"]);
        assert_eq!(std::fs::read(&path).unwrap(), b"keep\n");
        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }

    #[test]
    fn history_worker_spawn_failure_is_reported_without_filesystem_work() {
        let result = HistoryWorker::spawn_service_using(|| panic!("discovery must not run"), || panic!("deletion must not run"), |_| Err(std::io::Error::other("injected spawn failure")));
        assert!(matches!(result, Err(error) if error.to_string() == "injected spawn failure"));
    }

    #[test]
    fn rejected_deletion_preserves_the_entry_and_reports_full_or_disconnected() {
        let (requests, receiver) = mpsc::sync_channel(HISTORY_QUEUE_CAPACITY);
        let mut store = HistoryStore::new();
        store.worker = Some(HistoryWorker::client(requests, Arc::new(|| {})));
        store.add_session_command("preserved");
        for _ in 0..HISTORY_QUEUE_CAPACITY { store.reload(); }
        assert_eq!(store.remove(0), None);
        assert_eq!(store.items[0], "preserved");
        assert_eq!(store.session_commands[0], "preserved");
        assert!(store.take_error().unwrap().contains("busy"));
        drop(receiver);
        assert_eq!(store.remove(0), None);
        assert!(store.take_error().unwrap().contains("unavailable"));
        assert_eq!(store.items[0], "preserved");
    }

    #[test]
    fn stale_snapshot_cannot_resurrect_an_accepted_deletion() {
        let (requests, _receiver) = mpsc::sync_channel(HISTORY_QUEUE_CAPACITY);
        let mut store = HistoryStore::new();
        store.worker = Some(HistoryWorker::client(requests, Arc::new(|| {})));
        store.add_session_command("deleted");
        store.reload();
        let revision = store.revision;
        store.worker.as_ref().unwrap().reply.send(HistoryResult::Reload(revision, Ok(HistorySnapshot { items: VecDeque::from(["deleted".into()]), read_failed: false }))).unwrap();
        assert_eq!(store.remove(0).as_deref(), Some("deleted"));
        assert!(!store.poll());
        assert!(store.items.is_empty());
    }

    #[test]
    fn unreadable_optional_history_preserves_readable_candidates_and_reports_partial_failure() {
        let path = temp_history(".zsh_history", b"readable command\n");
        let paths = vec![path.parent().unwrap().to_owned(), path.clone()];
        let (wake_tx, wake_rx) = mpsc::channel();
        let mut store = HistoryStore::new();
        store.worker = Some(HistoryWorker::start_with(Arc::new(move || { let _ = wake_tx.send(()); }), move || paths.clone(), Vec::new).unwrap());
        store.reload();
        wake_rx.recv().unwrap();
        store.poll();
        assert_eq!(store.items, ["readable command"], "an unreadable optional candidate must not hide valid history");
        assert!(store.take_error().unwrap().contains("read"));
        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }

    #[test]
    fn worker_io_errors_complete_and_notify_without_logging_commands() {
        let path = temp_history(".bash_history", b"private command\n");
        let invalid = path.parent().unwrap().to_owned(); // Reading a directory fails on both supported OSes.
        let invalid_delete = invalid.clone();
        let (wake_tx, wake_rx) = mpsc::channel();
        let mut store = HistoryStore::new();
        store.worker = Some(HistoryWorker::start_with(Arc::new(move || { let _ = wake_tx.send(()); }), move || vec![invalid.clone()], move || vec![invalid_delete.clone()]).unwrap());
        store.add_session_command("private command");
        store.reload();
        wake_rx.recv().unwrap();
        assert!(!store.poll());
        assert_eq!(store.take_error(), Some("Unable to read shell history."));
        store.remove(0).unwrap();
        wake_rx.recv().unwrap();
        store.poll();
        let error = store.take_error().unwrap();
        assert!(error.contains("Unable to save"));
        assert!(!error.contains("private command"));
        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }

    #[test]
    fn parse_fzf_query_tokens() {
        let terms = parse_fzf_query("^git 'status !commit feat$");
        assert_eq!(
            terms,
            vec![
                FzfTerm::Prefix("git".into()),
                FzfTerm::Exact("status".into()),
                FzfTerm::Inverse("commit".into()),
                FzfTerm::Suffix("feat".into()),
            ]
        );
    }

    #[test]
    fn fzf_prefix_and_suffix_match() {
        let terms = parse_fzf_query("^git main$");
        let res = match_fzf_terms(&terms, "git push origin main", 0).expect("should match prefix & suffix");
        assert!(res.match_indices.contains(&0));
        assert!(res.match_indices.contains(&19));
    }

    #[test]
    fn fzf_inverse_match() {
        let terms = parse_fzf_query("git !commit");
        assert!(match_fzf_terms(&terms, "git status", 0).is_some());
        assert!(match_fzf_terms(&terms, "git commit -m initial", 0).is_none());
    }

    #[test]
    fn fzf_exact_match() {
        let terms = parse_fzf_query("'test");
        let res = match_fzf_terms(&terms, "cargo test --all", 0).expect("should match exact");
        assert_eq!(res.match_indices, vec![6, 7, 8, 9]);
    }

    #[test]
    fn fzf_subsequence_fuzzy_match() {
        let terms = parse_fzf_query("gcm");
        let res = match_fzf_terms(&terms, "git commit -m initial", 0).expect("should match subsequence");
        assert_eq!(res.match_indices, vec![0, 4, 6]);
    }

    #[test]
    fn fzf_smart_case_matching() {
        let terms_lower = parse_fzf_query("litty");
        assert!(match_fzf_terms(&terms_lower, "Litty Terminal", 0).is_some());

        let terms_upper = parse_fzf_query("Litty");
        assert!(match_fzf_terms(&terms_upper, "Litty Terminal", 0).is_some());
        assert!(match_fzf_terms(&terms_upper, "litty terminal", 0).is_none());
    }

    #[test]
    fn session_command_recording() {
        let mut store = HistoryStore {
            items: VecDeque::from(["older 1".into(), "older 2".into()]),
            ..HistoryStore::default()
        };
        store.add_session_command("just_now_command");
        assert_eq!(store.items[0], "just_now_command");
        assert_eq!(store.session_commands[0], "just_now_command");
    }

    #[test]
    fn filter_orders_by_relevance_or_recency_and_matches_everything_when_empty() {
        let store = HistoryStore {
            items: VecDeque::from(["git status".into(), "ls -la".into(), "git stash".into()]),
            ..HistoryStore::default()
        };
        let all = store.filter("  ", SortMode::Relevance);
        assert_eq!(all.iter().map(|m| m.item_index).collect::<Vec<_>>(), [0, 1, 2]);
        assert!(all.iter().all(|m| m.match_indices.is_empty()));
        let recent = store.filter("gt", SortMode::Recency);
        assert_eq!(recent.iter().map(|m| m.item_index).collect::<Vec<_>>(), [0, 2]);
        assert!(store.filter("zzz", SortMode::Relevance).is_empty());
    }

    #[test]
    fn parse_zsh_history_format() {
        let content = b": 1729835400:0;echo first\n: 1729835405:0;git commit -m \\\n  \"multiline commit\"\n: 1729835410:0;ls -la\n";
        let temp = std::env::temp_dir().join("litty_test_zsh_hist");
        std::fs::write(&temp, content).expect("write temp hist");
        let mut out = Vec::new();
        parse_history_file(&temp, &mut out);
        let _ = std::fs::remove_file(&temp);

        assert_eq!(out.len(), 3);
        assert_eq!(out[0], "ls -la");
        assert_eq!(out[1], "git commit -m \n  \"multiline commit\"");
        assert_eq!(out[2], "echo first");
    }

    #[test]
    fn parse_bash_history_format() {
        let content = b"#1729835400\ncargo check\n#1729835405\ngit status\n";
        let temp = std::env::temp_dir().join("litty_test_bash_hist");
        std::fs::write(&temp, content).expect("write temp hist");
        let mut out = Vec::new();
        parse_history_file(&temp, &mut out);
        let _ = std::fs::remove_file(&temp);

        assert_eq!(out.len(), 2);
        assert_eq!(out[0], "git status");
        assert_eq!(out[1], "cargo check");
    }

    fn temp_history(name: &str, content: &[u8]) -> PathBuf {
        static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("litty_hist_{}_{n}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("create temp dir");
        let path = dir.join(name);
        std::fs::write(&path, content).expect("write temp hist");
        path
    }

    #[test]
    fn zsh_metafied_entries_decode_and_survive_removal() {
        // "ăn" = C4 83 6E, stored metafied by zsh as C4 83 A3 6E.
        let content = b": 1:0;echo \xC4\x83\xA3n\n: 2:0;git status\n: 3:0;ls\n";
        let path = temp_history(".zsh_history", content);

        let mut out = Vec::new();
        parse_history_file(&path, &mut out);
        assert_eq!(out, vec!["ls", "git status", "echo ăn"]);

        assert!(remove_command_from_file(&path, "git status").expect("remove"));
        assert_eq!(std::fs::read(&path).unwrap(), b": 1:0;echo \xC4\x83\xA3n\n: 3:0;ls\n");

        assert!(remove_command_from_file(&path, "echo ăn").expect("remove unicode"));
        assert_eq!(std::fs::read(&path).unwrap(), b": 3:0;ls\n");
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn removal_drops_whole_multiline_entry() {
        let content = b": 1:0;git commit -m \\\n  \"multi\"\n: 2:0;ls\n";
        let path = temp_history(".zsh_history", content);
        assert!(remove_command_from_file(&path, "git commit -m \n  \"multi\"").expect("remove"));
        assert_eq!(std::fs::read(&path).unwrap(), b": 2:0;ls\n");
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn removal_drops_bash_timestamp_and_keeps_permissions() {
        use std::os::unix::fs::PermissionsExt;
        let path = temp_history(".bash_history", b"#100\ncargo check\n#200\ngit status\n");
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
        assert!(remove_command_from_file(&path, "cargo check").expect("remove"));
        assert_eq!(std::fs::read(&path).unwrap(), b"#200\ngit status\n");
        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
        assert!(!remove_command_from_file(&path, "not there").expect("no-op"));
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn fish_escaped_backslash_before_n_stays_a_backslash() {
        // On disk: `printf 'a\\nb'` (escaped backslash, then `n`) and `echo x\ny` (a newline).
        let content = b"- cmd: printf 'a\\\\nb'\n  when: 1\n- cmd: echo x\\ny\n  when: 2\n";
        let path = temp_history("fish_history", content);
        let mut out = Vec::new();
        parse_history_file(&path, &mut out);
        assert_eq!(out, vec!["echo x\ny", "printf 'a\\nb'"]);
        assert!(remove_command_from_file(&path, "printf 'a\\nb'").expect("remove"));
        assert_eq!(std::fs::read(&path).unwrap(), b"- cmd: echo x\\ny\n  when: 2\n");
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn removal_drops_fish_block() {
        let content = b"- cmd: ls\n  when: 1\n- cmd: echo a\\nb\n  when: 2\n  paths:\n    - b\n";
        let path = temp_history("fish_history", content);
        let mut out = Vec::new();
        parse_history_file(&path, &mut out);
        assert_eq!(out, vec!["echo a\nb", "ls"]);
        assert!(remove_command_from_file(&path, "echo a\nb").expect("remove"));
        assert_eq!(std::fs::read(&path).unwrap(), b"- cmd: ls\n  when: 1\n");
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn reload_if_changed_with_session_command() {
        let path = temp_history(".bash_history", b"disk command\n");
        let files = vec![path.clone()];
        let (wake_tx, wake_rx) = mpsc::sync_channel(1);
        let mut store = HistoryStore::new();
        store.worker = Some(HistoryWorker::start_with(Arc::new(move || { wake_tx.send(()).unwrap(); }), move || files.clone(), Vec::new).unwrap());
        store.reload_if_changed();
        store.add_session_command("prompt_command_before_open");
        wake_rx.recv().unwrap();
        assert!(store.poll());
        assert_eq!(store.items, ["prompt_command_before_open", "disk command"]);
        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }

    #[test]
    fn add_session_command_caps_items() {
        let mut store = HistoryStore::new();
        for i in 0..(MAX_HISTORY_ITEMS + 50) {
            store.add_session_command(&format!("cmd_{i}"));
        }
        assert_eq!(store.items.len(), MAX_HISTORY_ITEMS);
        assert_eq!(store.items[0], format!("cmd_{}", MAX_HISTORY_ITEMS + 49));
    }

    #[test]
    fn removal_deletes_every_occurrence_with_its_timestamp() {
        let content = b"#1\necho repeated\necho first\n#2\necho repeated\necho middle\necho repeated\n";
        let path = temp_history(".bash_history", content);
        assert!(remove_command_from_file(&path, "echo repeated").expect("remove"));
        assert_eq!(std::fs::read(&path).unwrap(), b"echo first\necho middle\n");
        assert!(!remove_command_from_file(&path, "echo repeated").expect("no-op"), "nothing left to remove");
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }
}
