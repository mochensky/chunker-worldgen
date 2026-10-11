//! Shared helpers for tests that compare against vanilla.
//!
//! They need `cargo xtask vanilla` (server jar in `vanilla/26.3/`) and, for oracle tests,
//! `cargo xtask oracle` (compiled Java oracle in `tools/oracle/build`). Missing prerequisites make
//! the tests print a notice and pass, so `cargo test` stays usable on machines without Java.

#![allow(dead_code)]

use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::Arc;

use chunker_data::blocks::BlockRegistry;
use chunker_data::{DataPacks, McVersion, PackSource};
use chunker_worldgen::registry::WorldgenRegistries;

/// How many oracle JVMs may run at once in one test binary (`CHUNKER_ORACLES`, default 2). Each
/// JVM loads the whole server, so many in parallel exhaust the RAM of an ordinary machine.
pub fn oracle_slots() -> usize {
    std::env::var("CHUNKER_ORACLES")
        .ok()
        .and_then(|v| v.parse().ok())
        .filter(|&n| n > 0)
        .unwrap_or(2)
}

/// Counting semaphore over `oracle_slots()`.
static RUNNING: std::sync::Mutex<usize> = std::sync::Mutex::new(0);
static FREED: std::sync::Condvar = std::sync::Condvar::new();

fn acquire_slot() {
    let mut n = RUNNING.lock().unwrap();
    while *n >= oracle_slots() {
        n = FREED.wait(n).unwrap();
    }
    *n += 1;
}

fn release_slot() {
    *RUNNING.lock().unwrap() -= 1;
    FREED.notify_one();
}

pub fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf()
}

pub fn vanilla_registries() -> Option<Arc<WorldgenRegistries>> {
    let jar = workspace_root().join("vanilla/26.3/server-26.3.jar");
    if !jar.exists() {
        eprintln!("skipping: {} missing (run `cargo xtask vanilla`)", jar.display());
        return None;
    }
    let mut packs = DataPacks::new();
    packs.push(PackSource::zip_file(&jar).expect("open server jar"));
    Some(Arc::new(WorldgenRegistries::new(
        packs,
        BlockRegistry::for_version(McVersion::V26_3),
    )))
}

pub struct Oracle {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
}

impl Oracle {
    pub fn start() -> Option<Oracle> {
        let root = workspace_root();
        let build = root.join("tools/oracle/build");
        let cp_file = build.join("classpath.txt");
        let Ok(cp) = std::fs::read_to_string(&cp_file) else {
            eprintln!("skipping: oracle not built (run `cargo xtask oracle`)");
            return None;
        };
        let sep = if cfg!(windows) { ";" } else { ":" };
        let classpath = format!("{}{sep}{}", build.display(), cp.trim());
        let work = root.join("target/oracle-work");
        std::fs::create_dir_all(&work).ok()?;
        // One stderr file per oracle process (the JVM's own crash reports end up there).
        static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let err = std::fs::File::create(work.join(format!("oracle-{}-{n}.err", std::process::id()))).ok()?;
        // Heap cap per JVM (`CHUNKER_ORACLE_XMX`, default 2g; the default would be 1/4 of the RAM).
        let xmx = std::env::var("CHUNKER_ORACLE_XMX").unwrap_or_else(|_| "2g".to_owned());
        acquire_slot();
        let child = Command::new("java")
            .current_dir(&work)
            .arg(format!("-Xmx{xmx}"))
            .arg("-cp")
            .arg(classpath)
            .arg("chunker.oracle.Oracle")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::from(err))
            .spawn();
        let Ok(mut child) = child else {
            release_slot();
            return None;
        };
        let (Some(stdin), Some(stdout)) = (child.stdin.take(), child.stdout.take()) else {
            let _ = child.kill();
            release_slot();
            return None;
        };
        Some(Oracle {
            child,
            stdin,
            stdout: BufReader::new(stdout),
        })
    }

    /// Sends queries and returns one response line per query (logging lines are skipped). Queries are
    /// written from another thread: with large answers both pipes fill up otherwise.
    pub fn query_batch(&mut self, queries: &[String]) -> Vec<String> {
        let (stdin, stdout) = (&mut self.stdin, &mut self.stdout);
        std::thread::scope(|scope| {
            let writer = scope.spawn(move || {
                for q in queries {
                    if writeln!(stdin, "{q}").is_err() {
                        return;
                    }
                }
                let _ = stdin.flush();
            });
            let mut out = Vec::with_capacity(queries.len());
            while out.len() < queries.len() {
                let mut line = String::new();
                if stdout.read_line(&mut line).unwrap() == 0 {
                    panic!("oracle exited early");
                }
                let line = line.trim();
                if line.starts_with("ok") || line.starts_with("error") {
                    assert!(!line.starts_with("error"), "oracle error: {line}");
                    out.push(line.to_owned());
                }
            }
            writer.join().unwrap();
            out
        })
    }
}

impl Drop for Oracle {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        release_slot();
    }
}

/// Java `31 * hash + bits` rolling hash used by the oracle for volume results.
pub fn java_hash(values: &[f32]) -> i64 {
    let mut hash: i64 = 1125899906842597;
    for v in values {
        hash = hash.wrapping_mul(31).wrapping_add(v.to_bits() as i32 as i64);
    }
    hash
}

/// The oracle's `describeChunk` format: block hashes, heightmaps, post-processing, per-section hashes.
pub fn describe_chunk(chunk: &mut chunker_worldgen::chunk::ProtoChunk) -> String {
    use chunker_worldgen::chunk::HeightmapKind;
    let mut all: i64 = 1125899906842597;
    let mut sections = String::new();
    for s in &chunk.sections {
        let mut h: i64 = 1125899906842597;
        for y in 0..16 {
            for z in 0..16 {
                for x in 0..16 {
                    let id = s.get(x, y, z).0 as i64;
                    h = h.wrapping_mul(31).wrapping_add(id);
                    all = all.wrapping_mul(31).wrapping_add(id);
                }
            }
        }
        sections.push_str(&format!(" {h}"));
    }
    let mut out = format!("ok {all}");
    for kind in HeightmapKind::ALL {
        if chunk.heightmaps.is_primed(kind) {
            let mut h: i64 = 1125899906842597;
            for z in 0..16 {
                for x in 0..16 {
                    h = h
                        .wrapping_mul(31)
                        .wrapping_add(chunk.heightmaps.first_available(kind, x, z) as i64);
                }
            }
            out.push_str(&format!(" {}={h}", kind.key()));
        } else {
            out.push_str(&format!(" {}=-", kind.key()));
        }
    }
    let mut pp: i64 = 1125899906842597;
    let mut count = 0;
    for list in &chunk.post_processing {
        for &o in list {
            pp = pp.wrapping_mul(31).wrapping_add(o as i64);
            count += 1;
        }
        pp = pp.wrapping_mul(31).wrapping_add(7);
    }
    out.push_str(&format!(" post={count}:{pp} |{sections}"));
    out
}
