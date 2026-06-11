use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::UNIX_EPOCH;

use anyhow::Result;
use rayon::prelude::*;

use crate::cache::HashCache;
use crate::hasher;

// Thresholds are passed in from the caller (configurable via Settings).
// Default: 8 bits.

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ImageEntry {
    pub path: PathBuf,
    pub size: u64,
    pub hash: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rot_hashes: Option<[u64; 4]>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct DuplicateGroup {
    pub hash: u64,
    pub files: Vec<ImageEntry>,
    pub is_rotation: bool,
}

struct UnionFind {
    parent: Vec<usize>,
    rank: Vec<usize>,
}

impl UnionFind {
    fn new(n: usize) -> Self {
        UnionFind {
            parent: (0..n).collect(),
            rank: vec![0; n],
        }
    }

    fn find(&mut self, x: usize) -> usize {
        if self.parent[x] != x {
            self.parent[x] = self.find(self.parent[x]);
        }
        self.parent[x]
    }

    fn union(&mut self, x: usize, y: usize) {
        let xr = self.find(x);
        let yr = self.find(y);
        if xr == yr {
            return;
        }
        if self.rank[xr] < self.rank[yr] {
            self.parent[xr] = yr;
        } else if self.rank[xr] > self.rank[yr] {
            self.parent[yr] = xr;
        } else {
            self.parent[yr] = xr;
            self.rank[xr] += 1;
        }
    }
}

pub fn find_duplicates(
    paths: &[PathBuf],
    cancel: &AtomicBool,
    cache: Option<&HashCache>,
    rotation_mode: bool,
    threshold: u32,
    progress: impl Fn(usize, usize, usize, &str) + Send + Sync,
) -> Result<Vec<DuplicateGroup>> {
    let total = paths.len();
    let counter = AtomicUsize::new(0);
    let last_pct = AtomicUsize::new(0);
    let failed = AtomicUsize::new(0);

    let entries: Vec<ImageEntry> = paths.par_iter().filter_map(|path| {
        if cancel.load(Ordering::Relaxed) {
            return None;
        }

        let meta = match std::fs::metadata(path) {
            Ok(m) => m,
            Err(e) => {
                eprintln!("Warning: Cannot access file {:?}: {}", path, e);
                failed.fetch_add(1, Ordering::Relaxed);
                return None;
            }
        };
        let size = meta.len();
        let mtime = meta.modified()
            .ok()
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .map(|d| d.as_secs())
            .unwrap_or(0);

        let (hash, rot_hashes) = if rotation_mode {
            // Try cache first — both regular and rotation hashes are stored.
            let cached = cache.and_then(|c| {
                let h = c.lookup(path, mtime, size);
                let rh = c.lookup_rot(path, mtime, size);
                h.zip(rh)
            });
            if let Some((h, rh)) = cached {
                (h, Some(rh))
            } else {
                match hasher::dhash_rotations(path, cancel) {
                    Ok(hashes) => {
                        if let Some(c) = cache {
                            c.insert(path, mtime, size, hashes[0]);
                            c.insert_rot(path, mtime, size, hashes);
                        }
                        (hashes[0], Some(hashes))
                    }
                    Err(e) => {
                        if let Some(c) = cache {
                            if let Some(h) = c.lookup(path, mtime, size) {
                                eprintln!("Warning: Failed to compute rotation hashes for {:?}: {}", path, e);
                                (h, None)
                            } else {
                                eprintln!("Warning: Failed to rotation-hash {:?}: {}", path, e);
                                failed.fetch_add(1, Ordering::Relaxed);
                                return None;
                            }
                        } else {
                            eprintln!("Warning: Failed to rotation-hash {:?}: {}", path, e);
                            failed.fetch_add(1, Ordering::Relaxed);
                            return None;
                        }
                    }
                }
            }
        } else {
            let h = if let Some(cache) = cache {
                if let Some(h) = cache.lookup(path, mtime, size) {
                    h
                } else {
                    match hasher::dhash(path, cancel) {
                        Ok(h) => {
                            cache.insert(path, mtime, size, h);
                            h
                        }
                        Err(e) => {
                            eprintln!("Warning: Failed to hash {:?}: {}", path, e);
                            failed.fetch_add(1, Ordering::Relaxed);
                            return None;
                        }
                    }
                }
            } else {
                match hasher::dhash(path, cancel) {
                    Ok(h) => h,
                    Err(e) => {
                        eprintln!("Warning: Failed to hash {:?}: {}", path, e);
                        failed.fetch_add(1, Ordering::Relaxed);
                        return None;
                    }
                }
            };
            (h, None)
        };

        let done = counter.fetch_add(1, Ordering::Relaxed) + 1;
        let pct = (done * 100) / total.max(1);
        let prev = last_pct.load(Ordering::Relaxed);
        if pct > prev || done == total {
            last_pct.store(pct, Ordering::Relaxed);
            let name = path.file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("");
            progress(done, total, failed.load(Ordering::Relaxed), name);
        }

        Some(ImageEntry {
            path: path.clone(),
            size,
            hash,
            rot_hashes,
        })
    }).collect();

    let failed_count = failed.load(Ordering::Relaxed);
    if failed_count > 0 {
        eprintln!(
            "Warning: {}/{} files could not be processed and were skipped.",
            failed_count, total
        );
    }

    if cancel.load(Ordering::Relaxed) {
        progress(counter.load(Ordering::Relaxed), total, failed.load(Ordering::Relaxed), "");
    }

    if entries.is_empty() {
        return Ok(Vec::new());
    }

    // Phase 2: Cluster by Hamming distance (near-duplicate detection).
    // Two images within HAMMING_THRESHOLD differing bits are grouped together.
    // Uses Union-Find to form transitive clusters.
    //
    // Runs single-threaded on the calling (background) thread so the event
    // loop always has a free CPU core — eliminates UI freeze risk.
    // Cancel is polled every 1000 inner comparisons for prompt cancellation.
    let n = entries.len();
    let mut uf = UnionFind::new(n);

    for i in 0..n {
        if cancel.load(Ordering::Relaxed) {
            break;
        }
        let h_i = entries[i].hash;
        let r_i = &entries[i].rot_hashes;
        for j in (i + 1)..n {
            // h0 vs h0 uses the configured threshold.
            if (h_i ^ entries[j].hash).count_ones() <= threshold {
                uf.union(i, j);
            } else if rotation_mode {
                // If h0 doesn't match, try rotation-invariant check.
                if let (Some(a), Some(b)) = (r_i, &entries[j].rot_hashes) {
                    if hasher::rotation_min_distance(a, b) <= threshold {
                        uf.union(i, j);
                    }
                }
            }
            if j % 1000 == 0 && cancel.load(Ordering::Relaxed) {
                break;
            }
        }
        if i % 128 == 0 || i == n - 1 {
            progress(i + 1, n, failed.load(Ordering::Relaxed), "");
        }
    }

    // Phase 3: Build groups from union roots
    let mut group_map: HashMap<usize, Vec<ImageEntry>> = HashMap::new();
    for (i, entry) in entries.into_iter().enumerate() {
        let root = uf.find(i);
        group_map.entry(root).or_default().push(entry);
    }

    let mut groups: Vec<DuplicateGroup> = group_map
        .into_iter()
        .filter(|(_, files)| files.len() > 1)
        .map(|(_, mut files)| {
            files.sort_by(|a, b| b.size.cmp(&a.size));
            DuplicateGroup {
                hash: files[0].hash,
                files,
                is_rotation: rotation_mode,
            }
        })
        .collect();

    groups.sort_by(|a, b| {
        let total_a: u64 = a.files.iter().map(|f| f.size).sum();
        let total_b: u64 = b.files.iter().map(|f| f.size).sum();
        total_b.cmp(&total_a)
    });

    Ok(groups)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicBool;

    #[test]
    fn test_find_duplicates_detects_identical() {
        let paths = vec![
            PathBuf::from("/tmp/imphash_test/photo1.png"),
            PathBuf::from("/tmp/imphash_test/photo1_copy.png"),
            PathBuf::from("/tmp/imphash_test/photo2.png"),
        ];
        let cancel = AtomicBool::new(false);
        let groups = find_duplicates(&paths, &cancel, None, false, 8, |_, _, _, _| {}).unwrap();
        assert_eq!(groups.len(), 1, "should find one duplicate group");
        assert_eq!(groups[0].files.len(), 2, "group should have 2 files");
    }

    #[test]
    fn test_find_duplicates_no_duplicates() {
        let paths = vec![
            PathBuf::from("/tmp/imphash_test/photo1.png"),
            PathBuf::from("/tmp/imphash_test/photo2.png"),
        ];
        let cancel = AtomicBool::new(false);
        let groups = find_duplicates(&paths, &cancel, None, false, 8, |_, _, _, _| {}).unwrap();
        assert_eq!(groups.len(), 0, "should find no duplicate groups");
    }

    #[test]
    fn test_near_duplicate_detection() {
        let paths = vec![
            PathBuf::from("/tmp/imphash_test/photo1.png"),
            PathBuf::from("/tmp/imphash_test/photo1_copy.png"),
        ];
        let cancel = AtomicBool::new(false);

        let groups = find_duplicates(&paths, &cancel, None, false, 8, |_, _, _, _| {}).unwrap();
        assert_eq!(groups.len(), 1, "identical files should form a group");
    }

    #[test]
    fn test_compressed_duplicates_natural() {
        let paths = vec![
            PathBuf::from("/tmp/imphash_test2/natural_highq.jpg"),
            PathBuf::from("/tmp/imphash_test2/natural_lowq.jpg"),
        ];
        let cancel = AtomicBool::new(false);
        let groups = find_duplicates(&paths, &cancel, None, false, 8, |_, _, _, _| {
        }).unwrap();

        eprintln!("natural highq+lowq: groups={}", groups.len());
        if !groups.is_empty() {
            eprintln!("  files in group: {}", groups[0].files.len());
        }

        assert_eq!(
            groups.len(), 1,
            "differently compressed same image should form a group"
        );
        assert_eq!(groups[0].files.len(), 2);
    }

    #[test]
    fn test_compressed_duplicates_all_variants() {
        let paths = vec![
            PathBuf::from("/tmp/imphash_test2/natural_highq.jpg"),
            PathBuf::from("/tmp/imphash_test2/natural_medium.jpg"),
            PathBuf::from("/tmp/imphash_test2/natural_lowq.jpg"),
            PathBuf::from("/tmp/imphash_test2/natural_png.png"),
        ];
        let cancel = AtomicBool::new(false);
        let groups = find_duplicates(&paths, &cancel, None, false, 8, |_, _, _, _| {
        }).unwrap();

        assert_eq!(
            groups.len(), 1,
            "all variants of same image should form one group, got {}",
            groups.len()
        );
        assert_eq!(groups[0].files.len(), 4);
    }
}
