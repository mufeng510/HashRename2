//! 重复检测(需求 §7/§8/§9/§10)。
//!
//! 流程:文件大小分组 → 同大小组计算哈希(由 processor 完成)→
//! (大小, 哈希) 分组 → **逐字节二次验证** → 确定真正重复组 →
//! 按自然排序选择保留文件。
//!
//! 逐字节验证确保即使发生理论上的 MD5 碰撞,不同内容的文件也
//! 不会被误删(需求 §9/§31)。

use crate::core::error::FileError;
use crate::core::models::{DuplicateGroup, FileEntry, HashValue};
use crate::core::progress::Progress;
use crate::core::sorter;
use std::collections::BTreeMap;
use std::io::Read;
use std::path::Path;

#[derive(Debug, Default)]
pub struct Detection {
    pub groups: Vec<DuplicateGroup>,
    pub errors: Vec<FileError>,
}

/// 内容验证缓冲区大小。
const VERIFY_BUFFER_SIZE: usize = 1024 * 1024;

/// 逐字节比较两个文件内容。任一文件读取失败即返回错误。
pub fn files_equal(a: &Path, b: &Path) -> std::io::Result<bool> {
    let mut fa = std::fs::File::open(a)?;
    let mut fb = std::fs::File::open(b)?;
    let mut ba = vec![0u8; VERIFY_BUFFER_SIZE];
    let mut bb = vec![0u8; VERIFY_BUFFER_SIZE];
    loop {
        let na = read_full(&mut fa, &mut ba)?;
        let nb = read_full(&mut fb, &mut bb)?;
        if na != nb {
            return Ok(false);
        }
        if na == 0 {
            return Ok(true);
        }
        if ba[..na] != bb[..nb] {
            return Ok(false);
        }
    }
}

/// 尽量读满缓冲区(避免 short-read 导致误判)。
fn read_full(f: &mut std::fs::File, buf: &mut [u8]) -> std::io::Result<usize> {
    let mut n = 0;
    while n < buf.len() {
        match f.read(&mut buf[n..]) {
            Ok(0) => break,
            Ok(k) => n += k,
            Err(ref e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e),
        }
    }
    Ok(n)
}

