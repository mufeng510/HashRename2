//! 进度上报与取消(需求 §16/§28)。CLI 与 GUI 共用同一套事件。

use crate::core::models::ProcessingResult;
use serde::Serialize;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

/// 处理阶段。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Stage {
    Recover,
    Scan,
    Hash,
    Verify,
    Plan,
    Trash,
    Rename,
    Done,
}

/// 进度事件(序列化后通过 Tauri Channel 发往前端;CLI 直接打印)。
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ProgressEvent {
    Stage {
        stage: Stage,
        message: String,
    },
    Counts {
        scanned: usize,
        duplicates: usize,
        kept: usize,
    },
    CurrentFile {
        file: String,
    },
    Progress {
        percent: f64,
    },
    Warning {
        message: String,
    },
    Finished {
        result: Box<ProcessingResult>,
    },
}

type Callback = Arc<dyn Fn(&ProgressEvent) + Send + Sync>;

/// 线程安全的进度上报器 + 取消令牌。
#[derive(Clone)]
pub struct Progress {
    cb: Callback,
    cancelled: Arc<AtomicBool>,
}

/// 句柄:持有取消权的一端(CLI Ctrl-C/GUI 取消按钮)。
pub struct ProgressHandle {
    progress: Progress,
}

impl Progress {
    /// 创建带回调的进度器(命名避开 `new` 应返回 Self 的惯例)。
    pub fn create(cb: impl Fn(&ProgressEvent) + Send + Sync + 'static) -> ProgressHandle {
        ProgressHandle {
            progress: Progress {
                cb: Arc::new(cb),
                cancelled: Arc::new(AtomicBool::new(false)),
            },
        }
    }

    /// 创建使用**外部共享取消令牌**的进度器(GUI 取消按钮/窗口关闭共用)。
    pub fn with_token(
        cb: impl Fn(&ProgressEvent) + Send + Sync + 'static,
        token: Arc<AtomicBool>,
    ) -> ProgressHandle {
        ProgressHandle {
            progress: Progress {
                cb: Arc::new(cb),
                cancelled: token,
            },
        }
    }
}

impl Default for Progress {
    fn default() -> Self {
        Progress {
            cb: Arc::new(|_| {}),
            cancelled: Arc::new(AtomicBool::new(false)),
        }
    }
}

impl ProgressHandle {
    pub fn reporter(&self) -> Progress {
        self.progress.clone()
    }

    pub fn cancel(&self) {
        self.progress.cancelled.store(true, Ordering::SeqCst);
    }
}

impl Progress {
    pub fn cancelled(&self) -> bool {
        self.cancelled.load(Ordering::SeqCst)
    }

    pub fn request_cancel(&self) {
        self.cancelled.store(true, Ordering::SeqCst);
    }

    pub fn emit(&self, event: &ProgressEvent) {
        (self.cb)(event);
    }

    pub fn stage(&self, stage: Stage, message: impl Into<String>) {
        self.emit(&ProgressEvent::Stage {
            stage,
            message: message.into(),
        });
    }

    pub fn counts(&self, scanned: usize, duplicates: usize, kept: usize) {
        self.emit(&ProgressEvent::Counts {
            scanned,
            duplicates,
            kept,
        });
    }

    pub fn current_file(&self, file: &str) {
        self.emit(&ProgressEvent::CurrentFile {
            file: file.to_string(),
        });
    }

    pub fn warn(&self, message: impl Into<String>) {
        self.emit(&ProgressEvent::Warning {
            message: message.into(),
        });
    }

    /// 上报百分比:百分比落在 [lo, hi) 区间内,按 fraction 插值。
    /// 由 processor 为各阶段分配区间;hasher/detector 在工作线程调用。
    pub fn tick_range(&self, fraction: f64, lo: f64, hi: f64) {
        let p = lo + (hi - lo) * fraction.clamp(0.0, 1.0);
        self.emit(&ProgressEvent::Progress { percent: p });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;

    #[test]
    fn events_reach_callback() {
        let counter = Arc::new(AtomicUsize::new(0));
        let c2 = counter.clone();
        let handle = Progress::create(move |e| {
            if matches!(e, ProgressEvent::Stage { .. }) {
                c2.fetch_add(1, Ordering::SeqCst);
            }
        });
        let p = handle.reporter();
        p.stage(Stage::Scan, "扫描中");
        p.stage(Stage::Hash, "哈希中");
        assert_eq!(counter.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn cancel_token() {
        let handle = Progress::create(|_| {});
        let p = handle.reporter();
        assert!(!p.cancelled());
        handle.cancel();
        assert!(p.cancelled());
        p.request_cancel();
        assert!(p.cancelled());
    }
}
