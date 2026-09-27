//! Parallel filesystem scanner.
//!
//! Recursion is parallelized with rayon over subdirectories. Progress is
//! reported through atomics; errors are counted and sampled, never fatal.

use crate::tree::{NodeKind, ScanError, ScanNode};
use rayon::prelude::*;
use std::cell::RefCell;
use std::collections::HashSet;
use std::ffi::{OsStr, OsString};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, UNIX_EPOCH};

/// Scanner options.
pub struct ScanOptions {
    pub hidden: bool,
    pub apparent_size: bool,
    pub follow_symlinks: bool,
    pub one_file_system: bool,
    pub count_links: bool,
    pub excludes: ExcludeMatcher,
    pub threads: Option<usize>,
    pub cancel: Arc<AtomicBool>,
    pub progress: Arc<ScanProgress>,
}

impl Default for ScanOptions {
    fn default() -> Self {
        Self {
            hidden: true,
            apparent_size: false,
            follow_symlinks: false,
            one_file_system: false,
            count_links: false,
            excludes: ExcludeMatcher::empty(),
            threads: None,
            cancel: Arc::new(AtomicBool::new(false)),
            progress: Arc::new(ScanProgress::default()),
        }
    }
}

/// Live scan counters, polled by the UI and the CLI progress line.
#[derive(Debug, Default)]
pub struct ScanProgress {
    pub entries: AtomicU64,
    pub bytes: AtomicU64,
    pub dirs: AtomicU64,
    pub errors: AtomicU64,
    pub done: AtomicBool,
}

