//! 平台相关的底层能力:no-replace 重命名、进程存活检测、控制台附加等。

pub mod fsutil;

#[cfg(target_os = "linux")]
pub mod linux;
#[cfg(target_os = "macos")]
pub mod macos;
#[cfg(windows)]
pub mod windows;

/// 内部文件(临时文件/journal/锁文件)统一前缀。扫描时排除。
pub const INTERNAL_PREFIX: &str = ".hashrename";

/// 文件系统对文件名是否大小写敏感(用于冲突检测)。
/// Windows 与 macOS 的默认文件系统均为大小写不敏感。
pub fn case_sensitive_fs() -> bool {
    cfg!(target_os = "linux")
}

/// 当前进程 ID。
pub fn current_pid() -> u32 {
    std::process::id()
}

/// 判断目标名是否与已占用名冲突(根据平台大小写敏感性)。
pub fn names_conflict(a: &str, b: &str) -> bool {
    if case_sensitive_fs() {
        a == b
    } else {
        a.eq_ignore_ascii_case(b)
    }
}

/// 以不覆盖已有文件的方式重命名。目标已存在时返回 AlreadyExists 错误,
/// 绝不覆盖用户文件(需求 §30)。
pub fn rename_no_replace(from: &std::path::Path, to: &std::path::Path) -> std::io::Result<()> {
    fsutil::rename_no_replace_impl(from, to)
}

/// 带简单重试的重命名(Windows 上杀毒软件/索引服务可能短暂锁定文件)。
pub fn rename_with_retry(from: &std::path::Path, to: &std::path::Path) -> std::io::Result<()> {
    fsutil::rename_with_retry(from, to)
}

/// 判断进程是否存活(用于目录锁的陈旧检测)。
pub fn pid_alive(pid: u32) -> bool {
    #[cfg(unix)]
    {
        // kill(pid, 0):0 表示仅探测。ESRCH = 不存在;EPERM = 存在但属其他用户。
        let r = unsafe { libc::kill(pid as libc::pid_t, 0) };
        if r == 0 {
            return true;
        }
        std::io::Error::last_os_error().raw_os_error() != Some(libc::ESRCH)
    }
    #[cfg(windows)]
    {
        use windows_sys::Win32::Foundation::CloseHandle;
        use windows_sys::Win32::System::Threading::{
            OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
        };
        unsafe {
            let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
            if h.is_null() {
                return false;
            }
            CloseHandle(h);
            true
        }
    }
}

/// Windows 下在 GUI 子系统二进制中重新附加父进程控制台,
/// 使 CLI 模式(`hashrename <dir>`)能正常输出。其他平台为空操作。
pub fn attach_parent_console() {
    #[cfg(windows)]
    unsafe {
        use windows_sys::Win32::System::Console::{AttachConsole, ATTACH_PARENT_PROCESS};
        AttachConsole(ATTACH_PARENT_PROCESS);
    }
    #[cfg(not(windows))]
    {}
}

/// 生成一次运行的随机 token(用于临时文件/journal 命名,避免多实例冲突)。
pub fn gen_token() -> String {
    use std::hash::{BuildHasher, Hasher};
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let mut h1 = std::collections::hash_map::RandomState::new().build_hasher();
    h1.write_u128(nanos);
    let a = h1.finish();
    let mut h2 = std::collections::hash_map::RandomState::new().build_hasher();
    h2.write_u64(a);
    let b = h2.finish();
    format!("{a:016x}{b:016x}")
}

/// 右键菜单安装(平台各自实现)。
pub fn install_context_menu() -> Result<Vec<String>, crate::core::error::HrError> {
    #[cfg(windows)]
    return windows::install_context_menu();
    #[cfg(target_os = "macos")]
    return macos::install_context_menu();
    #[cfg(target_os = "linux")]
    return linux::install_context_menu();
}

/// 右键菜单卸载(平台各自实现)。
pub fn uninstall_context_menu() -> Result<Vec<String>, crate::core::error::HrError> {
    #[cfg(windows)]
    return windows::uninstall_context_menu();
    #[cfg(target_os = "macos")]
    return macos::uninstall_context_menu();
    #[cfg(target_os = "linux")]
    return linux::uninstall_context_menu();
}
