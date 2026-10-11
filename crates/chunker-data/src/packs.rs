use std::collections::BTreeSet;
use std::fs;
use std::io::{Cursor, Read};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result};
use parking_lot::Mutex;
use rustc_hash::FxHashMap;
use serde_json::Value;

use crate::Identifier;

/// One resource source: a directory, an in-memory zip, or a sub-tree of a zip
/// (built-in feature packs live under `data/minecraft/datapacks/<name>/` inside the server jar).
type JarArchive = zip::ZipArchive<Cursor<Arc<[u8]>>>;

#[derive(Clone)]
pub enum PackSource {
    Directory(PathBuf),
    Zip {
        name: String,
        archive: Arc<Mutex<JarArchive>>,
        prefix: String,
        index: Arc<BTreeSet<String>>,
    },
}

impl PackSource {
    pub fn directory(path: impl Into<PathBuf>) -> Self {
        PackSource::Directory(path.into())
    }

    pub fn zip_bytes(name: impl Into<String>, bytes: Arc<[u8]>, prefix: impl Into<String>) -> Result<Self> {
        let archive = zip::ZipArchive::new(Cursor::new(bytes)).context("invalid zip")?;
        let prefix = prefix.into();
        let index: BTreeSet<String> = archive
            .file_names()
            .filter(|n| n.starts_with(&prefix) && !n.ends_with('/'))
            .map(|n| n[prefix.len()..].to_owned())
            .collect();
        Ok(PackSource::Zip {
            name: name.into(),
            archive: Arc::new(Mutex::new(archive)),
            prefix,
            index: Arc::new(index),
        })
    }

    pub fn zip_file(path: &Path) -> Result<Self> {
        let bytes: Arc<[u8]> = fs::read(path)
            .with_context(|| format!("reading {}", path.display()))?
            .into();
        Self::zip_bytes(path.display().to_string(), bytes, "")
    }

    /// Same archive, different root prefix (shares the decompressed central directory).
    pub fn with_prefix(&self, prefix: &str) -> Option<Self> {
        match self {
            PackSource::Zip { name, archive, .. } => {
                let guard = archive.lock();
                let index: BTreeSet<String> = guard
                    .file_names()
                    .filter(|n| n.starts_with(prefix) && !n.ends_with('/'))
                    .map(|n| n[prefix.len()..].to_owned())
                    .collect();
                drop(guard);
                Some(PackSource::Zip {
                    name: format!("{name}!{prefix}"),
                    archive: archive.clone(),
                    prefix: prefix.to_owned(),
                    index: Arc::new(index),
                })
            }
            PackSource::Directory(path) => Some(PackSource::Directory(path.join(prefix))),
        }
    }

    pub fn describe(&self) -> String {
        match self {
            PackSource::Directory(p) => p.display().to_string(),
            PackSource::Zip { name, .. } => name.clone(),
        }
    }

    fn read(&self, rel: &str) -> Option<Vec<u8>> {
        match self {
            PackSource::Directory(root) => fs::read(root.join(rel)).ok(),
            PackSource::Zip {
                archive,
                prefix,
                index,
                ..
            } => {
                if !index.contains(rel) {
                    return None;
                }
                let mut guard = archive.lock();
                let mut entry = guard.by_name(&format!("{prefix}{rel}")).ok()?;
                let mut out = Vec::with_capacity(entry.size() as usize);
                entry.read_to_end(&mut out).ok()?;
                Some(out)
            }
        }
    }

    /// Lists relative paths under `dir/` (recursive) that end with `suffix`.
    fn list(&self, dir: &str, suffix: &str, out: &mut BTreeSet<String>) {
        match self {
            PackSource::Directory(root) => {
                fn walk(base: &Path, cur: &Path, suffix: &str, out: &mut BTreeSet<String>) {
                    let Ok(entries) = fs::read_dir(cur) else { return };
                    for e in entries.flatten() {
                        let p = e.path();
                        if p.is_dir() {
                            walk(base, &p, suffix, out);
                        } else if let Ok(rel) = p.strip_prefix(base) {
                            let rel = rel.to_string_lossy().replace('\\', "/");
                            if rel.ends_with(suffix) {
                                out.insert(rel);
                            }
                        }
                    }
                }
                walk(root, &root.join(dir), suffix, out);
            }
            PackSource::Zip { index, .. } => {
                let start = format!("{dir}/");
                for name in index.range(start.clone()..) {
                    if !name.starts_with(&start) {
                        break;
                    }
                    if name.ends_with(suffix) {
                        out.insert(name.clone());
                    }
                }
            }
        }
    }

    fn namespaces(&self) -> BTreeSet<String> {
        let mut out = BTreeSet::new();
        match self {
            PackSource::Directory(root) => {
                if let Ok(entries) = fs::read_dir(root.join("data")) {
                    for e in entries.flatten() {
                        if e.path().is_dir() {
                            out.insert(e.file_name().to_string_lossy().into_owned());
                        }
                    }
                }
            }
            PackSource::Zip { index, .. } => {
                for name in index.iter() {
                    if let Some(rest) = name.strip_prefix("data/")
                        && let Some((ns, _)) = rest.split_once('/')
                    {
                        out.insert(ns.to_owned());
                    }
                }
            }
        }
        out
    }
}

/// An entry of a tag file after parsing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TagEntry {
    Element { id: Identifier, required: bool },
    Tag { id: Identifier, required: bool },
}

/// Ordered stack of data packs. Later packs override earlier ones (vanilla semantics).
#[derive(Clone, Default)]
pub struct DataPacks {
    sources: Vec<PackSource>,
}

impl DataPacks {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push(&mut self, source: PackSource) {
        self.sources.push(source);
    }