impl ScanProgress {
    pub fn snapshot(&self) -> ProgressSnapshot {
        ProgressSnapshot {
            entries: self.entries.load(Ordering::Relaxed),
            bytes: self.bytes.load(Ordering::Relaxed),
            dirs: self.dirs.load(Ordering::Relaxed),
            errors: self.errors.load(Ordering::Relaxed),
            done: self.done.load(Ordering::Relaxed),
        }
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct ProgressSnapshot {
    pub entries: u64,
    pub bytes: u64,
    pub dirs: u64,
    pub errors: u64,
    pub done: bool,
}

/// Result of one scan.
pub struct ScanOutcome {
    pub root: ScanNode,
    pub errors: Vec<ScanError>,
    pub error_count: u64,
    pub duration: Duration,
}

/// Plain-data scan specification, used by the CLI and the TUI rescan key.
#[derive(Clone, Debug)]
pub struct ScanSpec {
    pub paths: Vec<PathBuf>,
    pub hidden: bool,
    pub apparent_size: bool,
    pub follow_symlinks: bool,
    pub one_file_system: bool,
    pub count_links: bool,
    pub excludes: Vec<String>,
    pub threads: Option<usize>,
}

impl Default for ScanSpec {
    fn default() -> Self {
        Self {
            paths: vec![PathBuf::from(".")],
            hidden: true,
            apparent_size: false,
            follow_symlinks: false,
            one_file_system: false,
            count_links: false,
            excludes: Vec::new(),
            threads: None,
        }
    }
}

impl ScanSpec {
    /// Build fresh scan options (new cancellation flag and progress counters).
    pub fn options(
        &self,
        progress: Arc<ScanProgress>,
        cancel: Arc<AtomicBool>,
    ) -> Result<ScanOptions, String> {
        Ok(ScanOptions {
            hidden: self.hidden,
            apparent_size: self.apparent_size,
            follow_symlinks: self.follow_symlinks,
            one_file_system: self.one_file_system,
            count_links: self.count_links,
            excludes: ExcludeMatcher::new(&self.excludes)?,
            threads: self.threads,
            cancel,
            progress,
        })
    }
}

/// Scan every path in the spec, wrapping multiple roots in a synthetic root.
pub fn scan_all(
    spec: &ScanSpec,
    progress: Arc<ScanProgress>,
    cancel: Arc<AtomicBool>,
) -> Result<ScanOutcome, String> {
    let opts = spec.options(progress, cancel)?;
    if spec.paths.len() == 1 {
        return Ok(scan(&spec.paths[0], &opts));
    }
    let mut root = ScanNode::new(OsString::from("(total)"), NodeKind::Dir);
    let mut errors = Vec::new();
    let mut error_count = 0u64;
    let mut duration = Duration::ZERO;
    for path in &spec.paths {
        let outcome = scan(path, &opts);
        root.size += outcome.root.size;
        root.files += outcome.root.files;
        root.dirs += outcome.root.dirs;
        root.mtime = root.mtime.max(outcome.root.mtime);
        root.children.push(outcome.root);
        errors.extend(outcome.errors);
        error_count += outcome.error_count;
        duration += outcome.duration;
    }
    root.finish();
    Ok(ScanOutcome {
        root,
        errors,
        error_count,
        duration,
    })
}

/// Scan `root` and return the tree. Cancellable through `opts.cancel`.
pub fn scan(root: &Path, opts: &ScanOptions) -> ScanOutcome {
    let start = Instant::now();
    let ctx = Ctx::new(opts, root);
    let root_name: OsString = root.as_os_str().to_os_string();
    let pool = opts
        .threads
        .and_then(|n| rayon::ThreadPoolBuilder::new().num_threads(n).build().ok());
    let node = match fs::symlink_metadata(root) {
        Ok(m) if m.is_dir() => match &pool {
            Some(p) => p.install(|| scan_dir(root, root_name, &ctx)),
            None => scan_dir(root, root_name, &ctx),
        },
        Ok(m) => {
            let kind = if m.is_symlink() {
                NodeKind::Symlink
            } else if m.is_file() {
                NodeKind::File
            } else {
                NodeKind::Other
            };
            ScanNode::entry(
                root_name,
                kind,
                size_of(&m, opts.apparent_size),
                mtime_of(&m),
            )
        }
        Err(e) => {
            ctx.record_error(root, &e);
            let mut n = ScanNode::new(root_name, NodeKind::Dir);
            n.error = true;
            n.finish();
            n
        }
    };
    flush_progress(&opts.progress);
    opts.progress.done.store(true, Ordering::Relaxed);
    ScanOutcome {
        root: node,
        errors: ctx.take_errors(),
        error_count: ctx.error_count.load(Ordering::Relaxed),
        duration: start.elapsed(),
    }
}

/// Glob-based exclusion of entries by name or path.
#[derive(Clone, Debug, Default)]
pub struct ExcludeMatcher {
    set: Option<globset::GlobSet>,
}

impl ExcludeMatcher {
    pub fn empty() -> Self {
        Self { set: None }
    }

    pub fn new(patterns: &[String]) -> Result<Self, String> {
        if patterns.is_empty() {
            return Ok(Self::empty());
        }
        let mut builder = globset::GlobSetBuilder::new();
        for p in patterns {
            builder.add(
                globset::Glob::new(p).map_err(|e| format!("invalid exclude pattern {p:?}: {e}"))?,
            );
        }
        let set = builder
            .build()
            .map_err(|e| format!("invalid exclude patterns: {e}"))?;
        Ok(Self { set: Some(set) })
    }

    pub fn is_empty(&self) -> bool {
        self.set.is_none()
    }

    pub fn matches(&self, path: &Path, name: &OsStr) -> bool {
        let Some(set) = &self.set else {
            return false;
        };
        if set.is_match(name) {
            return true;
        }
        if set.is_match(path) {
            return true;
        }
        if cfg!(windows) {
            let s = path.to_string_lossy().replace('\\', "/");
            if set.is_match(s.as_str()) {
                return true;
            }
        }
        false
    }
}

struct Ctx<'a> {
    opts: &'a ScanOptions,
    errors: Mutex<Vec<ScanError>>,
    error_count: AtomicU64,
    hardlinks: Mutex<HashSet<(u64, u64)>>,
    seen_dirs: Mutex<HashSet<(u64, u64)>>,
    root_dev: Option<u64>,
}

impl<'a> Ctx<'a> {
    fn new(opts: &'a ScanOptions, root: &Path) -> Self {
        let root_dev = fs::symlink_metadata(root).ok().and_then(|m| dev_of(&m));
        Self {
            opts,
            errors: Mutex::new(Vec::new()),
            error_count: AtomicU64::new(0),
            hardlinks: Mutex::new(HashSet::new()),
            seen_dirs: Mutex::new(HashSet::new()),
            root_dev,
        }
    }

    fn cancelled(&self) -> bool {
        self.opts.cancel.load(Ordering::Relaxed)
    }

    fn record_error(&self, path: &Path, e: &std::io::Error) {
        self.error_count.fetch_add(1, Ordering::Relaxed);
        self.opts.progress.errors.fetch_add(1, Ordering::Relaxed);
        let mut errs = self.errors.lock().unwrap();
        if errs.len() < 50 {
            errs.push(ScanError {
                path: path.to_path_buf(),
                message: e.to_string(),
            });
        }
    }

    fn take_errors(&self) -> Vec<ScanError> {
        std::mem::take(&mut *self.errors.lock().unwrap())
    }

    /// Returns true when this file was already counted (duplicate hardlink).
    fn note_hardlink(&self, m: &fs::Metadata) -> bool {
        let Some(key) = link_key(m) else {
            return false;
        };
        let mut set = self.hardlinks.lock().unwrap();
        !set.insert(key)
    }

