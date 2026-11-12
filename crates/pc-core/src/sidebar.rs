//! The sidebar: this computer's folders as a tree that follows the active
//! panel, and the servers by protocol.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::connections::Connection;
use crate::remote::{Address, Scheme};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum SidebarTab {
    #[default]
    Tree,
    Favourites,
    /// Shown while KDE Connect has paired devices.
    Devices,
}

struct Node {
    path: PathBuf,
    name: String,
    expanded: bool,
    /// Its subfolders, once read.
    children: Option<Vec<Node>>,
}

impl Node {
    fn new(path: PathBuf, name: String) -> Self {
        Node { path, name, expanded: false, children: None }
    }

    fn read(&mut self, hidden: bool) {
        self.children = Some(subfolders(&self.path, hidden));
    }

    fn rows<'a>(&'a self, depth: usize, out: &mut Vec<Row<'a>>) {
        let leaf = self.children.as_ref().is_some_and(Vec::is_empty);
        out.push(Row { path: &self.path, name: &self.name, depth, expanded: self.expanded, leaf });
        if let (true, Some(children)) = (self.expanded, &self.children) {
            for child in children {
                child.rows(depth + 1, out);
            }
        }
    }

    fn find(&mut self, path: &Path) -> Option<&mut Node> {
        if self.path == path {
            return Some(self);
        }
        let children = self.children.as_mut()?;
        children.iter_mut().find(|c| path.starts_with(&c.path))?.find(path)
    }

    /// Reads again what is open, so new and removed folders show; closed
    /// ones are read when opened.
    fn refresh(&mut self, hidden: bool) {
        let Some(mut old) = self.children.take() else {
            return;
        };
        let mut fresh = subfolders(&self.path, hidden);
        for node in &mut fresh {
            if let Some(ix) = old.iter().position(|was| was.path == node.path && was.expanded) {
                node.expanded = true;
                node.children = old.swap_remove(ix).children;
                node.refresh(hidden);
            }
        }
        self.children = Some(fresh);
    }
}

/// A shown line of the tree.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Row<'a> {
    pub path: &'a Path,
    pub name: &'a str,
    pub depth: usize,
    pub expanded: bool,
    /// Read, and without subfolders.
    pub leaf: bool,
}

/// The folders of this computer, under the home folder and the file system's roots.
pub struct FolderTree {
    roots: Vec<Node>,
    hidden: bool,
}

impl FolderTree {
    pub fn new(hidden: bool) -> Self {
        let roots = roots().into_iter().map(|(name, path)| Node::new(path, name)).collect();
        FolderTree { roots, hidden }
    }

    pub fn rows(&self) -> Vec<Row<'_>> {
        let mut rows = Vec::new();
        for root in &self.roots {
            root.rows(0, &mut rows);
        }
        rows
    }

    /// Opens or closes a folder, reading its subfolders the first time.
    pub fn toggle(&mut self, path: &Path) {
        let hidden = self.hidden;
        if let Some(node) = self.node(path) {
            if node.children.is_none() {
                node.read(hidden);
            }
            node.expanded = !node.expanded;
        }
    }

    /// Opens the folders down to `path`'s parent under the nearest root;
    /// false when no root holds it.
    pub fn reveal(&mut self, path: &Path) -> bool {
        let hidden = self.hidden;
        let Some(mut node) = self.root_of(path) else {
            return false;
        };
        while node.path != path {
            if node.children.is_none() {
                node.read(hidden);
            }
            node.expanded = true;
            let children = node.children.as_mut().expect("just read");
            let at = match children.iter().position(|c| path.starts_with(&c.path)) {
                Some(at) => at,
                // Hidden, or made since: shown from now on.
                None => {
                    let name = path.strip_prefix(&node.path).ok().and_then(|rest| rest.iter().next());
                    let name = name.map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                    children.push(Node::new(node.path.join(&name), name));
                    children.len() - 1
                }
            };
            node = &mut children[at];
        }
        true
    }

    /// Reads again every open folder; with `hidden`, dot folders show.
    pub fn refresh(&mut self, hidden: bool) {
        self.hidden = hidden;
        for root in &mut self.roots {
            root.refresh(hidden);
        }
    }

    fn root_of(&mut self, path: &Path) -> Option<&mut Node> {
        let nearest = self.roots.iter().enumerate().filter(|(_, root)| path.starts_with(&root.path));
        let ix = nearest.max_by_key(|(_, root)| root.path.components().count())?.0;
        Some(&mut self.roots[ix])
    }

    fn node(&mut self, path: &Path) -> Option<&mut Node> {
        self.root_of(path)?.find(path)
    }
}

