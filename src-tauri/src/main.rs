//! HashRename — 文件哈希去重并序号重命名。
//!
//! 单一二进制同时承担:
//! - CLI 模式(终端中 `hashrename <目录>`)
//! - GUI 模式(文件管理器右键启动 / 直接双击)
//! - 右键菜单安装/卸载(`--install-context-menu`)

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    hashrename_lib::entrypoint();
}