    /// Returns true when this directory has not been visited yet.
    fn note_dir(&self, m: &fs::Metadata) -> bool {
        let Some(key) = link_key(m) else {
            return true;
        };
        let mut set = self.seen_dirs.lock().unwrap();
        set.insert(key)
    }
}

thread_local! {
    static PENDING: RefCell<(u64, u64)> = const { RefCell::new((0, 0)) };
}

fn progress_add(prog: &ScanProgress, entries: u64, bytes: u64) {
    PENDING.with(|p| {
        let mut p = p.borrow_mut();
        p.0 += entries;
        p.1 += bytes;
        if p.0 >= 512 {
            flush_pending(prog, &mut p);
        }
    });
}

fn flush_pending(prog: &ScanProgress, p: &mut (u64, u64)) {
    if p.0 > 0 {
        prog.entries.fetch_add(p.0, Ordering::Relaxed);
        p.0 = 0;
    }
    if p.1 > 0 {
        prog.bytes.fetch_add(p.1, Ordering::Relaxed);
        p.1 = 0;
    }
}

fn flush_progress(prog: &ScanProgress) {
    PENDING.with(|p| flush_pending(prog, &mut p.borrow_mut()));
}

fn scan_dir(path: &Path, name: OsString, ctx: &Ctx) -> ScanNode {
    let opts = ctx.opts;
    let prog = &opts.progress;
    let mut node = ScanNode::new(name, NodeKind::Dir);
    if let Ok(m) = fs::symlink_metadata(path) {
        node.own_size = size_of(&m, opts.apparent_size);
        node.mtime = mtime_of(&m);
    }
    if ctx.cancelled() {
        node.finish();
        return node;
    }
    let rd = match fs::read_dir(path) {
        Ok(rd) => rd,
        Err(e) => {
            ctx.record_error(path, &e);
            node.error = true;
            node.finish();
            return node;
        }
    };
    prog.dirs.fetch_add(1, Ordering::Relaxed);

    let mut children: Vec<ScanNode> = Vec::new();
    let mut subdirs: Vec<(PathBuf, OsString)> = Vec::new();
    for (i, entry) in rd.enumerate() {
        if i & 0xff == 0 && ctx.cancelled() {
            break;
        }
        let entry = match entry {
            Ok(e) => e,
            Err(e) => {
                ctx.record_error(path, &e);
                continue;
            }
        };
        let fname = entry.file_name();
        if !opts.hidden && is_hidden(&fname, &entry) {
            continue;
        }
        if !opts.excludes.is_empty() {
            let ep = entry.path();
            if opts.excludes.matches(&ep, &fname) {
                continue;
            }
        }
        let ftype = match entry.file_type() {
            Ok(t) => t,
            Err(e) => {
                ctx.record_error(&entry.path(), &e);
                continue;
            }
        };
        if ftype.is_dir() {
            if opts.one_file_system
                && let (Some(root_dev), Ok(m)) = (ctx.root_dev, entry.metadata())
                && dev_of(&m) != Some(root_dev)
            {
                continue;
            }
            progress_add(prog, 1, 0);
            subdirs.push((entry.path(), fname));
        } else if ftype.is_symlink() {
            progress_add(prog, 1, 0);
            if let Some(child) = symlink_child(&entry, fname, ctx, &mut subdirs) {
                children.push(child);
            }
        } else {
            match entry.metadata() {
                Ok(m) => {
                    if !opts.count_links && ctx.note_hardlink(&m) {
                        continue;
                    }
                    let size = size_of(&m, opts.apparent_size);
                    let mtime = mtime_of(&m);
                    children.push(ScanNode::entry(fname, NodeKind::File, size, mtime));
                    progress_add(prog, 1, size);
                }
                Err(e) => ctx.record_error(&entry.path(), &e),
            }
        }
    }
    flush_progress(prog);

    if !subdirs.is_empty() {
        let dir_nodes: Vec<ScanNode> = subdirs
            .into_par_iter()
            .map(|(p, n)| scan_dir(&p, n, ctx))
            .collect();
        children.extend(dir_nodes);
    }
    for child in children {
        node.absorb(child);
    }
    node.finish();
    node
}

/// Handle a symlink entry: returns a leaf node when it should be counted as
/// one, or pushes a followed directory into `subdirs`.
fn symlink_child(
    entry: &fs::DirEntry,
    fname: OsString,
    ctx: &Ctx,
    subdirs: &mut Vec<(PathBuf, OsString)>,
) -> Option<ScanNode> {
    let opts = ctx.opts;
    if opts.follow_symlinks {
        match fs::metadata(entry.path()) {
            Ok(m) if m.is_dir() => {
                if ctx.note_dir(&m) {
                    subdirs.push((entry.path(), fname));
                }
                None
            }
            Ok(m) => Some(ScanNode::entry(
                fname,
                NodeKind::Symlink,
                size_of(&m, opts.apparent_size),
                mtime_of(&m),
            )),
            Err(e) => {
                ctx.record_error(&entry.path(), &e);
                entry.metadata().ok().map(|lm| {
                    ScanNode::entry(
                        fname,
                        NodeKind::Symlink,
                        size_of(&lm, opts.apparent_size),
                        mtime_of(&lm),
                    )
                })
            }
        }
    } else {
        match entry.metadata() {
            Ok(m) => Some(ScanNode::entry(
                fname,
                NodeKind::Symlink,
                size_of(&m, opts.apparent_size),
                mtime_of(&m),
            )),
            Err(e) => {
                ctx.record_error(&entry.path(), &e);
                None
            }
        }
    }
}

/// Size of an entry: allocated blocks by default, logical bytes when
/// `apparent` is set. Windows has no allocated-block API in `std::fs`, so it
/// always reports logical size.
pub fn size_of(m: &fs::Metadata, apparent: bool) -> u64 {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if apparent { m.len() } else { m.blocks() * 512 }
    }
    #[cfg(not(unix))]
    {
        let _ = apparent;
        m.len()
    }
}

