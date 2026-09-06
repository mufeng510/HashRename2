//! Tauri 命令层:GUI 与核心业务逻辑的桥(需求 §25)。

use crate::core::models::ProcessingResult;
use crate::core::processor::{process_directory, ProcessOptions};
use crate::core::progress::{Progress, ProgressEvent};
use crate::core::trash::OsTrash;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use tauri::ipc::Channel;
use tauri::{Manager, State, WindowEvent};

pub struct AppState {
    /// 右键菜单/命令行启动时携带的目录(GUI 自动开始处理)。
    pub launch_dir: Option<String>,
    /// 是否有任务在运行(一个窗口同时只允许一个任务)。
    pub busy: Arc<Mutex<bool>>,
    /// 当前任务的取消令牌(与 Progress 共享)。
    pub cancel: Arc<AtomicBool>,
}

impl AppState {
    fn new(launch_dir: Option<String>) -> Self {
        AppState {
            launch_dir,
            busy: Arc::new(Mutex::new(false)),
            cancel: Arc::new(AtomicBool::new(false)),
        }
    }
}

#[derive(serde::Serialize)]
pub struct LaunchInfo {
    pub dir: Option<String>,
    pub version: &'static str,
}

#[tauri::command]
pub fn get_launch_info(state: State<'_, AppState>) -> LaunchInfo {
    LaunchInfo {
        dir: state.launch_dir.clone(),
        version: crate::cli::VERSION,
    }
}

#[tauri::command]
pub fn start_processing(
    dir: String,
    on_event: Channel<ProgressEvent>,
    state: State<'_, AppState>,
    // 可配置哈希算法:md5 / sha256 / xxh3(缺省 md5)
    hash_algorithm: Option<String>,
    // 预览模式:只输出计划,不修改任何文件
    dry_run: Option<bool>,
) -> Result<(), String> {
    {
        let mut busy = state.busy.lock().unwrap();
        if *busy {
            return Err("已有任务正在运行".to_string());
        }
        *busy = true;
    }

    // 前端传来的算法字符串在这里解析;无法识别直接报错,不静默回退
    let hash_algorithm = match hash_algorithm.as_deref() {
        None => crate::core::hasher::HashAlgorithm::default(),
        Some(s) => crate::core::hasher::HashAlgorithm::parse(s)
            .ok_or_else(|| format!("未知哈希算法: {s}(可选:md5 / sha256 / xxh3)"))?,
    };
    let opts = ProcessOptions {
        hash_algorithm,
        dry_run: dry_run.unwrap_or(false),
        ..ProcessOptions::default()
    };

    let path = PathBuf::from(&dir);
    let cancel = state.cancel.clone();
    let busy_flag = state.busy.clone();

    std::thread::spawn(move || {
        // 取消令牌与 AppState 共享:cancel_processing 命令 / 窗口关闭都会置位
        let send_channel = on_event.clone();
        let handle = Progress::with_token(
            move |e: &ProgressEvent| {
                // Channel 发送失败(窗口关闭等)可忽略:任务继续,保证数据安全
                let _ = send_channel.send(e.clone());
            },
            cancel.clone(),
        );
        let reporter = handle.reporter();

        let result = process_directory(&path, &opts, &reporter, Arc::new(OsTrash));

        let res: ProcessingResult = match result {
            Ok(r) => r,
            Err(e) => {
                // 致命错误(目录不存在/被锁):以错误结果形式发回
                ProcessingResult {
                    directory: path.display().to_string(),
                    errors: vec![crate::core::error::FileError::new(
                        "fatal",
                        &path,
                        e.to_string(),
                    )],
                    failed_count: 1,
                    ..ProcessingResult::default()
                }
            }
        };
        let _ = on_event.send(ProgressEvent::Finished {
            result: Box::new(res),
        });

        // 任务结束:复位状态
        cancel.store(false, Ordering::SeqCst);
        *busy_flag.lock().unwrap() = false;
    });

    Ok(())
}

#[tauri::command]
pub fn cancel_processing(state: State<'_, AppState>) {
    state.cancel.store(true, Ordering::SeqCst);
}

/// GUI 启动入口。
pub fn run_gui(launch_dir: Option<PathBuf>) {
    let launch = launch_dir.map(|d| d.display().to_string());

    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(AppState::new(launch))
        .invoke_handler(tauri::generate_handler![
            get_launch_info,
            start_processing,
            cancel_processing
        ])
        .on_window_event(|window, event| {
            // 窗口关闭 → 请求取消任务;两阶段重命名保证安全中断
            if let WindowEvent::CloseRequested { .. } = event {
                let state: State<AppState> = window.app_handle().state();
                state.cancel.store(true, Ordering::SeqCst);
            }
        })
        .run(tauri::generate_context!())
        .expect("HashRename GUI 启动失败");
}
