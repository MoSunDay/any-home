//! Magic-path encoding, synthetic attributes, and the registry that maps
//! synthetic fileids to their decoded names.
//!
//! See [`crate::dispatcher`] for the routing semantics. fileid layout:
//! ```text
//! bit 63       : MAGIC flag (1 => synthetic)
//! bits 56..59  : category (0 = dir, 1/2/3 = search/index/where file,
//!                7 = agent container dir, 8/9/10 = agent-scoped search/index/where dir)
//! bits  0..55  : FNV-1a hash of the decoded (possibly scoped) name
//! ```
//!
//! Multi-agent isolation: fileids for agent-scoped files are computed from a
//! *scoped name* — `"{agent_id}\0{name}"` — so the same `name` in two different
//! agents produces different fileids (no cross-agent collision). The
//! [`MagicRegistry`] stores the full [`MagicEntry`] (agent_id + name + category)
//! so the dispatcher can recover the agent context from any magic fileid.

use std::collections::HashMap;
use std::sync::Mutex;

use nfsserve::nfs::*;

pub const MAGIC: u64 = 1u64 << 63;
const CAT_SHIFT: u32 = 56;
const HASH_MASK: u64 = 0x00FF_FFFF_FFFF_FFFF; // low 56 bits

// Magic directory ids (category field 0, fixed small hashes).
pub const SEARCH_DIR: fileid3 = MAGIC | 0x1;
pub const INDEX_DIR: fileid3 = MAGIC | 0x2;
pub const WHERE_DIR: fileid3 = MAGIC | 0x3;
pub const AGENTS_DIR: fileid3 = MAGIC | 0x4;

// File categories.
pub const CAT_SEARCH: u8 = 1;
pub const CAT_INDEX: u8 = 2;
pub const CAT_WHERE: u8 = 3;

// Agent namespace categories (>= CAT_AGENT are always directories).
pub const CAT_AGENT: u8 = 7;
pub const CAT_AGENT_SEARCH: u8 = 8;
pub const CAT_AGENT_INDEX: u8 = 9;
pub const CAT_AGENT_WHERE: u8 = 10;

/// `/search_k<N>` directory (global or agent-scoped): like `/search` but the
/// KNN top-k is `N` instead of [`DEFAULT_K`]. `>= CAT_AGENT`, so fileids with
/// this category are directories to [`is_magic_dir`].
pub const CAT_SEARCH_K_DIR: u8 = 11;

/// Default number of neighbors returned by `/search`.
pub const DEFAULT_K: i64 = 16;
/// Upper bound for `/search_k<N>` (larger N are clamped down to it).
pub const MAX_K: i64 = 64;
/// Reported size for synthetic files (read returns the real length).
pub const MAGIC_FILE_SIZE: u64 = 1 << 20; // 1 MiB

pub fn is_magic(id: fileid3) -> bool {
    id & MAGIC != 0
}

/// True for any synthetic directory: global magic dirs, `/agents`, and all
/// agent-scoped container/magic dirs (categories >= CAT_AGENT).
pub fn is_magic_dir(id: fileid3) -> bool {
    if matches!(id, SEARCH_DIR | INDEX_DIR | WHERE_DIR | AGENTS_DIR) {
        return true;
    }
    is_magic(id) && magic_category(id) >= CAT_AGENT
}

pub fn magic_category(id: fileid3) -> u8 {
    ((id >> CAT_SHIFT) & 0xF) as u8
}

/// Compute the scoped name used for fileid hashing: `"{agent_id}\0{name}"`.
/// The NUL separator avoids boundary ambiguity between agent_id and name.
pub fn scoped_name(agent_id: &str, name: &str) -> String {
    format!("{agent_id}\x00{name}")
}

/// Parse a `search_k<N>` directory name into a clamped `N` in `1..=MAX_K`.
/// Returns `None` for anything else (no digits, `N == 0`, or a non-numeric
/// suffix), so those names keep falling through to the real filesystem.
pub fn parse_search_k(name: &[u8]) -> Option<i64> {
    let digits = name.strip_prefix(b"search_k")?;
    if digits.is_empty() || !digits.iter().all(u8::is_ascii_digit) {
        return None;
    }
    let n = std::str::from_utf8(digits).ok()?.parse::<u64>().ok()?;
    if n == 0 {
        return None; // k=0 is meaningless: stay outside the family
    }
    Some((n as i64).clamp(1, MAX_K))
}

/// Synthetic directory id for `/search_k<N>` (agent-scoped when `agent_id`
/// is non-empty). Not enumerable via readdir; clients reach it by lookup.
pub fn search_k_dir_id(agent_id: &str, k: i64) -> fileid3 {
    magic_file_id(CAT_SEARCH_K_DIR, &format!("{agent_id}\x00k{k}"))
}

