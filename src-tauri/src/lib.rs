//! HashRename 库入口:命令行分发 + Tauri GUI 启动。

pub mod cli;
pub mod commands;
pub mod core;
pub mod platform;

use std::io::IsTerminal;
use std::path::PathBuf;

/// 程序入口分发(需求 §23):
/// - `--help` / `--version`:打印后退出;
/// - `--install-context-menu` / `--uninstall-context-menu`:平台右键菜单;
/// - `--cli <目录>`:强制 CLI 模式(脚本/自动化场景);
/// - `--gui <目录>`:强制 GUI 模式;
/// - `<目录>`:终端(TTY)中 → CLI;无终端(右键菜单/双击启动)→ GUI;
/// - 无参数:GUI(窗口内选择文件夹)。
pub fn entrypoint() {
    // Windows GUI 子系统下先附加父控制台,CLI 输出才可见
    platform::attach_parent_console();

    let args: Vec<String> = std::env::args().skip(1).collect();

    let mut verbose = false;
    let mut dry_run = false;
    let mut cli_mode = false;
    let mut gui_mode = false;
    let mut hash_algorithm = crate::core::hasher::HashAlgorithm::default();
    let mut positional: Option<PathBuf> = None;
    let mut install_menu = false;
    let mut uninstall_menu = false;

    let mut iter = args.iter().peekable();
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "-h" | "--help" => {
                cli::print_help();
                return;
            }
            "-V" | "--version" => {
                println!("HashRename {}", cli::VERSION);
                return;
            }
            "--verbose" => verbose = true,
            "--dry-run" => dry_run = true,
            "--install-context-menu" => install_menu = true,
            "--uninstall-context-menu" => uninstall_menu = true,
            // 布尔标志:目录一律来自位置参数,避免吞掉后续标志
            "--cli" => cli_mode = true,
            "--gui" => gui_mode = true,
            "--hash" => match iter.next() {
                Some(v) => match crate::core::hasher::HashAlgorithm::parse(v) {
                    Some(a) => hash_algorithm = a,
                    None => {
                        eprintln!("未知哈希算法: {v}(可选:md5 / sha256 / xxh3)");
                        std::process::exit(2);
                    }
                },
                None => {
                    eprintln!("--hash 需要一个算法参数(md5 / sha256 / xxh3)");
                    std::process::exit(2);
                }
            },
            other if other.starts_with('-') => {
                eprintln!("未知参数: {other}");
                eprintln!("使用 --help 查看用法。");
                std::process::exit(2);
            }
            other => {
                if positional.is_none() {
                    positional = Some(PathBuf::from(other));
                } else {
                    eprintln!("只支持一个目录参数(收到多个)。");
                    std::process::exit(2);
                }
            }
        }
    }

    if install_menu || uninstall_menu {
        run_context_menu_command(install_menu);
        return;
    }

    let cli_opts = crate::core::processor::ProcessOptions {
        dry_run,
        hash_algorithm,
        ..crate::core::processor::ProcessOptions::default()
    };

    if cli_mode {
        // --cli:强制命令行模式,目录必须给出
        match positional {
            Some(dir) => {
                let code = cli::run_cli(dir, verbose, cli_opts);
                std::process::exit(code);
            }
            None => {
                eprintln!("--cli 需要一个目录参数,例如:hashrename --cli /path/to/dir");
                std::process::exit(2);
            }
        }
    }
    if gui_mode {
        // --gui:强制图形窗口(目录可选)
        run_gui(positional);
        return;
    }
    match positional {
        // 目录参数:TTY → CLI;非 TTY(文件管理器右键启动)→ GUI
        Some(dir) => {
            let interactive = std::io::stdout().is_terminal();
            if interactive {
                let code = cli::run_cli(dir, verbose, cli_opts);
                std::process::exit(code);
            } else {
                run_gui(Some(dir));
            }
        }
        None => run_gui(None),
    }
}

/// 安装/卸载右键菜单。Windows 无控制台时用消息框展示结果。
fn run_context_menu_command(install: bool) {
    let (result, title) = if install {
        (platform::install_context_menu(), "安装右键菜单")
    } else {
        (platform::uninstall_context_menu(), "卸载右键菜单")
    };
    let text = match result {
        Ok(lines) => lines.join("\n"),
        Err(e) => format!("失败:{e}"),
    };
    println!("{text}");

    #[cfg(windows)]
    {
        if !std::io::stdout().is_terminal() {
            platform::windows::show_message(&format!("HashRename — {title}"), &text);
        }
    }
    #[cfg(not(windows))]
    let _ = title;
}

/// 启动 Tauri GUI。
pub fn run_gui(launch_dir: Option<PathBuf>) {
    commands::run_gui(launch_dir);
}
