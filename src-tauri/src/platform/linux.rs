//! Linux 文件管理器右键菜单集成(需求 §22)。
//!
//! 统一走 `install_context_menu()` 入口,为检测到的文件管理器分别安装:
//! - Nautilus:用户脚本 `~/.local/share/nautilus/scripts/`
//! - Dolphin: 服务菜单 `~/.local/share/kio/servicemenus/`
//! - Nemo: 动作 `~/.local/share/nemo/actions/`
//! - Thunar: 自定义动作 `~/.config/Thunar/uca.xml`(合并)
//!
//! 全部为用户级安装,不需要 root;卸载逐项清理。

use crate::core::error::HrError;
use std::path::{Path, PathBuf};

const LABEL: &str = "Hash 去重并重命名";
const DESCRIPTION: &str = "按内容哈希去重并按序号重命名(仅处理当前文件夹)";

fn home() -> Result<PathBuf, HrError> {
    std::env::var("HOME")
        .map(PathBuf::from)
        .map_err(|_| HrError::Other("无法确定 HOME 目录".to_string()))
}

fn exe_path() -> Result<PathBuf, HrError> {
    std::env::current_exe().map_err(|e| HrError::Other(format!("无法确定程序路径: {e}")))
}

/// shell 单引号安全包裹。
fn sh_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

/// .desktop / uca.xml 的 Exec 内容(双引号包裹可执行文件)。
fn desktop_exec(exe: &Path) -> String {
    format!("\"{}\" --gui %f", exe.display())
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('\"', "&quot;")
}

fn write_file(path: &Path, content: &str, executable: bool) -> Result<(), HrError> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| HrError::io("install", parent, e))?;
    }
    std::fs::write(path, content).map_err(|e| HrError::io("install", path, e))?;
    #[cfg(unix)]
    if executable {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755))
            .map_err(|e| HrError::io("install", path, e))?;
    }
    Ok(())
}

// ---------------- Nautilus ----------------

fn nautilus_script_path() -> Result<PathBuf, HrError> {
    Ok(home()?.join(".local/share/nautilus/scripts/HashRename"))
}

fn install_nautilus(report: &mut Vec<String>) -> Result<(), HrError> {
    let exe = exe_path()?;
    let script = format!(
        "#!/bin/sh\n# 由 HashRename 生成 (--install-context-menu)\n\
         f=$(printf '%s' \"$NAUTILUS_SCRIPT_SELECTED_FILE_PATHS\" | head -n 1)\n\
         [ -n \"$f\" ] && [ -d \"$f\" ] && exec {} --gui \"$f\"\n",
        sh_quote(&exe.display().to_string())
    );
    write_file(&nautilus_script_path()?, &script, true)?;
    report.push("Nautilus:脚本已安装(右键 → Scripts → HashRename)".to_string());
    Ok(())
}

// ---------------- Dolphin (KDE) ----------------

fn dolphin_menu_path() -> Result<PathBuf, HrError> {
    Ok(home()?.join(".local/share/kio/servicemenus/hashrename.desktop"))
}

fn install_dolphin(report: &mut Vec<String>) -> Result<(), HrError> {
    let exe = exe_path()?;
    let desktop = format!(
        "[Desktop Entry]\nType=Service\nX-KDE-ServiceTypes=KonqPopupMenu/Plugin\n\
         MimeType=inode/directory\nActions=hashrename;\n\n\
         [Desktop Action hashrename]\nName={LABEL}\nName[en]=Hash dedupe & rename\n\
         Icon=text-x-generic\nExec={}\n",
        desktop_exec(&exe)
    );
    // KF6 要求 servicemenu 的 .desktop 具有可执行位
    write_file(&dolphin_menu_path()?, &desktop, true)?;
    report.push("Dolphin(KDE):服务菜单已安装".to_string());
    Ok(())
}

// ---------------- Nemo ----------------

fn nemo_action_path() -> Result<PathBuf, HrError> {
    Ok(home()?.join(".local/share/nemo/actions/hashrename.nemo_action"))
}

fn install_nemo(report: &mut Vec<String>) -> Result<(), HrError> {
    let exe = exe_path()?;
    let action = format!(
        "[Nemo Action]\nActive=true\nName={LABEL}\nComment={DESCRIPTION}\n\
         Exec={}\nIcon-Name=text-x-generic\nMimeType=inode/directory\nSelection=s\n",
        desktop_exec(&exe)
    );
    write_file(&nemo_action_path()?, &action, false)?;
    report.push("Nemo:动作已安装".to_string());
    Ok(())
}

// ---------------- Thunar ----------------

const UCA_UNIQUE_ID: &str = "hashrename-uid-001";

fn uca_path() -> Result<PathBuf, HrError> {
    Ok(home()?.join(".config/Thunar/uca.xml"))
}