pub fn mtime_of(m: &fs::Metadata) -> u32 {
    m.modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_secs().min(u32::MAX as u64) as u32)
        .unwrap_or(0)
}

fn is_hidden(name: &OsStr, entry: &fs::DirEntry) -> bool {
    if name.as_encoded_bytes().first() == Some(&b'.') {
        return true;
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if let Ok(m) = entry.metadata()
            && m.file_attributes() & 0x2 != 0
        {
            return true;
        }
    }
    #[cfg(not(windows))]
    {
        let _ = entry;
    }
    false
}

#[cfg(unix)]
fn dev_of(m: &fs::Metadata) -> Option<u64> {
    use std::os::unix::fs::MetadataExt;
    Some(m.dev())
}

#[cfg(not(unix))]
fn dev_of(_m: &fs::Metadata) -> Option<u64> {
    None
}

#[cfg(unix)]
fn link_key(m: &fs::Metadata) -> Option<(u64, u64)> {
    use std::os::unix::fs::MetadataExt;
    if m.nlink() <= 1 {
        return None;
    }
    Some((m.dev(), m.ino()))
}

#[cfg(not(unix))]
fn link_key(_m: &fs::Metadata) -> Option<(u64, u64)> {
    // Hardlink identity is not available on stable `std` for Windows; v1 does
    // not deduplicate there (`--count-links` is a no-op).
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::color::CategoryMap;
    use crate::tree::Tree;
    use std::io::Write;

    fn write_file(path: &Path, bytes: usize) {
        let mut f = fs::File::create(path).unwrap();
        f.write_all(&vec![b'x'; bytes]).unwrap();
    }

    fn scan_tree(root: &Path, opts: &ScanOptions) -> Tree {
        let outcome = scan(root, opts);
        Tree::from_scan(
            outcome.root,
            &CategoryMap::builtin(),
            outcome.errors,
            outcome.error_count,
            outcome.duration,
        )
    }

    fn find_child(
        t: &Tree,
        parent: crate::tree::NodeId,
        name: &str,
    ) -> Option<crate::tree::NodeId> {
        t.children_of(parent)
            .iter()
            .copied()
            .find(|&id| t.name(id) == name)
    }

    #[test]
    fn counts_files_and_sizes() {
        let tmp = tempfile::tempdir().unwrap();
        write_file(&tmp.path().join("a.bin"), 100);
        write_file(&tmp.path().join("b.bin"), 250);
        fs::create_dir(tmp.path().join("sub")).unwrap();
        write_file(&tmp.path().join("sub/c.bin"), 500);

        let opts = ScanOptions {
            apparent_size: true,
            ..Default::default()
        };
        let t = scan_tree(tmp.path(), &opts);

        assert_eq!(t.node(t.root()).files, 3);
        assert_eq!(t.node(t.root()).dirs, 1);
        // File sizes are exact under apparent_size.
        let a = find_child(&t, t.root(), "a.bin").unwrap();
        let b = find_child(&t, t.root(), "b.bin").unwrap();
        assert_eq!(t.node(a).size, 100);
        assert_eq!(t.node(b).size, 250);
        let sub = find_child(&t, t.root(), "sub").unwrap();
        assert_eq!(t.node(sub).size, 500);
        assert!(t.node(t.root()).size >= 850);
        assert_eq!(t.error_count, 0);
    }

    #[test]
    fn hidden_filter() {
        let tmp = tempfile::tempdir().unwrap();
        write_file(&tmp.path().join(".hidden"), 10);
        write_file(&tmp.path().join("visible"), 20);

        let opts = ScanOptions {
            hidden: false,
            apparent_size: true,
            ..Default::default()
        };
        let t = scan_tree(tmp.path(), &opts);
        assert_eq!(t.node(t.root()).files, 1);
        assert!(find_child(&t, t.root(), "visible").is_some());
        assert!(find_child(&t, t.root(), ".hidden").is_none());

        let opts = ScanOptions {
            apparent_size: true,
            ..Default::default()
        };
        let t = scan_tree(tmp.path(), &opts);
        assert_eq!(t.node(t.root()).files, 2);
    }

    #[test]
    fn excludes_by_name_and_glob() {
        let tmp = tempfile::tempdir().unwrap();
        write_file(&tmp.path().join("keep.txt"), 10);
        write_file(&tmp.path().join("skip.log"), 10);
        fs::create_dir(tmp.path().join("skipdir")).unwrap();
        write_file(&tmp.path().join("skipdir/inner.txt"), 10);

        let opts = ScanOptions {
            apparent_size: true,
            excludes: ExcludeMatcher::new(&["*.log".into(), "skipdir".into()]).unwrap(),
            ..Default::default()
        };
        let t = scan_tree(tmp.path(), &opts);
        assert_eq!(t.node(t.root()).files, 1);
        assert!(find_child(&t, t.root(), "keep.txt").is_some());
        assert!(find_child(&t, t.root(), "skipdir").is_none());
    }

    #[cfg(unix)]
    #[test]
    fn counts_symlinks_without_following() {
        let tmp = tempfile::tempdir().unwrap();
        write_file(&tmp.path().join("target.bin"), 400);
        std::os::unix::fs::symlink(tmp.path().join("target.bin"), tmp.path().join("link")).unwrap();

        let mut opts = ScanOptions::default();
        opts.apparent_size = true;
        let t = scan_tree(tmp.path(), &opts);
        assert_eq!(t.node(t.root()).files, 2);
        let link = find_child(&t, t.root(), "link").unwrap();
        assert_eq!(t.node(link).kind, NodeKind::Symlink);
        assert!(t.node(link).size < 400);
    }

    #[cfg(unix)]
    #[test]
    fn deduplicates_hardlinks() {
        let tmp = tempfile::tempdir().unwrap();
        write_file(&tmp.path().join("original.bin"), 300);
        fs::hard_link(tmp.path().join("original.bin"), tmp.path().join("copy.bin")).unwrap();

        let mut opts = ScanOptions::default();
        opts.apparent_size = true;
        let t = scan_tree(tmp.path(), &opts);
        assert_eq!(t.node(t.root()).files, 1);

        let mut opts = ScanOptions::default();
        opts.apparent_size = true;
        opts.count_links = true;
        let t = scan_tree(tmp.path(), &opts);
        assert_eq!(t.node(t.root()).files, 2);
    }

    #[cfg(unix)]
    #[test]
    fn reports_permission_errors() {
        use std::os::unix::fs::PermissionsExt;
        if unsafe { libc_geteuid() } == 0 {
            return; // root can read anything
        }
        let tmp = tempfile::tempdir().unwrap();
        let locked = tmp.path().join("locked");
        fs::create_dir(&locked).unwrap();
        write_file(&locked.join("secret.bin"), 10);
        fs::set_permissions(&locked, fs::Permissions::from_mode(0o000)).unwrap();

        let opts = ScanOptions::default();
        let t = scan_tree(tmp.path(), &opts);
        assert_eq!(t.error_count, 1);
        assert_eq!(t.errors.len(), 1);
        assert_eq!(t.node(t.root()).files, 0);

        fs::set_permissions(&locked, fs::Permissions::from_mode(0o755)).unwrap();
    }

    #[cfg(unix)]
    unsafe fn libc_geteuid() -> u32 {
        unsafe extern "C" {
            fn geteuid() -> u32;
        }
        unsafe { geteuid() }
    }

    #[test]
    fn missing_root_records_error() {
        let tmp = tempfile::tempdir().unwrap();
        let missing = tmp.path().join("does-not-exist");
        let opts = ScanOptions::default();
        let t = scan_tree(&missing, &opts);
        assert_eq!(t.error_count, 1);
    }
}
