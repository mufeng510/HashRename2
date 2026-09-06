//! 统一错误类型。

use serde::Serialize;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum HrError {
    #[error("无法访问目录 {path}: {source}")]
    Directory {
        path: String,
        source: std::io::Error,
    },

    #[error("目录正在被另一个 HashRename 任务处理(锁文件:{path})")]
    Locked { path: String },

    #[error("{operation} 失败 ({path}): {source}")]
    Io {
        operation: &'static str,
        path: String,
        source: std::io::Error,
    },

    #[error("无法移动到回收站 ({path}): {message}")]
    Trash { path: String, message: String },

    #[error("重命名冲突 ({path}): {message}")]
    Conflict { path: String, message: String },

    #[error("任务已取消")]
    Cancelled,

    #[error("{0}")]
    Other(String),
}

impl HrError {
    pub fn io(op: &'static str, path: impl AsRef<std::path::Path>, e: std::io::Error) -> Self {
        HrError::Io {
            operation: op,
            path: path.as_ref().display().to_string(),
            source: e,
        }
    }
}

/// 单个文件级别的一次性错误(可序列化,用于结果展示)。
#[derive(Debug, Clone, Serialize)]
pub struct FileError {
    /// 出错文件的路径(截断过长路径仅用于展示)。
    pub path: String,
    /// 操作类别:scan / hash / verify / trash / rename / recover
    pub operation: String,
    pub message: String,
}

impl FileError {
    pub fn new(
        operation: &str,
        path: impl AsRef<std::path::Path>,
        message: impl Into<String>,
    ) -> Self {
        FileError {
            path: path.as_ref().display().to_string(),
            operation: operation.to_string(),
            message: message.into(),
        }
    }
}

impl From<&HrError> for String {
    fn from(e: &HrError) -> String {
        e.to_string()
    }
}