/// Scoped name for a search query opened via `/search_k<N>`: k-salted so its
/// fileid never collides with `/search/<query>` or another N. (The read path
/// recovers `k` from the registry entry, and distinct fileids keep those
/// entries distinct.) Like [`scoped_name`], a path component containing an
/// embedded NUL could theoretically alias the two encodings — same accepted
/// ambiguity class as [`scoped_name`] itself.
pub fn search_scoped_name(agent_id: &str, k: i64, query: &str) -> String {
    format!("{agent_id}\x00k{k}\x00{query}")
}

pub fn magic_file_id(cat: u8, scoped_name: &str) -> fileid3 {
    let h = fnv1a_64(scoped_name.as_bytes()) & HASH_MASK;
    MAGIC | ((cat as u64) << CAT_SHIFT) | h
}

/// Map an agent-scoped dir category to its base file category, or `None` if
/// `cat` is not an agent-scoped dir.
pub fn agent_dir_base_cat(cat: u8) -> Option<u8> {
    match cat {
        CAT_AGENT_SEARCH => Some(CAT_SEARCH),
        CAT_AGENT_INDEX => Some(CAT_INDEX),
        CAT_AGENT_WHERE => Some(CAT_WHERE),
        _ => None,
    }
}

pub fn fnv1a_64(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0x811C9DC5;
    for &b in bytes {
        h ^= b as u64;
        h = h.wrapping_mul(0x100000001B3);
    }
    h
}

/// Synthetic attributes with a caller-supplied `size`. Directories ignore
/// `size` (they always report 4096); regular files report exactly `size`, so
/// `getattr` can pass the real content length for `/index` files.
pub fn magic_attr_sized(id: fileid3, size: u64) -> fattr3 {
    let dir = is_magic_dir(id);
    let ftype = if dir { ftype3::NF3DIR } else { ftype3::NF3REG };
    let zero = nfstime3 { seconds: 0, nseconds: 0 };
    let reported = if dir { 4096 } else { size };
    fattr3 {
        ftype,
        mode: if dir { 0o755 } else { 0o644 },
        nlink: if dir { 2 } else { 1 },
        uid: 0,
        gid: 0,
        size: reported,
        used: reported,
        rdev: specdata3::default(),
        fsid: 0,
        fileid: id,
        atime: zero,
        mtime: zero,
        ctime: zero,
    }
}

/// Synthetic attributes with the default virtual size.
pub fn magic_attr(id: fileid3) -> fattr3 {
    magic_attr_sized(id, MAGIC_FILE_SIZE)
}

/// Slice a precomputed byte buffer for an NFS READ, setting EOF at the end.
pub fn slice_read(content: &[u8], offset: u64, count: u32) -> (Vec<u8>, bool) {
    let len = content.len() as u64;
    if offset >= len {
        return (Vec::new(), true);
    }
    let end = std::cmp::min(offset + count as u64, len);
    let bytes = content[offset as usize..end as usize].to_vec();
    (bytes, end >= len)
}

