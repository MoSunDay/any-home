//! Magic-path encoding, synthetic attributes, and the registry that maps
//! synthetic fileids to their decoded names.
//!
//! See [`crate::dispatcher`] for the routing semantics. fileid layout:
//! ```text
//! bit 63       : MAGIC flag (1 => synthetic)
//! bits 56..59  : category (0 = dir, 1/2/3 = search/index/where file)
//! bits  0..55  : FNV-1a hash of the decoded name
//! ```

use std::collections::HashMap;
use std::sync::Mutex;

use nfsserve::nfs::*;

pub const MAGIC: u64 = 1u64 << 63;
const CAT_SHIFT: u32 = 56;
const HASH_MASK: u64 = 0x00FF_FFFF_FFFF_FFFF; // low 56 bits

// Magic directory ids (category 0, fixed small hashes).
pub const SEARCH_DIR: fileid3 = MAGIC | 0x1;
pub const INDEX_DIR: fileid3 = MAGIC | 0x2;
pub const WHERE_DIR: fileid3 = MAGIC | 0x3;

// File categories.
pub const CAT_SEARCH: u8 = 1;
pub const CAT_INDEX: u8 = 2;
pub const CAT_WHERE: u8 = 3;

/// Default number of neighbors returned by `/search`.
pub const DEFAULT_K: i64 = 16;
/// Reported size for synthetic files (read returns the real length).
pub const MAGIC_FILE_SIZE: u64 = 1 << 20; // 1 MiB

pub fn is_magic(id: fileid3) -> bool {
    id & MAGIC != 0
}

pub fn is_magic_dir(id: fileid3) -> bool {
    matches!(id, SEARCH_DIR | INDEX_DIR | WHERE_DIR)
}

pub fn magic_category(id: fileid3) -> u8 {
    ((id >> CAT_SHIFT) & 0xF) as u8
}

pub fn magic_file_id(cat: u8, name: &str) -> fileid3 {
    let h = fnv1a_64(name.as_bytes()) & HASH_MASK;
    MAGIC | ((cat as u64) << CAT_SHIFT) | h
}

pub fn fnv1a_64(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0x811C9DC5;
    for &b in bytes {
        h ^= b as u64;
        h = h.wrapping_mul(0x100000001B3);
    }
    h
}

/// Synthetic attributes so the client issues the reads/writes we intercept.
pub fn magic_attr(id: fileid3) -> fattr3 {
    let dir = is_magic_dir(id);
    let ftype = if dir { ftype3::NF3DIR } else { ftype3::NF3REG };
    let zero = nfstime3 { seconds: 0, nseconds: 0 };
    let size = if dir { 4096 } else { MAGIC_FILE_SIZE };
    fattr3 {
        ftype,
        mode: if dir { 0o755 } else { 0o644 },
        nlink: if dir { 2 } else { 1 },
        uid: 0,
        gid: 0,
        size,
        used: size,
        rdev: specdata3::default(),
        fsid: 0,
        fileid: id,
        atime: zero,
        mtime: zero,
        ctime: zero,
    }
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

/// Registry of synthetic files seen via lookup/create: fileid -> decoded name.
/// Process-local; not persisted.
#[derive(Default)]
pub struct MagicRegistry {
    inner: Mutex<HashMap<fileid3, String>>,
}

impl MagicRegistry {
    pub fn register(&self, id: fileid3, name: String) {
        self.inner.lock().unwrap().entry(id).or_insert(name);
    }

    pub fn get(&self, id: fileid3) -> Option<String> {
        self.inner.lock().unwrap().get(&id).cloned()
    }

    pub fn forget(&self, id: fileid3) {
        self.inner.lock().unwrap().remove(&id);
    }

    /// All (id, name) pairs whose category matches `cat`.
    pub fn list_for_cat(&self, cat: u8) -> Vec<(fileid3, String)> {
        self.inner
            .lock()
            .unwrap()
            .iter()
            .filter(|(id, _)| magic_category(**id) == cat)
            .map(|(id, name)| (*id, name.clone()))
            .collect()
    }
}
