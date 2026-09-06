//! 目录级任务锁(需求 §19):同一目录同时只允许一个 HashRename 任务。
//!
//! 实现:`.hashrename.lock` 以 create_new(独占创建)方式获取;
//! 文件内容记录 pid/时间,若持有者进程已死亡则安全接管(防崩溃死锁)。

use crate::core::error::HrError;
use crate::platform;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

pub const LOCK_FILE_NAME: &str = ".hashrename.lock";

#[derive(Debug, Serialize, Deserialize)]
struct LockContent {
    pid: u32,
    token: String,
    started_at_ms: u128,
}

/// 持有的目录锁;Drop 时自动释放。
#[derive(Debug)]
pub struct DirLock {
    path: PathBuf,
    released: bool,
}

impl DirLock {
    /// 显式释放。
    pub fn release(mut self) {
        self.release_inner();
    }

    fn release_inner(&mut self) {
        if !self.released {
            self.released = true;
            let _ = std::fs::remove_file(&self.path);
        }
    }
}

impl Drop for DirLock {
    fn drop(&mut self) {
        self.release_inner();
    }
}

fn now_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0)
}

/// 尝试获取目录锁。
pub fn acquire(dir: &Path) -> Result<DirLock, HrError> {
    let lock_path = dir.join(LOCK_FILE_NAME);
    let token = platform::gen_token();

    for _attempt in 0..3 {
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&lock_path)
        {
            Ok(mut f) => {
                let content = LockContent {
                    pid: platform::current_pid(),
                    token: token.clone(),
                    started_at_ms: now_ms(),
                };
                use std::io::Write;
                let _ = writeln!(f, "{}", serde_json::to_string(&content).unwrap_or_default());
                return Ok(DirLock {
                    path: lock_path,
                    released: false,
                });
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                if lock_is_stale(&lock_path) {
                    // 陈旧锁:持有者已死亡,移除后重试(与其他接管者竞争 create_new)
                    let _ = std::fs::remove_file(&lock_path);
                    continue;
                }
                return Err(HrError::Locked {
                    path: lock_path.display().to_string(),
                });
            }
            Err(e) => {
                return Err(HrError::io("lock", &lock_path, e));
            }
        }
    }
    Err(HrError::Locked {
        path: lock_path.display().to_string(),
    })
}

/// 锁是否陈旧:内容损坏 / 持有者进程死亡 / 超过 24 小时。
fn lock_is_stale(path: &Path) -> bool {
    let content = match std::fs::read(path) {
        Ok(c) => c,
        Err(_) => return true, // 读不了视为陈旧
    };
    let parsed: Option<LockContent> = serde_json::from_slice(&content).ok();
    match parsed {
        None => true, // 损坏 → 陈旧
        Some(lc) => {
            if now_ms().saturating_sub(lc.started_at_ms) > 24 * 3600 * 1000 {
                return true;
            }
            if lc.pid == platform::current_pid() {
                // 同一进程(例如 GUI 同进程第二次调用):视为活动
                return false;
            }
            !platform::pid_alive(lc.pid)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmpdir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!(
            "hr_lock_{tag}_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .subsec_nanos()
        ));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn second_acquire_fails_while_held() {
        let dir = tmpdir("held");
        let _lock = acquire(&dir).unwrap();
        let second = acquire(&dir);
        assert!(second.is_err(), "同一目录第二个任务必须被拒绝");
        match second.unwrap_err() {
            HrError::Locked { .. } => {}
            other => panic!("应为 Locked 错误: {other}"),
        }
        _lock.release();
        // 释放后可再次获取
        let third = acquire(&dir);
        assert!(third.is_ok());
        drop(third);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn stale_lock_with_dead_pid_is_taken_over() {
        let dir = tmpdir("stale");
        let lock_path = dir.join(LOCK_FILE_NAME);
        // 写入一个不存在的 pid(试探不可能出现的极大值)
        let dead_pid = {
            // 找一个大概率不存在的 pid
            let candidate: u32 = 4_000_000_000;
            assert!(!platform::pid_alive(candidate));
            candidate
        };
        let lc = LockContent {
            pid: dead_pid,
            token: "dead".to_string(),
            started_at_ms: now_ms(),
        };
        std::fs::write(&lock_path, serde_json::to_vec(&lc).unwrap()).unwrap();
        let lock = acquire(&dir);
        assert!(lock.is_ok(), "死进程的陈旧锁应被接管");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn corrupted_lock_is_taken_over() {
        let dir = tmpdir("corrupt");
        std::fs::write(dir.join(LOCK_FILE_NAME), b"not-json{{{").unwrap();
        assert!(acquire(&dir).is_ok());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn drop_releases_lock() {
        let dir = tmpdir("drop");
        {
            let _lock = acquire(&dir).unwrap();
        } // Drop
        assert!(!dir.join(LOCK_FILE_NAME).exists());
        assert!(acquire(&dir).is_ok());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