/// 从已算好哈希的文件中检测重复组。
///
/// `hashes[i]` 为 `entries[i]` 的哈希;None 表示未参与哈希(不需要比较,
/// 或哈希失败——失败的文件视为"无法判定",一律保留,绝不误删)。
pub fn detect_duplicates(
    entries: &[FileEntry],
    hashes: &[Option<HashValue>],
    progress: &Progress,
) -> Detection {
    let mut detection = Detection::default();

    // 1. 按大小分组(需求 §8:第一层预筛选)
    let mut by_size: BTreeMap<u64, Vec<usize>> = BTreeMap::new();
    for (i, e) in entries.iter().enumerate() {
        by_size.entry(e.size).or_default().push(i);
    }

    // 2. 大小相同的组按 (大小, 哈希) 再分组;大小唯一的文件不可能重复
    let mut by_hash: BTreeMap<(u64, String), Vec<usize>> = BTreeMap::new();
    for (size, idxs) in &by_size {
        if idxs.len() < 2 {
            continue;
        }
        for &i in idxs {
            match &hashes[i] {
                Some(h) => by_hash.entry((*size, h.hex())).or_default().push(i),
                None => { /* 未哈希(失败/取消):不参与重复判定,保留 */ }
            }
        }
    }

    // 3. 对每个 (大小, 哈希) 多成员组做逐字节验证聚类
    let total_groups = by_hash.values().filter(|v| v.len() > 1).count();
    let mut processed = 0usize;

    for ((size, hash_hex), idxs) in &by_hash {
        if idxs.len() < 2 {
            continue;
        }
        if progress.cancelled() {
            return detection;
        }

        // 聚类:与每个已确认代表比较;内容一致 → 并入该簇,否则自成代表。
        // 这样即使哈希碰撞(内容不同),也只是拆成多簇,不会误删。
        let mut clusters: Vec<Vec<usize>> = Vec::new();
        'member: for &i in idxs {
            for c in clusters.iter_mut() {
                match files_equal(&entries[c[0]].path, &entries[i].path) {
                    Ok(true) => {
                        c.push(i);
                        continue 'member;
                    }
                    Ok(false) => continue,
                    Err(e) => {
                        // 无法读取验证:该文件视为"无法判定",保留,记录错误
                        detection.errors.push(FileError::new(
                            "verify",
                            &entries[i].path,
                            format!("内容二次验证失败: {e}"),
                        ));
                        continue 'member;
                    }
                }
            }
            clusters.push(vec![i]);
        }

        for cluster in clusters {
            if cluster.len() < 2 {
                continue;
            }
            // 保留规则(需求 §10):自然排序最靠前者保留。
            // entries 已按自然排序,cluster 内 index 越小越靠前。
            let mut members = cluster;
            members.sort_unstable();
            let keep_idx = members[0];
            let duplicates: Vec<FileEntry> =
                members[1..].iter().map(|&i| entries[i].clone()).collect();
            detection.groups.push(DuplicateGroup {
                size: *size,
                hash: HashValue(
                    hash_hex
                        .as_bytes()
                        .chunks(2)
                        .filter_map(|p| {
                            u8::from_str_radix(std::str::from_utf8(p).unwrap_or("0"), 16).ok()
                        })
                        .collect(),
                ),
                keep: entries[keep_idx].clone(),
                duplicates,
            });
        }

        processed += 1;
        progress.tick_range(processed as f64 / total_groups.max(1) as f64, 0.63, 0.71);
    }

    // 稳定输出:组按 (大小, 哈希) 字典序,天然确定
    detection.groups.sort_by(|a, b| {
        a.size
            .cmp(&b.size)
            .then_with(|| a.hash.hex().cmp(&b.hash.hex()))
    });
    // keep 内部再确保自然序(理论冗余,防御性)
    for g in &mut detection.groups {
        sorter::sort_names(&mut g.duplicates, |f| &f.file_name);
    }

    detection
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::hasher::{compute_hashes, Md5Hasher};
    use std::sync::Arc;

    fn tmpdir(tag: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!(
            "hr_det_{tag}_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .subsec_nanos()
        ));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn setup(dir: &Path, files: &[(&str, &[u8])]) -> (Vec<FileEntry>, Vec<Option<HashValue>>) {
        let mut entries: Vec<FileEntry> = Vec::new();
        for (name, content) in files {
            let p = dir.join(name);
            std::fs::write(&p, content).unwrap();
            entries.push(FileEntry {
                path: p,
                file_name: name.to_string(),
                extension: None,
                size: content.len() as u64,
                hash: None,
            });
        }
        sorter::sort_names(&mut entries, |f| &f.file_name);
        let needed: Vec<bool> = entries.iter().map(|_| true).collect();
        let hashes = compute_hashes(
            &entries,
            &needed,
            Arc::new(Md5Hasher),
            2,
            &Progress::default(),
        );
        let hashes = hashes
            .into_iter()
            .map(|r| r.map(|x| x.ok().unwrap()))
            .collect();
        (entries, hashes)
    }

    #[test]
    fn identical_content_across_extensions() {
        let dir = tmpdir("ext");
        // 相同内容、不同文件名/扩展名(需求 §7)
        let (entries, hashes) = setup(
            &dir,
            &[
                ("a.jpg", b"DATA1"),
                ("b.png", b"DATA1"),
                ("c.webp", b"OTHER"),
            ],
        );
        let d = detect_duplicates(&entries, &hashes, &Progress::default());
        assert_eq!(d.groups.len(), 1);
        assert_eq!(d.groups[0].keep.file_name, "a.jpg");
        assert_eq!(d.groups[0].duplicates.len(), 1);
        assert_eq!(d.groups[0].duplicates[0].file_name, "b.png");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn same_size_different_content_not_duplicates() {
        let dir = tmpdir("samesize");
        let (entries, hashes) = setup(&dir, &[("x.bin", b"AAAA"), ("y.bin", b"BBBB")]);
        let d = detect_duplicates(&entries, &hashes, &Progress::default());
        assert!(d.groups.is_empty());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn different_sizes_never_compared() {
        let dir = tmpdir("sizes");
        let (entries, hashes) = setup(&dir, &[("a", b"same"), ("b", b"same!")]);
        let d = detect_duplicates(&entries, &hashes, &Progress::default());
        assert!(d.groups.is_empty());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// 模拟 MD5 碰撞:恶意 Hasher 对所有文件返回相同摘要。
    /// 二次验证必须阻止误删(需求 §9)。
    struct CollidingHasher;
    impl crate::core::hasher::Hasher for CollidingHasher {
        fn algorithm(&self) -> &'static str {
            "colliding"
        }
        fn hash_reader(&self, _r: &mut dyn std::io::Read) -> std::io::Result<HashValue> {
            Ok(HashValue(vec![0xAB; 16]))
        }
    }

    #[test]
    fn md5_collision_guarded_by_byte_verify() {
        let dir = tmpdir("collide");
        let mut entries: Vec<FileEntry> = Vec::new();
        for (name, content) in [
            ("f1.bin", &b"CONTENT-ONE"[..]),
            ("f2.bin", &b"CONTENT-TWO"[..]),
            ("f3.bin", &b"CONTENT-ONE"[..]),
        ] {
            let p = dir.join(name);
            std::fs::write(&p, content).unwrap();
            entries.push(FileEntry {
                path: p,
                file_name: name.to_string(),
                extension: None,
                size: content.len() as u64,
                hash: None,
            });
        }
        sorter::sort_names(&mut entries, |f| &f.file_name);
        let needed: Vec<bool> = entries.iter().map(|_| true).collect();
        let hashes = compute_hashes(
            &entries,
            &needed,
            Arc::new(CollidingHasher),
            2,
            &Progress::default(),
        );
        let hashes: Vec<Option<HashValue>> =
            hashes.into_iter().map(|r| r.and_then(|x| x.ok())).collect();

        let d = detect_duplicates(&entries, &hashes, &Progress::default());
        // f1/f3 内容相同 → 1 组;f2 内容不同 → 绝不能进组
        assert_eq!(d.groups.len(), 1);
        assert_eq!(d.groups[0].keep.file_name, "f1.bin");
        assert_eq!(d.groups[0].duplicates.len(), 1);
        assert_eq!(d.groups[0].duplicates[0].file_name, "f3.bin");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn keep_is_natural_sort_first() {
        let dir = tmpdir("keep");
        let (entries, hashes) = setup(
            &dir,
            &[
                ("IMG_2.jpg", b"same"),
                ("IMG_10.jpg", b"same"),
                ("IMG_1.jpg", b"same"),
            ],
        );
        let d = detect_duplicates(&entries, &hashes, &Progress::default());
        assert_eq!(d.groups.len(), 1);
        assert_eq!(d.groups[0].keep.file_name, "IMG_1.jpg");
        let dup_names: Vec<&str> = d.groups[0]
            .duplicates
            .iter()
            .map(|f| f.file_name.as_str())
            .collect();
        assert_eq!(dup_names, vec!["IMG_2.jpg", "IMG_10.jpg"]);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn empty_files_all_duplicates_of_each_other() {
        let dir = tmpdir("empty");
        let (entries, hashes) = setup(&dir, &[("e1", b""), ("e2", b""), ("e3", b"")]);
        let d = detect_duplicates(&entries, &hashes, &Progress::default());
        assert_eq!(d.groups.len(), 1);
        assert_eq!(d.groups[0].duplicates.len(), 2);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn files_equal_detects_difference() {
        let dir = tmpdir("cmp");
        let a = dir.join("a.bin");
        let b = dir.join("b.bin");
        std::fs::write(&a, vec![1u8; 3 * VERIFY_BUFFER_SIZE + 7]).unwrap();
        std::fs::write(&b, vec![1u8; 3 * VERIFY_BUFFER_SIZE + 7]).unwrap();
        assert!(files_equal(&a, &b).unwrap());
        let mut data = vec![1u8; 3 * VERIFY_BUFFER_SIZE + 7];
        data[3 * VERIFY_BUFFER_SIZE + 3] = 2;
        std::fs::write(&b, &data).unwrap();
        assert!(!files_equal(&a, &b).unwrap());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
