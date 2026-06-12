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
    /// True when the image has too little contrast for a reliable hash.
    /// Clustering treats these conservatively (exact-match only).
    #[serde(default)]
    pub low_variance: bool,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct DuplicateGroup {
    pub hash: u64,
    pub files: Vec<ImageEntry>,
    pub is_rotation: bool,
}

// Union-Find removed — replaced by star clustering to prevent
// transitive chaining of fragile hash bits in dark/low-contrast images.

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

        let (hash, rot_hashes, low_variance) = if rotation_mode {
            // Try cache first — both regular and rotation hashes are stored.
            let cached = cache.and_then(|c| {
                let h = c.lookup(path, mtime, size);
                let rh = c.lookup_rot(path, mtime, size);
                h.zip(rh)
            });
            if let Some((h, rh)) = cached {
                // Cached entries don't store the variance flag; recompute is cheap
                // on the 9×8 grid but we'd need the pixels.  Accept cached as reliable.
                (h, Some(rh), false)
            } else {
                match hasher::dhash_rotations(path, cancel) {
                    Ok((hashes, low_var)) => {
                        if let Some(c) = cache {
                            c.insert(path, mtime, size, hashes[0]);
                            c.insert_rot(path, mtime, size, hashes);
                        }
                        (hashes[0], Some(hashes), low_var)
                    }
                    Err(e) => {
                        if let Some(c) = cache {
                            if let Some(h) = c.lookup(path, mtime, size) {
                                eprintln!("Warning: Failed to compute rotation hashes for {:?}: {}", path, e);
                                (h, None, false)
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
            let (h, low_var) = if let Some(cache) = cache {
                if let Some(h) = cache.lookup(path, mtime, size) {
                    (h, false)
                } else {
                    match hasher::dhash(path, cancel) {
                        Ok((h, lv)) => {
                            cache.insert(path, mtime, size, h);
                            (h, lv)
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
                    Ok((h, lv)) => (h, lv),
                    Err(e) => {
                        eprintln!("Warning: Failed to hash {:?}: {}", path, e);
                        failed.fetch_add(1, Ordering::Relaxed);
                        return None;
                    }
                }
            };
            (h, None, low_var)
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
            low_variance,
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
    //
    // Uses **star clustering** (not Union-Find) to prevent transitive
    // chaining.  Each group has an "anchor" (the first ungrouped entry
    // encountered); every other member must be within `threshold` of the
    // anchor itself — not merely within threshold of *some* member.
    //
    // Why this matters: dark / low-contrast images have many "fragile"
    // hash bits (decided by ≤2 pixel-value differences) that flip
    // randomly.  With Union-Find (single-linkage), a chain of 1-bit
    // flips can merge hundreds of unrelated dark photos into one giant
    // group.  Star clustering breaks those chains.
    //
    // Runs single-threaded on the calling (background) thread so the
    // event loop always has a free CPU core.
    let n = entries.len();
    let mut grouped = vec![false; n];
    let mut group_indices: Vec<Vec<usize>> = Vec::new();

    for i in 0..n {
        if cancel.load(Ordering::Relaxed) {
            break;
        }
        if grouped[i] {
            continue;
        }

        let h_anchor = entries[i].hash;
        let r_anchor = &entries[i].rot_hashes;
        let lv_anchor = entries[i].low_variance;
        let mut members: Vec<usize> = vec![i];

        for j in (i + 1)..n {
            if grouped[j] {
                continue;
            }
            // When either image is low-variance its hash is unreliable:
            // only group them if the hashes match exactly (distance == 0).
            // Same for degenerate hashes (all bytes nearly identical) —
            // these cluster in hash-space and cause false positives.
            let effective_threshold = if lv_anchor || entries[j].low_variance
                || hasher::is_degenerate_hash(h_anchor)
                || hasher::is_degenerate_hash(entries[j].hash)
            {
                0
            } else {
                threshold
            };

            // Compare candidate to the ANCHOR only (not to other members).
            let matched = if (h_anchor ^ entries[j].hash).count_ones() <= effective_threshold {
                true
            } else if rotation_mode && effective_threshold > 0 {
                if let (Some(a), Some(b)) = (r_anchor, &entries[j].rot_hashes) {
                    hasher::rotation_min_distance(a, b) <= effective_threshold
                } else {
                    false
                }
            } else {
                false
            };

            if matched {
                members.push(j);
            }

            if j % 1000 == 0 && cancel.load(Ordering::Relaxed) {
                break;
            }
        }

        if members.len() > 1 {
            for &idx in &members {
                grouped[idx] = true;
            }
            group_indices.push(members);
        }

        if i % 128 == 0 || i == n - 1 {
            progress(i + 1, n, failed.load(Ordering::Relaxed), "");
        }
    }

    // Phase 3: Build DuplicateGroup structs from index lists
    let mut groups: Vec<DuplicateGroup> = group_indices
        .into_iter()
        .map(|idxs| {
            let mut files: Vec<ImageEntry> = idxs.into_iter()
                .map(|i| entries[i].clone())
                .collect();
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

    // === DEBUG: dump group details to stderr ===
    eprintln!("=== DEDUPE DEBUG: {} groups, threshold={}, rotation={} ===",
        groups.len(), threshold, rotation_mode);
    for (gi, group) in groups.iter().enumerate() {
        if group.files.len() > 1 {
            eprintln!("Group #{} ({} files, anchor_hash={:016x}):",
                gi, group.files.len(), group.hash);
            let anchor_rot = &group.files[0].rot_hashes;
            for f in &group.files {
                let d = (f.hash ^ group.hash).count_ones();
                let rot_info = if let (Some(ar), Some(fr)) = (anchor_rot, &f.rot_hashes) {
                    let rot_d = hasher::rotation_min_distance(ar, fr);
                    format!(" rot_d={}", rot_d)
                } else {
                    String::new()
                };
                eprintln!("  d={:2} hash={:016x} lv={} degen={}{} {}",
                    d, f.hash, f.low_variance,
                    hasher::is_degenerate_hash(f.hash), rot_info,
                    f.path.file_name().and_then(|n| n.to_str()).unwrap_or("?"));
            }
        }
    }
    eprintln!("=== END DEDUPE DEBUG ===");

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
