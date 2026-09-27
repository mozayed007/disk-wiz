//! Tree model: scan output and the compact arena tree used for rendering.

use crate::color::{CategoryMap, Rgb};
use std::borrow::Cow;
use std::collections::VecDeque;
use std::ffi::{OsStr, OsString};
use std::path::PathBuf;
use std::time::Duration;

pub type NodeId = u32;
pub const NO_PARENT: u32 = u32::MAX;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum NodeKind {
    Dir,
    File,
    Symlink,
    Other,
}

impl NodeKind {
    pub fn as_str(self) -> &'static str {
        match self {
            NodeKind::Dir => "dir",
            NodeKind::File => "file",
            NodeKind::Symlink => "symlink",
            NodeKind::Other => "other",
        }
    }
}

/// Which metric the UI sorts and colors by.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum SizeMode {
    #[default]
    Size,
    Files,
    Age,
}

impl SizeMode {
    pub fn label(self) -> &'static str {
        match self {
            SizeMode::Size => "Size",
            SizeMode::Files => "Files",
            SizeMode::Age => "Age",
        }
    }

    pub fn next(self) -> SizeMode {
        match self {
            SizeMode::Size => SizeMode::Files,
            SizeMode::Files => SizeMode::Age,
            SizeMode::Age => SizeMode::Size,
        }
    }

    pub fn sort_key(self) -> SortKey {
        match self {
            SizeMode::Size => SortKey::Size,
            SizeMode::Files => SortKey::Files,
            SizeMode::Age => SortKey::Mtime,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum SortKey {
    #[default]
    Size,
    Files,
    Mtime,
    Name,
}

impl SortKey {
    pub fn parse(s: &str) -> Option<SortKey> {
        Some(match s.trim().to_ascii_lowercase().as_str() {
            "size" => SortKey::Size,
            "files" => SortKey::Files,
            "mtime" | "age" | "time" => SortKey::Mtime,
            "name" => SortKey::Name,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone)]
pub struct ScanError {
    pub path: PathBuf,
    pub message: String,
}

/// A node produced by the scanner, before flattening.
#[derive(Debug)]
pub struct ScanNode {
    pub name: OsString,
    pub kind: NodeKind,
    /// Size of this entry itself (dir metadata, file bytes, link bytes).
    pub own_size: u64,
    /// Total size including children.
    pub size: u64,
    /// Aggregated count of non-directory entries.
    pub files: u32,
    /// Aggregated count of directories below this node.
    pub dirs: u32,
    /// Newest mtime in the subtree (epoch seconds).
    pub mtime: u32,
    /// True when the directory could not be read.
    pub error: bool,
    pub children: Vec<ScanNode>,
}

impl ScanNode {
    pub fn new(name: OsString, kind: NodeKind) -> Self {
        Self {
            name,
            kind,
            own_size: 0,
            size: 0,
            files: 0,
            dirs: 0,
            mtime: 0,
            error: false,
            children: Vec::new(),
        }
    }

    /// A leaf entry (file, symlink, or other).
    pub fn entry(name: OsString, kind: NodeKind, size: u64, mtime: u32) -> Self {
        Self {
            name,
            kind,
            own_size: size,
            size,
            files: 1,
            dirs: 0,
            mtime,
            error: false,
            children: Vec::new(),
        }
    }

    /// Add this node's own size into its total (called after children).
    pub fn finish(&mut self) {
        self.size += self.own_size;
    }

    /// Merge a finished child into this directory node.
    pub fn absorb(&mut self, child: ScanNode) {
        self.size += child.size;
        self.files += child.files;
        if child.kind == NodeKind::Dir {
            self.dirs += child.dirs + 1;
        }
        self.mtime = self.mtime.max(child.mtime);
        self.children.push(child);
    }
}

/// A flattened node in the arena tree.
#[derive(Debug, Clone, Copy)]
pub struct Node {
    pub name_start: u32,
    pub name_len: u32,
    pub size: u64,
    pub files: u32,
    pub dirs: u32,
    pub mtime: u32,
    pub kind: NodeKind,
    pub category: u16,
    pub parent: u32,
    pub child_start: u32,
    pub child_count: u32,
}

/// Compact arena tree: nodes in BFS order, names in a byte arena, child
/// lists in a CSR-style index.
#[derive(Debug)]
pub struct Tree {
    nodes: Vec<Node>,
    names: Vec<u8>,
    children: Vec<NodeId>,
    pub category_ids: Vec<String>,
    pub category_colors: Vec<Rgb>,
    /// Total bytes owned per category (each byte counted once).
    pub category_totals: Vec<u64>,
    /// True when the root is a synthetic wrapper around multiple scan paths.
    pub synthetic_root: bool,
    pub errors: Vec<ScanError>,
    pub error_count: u64,
    pub scan_duration: Duration,
}

impl Tree {
    /// Flatten a scanned tree, assigning categories.
    pub fn from_scan(
        root: ScanNode,
        cats: &CategoryMap,
        errors: Vec<ScanError>,
        error_count: u64,
        scan_duration: Duration,
    ) -> Tree {
        Self::from_scan_with(root, cats, errors, error_count, scan_duration, false)
    }

    /// Flatten a scanned tree; `synthetic_root` marks a wrapper around
    /// multiple scan paths, whose own name is skipped when building paths.
    pub fn from_scan_with(
        root: ScanNode,
        cats: &CategoryMap,
        errors: Vec<ScanError>,
        error_count: u64,
        scan_duration: Duration,
        synthetic_root: bool,
    ) -> Tree {
        let mut nodes: Vec<Node> = Vec::new();
        let mut names: Vec<u8> = Vec::new();
        let mut children: Vec<NodeId> = Vec::new();
        let mut category_totals = vec![0u64; cats.categories.len()];
        let mut queue: VecDeque<(ScanNode, u32, Option<u16>, NodeId)> = VecDeque::new();
        queue.push_back((root, NO_PARENT, None, 0));
        let mut next_id: NodeId = 1;

        while let Some((sn, parent, inherited, id)) = queue.pop_front() {
            debug_assert_eq!(id as usize, nodes.len());
            let classified = cats.classify(&sn.name.to_string_lossy(), sn.kind, inherited);
            if let Some(slot) = category_totals.get_mut(classified.category as usize) {
                *slot += sn.own_size;
            }
            let name_start = names.len() as u32;
            names.extend_from_slice(sn.name.as_encoded_bytes());
            let name_len = names.len() as u32 - name_start;
            let child_start = children.len() as u32;
            let child_count = sn.children.len() as u32;
            for child in sn.children {
                let cid = next_id;
                next_id += 1;
                children.push(cid);
                queue.push_back((child, id, classified.propagate, cid));
            }
            nodes.push(Node {
                name_start,
                name_len,
                size: sn.size,
                files: sn.files,
                dirs: sn.dirs,
                mtime: sn.mtime,
                kind: sn.kind,
                category: classified.category,
                parent,
                child_start,
                child_count,
            });
        }

        Tree {
            nodes,
            names,
            children,
            category_ids: cats.categories.iter().map(|c| c.id.clone()).collect(),
            category_colors: cats.categories.iter().map(|c| c.rgb).collect(),
            category_totals,
            synthetic_root,
            errors,
            error_count,
            scan_duration,
        }
    }

    pub fn root(&self) -> NodeId {
        0
    }

    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    pub fn node(&self, id: NodeId) -> &Node {
        &self.nodes[id as usize]
    }

    pub fn name(&self, id: NodeId) -> Cow<'_, str> {
        let n = &self.nodes[id as usize];
        let bytes = &self.names[n.name_start as usize..(n.name_start + n.name_len) as usize];
        String::from_utf8_lossy(bytes)
    }

    /// The raw OS name (used for file operations).
    pub fn name_os(&self, id: NodeId) -> &OsStr {
        let n = &self.nodes[id as usize];
        let bytes = &self.names[n.name_start as usize..(n.name_start + n.name_len) as usize];
        // SAFETY: the bytes originate from `OsStr::as_encoded_bytes`.
        unsafe { OsStr::from_encoded_bytes_unchecked(bytes) }
    }

    pub fn is_dir(&self, id: NodeId) -> bool {
        self.nodes[id as usize].kind == NodeKind::Dir
    }

    pub fn children_of(&self, id: NodeId) -> &[NodeId] {
        let n = &self.nodes[id as usize];
        let start = n.child_start as usize;
        &self.children[start..start + n.child_count as usize]
    }

    /// Full display path for a node, built from the root name downwards.
    pub fn path_of(&self, id: NodeId) -> PathBuf {
        let mut parts: Vec<&OsStr> = Vec::new();
        let mut cur = id;
        loop {
            if !(self.synthetic_root && cur == self.root()) {
                parts.push(self.name_os(cur));
            }
            let parent = self.nodes[cur as usize].parent;
            if parent == NO_PARENT {
                break;
            }
            cur = parent;
        }
        let mut path = PathBuf::new();
        for part in parts.iter().rev() {
            path.push(part);
        }
        path
    }

    /// Depth of a node relative to the root (root is 0).
    pub fn depth_of(&self, id: NodeId) -> usize {
        let mut d = 0;
        let mut cur = id;
        while self.nodes[cur as usize].parent != NO_PARENT {
            cur = self.nodes[cur as usize].parent;
            d += 1;
        }
        d
    }

    /// The value used to sort/scale a node under a given key.
    pub fn value_for(&self, id: NodeId, key: SortKey) -> u64 {
        let n = &self.nodes[id as usize];
        match key {
            SortKey::Size => n.size,
            SortKey::Files => n.files as u64,
            SortKey::Mtime => n.mtime as u64,
            SortKey::Name => 0,
        }
    }

    /// Order two nodes under a sort key: descending for metrics, ascending for
    /// names, with a stable name tiebreak.
    pub fn compare(&self, a: NodeId, b: NodeId, key: SortKey) -> std::cmp::Ordering {
        let ord = match key {
            SortKey::Name => self.name(a).cmp(&self.name(b)),
            k => self.value_for(b, k).cmp(&self.value_for(a, k)),
        };
        ord.then_with(|| self.name(a).cmp(&self.name(b)))
    }

    /// Children of a node sorted for display.
    pub fn sorted_children(&self, id: NodeId, key: SortKey, reverse: bool) -> Vec<NodeId> {
        let mut v: Vec<NodeId> = self.children_of(id).to_vec();
        v.sort_by(|&a, &b| self.compare(a, b, key));
        if reverse {
            v.reverse();
        }
        v
    }

    /// The ancestor chain from the root to `id` (inclusive).
    pub fn ancestors(&self, id: NodeId) -> Vec<NodeId> {
        let mut chain = Vec::new();
        let mut cur = id;
        loop {
            chain.push(cur);
            let parent = self.nodes[cur as usize].parent;
            if parent == NO_PARENT {
                break;
            }
            cur = parent;
        }
        chain.reverse();
        chain
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::color::CategoryMap;

    fn leaf(name: &str, size: u64) -> ScanNode {
        ScanNode::entry(OsString::from(name), NodeKind::File, size, 100)
    }

    fn dir(name: &str, children: Vec<ScanNode>) -> ScanNode {
        let mut d = ScanNode::new(OsString::from(name), NodeKind::Dir);
        for c in children {
            d.absorb(c);
        }
        d.finish();
        d
    }

    fn sample() -> Tree {
        let root = dir(
            "root",
            vec![
                dir("a", vec![leaf("x", 300), leaf("y", 100)]),
                dir("b", vec![leaf("z", 200)]),
            ],
        );
        Tree::from_scan(root, &CategoryMap::builtin(), Vec::new(), 0, Duration::ZERO)
    }

    #[test]
    fn flattens_in_bfs_order_with_children() {
        let t = sample();
        assert_eq!(t.len(), 6);
        assert_eq!(t.name(t.root()), "root");
        let kids: Vec<String> = t
            .children_of(t.root())
            .iter()
            .map(|&id| t.name(id).into_owned())
            .collect();
        assert_eq!(kids, vec!["a", "b"]);
        let a = t.children_of(t.root())[0];
        let a_kids: Vec<String> = t
            .children_of(a)
            .iter()
            .map(|&id| t.name(id).into_owned())
            .collect();
        assert_eq!(a_kids, vec!["x", "y"]);
    }

    #[test]
    fn aggregates_sizes_and_counts() {
        let t = sample();
        assert_eq!(t.node(t.root()).size, 600);
        assert_eq!(t.node(t.root()).files, 3);
        assert_eq!(t.node(t.root()).dirs, 2);
    }

    #[test]
    fn sorts_children_descending() {
        let t = sample();
        let sorted = t.sorted_children(t.root(), SortKey::Size, false);
        assert_eq!(t.name(sorted[0]), "a");
        assert_eq!(t.name(sorted[1]), "b");
        let a = sorted[0];
        let a_sorted = t.sorted_children(a, SortKey::Size, false);
        assert_eq!(t.name(a_sorted[0]), "x");
        let by_name = t.sorted_children(t.root(), SortKey::Name, false);
        assert_eq!(t.name(by_name[0]), "a");
    }

    #[test]
    fn builds_paths() {
        let t = sample();
        let a = t.children_of(t.root())[0];
        let x = t.children_of(a)[0];
        let p = t.path_of(x);
        let s = p.to_string_lossy().replace('\\', "/");
        assert_eq!(s, "root/a/x");
    }

    #[test]
    fn assigns_categories_with_inheritance() {
        let mut root = ScanNode::new(OsString::from("root"), NodeKind::Dir);
        let mut git = ScanNode::new(OsString::from(".git"), NodeKind::Dir);
        git.absorb(leaf("HEAD", 10));
        git.finish();
        root.absorb(git);
        root.absorb(leaf("main.rs", 20));
        root.finish();
        let t = Tree::from_scan(root, &CategoryMap::builtin(), Vec::new(), 0, Duration::ZERO);
        let cats = &t.category_ids;
        let git_id = cats.iter().position(|c| c == "git").unwrap() as u16;
        let code_id = cats.iter().position(|c| c == "code").unwrap() as u16;
        let git_node = t.children_of(t.root())[0];
        assert_eq!(t.node(git_node).category, git_id);
        let head = t.children_of(git_node)[0];
        assert_eq!(t.node(head).category, git_id);
        let rs = t.children_of(t.root())[1];
        assert_eq!(t.node(rs).category, code_id);
    }
}
