//! macOS Finder 集成(需求 §21)。
//!
//! 机制:Automator **Quick Action**(快速操作 / 服务),macOS 10.14+ 的
//! 主流稳定方案。安装 `~/Library/Services/HashRename.workflow`,在 Finder
//! 中右键文件夹 → 快速操作/服务 → Hash 去重并重命名。
//!
//! 说明:
//! - Quick Action 以"作为参数传递的文件/文件夹"运行 shell,
//!   直接调用 HashRename 二进制的 GUI 模式,无需用户额外授权;
//! - 首次访问 受保护目录(桌面/文稿/下载)时系统会弹出 TCC 授权,
//!   允许一次即可(见 README);
//! - .workflow 由本程序生成(--install-context-menu),卸载时删除。

use crate::core::error::HrError;
use std::path::PathBuf;

const LABEL: &str = "Hash 去重并重命名";
const WORKFLOW_DIR_NAME: &str = "HashRename.workflow";

fn home() -> Result<PathBuf, HrError> {
    std::env::var("HOME")
        .map(PathBuf::from)
        .map_err(|_| HrError::Other("无法确定 HOME 目录".to_string()))
}

fn workflow_root() -> Result<PathBuf, HrError> {
    Ok(home()?.join("Library/Services").join(WORKFLOW_DIR_NAME))
}

fn exe_path() -> Result<PathBuf, HrError> {
    std::env::current_exe().map_err(|e| HrError::Other(format!("无法确定程序路径: {e}")))
}

fn sh_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('\"', "&quot;")
}

fn info_plist() -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
	<key>NSServices</key>
	<array>
		<dict>
			<key>NSBackgroundColorName</key>
			<string>background</string>
			<key>NSIconName</key>
			<string>NSActionTemplate</string>
			<key>NSMenuItem</key>
			<dict>
				<key>default</key>
				<string>{LABEL}</string>
			</dict>
			<key>NSMessage</key>
			<string>runWorkflowAsService</string>
			<key>NSRequiredContext</key>
			<dict>
				<key>NSApplicationIdentifier</key>
				<string>com.apple.finder</string>
			</dict>
			<key>NSSendFileTypes</key>
			<array>
				<string>public.folder</string>
			</array>
		</dict>
	</array>
</dict>
</plist>
"#
    )
}

