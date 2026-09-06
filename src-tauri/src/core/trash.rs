//! 回收站抽象(需求 §11):重复文件必须进入系统回收站,禁止永久删除。
//!
//! 后端使用 `trash` crate,其按平台调用系统机制:
//! - Windows:Recycle Bin(IFileOperation)
//! - macOS:Trash(NSWorkspace)
//! - Linux:FreeDesktop Trash(~/.local/share/Trash 或挂载点 .Trash-uid)
//!
//! Linux 环境没有可用 Trash 机制时返回错误(安全失败),绝不回退到 rm。

use crate::core::error::HrError;
use std::path::Path;

pub trait TrashProvider: Send + Sync {
    fn name(&self) -> &'static str;
    /// 将文件移入系统回收站。失败即报错,不得永久删除。
    fn send_to_trash(&self, path: &Path) -> Result<(), HrError>;
}

/// 系统回收站实现(trash crate 后端)。
pub struct OsTrash;

impl TrashProvider for OsTrash {
    fn name(&self) -> &'static str {
        "os-trash"
    }

    fn send_to_trash(&self, path: &Path) -> Result<(), HrError> {
        // 重试一次:桌面环境/索引服务可能短暂占用文件
        let mut last: Option<String> = None;
        for _ in 0..2 {
            match trash::delete(path) {
                Ok(()) => return Ok(()),
                Err(e) => last = Some(e.to_string()),
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        Err(HrError::Trash {
            path: path.display().to_string(),
            message: last.unwrap_or_else(|| "未知错误".to_string()),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trash_moves_file_not_delete() {
        let t = OsTrash;
        let dir = std::env::temp_dir().join(format!(
            "hr_trash_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .subsec_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("trash_me.txt");
        std::fs::write(&p, b"recoverable content").unwrap();

        if let Err(e) = t.send_to_trash(&p) {
            // 环境无回收站机制(如某些最小化容器):原文件必须仍在 → 跳过
            assert!(p.exists(), "回收站操作失败时原文件必须保留(安全失败): {e}");
            eprintln!("环境无可用回收站,跳过该测试: {e}");
            std::fs::remove_dir_all(&dir).unwrap();
            return;
        }
        assert!(!p.exists(), "原文件应已移出");

        // Linux 上可直接验证进入回收站且内容完好(未永久删除)
        #[cfg(target_os = "linux")]
        {
            let home = std::path::PathBuf::from(
                std::env::var("HOME").unwrap_or_else(|_| "/tmp".to_string()),
            );
            let trash_files = home.join(".local/share/Trash/files");
            let mut found = false;
            if let Ok(rd) = std::fs::read_dir(&trash_files) {
                for e in rd.flatten() {
                    if std::fs::read(e.path())
                        .map(|c| c == b"recoverable content")
                        .unwrap_or(false)
                    {
                        found = true;
                        let _ = trash::delete(e.path()); // 清理测试垃圾
                        break;
                    }
                }
            }
            assert!(found, "文件应可在系统回收站中找到(内容一致)");
        }

        std::fs::remove_dir_all(&dir).unwrap();
    }
}
