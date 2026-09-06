//! 目录扫描(需求 §5/§6/§18)。
//!
//! - 只扫描当前目录,**绝不递归**子目录;
//! - 只处理普通文件;子目录、符号链接(无论指向目录还是文件)、
//!   其他特殊文件全部跳过;
//! - HashRename 自身的内部文件(临时文件/journal/锁,前缀 `.hashrename`)
//!   一律排除;
//! - 扫描快照确定后,运行期间目录的新增变化不再纳入(需求 §18)。

use crate::core::error::{FileError, HrError};
use crate::core::models::FileEntry;
use crate::core::sorter;
use crate::platform::INTERNAL_PREFIX;
use std::path::{Path, PathBuf};

#[derive(Debug, Default)]
pub struct ScanResult {
    pub files: Vec<FileEntry>,
    /// 跳过的条目(用于冲突检测与展示)。
    pub skipped: Vec<SkippedEntry>,
    /// 内部文件数量(临时/journal/锁等)。
    pub internal_count: usize,
    /// 扫描阶段记录的错误(如 read_dir 迭代失败)。
    pub errors: Vec<FileError>,
}

#[derive(Debug, Clone)]
pub struct SkippedEntry {
    pub path: PathBuf,
    /// "directory" | "symlink" | "special"
    pub kind: String,
}

/// 扫描目录快照(不递归)。
pub fn scan_directory(dir: &Path) -> Result<ScanResult, HrError> {
    let rd = std::fs::read_dir(dir).map_err(|e| HrError::Directory {
        path: dir.display().to_string(),
        source: e,
    })?;

    let mut result = ScanResult::default();

    for entry in rd {
        let entry = match entry {
            Ok(e) => e,
            Err(e) => {
                result.errors.push(FileError::new(
                    "scan",
                    dir,
                    format!("读取目录条目失败: {e}"),
                ));
                continue;
            }
        };

        let name = entry.file_name();
        let name_str = name.to_string_lossy().to_string();

        // 内部文件:.hashrename_* 一律跳过且不计数为 skipped
        if name_str.starts_with(INTERNAL_PREFIX) {
            result.internal_count += 1;
            continue;
        }

        // file_type() 不跟随符号链接,能正确识别 symlink/目录/特殊文件
        let ft = match entry.file_type() {
            Ok(ft) => ft,
            Err(e) => {
                result.errors.push(FileError::new(
                    "scan",
                    entry.path(),
                    format!("无法识别文件类型: {e}"),
                ));
                continue;
            }
        };

        if ft.is_symlink() {
            result.skipped.push(SkippedEntry {
                path: entry.path(),
                kind: "symlink".to_string(),
            });
            continue;
        }
        if ft.is_dir() {
            result.skipped.push(SkippedEntry {
                path: entry.path(),
                kind: "directory".to_string(),
            });
            continue;
        }
        if !ft.is_file() {
            result.skipped.push(SkippedEntry {
                path: entry.path(),
                kind: "special".to_string(),
            });
            continue;
        }

        // DirEntry::metadata 不跟随符号链接(此处已排除 symlink,均为普通文件)
        let meta = match entry.metadata() {
            Ok(m) => m,
            Err(e) => {
                // 文件可能在扫描瞬间被删除(需求 §18):记录并跳过
                result.errors.push(FileError::new(
                    "scan",
                    entry.path(),
                    format!("无法获取文件信息: {e}"),
                ));
                continue;
            }
        };

        let path = entry.path();
        let extension = path.extension().map(|e| e.to_string_lossy().to_string());
        result.files.push(FileEntry {
            path,
            file_name: name_str,
            extension,
            size: meta.len(),
            hash: None,
        });
    }

    // 扫描后立即按自然排序,保证后续各阶段的确定性
    sorter::sort_names(&mut result.files, |f| &f.file_name);

    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmpdir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!(
            "hr_scan_{tag}_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .subsec_nanos()
        ));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn scans_only_top_level_regular_files() {
        let dir = tmpdir("top");
        std::fs::write(dir.join("a.jpg"), b"1").unwrap();
        std::fs::write(dir.join("b.txt"), b"22").unwrap();
        std::fs::create_dir(dir.join("sub")).unwrap();
        std::fs::write(dir.join("sub").join("c.jpg"), b"333").unwrap();
        std::fs::write(dir.join(".hashrename_tmp_999_deadbeef_1"), b"x").unwrap();
        std::fs::write(dir.join(".hashrename.lock"), b"y").unwrap();
        // 隐藏文件(非内部前缀)应参与处理
        std::fs::write(dir.join(".hidden"), b"z").unwrap();

        let r = scan_directory(&dir).unwrap();
        let mut names: Vec<&str> = r.files.iter().map(|f| f.file_name.as_str()).collect();
        names.sort();
        assert_eq!(names, vec![".hidden", "a.jpg", "b.txt"]);
        assert_eq!(r.skipped.len(), 1);
        assert_eq!(r.skipped[0].kind, "directory");
        assert_eq!(r.internal_count, 2);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn symlinks_are_skipped() {
        let dir = tmpdir("sym");
        let target = tmpdir("sym_target");
        std::fs::write(target.join("real.txt"), b"hello").unwrap();
        // 指向目录的符号链接
        std::os::unix::fs::symlink(&target, dir.join("dirlink")).unwrap();
        // 指向文件的符号链接
        std::os::unix::fs::symlink(target.join("real.txt"), dir.join("filelink.txt")).unwrap();
        std::fs::write(dir.join("normal.txt"), b"x").unwrap();

        let r = scan_directory(&dir).unwrap();
        assert_eq!(r.files.len(), 1);
        assert_eq!(r.files[0].file_name, "normal.txt");
        let kinds: Vec<&str> = r.skipped.iter().map(|s| s.kind.as_str()).collect();
        assert!(kinds.contains(&"symlink"));
        assert_eq!(r.skipped.len(), 2);
        std::fs::remove_dir_all(&dir).unwrap();
        std::fs::remove_dir_all(&target).unwrap();
    }

    #[test]
    fn entries_sorted_naturally() {
        let dir = tmpdir("sorted");
        for n in ["10.jpg", "2.jpg", "1.jpg"] {
            std::fs::write(dir.join(n), b"x").unwrap();
        }
        let r = scan_directory(&dir).unwrap();
        let names: Vec<&str> = r.files.iter().map(|f| f.file_name.as_str()).collect();
        assert_eq!(names, vec!["1.jpg", "2.jpg", "10.jpg"]);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn extension_preserved() {
        let dir = tmpdir("ext");
        std::fs::write(dir.join("a.JPG"), b"x").unwrap();
        std::fs::write(dir.join("b"), b"x").unwrap();
        std::fs::write(dir.join("c.tar.gz"), b"x").unwrap();
        let r = scan_directory(&dir).unwrap();
        let by_name = |n: &str| {
            r.files
                .iter()
                .find(|f| f.file_name == n)
                .unwrap()
                .extension
                .clone()
        };
        assert_eq!(by_name("a.JPG"), Some("JPG".to_string()));
        assert_eq!(by_name("b"), None);
        assert_eq!(by_name("c.tar.gz"), Some("gz".to_string()));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn nonexistent_dir_is_error() {
        assert!(scan_directory(Path::new("/nonexistent/hashrename/definitely/not/here")).is_err());
    }
}