fn uca_action_xml(exe: &Path) -> String {
    format!(
        "<action>\n<icon>text-x-generic</icon>\n<name>{LABEL}</name>\n\
         <unique-id>{UCA_UNIQUE_ID}</unique-id>\n\
         <command>{}</command>\n<description>{DESCRIPTION}</description>\n\
         <patterns>*</patterns>\n<directories/>\n</action>\n",
        xml_escape(&desktop_exec(exe))
    )
}

fn install_thunar(report: &mut Vec<String>) -> Result<(), HrError> {
    let exe = exe_path()?;
    let path = uca_path()?;
    let block = uca_action_xml(&exe);
    if path.exists() {
        let content =
            std::fs::read_to_string(&path).map_err(|e| HrError::io("install", &path, e))?;
        if content.contains(&format!("<unique-id>{UCA_UNIQUE_ID}")) {
            report.push("Thunar:自定义动作已存在".to_string());
            return Ok(());
        }
        let marker = "</actions>";
        let Some(pos) = content.rfind(marker) else {
            return Err(HrError::Other(format!(
                "Thunar 配置文件 {} 格式异常(缺少 </actions>),已跳过",
                path.display()
            )));
        };
        let merged = format!("{}{}{}", &content[..pos], block, &content[pos..]);
        write_file(&path, &merged, false)?;
    } else {
        let full =
            format!("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<actions>\n{block}</actions>\n");
        write_file(&path, &full, false)?;
    }
    report.push("Thunar:自定义动作已安装".to_string());
    Ok(())
}

fn uninstall_thunar(report: &mut Vec<String>) {
    let Ok(path) = uca_path() else { return };
    if !path.exists() {
        return;
    }
    let Ok(content) = std::fs::read_to_string(&path) else {
        return;
    };
    let Some(marker_pos) = content.find(&format!("<unique-id>{UCA_UNIQUE_ID}")) else {
        return;
    };
    // 找到包围该 unique-id 的 <action>...</action> 块
    let Some(start) = content[..marker_pos].rfind("<action>") else {
        return;
    };
    let Some(end_rel) = content[marker_pos..].find("</action>") else {
        return;
    };
    let end = marker_pos + end_rel + "</action>".len();
    let removed = format!("{}{}", &content[..start], &content[end..]);
    if write_file(&path, &removed, false).is_ok() {
        report.push("Thunar:自定义动作已移除".to_string());
    }
}

// ---------------- 入口 ----------------

pub fn install_context_menu() -> Result<Vec<String>, HrError> {
    let mut report = Vec::new();
    let mut any = false;
    let mut errors = Vec::new();

    if let Err(e) = install_nautilus(&mut report) {
        errors.push(format!("Nautilus: {e}"));
    } else {
        any = true;
    }
    if let Err(e) = install_dolphin(&mut report) {
        errors.push(format!("Dolphin: {e}"));
    } else {
        any = true;
    }
    if let Err(e) = install_nemo(&mut report) {
        errors.push(format!("Nemo: {e}"));
    } else {
        any = true;
    }
    if let Err(e) = install_thunar(&mut report) {
        errors.push(format!("Thunar: {e}"));
    } else {
        any = true;
    }

    // 全部失败视为安装失败(报告具体原因)
    if !any && !errors.is_empty() {
        return Err(HrError::Other(format!(
            "所有文件管理器集成均安装失败:\n{}",
            errors.join("\n")
        )));
    }
    report.extend(errors);
    Ok(report)
}

pub fn uninstall_context_menu() -> Result<Vec<String>, HrError> {
    let mut report = Vec::new();
    let mut removed = 0usize;

    if let Ok(p) = nautilus_script_path() {
        if p.exists() {
            let _ = std::fs::remove_file(&p);
            report.push("Nautilus:脚本已移除".to_string());
            removed += 1;
        }
    }
    if let Ok(p) = dolphin_menu_path() {
        if p.exists() {
            let _ = std::fs::remove_file(&p);
            report.push("Dolphin:服务菜单已移除".to_string());
            removed += 1;
        }
    }
    if let Ok(p) = nemo_action_path() {
        if p.exists() {
            let _ = std::fs::remove_file(&p);
            report.push("Nemo:动作已移除".to_string());
            removed += 1;
        }
    }
    uninstall_thunar(&mut report);

    if report.is_empty() {
        report.push("未发现已安装的文件管理器集成".to_string());
    }
    let _ = removed;
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sh_quote_handles_spaces_and_quotes() {
        assert_eq!(
            sh_quote("/opt/My App/hashrename"),
            "'/opt/My App/hashrename'"
        );
        assert_eq!(sh_quote("a'b"), "'a'\\''b'");
    }

    #[test]
    fn xml_escape_escapes_specials() {
        assert_eq!(xml_escape("a<b&c>\"d"), "a&lt;b&amp;c&gt;&quot;d");
    }
}
