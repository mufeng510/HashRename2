//! 核心编排器:九阶段安全流水线(需求 §28)。
//!
//! 扫描 → 恢复 → 哈希 → 确认重复 → 生成计划 → 回收站 → 重命名计划 →
//! 临时重命名 → 最终重命名 → 报告。先建完整计划,再执行;
//! 任何单文件失败不影响整体,任何致命失败保证目录一致。

use crate::core::duplicate_detector;
use crate::core::error::{FileError, HrError};
use crate::core::hasher::{compute_hashes, Hasher};
use crate::core::lock;
use crate::core::models::ProcessingResult;
use crate::core::progress::{Progress, Stage};
use crate::core::rename_planner::{plan_renames, OccupiedNames, RenamePlan};
use crate::core::renamer;
use crate::core::scanner;
use crate::core::trash::TrashProvider;
use std::collections::HashSet;
use std::path::Path;
use std::sync::Arc;
use std::time::Instant;

#[derive(Debug, Clone)]
pub struct ProcessOptions {
    /// 并行哈希线程数。
    pub hash_threads: usize,
}

impl Default for ProcessOptions {
    fn default() -> Self {
        ProcessOptions {
            hash_threads: crate::core::hasher::default_thread_count(),
        }
    }
}

/// 处理一个目录。返回完整统计;致命错误(目录无效/被锁)经 `Err` 返回。
pub fn process_directory(
    dir: &Path,
    opts: &ProcessOptions,
    progress: &Progress,
    trash_provider: Arc<dyn TrashProvider>,
    hasher: Arc<dyn Hasher>,
) -> Result<ProcessingResult, HrError> {
    let t0 = Instant::now();
    let mut result = ProcessingResult::default();

    // 阶段 0:目录校验
    let dir = dir
        .canonicalize()
        .map_err(|e| crate::core::error::HrError::Directory {
            path: dir.display().to_string(),
            source: e,
        })?;
    if !dir.is_dir() {
        return Err(crate::core::error::HrError::Directory {
            path: dir.display().to_string(),
            source: std::io::Error::other("不是目录"),
        });
    }
    result.directory = dir.display().to_string();

    progress.stage(Stage::Recover, "正在检查未完成任务...");
    progress.tick_range(1.0, 0.0, 0.01);

    // 阶段 0.5:目录锁(需求 §19)
    let dir_lock = lock::acquire(&dir)?;

    // 阶段 0.7:恢复上次中断的重命名(需求 §29)
    let recovery = renamer::recover_pending(&dir, progress);
    result.recovered_finals = recovery.recovered_finals;
    result.restored_temps = recovery.restored_temps;
    result.orphan_temps = recovery.orphan_temps;
    result.warnings.extend(recovery.warnings);
    result.errors.extend(recovery.errors);
    progress.tick_range(1.0, 0.01, 0.03);

    // 阶段 1:扫描(只扫描当前目录,不递归)
    progress.stage(Stage::Scan, "正在扫描文件...");
    let scan = scanner::scan_directory(&dir)?;
    result.scanned_count = scan.files.len();
    result.skipped_count = scan.skipped.len();
    result.errors.extend(scan.errors);
    progress.tick_range(1.0, 0.03, 0.03);
    progress.current_file(&format!("{} 个文件", scan.files.len()));

    // 阶段 2:文件大小预筛选(需求 §8)
    let mut by_size: std::collections::BTreeMap<u64, Vec<usize>> = Default::default();
    for (i, f) in scan.files.iter().enumerate() {
        by_size.entry(f.size).or_default().push(i);
    }
    let needed: Vec<bool> = (0..scan.files.len())
        .map(|i| {
            by_size
                .get(&scan.files[i].size)
                .is_some_and(|v| v.len() > 1)
        })
        .collect();
    let to_hash = needed.iter().filter(|&&b| b).count();
    progress.stage(
        Stage::Hash,
        format!("正在计算 MD5({to_hash} 个候选文件)..."),
    );

    // 阶段 3:并行流式 MD5
    let hashes = compute_hashes(&scan.files, &needed, hasher, opts.hash_threads, progress);
    progress.tick_range(1.0, 0.03, 0.63);

    if progress.cancelled() {
        finish_cancelled(&mut result, t0);
        return Ok(result);
    }

    // 哈希失败(文件消失/无权限等):记录错误,该文件不参与去重,保留原名
    let hash_values: Vec<Option<crate::core::models::HashValue>> = hashes
        .into_iter()
        .enumerate()
        .map(|(i, r)| match r {
            Some(Ok(h)) => Some(h),
            Some(Err(e)) => {
                result.errors.push(e);
                None
            }
            None => {
                let _ = i;
                None
            }
        })
        .collect();

    // 阶段 4:确认重复(大小 + MD5 + 逐字节二次验证)
    progress.stage(Stage::Verify, "正在验证重复文件...");
    let detection = duplicate_detector::detect_duplicates(&scan.files, &hash_values, progress);
    result.errors.extend(detection.errors);
    result.duplicate_groups = detection.groups.len();
    result.duplicate_count = detection.groups.iter().map(|g| g.duplicates.len()).sum();
    progress.tick_range(1.0, 0.63, 0.71);

    if progress.cancelled() {
        finish_cancelled(&mut result, t0);
        return Ok(result);
    }

    // 阶段 5:重复文件移入回收站(需求 §11)
    progress.stage(
        Stage::Trash,
        format!("正在移动 {} 个重复文件到回收站...", result.duplicate_count),
    );
    let mut trashed_indices: HashSet<usize> = HashSet::new();
    let index_of: std::collections::HashMap<std::path::PathBuf, usize> = scan
        .files
        .iter()
        .enumerate()
        .map(|(i, f)| (f.path.clone(), i))
        .collect();
    'trash_loop: for (gi, group) in detection.groups.iter().enumerate() {
        for dup in &group.duplicates {
            if progress.cancelled() {
                break 'trash_loop;
            }
            let idx = match index_of.get(&dup.path) {
                Some(i) => *i,
                None => continue,
            };
            match trash_provider.send_to_trash(&dup.path) {
                Ok(()) => {
                    trashed_indices.insert(idx);
                    result.trashed_count += 1;
                }
                Err(e) => {
                    // 回收站失败:安全停止并报告(需求 §41),不永久删除
                    result
                        .errors
                        .push(FileError::new("trash", &dup.path, e.to_string()));
                    result.warnings.push(
                        "回收站操作失败,已停止移动其余重复文件(文件未删除,保持原状)".to_string(),
                    );
                    break 'trash_loop;
                }
            }
            progress.tick_range(
                (gi + 1) as f64 / detection.groups.len().max(1) as f64,
                0.72,
                0.82,
            );
        }
    }
    progress.tick_range(1.0, 0.72, 0.82);

    // 阶段 6:生成重命名计划(剩余文件 = 未被成功移入回收站的文件)
    let remaining: Vec<crate::core::models::FileEntry> = scan
        .files
        .iter()
        .enumerate()
        .filter(|(i, _)| !trashed_indices.contains(i))
        .map(|(_, f)| f.clone())
        .collect();
    result.kept_count = remaining.len();

    progress.counts(
        result.scanned_count,
        result.duplicate_count,
        result.kept_count,
    );

    progress.stage(Stage::Plan, "正在生成重命名计划...");
    let mut occupied = OccupiedNames::new();
    for s in &scan.skipped {
        if let Some(name) = s.path.file_name() {
            occupied.insert(&name.to_string_lossy());
        }
    }
    let token = crate::platform::gen_token();
    let plan: RenamePlan = plan_renames(
        &remaining,
        &occupied,
        crate::platform::current_pid(),
        &token,
    );
    for b in &plan.blocked {
        result.errors.push(FileError::new(
            "rename",
            &remaining[b.entry_index].path,
            b.reason.clone(),
        ));
    }
    progress.tick_range(1.0, 0.71, 0.82);

    // 阶段 7+8:journal → 临时重命名 → 最终重命名
    if !plan.ops.is_empty() && !progress.cancelled() {
        match renamer::write_journal(&dir, crate::platform::current_pid(), &token, &plan) {
            Ok(journal_file) => {
                let outcome = renamer::execute_plan(&dir, &plan, progress);
                result.renamed_count = outcome.renamed;
                result.errors.extend(outcome.errors);
                // 已无残留临时文件时 journal 不再需要;否则保留待下次运行恢复
                if unresolved_temp_count(&dir) == 0 {
                    renamer::remove_journal(&journal_file);
                }
            }
            Err(e) => {
                // journal 写不进去:放弃重命名(全部保持原名),零风险
                result.errors.push(e);
                result
                    .warnings
                    .push("无法写入 journal,已放弃重命名阶段".to_string());
            }
        }
    } else if progress.cancelled() {
        finish_cancelled(&mut result, t0);
        return Ok(result);
    }

    drop(dir_lock);

    result.cancelled = progress.cancelled();
    result.failed_count = result.errors.len();
    result.elapsed_ms = t0.elapsed().as_millis() as u64;

    progress.stage(Stage::Done, "处理完成");
    progress.emit(&crate::core::progress::ProgressEvent::Finished {
        result: result.clone(),
    });
    Ok(result)
}

fn finish_cancelled(result: &mut ProcessingResult, t0: Instant) {
    result.cancelled = true;
    result.failed_count = result.errors.len();
    result.elapsed_ms = t0.elapsed().as_millis() as u64;
    result.warnings.push("任务已取消".to_string());
    // 保持目录一致性:取消发生在计划执行之前,无需回滚
}

/// 目录中残留的临时文件数(用于决定是否保留 journal)。
fn unresolved_temp_count(dir: &Path) -> usize {
    std::fs::read_dir(dir)
        .map(|rd| {
            rd.flatten()
                .filter(|e| {
                    e.file_name()
                        .to_string_lossy()
                        .starts_with(&format!("{}_tmp_", crate::platform::INTERNAL_PREFIX))
                })
                .count()
        })
        .unwrap_or(0)
}