    pub fn sources(&self) -> &[PackSource] {
        &self.sources
    }

    fn resource_path(kind: &str, id: &Identifier, ext: &str) -> String {
        format!("data/{}/{}/{}.{ext}", id.namespace(), kind, id.path())
    }

    /// Raw bytes of the top-most file `data/<ns>/<kind>/<path>.<ext>`.
    pub fn read_raw(&self, kind: &str, id: &Identifier, ext: &str) -> Option<Vec<u8>> {
        let rel = Self::resource_path(kind, id, ext);
        self.sources.iter().rev().find_map(|s| s.read(&rel))
    }

    /// Parses the top-most JSON resource, e.g. `read_json("worldgen/noise", minecraft:temperature)`.
    pub fn read_json(&self, kind: &str, id: &Identifier) -> Result<Option<Value>> {
        let Some(bytes) = self.read_raw(kind, id, "json") else {
            return Ok(None);
        };
        let value = serde_json::from_slice(&bytes).with_context(|| format!("parsing {kind} {id}"))?;
        Ok(Some(value))
    }

    /// Like [`DataPacks::read_json`] but errors when missing.
    pub fn require_json(&self, kind: &str, id: &Identifier) -> Result<Value> {
        self.read_json(kind, id)?
            .with_context(|| format!("missing {kind} {id}"))
    }

    /// All ids of a registry kind across packs (`worldgen/biome` -> every biome).
    pub fn list(&self, kind: &str) -> Vec<Identifier> {
        let mut ids = BTreeSet::new();
        for source in &self.sources {
            for ns in source.namespaces() {
                let dir = format!("data/{ns}/{kind}");
                let mut files = BTreeSet::new();
                source.list(&dir, ".json", &mut files);
                for f in files {
                    let path = &f[dir.len() + 1..f.len() - ".json".len()];
                    if let Some(id) = Identifier::parse(&format!("{ns}:{path}")) {
                        ids.insert(id);
                    }
                }
            }
        }
        ids.into_iter().collect()
    }

    /// Raw merged entries of a tag (`kind` = `block`, `worldgen/biome`, ...), with vanilla
    /// `replace` handling. Nested tags are not expanded here.
    pub fn tag_entries(&self, kind: &str, id: &Identifier) -> Result<Vec<TagEntry>> {
        let rel = format!("data/{}/tags/{}/{}.json", id.namespace(), kind, id.path());
        let mut entries: Vec<TagEntry> = Vec::new();
        for source in &self.sources {
            let Some(bytes) = source.read(&rel) else { continue };
            let json: Value = serde_json::from_slice(&bytes).with_context(|| format!("tag {kind} {id}"))?;
            if json.get("replace").and_then(Value::as_bool).unwrap_or(false) {
                entries.clear();
            }
            for v in json.get("values").and_then(Value::as_array).into_iter().flatten() {
                let (raw, required) = match v {
                    Value::String(s) => (s.as_str(), true),
                    Value::Object(o) => (
                        o.get("id").and_then(Value::as_str).unwrap_or_default(),
                        o.get("required").and_then(Value::as_bool).unwrap_or(true),
                    ),
                    _ => continue,
                };
                let entry = if let Some(tag) = raw.strip_prefix('#') {
                    TagEntry::Tag {
                        id: Identifier::parse(tag).context("bad tag ref")?,
                        required,
                    }
                } else {
                    TagEntry::Element {
                        id: Identifier::parse(raw).context("bad tag element")?,
                        required,
                    }
                };
                entries.push(entry);
            }
        }
        Ok(entries)
    }

    /// Fully resolved tag members in vanilla order (duplicates removed, first occurrence kept).
    /// `exists` filters optional elements that are not registered.
    pub fn resolve_tag(
        &self,
        kind: &str,
        id: &Identifier,
        exists: &dyn Fn(&Identifier) -> bool,
    ) -> Result<Vec<Identifier>> {
        let mut cache: FxHashMap<Identifier, Vec<Identifier>> = FxHashMap::default();
        let mut stack = Vec::new();
        self.resolve_tag_inner(kind, id, exists, &mut cache, &mut stack)
    }

    fn resolve_tag_inner(
        &self,
        kind: &str,
        id: &Identifier,
        exists: &dyn Fn(&Identifier) -> bool,
        cache: &mut FxHashMap<Identifier, Vec<Identifier>>,
        stack: &mut Vec<Identifier>,
    ) -> Result<Vec<Identifier>> {
        if let Some(done) = cache.get(id) {
            return Ok(done.clone());
        }
        if stack.contains(id) {
            anyhow::bail!("tag cycle at #{id}");
        }
        stack.push(id.clone());
        let mut out: Vec<Identifier> = Vec::new();
        let push = |out: &mut Vec<Identifier>, e: Identifier| {
            if !out.contains(&e) {
                out.push(e);
            }
        };
        for entry in self.tag_entries(kind, id)? {
            match entry {
                TagEntry::Element { id: e, required } => {
                    if exists(&e) {
                        push(&mut out, e);
                    } else if required {
                        anyhow::bail!("tag #{id} references missing element {e}");
                    }
                }
                TagEntry::Tag { id: t, required } => {
                    let has = self.sources.iter().any(|s| {
                        s.read(&format!("data/{}/tags/{}/{}.json", t.namespace(), kind, t.path()))
                            .is_some()
                    });
                    if !has {
                        if required {
                            anyhow::bail!("tag #{id} references missing tag #{t}");
                        }
                        continue;
                    }
                    for e in self.resolve_tag_inner(kind, &t, exists, cache, stack)? {
                        push(&mut out, e);
                    }
                }
            }
        }
        stack.pop();
        cache.insert(id.clone(), out.clone());
        Ok(out)
    }
}
