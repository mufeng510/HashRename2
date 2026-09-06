//! CLI 模式(需求 §23/§24):与右键菜单/GUI 完全共用核心业务逻辑。

use crate::core::models::ProcessingResult;
use crate::core::processor::{process_directory, ProcessOptions};
use crate::core::progress::{Progress, ProgressEvent};
use crate::core::trash::OsTrash;
use std::io::IsTerminal;
use std::path::PathBuf;
use std::sync::Arc;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

pub fn print_help() {
    println!(
        "HashRename v{VERSION} — 文件哈希去重并序号重命名

用法:
  hashrename <目录>            处理指定目录(终端中运行时直接执行)
  hashrename --cli <目录>      强制命令行模式(脚本/自动化场景)
  hashrename --gui <目录>      打开图形窗口处理指定目录
  hashrename                   打开图形窗口(可选择文件夹)
  hashrename --install-context-menu    安装系统右键菜单
  hashrename --uninstall-context-menu  卸载系统右键菜单
  hashrename --help            显示本帮助
  hashrename --version         显示版本

选项:
  --hash <算法>                哈希算法:md5(默认)/ sha256 / xxh3
  --dry-run                    预览模式:只输出计划,不修改任何文件
  --verbose                    输出详细进度与调试信息

说明:
  - 只处理目录中的普通文件,绝不递归子目录
  - 重复判定 = 相同大小 + 相同 MD5 + 逐字节内容一致
  - 重复文件移入系统回收站,绝不永久删除
  - 剩余文件按原始文件名自然排序重命名为 001.xxx、002.xxx……"
    );
}

/// CLI 进度打印器:TTY 时刷新单行进度条,非 TTY 时仅阶段性输出。
struct CliPrinter {
    verbose: bool,
    is_tty: bool,
}

impl CliPrinter {
    fn new(verbose: bool) -> Self {
        CliPrinter {
            verbose,
            is_tty: std::io::stdout().is_terminal(),
        }
    }

    fn clear_line(&self) {
        if self.is_tty {
            print!("\r\x1b[2K");
        }
    }

    fn flush(&self) {
        use std::io::Write;
        let _ = std::io::stdout().flush();
    }

    fn handle(&self, event: &ProgressEvent) {
        match event {
            ProgressEvent::Stage { message, .. } => {
                self.clear_line();
                println!("[{message}]");
                self.flush();
            }
            ProgressEvent::CurrentFile { file } => {
                if self.verbose {
                    self.clear_line();
                    println!("  · {file}");
                    self.flush();
                }
            }
            ProgressEvent::Progress { percent } => {
                if self.is_tty {
                    let filled = ((percent / 2.5).round() as usize).min(40);
                    let bar = "█".repeat(filled);
                    let pad = "░".repeat(40 - filled);
                    print!("\r\x1b[2K  进度 {percent:5.1}%  |{bar}{pad}|");
                    self.flush();
                }
            }
            ProgressEvent::Warning { message } => {
                self.clear_line();
                println!("  ⚠ {message}");
                self.flush();
            }
            ProgressEvent::Counts { .. } | ProgressEvent::Finished { .. } => {}
        }
    }
}

/// 执行 CLI 处理流程,返回进程退出码。
///
/// Ctrl-C 直接终止进程是安全的:两阶段重命名 + journal 保证任意时刻
/// 被终止都不会破坏文件状态,下次运行自动恢复(需求 §29)。
pub fn run_cli(dir: PathBuf, verbose: bool, opts: ProcessOptions) -> i32 {
    println!("HashRename v{VERSION}");
    println!("目录: {}", dir.display());
    if opts.dry_run {
        println!("模式: 预览(不修改任何文件)");
    }
    println!("算法: {}", opts.hash_algorithm.as_str());
    println!();

    let printer = Arc::new(CliPrinter::new(verbose));
    let p2 = printer.clone();
    let handle = Progress::create(move |e: &ProgressEvent| p2.handle(e));
    let progress = handle.reporter();

    match process_directory(&dir, &opts, &progress, Arc::new(OsTrash)) {
        Ok(res) => {
            printer.clear_line();
            print_summary(&res);
            if res.cancelled || res.failed_count > 0 {
                1
            } else {
                0
            }
        }
        Err(e) => {
            eprintln!("错误: {e}");
            2
        }
    }
}

fn print_summary(res: &ProcessingResult) {
    println!();
    println!("====================");
    if res.cancelled {
        println!("HashRename 已取消");
    } else if res.dry_run {
        println!("HashRename 预览完成(未修改任何文件)");
    } else if res.failed_count > 0 {
        println!("HashRename 完成(存在部分错误)");
    } else {
        println!("HashRename 完成");
    }
    println!("目录: {}", res.directory);
    if !res.hash_algorithm.is_empty() {
        println!("哈希算法: {}", res.hash_algorithm);
    }
    println!("扫描文件: {}", res.scanned_count);
    println!("发现重复: {}", res.duplicate_count);
    if res.dry_run {
        println!("计划移入回收站: {}", res.planned_trashes.len());
        println!("计划重命名: {}", res.planned_renames.len());
        if !res.planned_trashes.is_empty() {
            println!();
            println!("将移入回收站(最多显示 30 条):");
            for name in res.planned_trashes.iter().take(30) {
                println!("  ☠ {name}");
            }
        }
        if !res.planned_renames.is_empty() {
            println!();
            println!("将重命名(最多显示 50 条):");
            for p in res.planned_renames.iter().take(50) {
                println!("  {} → {}", p.from, p.to);
            }
        }
        for w in &res.warnings {
            println!("⚠ {w}");
        }
        return;
    }
    println!("移入回收站: {}", res.trashed_count);
    println!("最终文件: {}", res.kept_count);
    println!("重命名: {}", res.renamed_count);
    println!("失败: {}", res.failed_count);
    println!("处理耗时: {:.1} 秒", res.elapsed_ms as f64 / 1000.0);

    if !res.warnings.is_empty() {
        println!();
        for w in &res.warnings {
            println!("⚠ {w}");
        }
    }
    if !res.errors.is_empty() {
        println!();
        println!("错误明细(最多显示 20 条):");
        for e in res.errors.iter().take(20) {
            println!("  [{}] {}: {}", e.operation, e.path, e.message);
        }
        if res.errors.len() > 20 {
            println!("  ... 共 {} 条错误", res.errors.len());
        }
    }
}
