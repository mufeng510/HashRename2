//! no-replace 重命名的各平台实现与重试辅助。

use std::io;
use std::path::Path;
use std::time::Duration;

/// Linux:renameat2(RENAME_NOREPLACE)。内核 < 3.15 或文件系统不支持时回退到
/// "先 stat 再 rename"(存在极小 TOCTOU 窗口,见 README 已知限制)。
#[cfg(target_os = "linux")]
pub fn rename_no_replace_impl(from: &Path, to: &Path) -> io::Result<()> {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;

    let c_from = CString::new(from.as_os_str().as_bytes())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "路径包含 NUL 字节"))?;
    let c_to = CString::new(to.as_os_str().as_bytes())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "路径包含 NUL 字节"))?;

    let ret = unsafe {
        libc::renameat2(
            libc::AT_FDCWD,
            c_from.as_ptr(),
            libc::AT_FDCWD,
            c_to.as_ptr(),
            libc::RENAME_NOREPLACE,
        )
    };
    if ret == 0 {
        return Ok(());
    }
    let err = io::Error::last_os_error();
    match err.raw_os_error() {
        // ENOSYS:内核太老;EINVAL:文件系统/内核不支持该标志 → 回退
        Some(libc::ENOSYS) | Some(libc::EINVAL) => fallback_stat_then_rename(from, to),
        _ => Err(err),
    }
}

/// macOS:renamex_np(RENAME_EXCL)。不支持时回退。
#[cfg(target_os = "macos")]
pub fn rename_no_replace_impl(from: &Path, to: &Path) -> io::Result<()> {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;

    let c_from = CString::new(from.as_os_str().as_bytes())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "路径包含 NUL 字节"))?;
    let c_to = CString::new(to.as_os_str().as_bytes())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "路径包含 NUL 字节"))?;

    // Apple 平台的 renamex_np 签名为 (from, to, flags),无 attrlist 参数
    let ret = unsafe { libc::renamex_np(c_from.as_ptr(), c_to.as_ptr(), libc::RENAME_EXCL) };
    if ret == 0 {
        return Ok(());
    }
    let err = io::Error::last_os_error();
    match err.raw_os_error() {
        Some(libc::ENOSYS) | Some(libc::EINVAL) => fallback_stat_then_rename(from, to),
        _ => Err(err),
    }
}

/// Windows:MoveFileExW 不带 MOVEFILE_REPLACE_EXISTING,目标存在时失败。
#[cfg(windows)]
pub fn rename_no_replace_impl(from: &Path, to: &Path) -> io::Result<()> {
    fn to_wide(p: &Path) -> Vec<u16> {
        use std::os::windows::ffi::OsStrExt;
        let mut v: Vec<u16> = p.as_os_str().encode_wide().collect();
        v.push(0);
        v
    }
    let ret = unsafe {
        windows_sys::Win32::Storage::FileSystem::MoveFileExW(
            to_wide(from).as_ptr(),
            to_wide(to).as_ptr(),
            windows_sys::Win32::Storage::FileSystem::MOVEFILE_WRITE_THROUGH,
        )
    };
    if ret == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

/// 回退方案:先检查目标不存在再 rename(不使用会覆盖的语义之外的手段)。
fn fallback_stat_then_rename(from: &Path, to: &Path) -> io::Result<()> {
    if std::fs::symlink_metadata(to).is_ok() {
        return Err(io::Error::new(io::ErrorKind::AlreadyExists, "目标已存在"));
    }
    std::fs::rename(from, to)
}

/// 带简单重试的重命名:Windows 上文件可能被杀毒软件/索引服务短暂占用。
/// AlreadyExists / NotFound 不重试(那是真实状态,不是暂时锁)。
pub fn rename_with_retry(from: &Path, to: &Path) -> io::Result<()> {
    let mut last = match std::fs::rename(from, to) {
        Ok(()) => return Ok(()),
        Err(e) => e,
    };
    for _ in 0..2 {
        match last.kind() {
            io::ErrorKind::AlreadyExists | io::ErrorKind::NotFound => return Err(last),
            _ => {}
        }
        std::thread::sleep(Duration::from_millis(80));
        match std::fs::rename(from, to) {
            Ok(()) => return Ok(()),
            Err(e) => last = e,
        }
    }
    Err(last)
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    #[test]
    fn no_replace_rejects_existing_target() {
        let dir = std::env::temp_dir().join(format!(
            "hashrename_test_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .subsec_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let a = dir.join("a.txt");
        let b = dir.join("b.txt");
        std::fs::write(&a, b"aaa").unwrap();
        std::fs::write(&b, b"bbb").unwrap();
        let err = rename_no_replace_impl(&a, &b).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::AlreadyExists);
        // 内容都未被破坏
        assert_eq!(std::fs::read(&a).unwrap(), b"aaa");
        assert_eq!(std::fs::read(&b).unwrap(), b"bbb");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn no_replace_succeeds_when_free() {
        let dir = std::env::temp_dir().join(format!(
            "hashrename_test2_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .subsec_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let a = dir.join("x.txt");
        let b = dir.join("y.txt");
        std::fs::write(&a, b"data").unwrap();
        rename_no_replace_impl(&a, &b).unwrap();
        assert!(!a.exists());
        assert_eq!(std::fs::read(&b).unwrap(), b"data");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
