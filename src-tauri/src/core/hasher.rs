//! Hasher 抽象(需求 §39)与并行计算。
//!
//! 支持的算法(可配置,默认 MD5):
//! - `md5`(RFC 1321,128 位)
//! - `sha256`(160 位安全性更高的密码学哈希,256 位)
//! - `xxh3`(XXH3-64,非加密但极快;碰撞防护由逐字节二次验证兜底)

use crate::core::error::{FileError, HrError};
use crate::core::models::{FileEntry, HashValue};
use crate::core::progress::Progress;
use std::io::{BufReader, Read};
use std::path::Path;
use std::sync::Arc;
/// 每次读取的缓冲区大小(流式哈希,不把整个文件读入内存)。
pub const HASH_BUFFER_SIZE: usize = 256 * 1024;

/// 可配置的哈希算法。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum HashAlgorithm {
    /// MD5(默认,需求第一版指定)。
    #[default]
    Md5,
    Sha256,
    Xxh3,
}

impl HashAlgorithm {
    pub fn as_str(&self) -> &'static str {
        match self {
            HashAlgorithm::Md5 => "md5",
            HashAlgorithm::Sha256 => "sha256",
            HashAlgorithm::Xxh3 => "xxh3",
        }
    }

    /// 解析用户输入;无法识别时返回 None(调用方报错,不静默回退)。
    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "md5" => Some(HashAlgorithm::Md5),
            "sha256" | "sha-256" => Some(HashAlgorithm::Sha256),
            "xxh3" | "xxhash" | "xxhash3" => Some(HashAlgorithm::Xxh3),
            _ => None,
        }
    }

    /// 全部算法(供 GUI 下拉框与帮助文本)。
    pub fn all() -> &'static [HashAlgorithm] {
        &[
            HashAlgorithm::Md5,
            HashAlgorithm::Sha256,
            HashAlgorithm::Xxh3,
        ]
    }

    pub fn create(self) -> Arc<dyn Hasher> {
        match self {
            HashAlgorithm::Md5 => Arc::new(Md5Hasher),
            HashAlgorithm::Sha256 => Arc::new(Sha256Hasher),
            HashAlgorithm::Xxh3 => Arc::new(Xxh3Hasher),
        }
    }
}

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

/// MD5(默认算法)。
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

/// SHA-256(密码学哈希,适合对碰撞敏感的场景)。
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

/// XXH3-64(非加密、极快;理论碰撞率高于 MD5,由逐字节二次验证兜底,
/// 见需求 §9 —— 安全性不受算法选择影响)。
pub struct Xxh3Hasher;

impl Hasher for Xxh3Hasher {
    fn algorithm(&self) -> &'static str {
        "xxh3"
    }

    fn hash_reader(&self, r: &mut dyn Read) -> std::io::Result<HashValue> {
        let mut hasher = xxhash_rust::xxh3::Xxh3::new();
        let mut buf = vec![0u8; HASH_BUFFER_SIZE];
        loop {
            let n = r.read(&mut buf)?;
            if n == 0 {
                break;
            }
            hasher.update(&buf[..n]);
        }
        Ok(HashValue(hasher.digest().to_le_bytes().to_vec()))
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

    #[test]
    fn xxh3_deterministic_and_size_differentiating() {
        let h = Xxh3Hasher;
        let dir = std::env::temp_dir().join(format!("hr_hash3_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let pa = dir.join("a.bin");
        let pb = dir.join("b.bin");
        std::fs::write(&pa, b"payload-1").unwrap();
        std::fs::write(&pb, b"payload-2").unwrap();
        let va = h.hash_file(&pa).unwrap();
        assert_eq!(va.0.len(), 8, "XXH3-64 摘要为 8 字节");
        // 确定性:同一内容重复哈希结果一致
        assert_eq!(h.hash_file(&pa).unwrap(), va);
        // 区分不同内容
        assert_ne!(h.hash_file(&pb).unwrap(), va);
        // 大文件(跨缓冲区块)确定性
        let pc = dir.join("big.bin");
        let data = vec![0x5Au8; 3 * HASH_BUFFER_SIZE + 17];
        std::fs::write(&pc, &data).unwrap();
        let v1 = h.hash_file(&pc).unwrap();
        let mut r: &[u8] = &data;
        assert_eq!(h.hash_reader(&mut r).unwrap(), v1);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn algorithm_parse_and_create() {
        // 解析:大小写不敏感,别名宽容,未知值必须报错而非静默回退
        assert_eq!(HashAlgorithm::parse("md5"), Some(HashAlgorithm::Md5));
        assert_eq!(HashAlgorithm::parse("MD5"), Some(HashAlgorithm::Md5));
        assert_eq!(HashAlgorithm::parse("sha256"), Some(HashAlgorithm::Sha256));
        assert_eq!(HashAlgorithm::parse("SHA-256"), Some(HashAlgorithm::Sha256));
        assert_eq!(HashAlgorithm::parse(" xxh3 "), Some(HashAlgorithm::Xxh3));
        assert_eq!(HashAlgorithm::parse("xxhash3"), Some(HashAlgorithm::Xxh3));
        assert_eq!(HashAlgorithm::parse("sha1"), None);
        assert_eq!(HashAlgorithm::parse(""), None);
        assert_eq!(HashAlgorithm::parse("md5x"), None);
        // 默认算法是 MD5(需求第一版指定)
        assert_eq!(HashAlgorithm::default(), HashAlgorithm::Md5);
        // all() 供 GUI 下拉框
        assert_eq!(HashAlgorithm::all().len(), 3);
        // 工厂方法产出对应算法
        assert_eq!(HashAlgorithm::Xxh3.create().algorithm(), "xxh3");
        assert_eq!(HashAlgorithm::Sha256.create().algorithm(), "sha256");
        assert_eq!(HashAlgorithm::Md5.create().algorithm(), "md5");
    }
}