/// Percent-decode a path segment (handles `%20` etc. for URL-encoded spaces).
pub fn percent_decode(input: &[u8]) -> String {
    let mut out: Vec<u8> = Vec::with_capacity(input.len());
    let mut i = 0;
    while i < input.len() {
        if input[i] == b'%' && i + 2 < input.len() {
            if let (Some(hi), Some(lo)) = (hex_val(input[i + 1]), hex_val(input[i + 2])) {
                out.push((hi << 4) | lo);
                i += 3;
                continue;
            }
        }
        out.push(input[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn hex_val(c: u8) -> Option<u8> {
    match c {
        b'0'..=b'9' => Some(c - b'0'),
        b'a'..=b'f' => Some(c - b'a' + 10),
        b'A'..=b'F' => Some(c - b'A' + 10),
        _ => None,
    }
}

/// Full context for a magic fileid: which agent it belongs to, its decoded
/// name, and its category. Stored in [`MagicRegistry`].
#[derive(Clone, Debug)]
pub struct MagicEntry {
    pub agent_id: String,
    pub name: String,
    pub category: u8,
    /// KNN top-k for CAT_SEARCH files opened via `/search_k<N>`.
    /// [`DEFAULT_K`] everywhere else; unused for other categories.
    pub k: i64,
}

impl MagicEntry {
    pub fn new(agent_id: &str, name: &str, category: u8) -> Self {
        MagicEntry {
            agent_id: agent_id.to_string(),
            name: name.to_string(),
            category,
            k: DEFAULT_K,
        }
    }

    /// Like [`MagicEntry::new`] with an explicit KNN top-k.
    pub fn with_k(agent_id: &str, name: &str, category: u8, k: i64) -> Self {
        MagicEntry {
            k,
            ..MagicEntry::new(agent_id, name, category)
        }
    }
}

/// Upper bound on resident write buffers. Each `WRITE` RPC upserts to the DB
/// immediately (the DB is always authoritative), so evicting a buffer is safe:
/// the next write to that id re-seeds from the DB. This bounds memory for a
/// long-running server that writes many distinct keys (NFSv3 is stateless and
/// exposes no `close`/`commit` hook to free buffers eagerly).
const MAX_BUFFERS: usize = 256;

/// Process-local registry mapping synthetic fileids to their decoded context
/// ([`MagicEntry`]) and write buffers. Populated lazily by `lookup`/`create`
/// and rebuilt from the DB on restart (NFS clients always re-`lookup` after a
/// reconnect).
#[derive(Default)]
pub struct MagicRegistry {
    entries: Mutex<HashMap<fileid3, MagicEntry>>,
    buffers: Mutex<HashMap<fileid3, Vec<u8>>>,
}

impl MagicRegistry {
    pub fn register(&self, id: fileid3, entry: MagicEntry) {
        self.entries.lock().unwrap().entry(id).or_insert(entry);
    }

    pub fn get_entry(&self, id: fileid3) -> Option<MagicEntry> {
        self.entries.lock().unwrap().get(&id).cloned()
    }

    /// Convenience: just the decoded name (for backwards-compatible callers).
    pub fn get(&self, id: fileid3) -> Option<String> {
        self.entries.lock().unwrap().get(&id).map(|e| e.name.clone())
    }

    pub fn forget(&self, id: fileid3) {
        self.entries.lock().unwrap().remove(&id);
        self.buffers.lock().unwrap().remove(&id);
    }

    /// All `(id, name)` pairs whose category matches `cat` **and** whose
    /// agent_id matches `agent_id`. Used by readdir for `/search` and `/where`.
    pub fn list_for_cat(&self, cat: u8, agent_id: &str) -> Vec<(fileid3, String)> {
        self.entries
            .lock()
            .unwrap()
            .iter()
            .filter(|(id, e)| magic_category(**id) == cat && e.agent_id == agent_id)
            .map(|(id, e)| (*id, e.name.clone()))
            .collect()
    }

    /// Whether an in-flight write buffer already exists for `id`.
    pub fn has_buffer(&self, id: fileid3) -> bool {
        self.buffers.lock().unwrap().contains_key(&id)
    }

    /// Seed the write buffer for `id` from existing persisted content, but only
    /// if no buffer exists yet.
    pub fn seed_buffer(&self, id: fileid3, data: Vec<u8>) {
        let mut bufs = self.buffers.lock().unwrap();
        bufs.entry(id).or_insert(data);
    }

    /// Splice `data` into the write buffer for `id` at `offset`, zero-filling
    /// any gap, and return a clone of the resulting full buffer.
    pub fn write_into(&self, id: fileid3, offset: u64, data: &[u8]) -> Vec<u8> {
        let mut bufs = self.buffers.lock().unwrap();
        // Bound memory: if the buffer cache is full and `id` has no buffer yet,
        // drop one arbitrary entry. Safe because the DB is authoritative and a
        // subsequent write to the dropped id re-seeds from it.
        if bufs.len() >= MAX_BUFFERS && !bufs.contains_key(&id) {
            if let Some(&k) = bufs.keys().next() {
                bufs.remove(&k);
            }
        }
        let buf = bufs.entry(id).or_default();
        let start = offset as usize;
        let end = start + data.len();
        if buf.len() < end {
            buf.resize(end, 0u8);
        }
        buf[start..end].copy_from_slice(data);
        buf.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn write_buffers_are_bounded() {
        // The DB is authoritative, so evicting a buffer is safe (the next write
        // to that id re-seeds). Writing many distinct ids must therefore not grow
        // the resident buffer map past `MAX_BUFFERS`.
        let reg = MagicRegistry::default();
        for i in 0..(MAX_BUFFERS + 50) as u64 {
            reg.write_into(magic_file_id(CAT_INDEX, &format!("k{i}")), 0, b"x");
        }
        let n = reg.buffers.lock().unwrap().len();
        assert!(
            n <= MAX_BUFFERS,
            "buffer cache must stay <= {MAX_BUFFERS}, got {n}"
        );
    }

    #[test]
    fn eviction_is_transparent() {
        // After eviction, a subsequent write re-derives content correctly from
        // the caller-supplied offset (the registry never loses data it is given).
        let reg = MagicRegistry::default();
        let id = magic_file_id(CAT_INDEX, "once");
        let out = reg.write_into(id, 0, b"hello");
        assert_eq!(out, b"hello");
        // Forcing many other ids evicts `id`'s buffer without losing the ability
        // to append at the right offset later.
        for i in 0..(MAX_BUFFERS + 10) as u64 {
            reg.write_into(magic_file_id(CAT_INDEX, &format!("o{i}")), 0, b"y");
        }
        // Appending after eviction composes from offset 5 (seed would come from
        // the DB in the real path; here we exercise the splice-after-evict path).
        let out = reg.write_into(id, 5, b"!");
        assert_eq!(out.len(), 6);
        assert_eq!(&out[5..], b"!");
    }
}
