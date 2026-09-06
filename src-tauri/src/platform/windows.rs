//! Windows 平台支持:Explorer 文件夹右键菜单(HKCU 注册表)、
//! 控制台附加、消息框。

use crate::core::error::HrError;
use winreg::enums::HKEY_CURRENT_USER;
use winreg::RegKey;

const MENU_LABEL: &str = "Hash 去重并重命名";
const DIR_KEY: &str = r"Software\Classes\Directory\shell\HashRename";
const BG_KEY: &str = r"Software\Classes\Directory\Background\shell\HashRename";

fn exe_path() -> Result<String, HrError> {
    match std::env::current_exe() {
        Ok(p) => Ok(p.display().to_string()),
        Err(e) => Err(HrError::Other(format!("无法确定程序路径: {e}"))),
    }
}

pub fn install_context_menu() -> Result<Vec<String>, HrError> {
    let exe = exe_path()?;
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    for base in [DIR_KEY, BG_KEY] {
        let (key, _) = hkcu
            .create_subkey(base)
            .map_err(|e| HrError::Other(format!("写入注册表失败({base}): {e}")))?;
        key.set_value("", &MENU_LABEL)
            .map_err(|e| HrError::Other(format!("写入注册表值失败({base}): {e}")))?;
        key.set_value("Icon", &exe)
            .map_err(|e| HrError::Other(format!("写入注册表值失败({base}): {e}")))?;
        let (cmd, _) = hkcu
            .create_subkey(format!("{base}\\command"))
            .map_err(|e| HrError::Other(format!("写入注册表失败({base}\\command): {e}")))?;
        cmd.set_value("", &format!("\"{exe}\" \"%V\""))
            .map_err(|e| HrError::Other(format!("写入注册表值失败({base}\\command): {e}")))?;
    }
    Ok(vec![
        "Explorer 文件夹右键菜单已安装(当前用户)".to_string(),
        "右键任意文件夹 → Hash 去重并重命名".to_string(),
        format!("命令: \"{exe}\" \"%V\""),
    ])
}

pub fn uninstall_context_menu() -> Result<Vec<String>, HrError> {
    let mut report = Vec::new();
    for base in [DIR_KEY, BG_KEY] {
        delete_tree(base);
        report.push(format!("注册表项已删除: HKCU\\{base}"));
    }
    Ok(report)
}

/// RegDeleteTreeW:删除键及其全部子键。键不存在时静默忽略(幂等卸载)。
fn delete_tree(subkey: &str) {
    use windows_sys::Win32::System::Registry::{RegDeleteTreeW, HKEY_CURRENT_USER};
    let wide: Vec<u16> = subkey.encode_utf16().chain(std::iter::once(0)).collect();
    unsafe {
        RegDeleteTreeW(HKEY_CURRENT_USER, wide.as_ptr());
    }
}

/// 无控制台场景(双击运行 --install-context-menu)的结果展示。
pub fn show_message(title: &str, text: &str) {
    fn to_wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }
    use windows_sys::Win32::UI::WindowsAndMessaging::{MessageBoxW, MB_ICONINFORMATION, MB_OK};
    unsafe {
        MessageBoxW(
            std::ptr::null_mut(),
            to_wide(text).as_ptr(),
            to_wide(title).as_ptr(),
            MB_OK | MB_ICONINFORMATION,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_paths_are_consistent() {
        assert!(DIR_KEY.starts_with(r"Software\Classes\Directory"));
        assert!(BG_KEY.contains("Background"));
    }
}
