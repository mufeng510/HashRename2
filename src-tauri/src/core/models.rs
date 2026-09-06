//! 核心业务模型:FileEntry / DuplicateGroup / RenameOperation / ProcessingResult。

use serde::Serialize;
use std::fmt;
use std::path::PathBuf;

/// 哈希摘要(算法无关的字节向量,配合 `Hasher::algorithm` 使用)。
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct HashValue(pub Vec<u8>);

impl HashValue {
    pub fn hex(&self) -> String {
        self.0.iter().map(|b| format!("{b:02x}")).collect()
    }
}

impl fmt::Display for HashValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.hex())
    }
}

/// 目录中的单个普通文件。
#[derive(Debug, Clone)]
pub struct FileEntry {
    pub path: PathBuf,
    /// 文件名(含扩展名),用于排序与展示。
    pub file_name: String,
    /// 扩展名(不含点,保持原始大小写);无扩展名为 None。
    pub extension: Option<String>,
    pub size: u64,
    pub hash: Option<HashValue>,
}

/// 一组内容完全相同的文件(大小 + 哈希 + 逐字节验证均一致)。
#[derive(Debug, Clone)]
pub struct DuplicateGroup {
    pub size: u64,
    pub hash: HashValue,
    /// 自然排序最靠前、被保留的文件。
    pub keep: FileEntry,
    /// 重复文件(将被移入回收站)。
    pub duplicates: Vec<FileEntry>,
}

/// 处理结果统计(需求 §26)。
#[derive(Debug, Clone, Default, Serialize)]
pub struct ProcessingResult {
    pub directory: String,
    /// 扫描到的普通文件数量。
    pub scanned_count: usize,
    /// 跳过的条目(子目录/符号链接/特殊文件)数量。
    pub skipped_count: usize,
    /// 重复内容组数。
    pub duplicate_groups: usize,
    /// 检出为重复的文件数。
    pub duplicate_count: usize,
    /// 成功移入回收站的重复文件数。
    pub trashed_count: usize,
    /// 去重后保留(参与重命名阶段)的文件数。
    pub kept_count: usize,
    /// 成功重命名(含本来就已是目标名)的文件数。
    pub renamed_count: usize,
    /// 失败的文件操作数。
    pub failed_count: usize,
    pub cancelled: bool,
    /// 异常恢复:补完的最终重命名数。
    pub recovered_finals: usize,
    /// 异常恢复:还原的临时文件数。
    pub restored_temps: usize,
    /// 无法识别来源的遗留临时文件数(已排除在处理之外)。
    pub orphan_temps: usize,
    pub elapsed_ms: u64,
    pub errors: Vec<crate::core::error::FileError>,
    pub warnings: Vec<String>,
}

impl ProcessingResult {
    pub fn ok(&self) -> bool {
        !self.cancelled && self.failed_count == 0
    }
}