fn document_wflow(exe_display: &str) -> String {
    // 先做 shell 引用,再对结果做 XML 转义(嵌入 <string> 文本)
    let script = xml_escape(&format!("{} --gui \"$@\"", sh_quote(exe_display)));
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
	<key>AMApplicationBuild</key>
	<string>523</string>
	<key>AMApplicationVersion</key>
	<string>2.1</string>
	<key>AMDocumentVersion</key>
	<string>2</string>
	<key>actions</key>
	<array>
		<dict>
			<key>action</key>
			<dict>
				<key>AMAccepts</key>
				<dict>
					<key>Container</key>
					<string>List</string>
					<key>Optional</key>
					<true/>
					<key>Types</key>
					<array>
						<string>com.apple.cocoa.string</string>
					</array>
				</dict>
				<key>AMActionVersion</key>
				<string>2.0.3</string>
				<key>AMApplication</key>
				<array>
					<string>Automator</string>
				</array>
				<key>AMParameterProperties</key>
				<dict>
					<key>COMMAND_STRING</key>
					<dict/>
					<key>CheckedForUserDefaultShell</key>
					<dict/>
					<key>inputMethod</key>
					<dict/>
					<key>shell</key>
					<dict/>
					<key>source</key>
					<dict/>
				</dict>
				<key>AMProvides</key>
				<dict>
					<key>Container</key>
					<string>List</string>
					<key>Types</key>
					<array>
						<string>com.apple.cocoa.string</string>
					</array>
				</dict>
				<key>ActionBundlePath</key>
				<string>/System/Library/Automator/Run Shell Script.action</string>
				<key>ActionName</key>
				<string>Run Shell Script</string>
				<key>ActionParameters</key>
				<dict>
					<key>COMMAND_STRING</key>
					<string>{script}</string>
					<key>CheckedForUserDefaultShell</key>
					<true/>
					<key>inputMethod</key>
					<integer>1</integer>
					<key>shell</key>
					<string>/bin/zsh</string>
					<key>source</key>
					<string></string>
				</dict>
				<key>BundleIdentifier</key>
				<string>com.apple.RunShellScript</string>
				<key>CFBundleVersion</key>
				<string>2.0.3</string>
				<key>CanShowSelectedItemsWhenRun</key>
				<false/>
				<key>CanShowWhenRun</key>
				<true/>
				<key>Category</key>
				<array>
					<string>AMCategoryUtilities</string>
				</array>
				<key>Class Name</key>
				<string>RunShellScriptAction</string>
				<key>InputUUID</key>
				<string>7A6F1C2D-0001-4A5B-8C9D-000000000001</string>
				<key>Keywords</key>
				<array>
					<string>Shell</string>
				</array>
				<key>OutputUUID</key>
				<string>7A6F1C2D-0002-4A5B-8C9D-000000000002</string>
				<key>UUID</key>
				<string>7A6F1C2D-0003-4A5B-8C9D-000000000003</string>
				<key>UnlocalizedApplications</key>
				<array>
					<string>Automator</string>
				</array>
				<key>arguments</key>
				<dict>
					<key>0</key>
					<dict>
						<key>default value</key>
						<integer>0</integer>
						<key>name</key>
						<string>inputMethod</string>
						<key>required</key>
						<string>0</string>
						<key>type</key>
						<string>0</string>
						<key>uuid</key>
						<string>7A6F1C2D-0004-4A5B-8C9D-000000000004</string>
					</dict>
					<key>1</key>
					<dict>
						<key>default value</key>
						<false/>
						<key>name</key>
						<string>CheckedForUserDefaultShell</string>
						<key>required</key>
						<string>0</string>
						<key>type</key>
						<string>0</string>
						<key>uuid</key>
						<string>7A6F1C2D-0005-4A5B-8C9D-000000000005</string>
					</dict>
					<key>2</key>
					<dict>
						<key>default value</key>
						<string></string>
						<key>name</key>
						<string>source</string>
						<key>required</key>
						<string>0</string>
						<key>type</key>
						<string>0</string>
						<key>uuid</key>
						<string>7A6F1C2D-0006-4A5B-8C9D-000000000006</string>
					</dict>
					<key>3</key>
					<dict>
						<key>default value</key>
						<string></string>
						<key>name</key>
						<string>COMMAND_STRING</string>
						<key>required</key>
						<string>0</string>
						<key>type</key>
						<string>0</string>
						<key>uuid</key>
						<string>7A6F1C2D-0007-4A5B-8C9D-000000000007</string>
					</dict>
					<key>4</key>
					<dict>
						<key>default value</key>
						<string>/bin/zsh</string>
						<key>name</key>
						<string>shell</string>
						<key>required</key>
						<string>0</string>
						<key>type</key>
						<string>0</string>
						<key>uuid</key>
						<string>7A6F1C2D-0008-4A5B-8C9D-000000000008</string>
					</dict>
				</dict>
				<key>isViewVisible</key>
				<integer>1</integer>
				<key>location</key>
				<string>309.000000:253.000000</string>
				<key>nibPath</key>
				<string>/System/Library/Automator/Run Shell Script.action/Contents/Resources/Base.lproj/main.nib</string>
			</dict>
			<key>isViewVisible</key>
			<integer>1</integer>
		</dict>
	</array>
	<key>connectors</key>
	<dict/>
	<key>workflowMetaData</key>
	<dict>
		<key>applicationBundleIDsByPath</key>
		<dict/>
		<key>applicationPaths</key>
		<array/>
		<key>inputTypeIdentifier</key>
		<string>com.apple.Automator.fileSystemObject.folder</string>
		<key>outputTypeIdentifier</key>
		<string>com.apple.Automator.nothing</string>
		<key>presentationMode</key>
		<integer>11</integer>
		<key>processesInput</key>
		<integer>0</integer>
		<key>serviceInputTypeIdentifier</key>
		<string>com.apple.Automator.fileSystemObject.folder</string>
		<key>serviceOutputTypeIdentifier</key>
		<string>com.apple.Automator.nothing</string>
		<key>serviceProcessesInput</key>
		<integer>0</integer>
		<key>systemImageName</key>
		<string>NSActionTemplate</string>
		<key>useAutomaticInputType</key>
		<integer>0</integer>
		<key>workflowTypeIdentifier</key>
		<string>com.apple.Automator.servicesMenu</string>
	</dict>
</dict>
</plist>
"#
    )
}

pub fn install_context_menu() -> Result<Vec<String>, HrError> {
    let exe = exe_path()?;
    let root = workflow_root()?;
    let contents = root.join("Contents");
    std::fs::create_dir_all(&contents).map_err(|e| HrError::io("install", &contents, e))?;

    std::fs::write(contents.join("Info.plist"), info_plist())
        .map_err(|e| HrError::io("install", &contents.join("Info.plist"), e))?;
    std::fs::write(
        contents.join("document.wflow"),
        document_wflow(&exe.display().to_string()),
    )
    .map_err(|e| HrError::io("install", &contents.join("document.wflow"), e))?;

    Ok(vec![
        format!("Quick Action 已安装: {}", root.display()),
        "在 Finder 中右键文件夹 → 快速操作(或 服务)→ Hash 去重并重命名".to_string(),
        "如未立即出现,请注销重新登录或在 系统设置 → 隐私与安全性 → 扩展 中确认".to_string(),
    ])
}

pub fn uninstall_context_menu() -> Result<Vec<String>, HrError> {
    let root = workflow_root()?;
    if root.exists() {
        std::fs::remove_dir_all(&root).map_err(|e| HrError::io("uninstall", &root, e))?;
        Ok(vec![format!("Quick Action 已移除: {}", root.display())])
    } else {
        Ok(vec!["未发现已安装的 Quick Action".to_string()])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plist_escape_and_quote() {
        assert_eq!(
            sh_quote("/Applications/HashRename.app/x"),
            "'/Applications/HashRename.app/x'"
        );
        assert_eq!(xml_escape("a&b"), "a&amp;b");
    }

    #[test]
    fn generated_plist_contains_key_fields() {
        let plist = info_plist();
        assert!(plist.contains("runWorkflowAsService"));
        assert!(plist.contains("public.folder"));
        assert!(plist.contains(LABEL));
        let wflow = document_wflow("/usr/local/bin/hashrename");
        assert!(wflow.contains("RunShellScript"));
        assert!(wflow.contains("--gui"));
        // shell 引用 + XML 转义链条
        assert!(
            wflow.contains("&#39;/usr/local/bin/hashrename&#39; --gui &quot;$@&quot;")
                || wflow.contains("'/usr/local/bin/hashrename' --gui")
        );
    }
}