/// Home first, then the file system's roots.
fn roots() -> Vec<(String, PathBuf)> {
    let home = std::env::home_dir().map(|home| ("Home".to_owned(), home));
    let mut roots: Vec<_> = home.into_iter().collect();
    #[cfg(windows)]
    for letter in 'C'..='Z' {
        let root = format!("{letter}:\\");
        if Path::new(&root).exists() {
            roots.push((root.clone(), root.into()));
        }
    }
    #[cfg(not(windows))]
    roots.push(("/".to_owned(), PathBuf::from("/")));
    roots
}

/// `path`'s folders by name; none where it cannot be read.
fn subfolders(path: &Path, hidden: bool) -> Vec<Node> {
    let Ok(dir) = std::fs::read_dir(path) else {
        return Vec::new();
    };
    let mut nodes: Vec<Node> = dir
        .flatten()
        .filter(|entry| entry.path().is_dir())
        .map(|entry| Node::new(entry.path(), entry.file_name().to_string_lossy().into_owned()))
        .filter(|node| hidden || !node.name.starts_with('.'))
        .collect();
    nodes.sort_by_cached_key(|node| node.name.to_lowercase());
    nodes
}

/// A server of the Favourites tab.
#[derive(Clone, Debug, PartialEq)]
pub struct Favourite {
    pub name: String,
    pub detail: String,
    /// Its place among the saved; none for a host of `~/.ssh/config`.
    pub saved: Option<usize>,
    pub address: Address,
}

/// The servers by protocol: the saved, then `~/.ssh/config`'s hosts under
/// SFTP. Protocols with none are left out.
pub fn favourites(saved: &[Connection], hosts: &[String]) -> Vec<(Scheme, Vec<Favourite>)> {
    let groups = Scheme::ALL.map(|scheme| {
        let saved = saved.iter().enumerate().filter(|(_, c)| c.scheme == scheme);
        let mut servers: Vec<Favourite> = saved
            .map(|(ix, connection)| Favourite {
                name: connection.name.clone(),
                detail: connection.url().split_once("://").map_or_else(String::new, |(_, rest)| rest.to_owned()),
                saved: Some(ix),
                address: connection.address(),
            })
            .collect();
        if scheme == Scheme::Sftp {
            servers.extend(hosts.iter().map(|host| Favourite {
                name: host.clone(),
                detail: "~/.ssh/config".to_owned(),
                saved: None,
                address: Address { scheme, user: None, host: host.clone(), port: None, path: None },
            }));
        }
        (scheme, servers)
    });
    groups.into_iter().filter(|(_, servers)| !servers.is_empty()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reveals_reads_and_refreshes() {
        let root = std::env::temp_dir().join(format!("pc-tree-{}", std::process::id()));
        for dir in ["b/inner", "A", ".hidden", "c"] {
            std::fs::create_dir_all(root.join(dir)).unwrap();
        }
        std::fs::write(root.join("file"), "").unwrap();
        let mut tree = FolderTree { roots: vec![Node::new(root.clone(), "test".into())], hidden: false };
        assert_eq!(tree.rows().len(), 1);
        assert!(tree.reveal(&root.join("b/inner")));
        let names: Vec<_> = tree.rows().iter().map(|row| (row.name, row.depth)).collect();
        assert_eq!(names, [("test", 0), ("A", 1), ("b", 1), ("inner", 2), ("c", 1)]);
        assert!(!tree.reveal(Path::new("/elsewhere")));
        // A new folder shows, the open ones stay open; hidden ones on request.
        std::fs::create_dir(root.join("b/new")).unwrap();
        tree.refresh(true);
        let names: Vec<_> = tree.rows().iter().map(|row| row.name).collect();
        assert_eq!(names, ["test", ".hidden", "A", "b", "inner", "new", "c"]);
        tree.toggle(&root.join("b"));
        assert_eq!(tree.rows().len(), 5);
        tree.toggle(&root.join("c"));
        let c = tree.rows().into_iter().find(|row| row.name == "c").unwrap();
        assert!(c.expanded && c.leaf);
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn groups_servers_by_protocol() {
        let ftp = Connection { name: "nas".into(), scheme: Scheme::Ftp, host: "nas.lan".into(), user: Some("me".into()), ..Default::default() };
        let groups = favourites(&[ftp], &["homelab".into()]);
        let shown: Vec<_> = groups.iter().map(|(scheme, servers)| (*scheme, servers.len())).collect();
        assert_eq!(shown, [(Scheme::Sftp, 1), (Scheme::Ftp, 1)]);
        assert_eq!(groups[1].1[0].detail, "me@nas.lan");
        assert_eq!((groups[1].1[0].saved, groups[0].1[0].saved), (Some(0), None));
    }
}
