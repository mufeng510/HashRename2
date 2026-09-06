//! Hasher 抽象(需求 §39)与并行计算。
//!
//! 第一版默认 MD5;架构上允许未来替换为 SHA-256 等(见 `Sha256Hasher`)。

use crate::core::error::{FileError, HrError};
use crate::core::models::{FileEntry, HashValue};
use crate::core::progress::Progress;
use std::io::{BufReader, Read};
use std::path::Path;
use std::sync::Arc;
/// 每次读取的缓冲区大小(流式哈希,不把整个文件读入内存)。
pub const HASH_BUFFER_SIZE: usize = 256 * 1024;

/// 哈希算法抽象。实现必须流式读取,不得将整个文件载入内存。
pub trait Hasher: Send + Sync {
    /// 算法名称(如 "md5")。
    fn algorithm(&self) -> &'static str;

    /// 从任意 Reader 计算哈希。
    fn hash_reader(&self, r: &mut dyn Read) -> std::io::Result<HashValue>;

    /// 对文件流式计算哈希。
    fn hash_file(&self, path: &Path) -> Result<HashValue, HrError> {
        let f = std::fs::File::open(path).map_err(|e| HrError::io("hash-open", path, e))?;
        let mut br = BufReader::with_capacity(HASH_BUFFER_SIZE, f);
        self.hash_reader(&mut br)
            .map_err(|e| HrError::io("hash-read", path, e))
    }
}

/// MD5(第一版默认算法)。
pub struct Md5Hasher;

impl Hasher for Md5Hasher {
    fn algorithm(&self) -> &'static str {
        "md5"
    }

    fn hash_reader(&self, r: &mut dyn Read) -> std::io::Result<HashValue> {
        use md5::Digest;
        let mut hasher = md5::Md5::new();
        let mut buf = vec![0u8; HASH_BUFFER_SIZE];
        loop {
            let n = r.read(&mut buf)?;
            if n == 0 {
                break;
            }
            hasher.update(&buf[..n]);
        }
        Ok(HashValue(hasher.finalize().to_vec()))
    }
}

/// SHA-256(证明 Hasher 抽象可扩展,第一版未在业务中启用)。
pub struct Sha256Hasher;

impl Hasher for Sha256Hasher {
    fn algorithm(&self) -> &'static str {
        "sha256"
    }

    fn hash_reader(&self, r: &mut dyn Read) -> std::io::Result<HashValue> {
        use sha2::Digest;
        let mut hasher = sha2::Sha256::new();
        let mut buf = vec![0u8; HASH_BUFFER_SIZE];
        loop {
            let n = r.read(&mut buf)?;
            if n == 0 {
                break;
            }
            hasher.update(&buf[..n]);
        }
        Ok(HashValue(hasher.finalize().to_vec()))
    }
}

/// 并行哈希线程数上限:避免磁盘 IO 过载与句柄暴涨。
pub fn default_thread_count() -> usize {
    std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4)
        .clamp(1, 8)
}

/// 对 `needed[i] == true` 的文件并行计算哈希(流式读取,限制线程数)。
/// 返回与 `entries` 等长的结果向量;不需要哈希的位置为 None。
/// 单个文件失败不影响其他文件(错误记录在返回值中)。
pub fn compute_hashes(
    entries: &[FileEntry],
    needed: &[bool],
    hasher: Arc<dyn Hasher>,
    threads: usize,
    progress: &Progress,
) -> Vec<Option<Result<HashValue, FileError>>> {
    let n = entries.len();
    let mut results: Vec<Option<Result<HashValue, FileError>>> = vec![None; n];

    let indices: Vec<usize> = (0..n).filter(|&i| needed[i]).collect();
    let total = indices.len();
    if total == 0 {
        return results;
    }

    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(threads.max(1))
        .build()
        .expect("创建线程池失败");
    let done = Arc::new(std::sync::atomic::AtomicUsize::new(0));

    let computed: Vec<(usize, Result<HashValue, FileError>)> = pool.install(|| {
        use rayon::prelude::*;
        indices
            .into_par_iter()
            .map(|i| {
                if progress.cancelled() {
                    return (
                        i,
                        Err(FileError::new("hash", &entries[i].path, "任务已取消")),
                    );
                }
                let r = match hasher.hash_file(&entries[i].path) {
                    Ok(h) => Ok(h),
                    Err(e) => Err(FileError::new(
                        "hash",
                        &entries[i].path,
                        format!("无法读取文件进行哈希: {e}"),
                    )),
                };
                let d = done.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
                progress.current_file(&entries[i].file_name);
                progress.tick_range(d as f64 / total as f64, 0.03, 0.63);
                (i, r)
            })
            .collect()
    });

    for (i, r) in computed {
        results[i] = Some(r);
    }
    results
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn md5_known_vectors() {
        let h = Md5Hasher;
        let dir = std::env::temp_dir().join(format!("hr_hash_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();

        // 空文件 → d41d8cd98f00b204e9800998ecf8427e
        let p = dir.join("empty.bin");
        std::fs::write(&p, b"").unwrap();
        assert_eq!(
            h.hash_file(&p).unwrap().hex(),
            "d41d8cd98f00b204e9800998ecf8427e"
        );

        // "abc" → 900150983cd24fb0d6963f7d28e17f72(RFC 1321 测试向量)
        let p = dir.join("abc.bin");
        std::fs::write(&p, b"abc").unwrap();
        assert_eq!(
            h.hash_file(&p).unwrap().hex(),
            "900150983cd24fb0d6963f7d28e17f72"
        );

        // 大文件(跨多个缓冲区块)与标准实现一致:重复 100 万次 "abc" 前 1MB
        let p = dir.join("big.bin");
        let chunk = b"0123456789abcdef";
        let mut data = Vec::with_capacity(1 << 20);
        while data.len() < 1 << 20 {
            data.extend_from_slice(chunk);
        }
        data.truncate(1 << 20);
        std::fs::write(&p, &data).unwrap();
        let streamed = h.hash_file(&p).unwrap().hex();
        // 用单次读取的 md5 交叉验证
        use md5::Digest;
        let mut m = md5::Md5::new();
        m.update(&data);
        assert_eq!(streamed, hex_of(&m.finalize()));

        std::fs::remove_dir_all(&dir).unwrap();
    }

    fn hex_of(d: &[u8]) -> String {
        d.iter().map(|b| format!("{b:02x}")).collect()
    }

    #[test]
    fn sha256_extensibility() {
        // 验证 Hasher trait 可扩展到其他算法(需求 §39)
        let h = Sha256Hasher;
        let dir = std::env::temp_dir().join(format!("hr_hash2_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("a.txt");
        std::fs::write(&p, b"abc").unwrap();
        let v = h.hash_file(&p).unwrap();
        assert_eq!(v.0.len(), 32);
        assert_eq!(h.algorithm(), "sha256");
        // SHA-256("abc") 前缀
        assert!(v.hex().starts_with("ba7816bf8f01cfea414140de5dae2223"));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn hash_reader_streaming_no_huge_alloc() {
        // 12MB 数据分块读取,确保流式逻辑正确
        let h = Md5Hasher;
        let data = vec![0xABu8; 12 * 1024 * 1024];
        let mut r: &[u8] = &data;
        let v1 = h.hash_reader(&mut r).unwrap();
        use md5::Digest;
        let mut m = md5::Md5::new();
        m.update(&data);
        assert_eq!(v1.hex(), hex_of(&m.finalize()));
    }
}
