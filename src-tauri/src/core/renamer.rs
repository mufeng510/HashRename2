//! 两阶段重命名执行 + journal + 崩溃恢复(需求 §15/§29/§30)。
//!
//! 安全模型:
//! - 先写 journal(完整计划),再做临时重命名,再做最终重命名,成功后删 journal;
//! - 任意时刻崩溃:journal + 临时文件足以恢复(补完最终重命名或还原原名);
//! - 所有最终重命名使用 no-replace 语义,绝不覆盖;
//! - journal 写失败 → 放弃重命名阶段(文件全部保持原名),零风险。

use crate::core::error::FileError;
use crate::core::progress::{Progress, Stage};
use crate::core::rename_planner::{RenameOp, RenamePlan};
use crate::platform;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// journal 文件名:`.hashrename_journal_<pid>_<token>.json`
pub fn journal_path(dir: &Path, pid: u32, token: &str) -> PathBuf {
    dir.join(format!(
        "{}_journal_{pid}_{token}.json",
        platform::INTERNAL_PREFIX
    ))
}

/// journal 单条记录(公开供集成测试与故障排查使用)。
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct JournalEntry {
    pub temp: String,
    pub original: String,
    #[serde(rename = "final")]
    pub final_name: String,
    /// noop 操作不产生临时文件,恢复时跳过。
    pub noop: bool,
}

/// journal 文件结构(公开供集成测试与故障排查使用)。
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Journal {
    pub version: u32,
    pub token: String,
    pub directory: String,
    pub entries: Vec<JournalEntry>,
}

/// 将计划写入 journal(在任何文件操作之前调用)。
pub fn write_journal(
    dir: &Path,
    pid: u32,
    token: &str,
    plan: &RenamePlan,
) -> Result<PathBuf, FileError> {
    let journal = Journal {
        version: 1,
        token: token.to_string(),
        directory: dir.display().to_string(),
        entries: plan
            .ops
            .iter()
            .map(|op| JournalEntry {
                temp: op.temporary_name.clone(),
                original: op.original_name.clone(),
                final_name: op.final_name.clone(),
                noop: op.is_noop,
            })
            .collect(),
    };
    let path = journal_path(dir, pid, token);
    let data = serde_json::to_vec_pretty(&journal)
        .map_err(|e| FileError::new("plan", &path, format!("journal 序列化失败: {e}")))?;
    // 先写临时 journal 文件再原子改名,避免 journal 写一半留下残缺文件
    let tmp_journal = dir.join(format!(
        "{}_journal_{}_{}.part",
        platform::INTERNAL_PREFIX,
        pid,
        token
    ));
    std::fs::write(&tmp_journal, &data)
        .and_then(|_| std::fs::rename(&tmp_journal, &path))
        .map_err(|e| {
            let _ = std::fs::remove_file(&tmp_journal);
            FileError::new(
                "plan",
                &path,
                format!("无法写入 journal,为安全起见放弃重命名: {e}"),
            )
        })?;
    Ok(path)
}

#[derive(Debug, Default)]
pub struct RecoveryReport {
    /// 补完的最终重命名数(上次运行中断在阶段 8)。
    pub recovered_finals: usize,
    /// 还原为原名的临时文件数。
    pub restored_temps: usize,
    /// 无法处理的遗留临时文件数(无 journal 信息)。
    pub orphan_temps: usize,
    /// 清理的 journal 数。
    pub removed_journals: usize,
    pub errors: Vec<FileError>,
    pub warnings: Vec<String>,
}

/// 运行开始时恢复上次中断的重命名(需求 §29)。
pub fn recover_pending(dir: &Path, progress: &Progress) -> RecoveryReport {
    let mut report = RecoveryReport::default();

    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return report,
    };

    let mut journals: Vec<PathBuf> = Vec::new();
    let mut orphan_temps: Vec<PathBuf> = Vec::new();

    for entry in entries.flatten() {
        let name = entry.file_name();
        let name_str = name.to_string_lossy().to_string();
        if name_str.starts_with(&format!("{}_journal_", platform::INTERNAL_PREFIX)) {
            journals.push(entry.path());
        } else if name_str.starts_with(&format!("{}_tmp_", platform::INTERNAL_PREFIX)) {
            orphan_temps.push(entry.path());
        }
    }

    // 无 journal 的孤儿临时文件:无法知道原名,保留并警告(不删除用户数据)
    report.orphan_temps = orphan_temps.len();
    if !orphan_temps.is_empty() {
        for p in &orphan_temps {
            report.warnings.push(format!(
                "发现无法识别来源的临时文件 {},已排除在处理之外,请手动检查",
                p.display()
            ));
        }
    }

    for jpath in journals {
        match std::fs::read(&jpath)
            .ok()
            .and_then(|d| serde_json::from_slice::<Journal>(&d).ok())
        {
            Some(journal) => {
                for e in &journal.entries {
                    if e.noop {
                        continue;
                    }
                    let temp = dir.join(&e.temp);
                    let final_p = dir.join(&e.final_name);
                    let orig = dir.join(&e.original);
                    if temp.exists() {
                        // 中断的文件:优先补完最终重命名(保留已完成操作的语义)
                        match platform::rename_no_replace(&temp, &final_p) {
                            Ok(()) => report.recovered_finals += 1,
                            Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => {
                                // 目标被占:还原原名
                                match platform::rename_no_replace(&temp, &orig) {
                                    Ok(()) => report.restored_temps += 1,
                                    Err(e2) => report.errors.push(FileError::new(
                                        "recover",
                                        &temp,
                                        format!("无法恢复临时文件: {e2}"),
                                    )),
                                }
                            }
                            Err(err) => report.errors.push(FileError::new(
                                "recover",
                                &temp,
                                format!("无法完成中断的重命名: {err}"),
                            )),
                        }
                    }
                    // temp 不存在:要么原文件还在(未开始),要么已完成 —— 均无需处理
                }
                match std::fs::remove_file(&jpath) {
                    Ok(()) => report.removed_journals += 1,
                    Err(e) => report.errors.push(FileError::new(
                        "recover",
                        &jpath,
                        format!("无法删除 journal: {e}"),
                    )),
                }
            }
            None => {
                // journal 损坏:无法恢复,保留现场并警告(不删除,方便排查)
                report.warnings.push(format!(
                    "journal 文件损坏,无法自动恢复: {}",
                    jpath.display()
                ));
            }
        }
    }

    if report.recovered_finals > 0 || report.restored_temps > 0 {
        progress.warn(format!(
            "已恢复上次中断的任务:补完重命名 {} 个,还原 {} 个",
            report.recovered_finals, report.restored_temps
        ));
    }
    report
}

#[derive(Debug, Default)]
pub struct ExecOutcome {
    pub renamed: usize,
    /// 临时改名失败后回滚导致的整批中止。
    pub aborted: bool,
    pub errors: Vec<FileError>,
}

/// 执行两阶段重命名(阶段 7:临时重命名;阶段 8:最终重命名)。
pub fn execute_plan(dir: &Path, plan: &RenamePlan, progress: &Progress) -> ExecOutcome {
    let mut outcome = ExecOutcome::default();
    if plan.ops.is_empty() {
        return outcome;
    }

    progress.stage(Stage::Rename, "正在执行重命名计划...");

    // ---- 阶段 7:原文件 → 临时唯一名 ----
    let mut tempped: Vec<&RenameOp> = Vec::new();
    let total = plan.ops.len();
    for (k, op) in plan.ops.iter().enumerate() {
        if progress.cancelled() {
            rollback_temps(dir, &tempped, &mut outcome);
            outcome.aborted = true;
            return outcome;
        }
        if op.is_noop {
            outcome.renamed += 1; // 本来就是目标名
            progress.tick_range((k + 1) as f64 / total as f64, 0.82, 0.91);
            continue;
        }
        let temp_path = dir.join(&op.temporary_name);
        match platform::rename_with_retry(&op.original, &temp_path) {
            Ok(()) => tempped.push(op),
            Err(e) => {
                outcome.errors.push(FileError::new(
                    "rename",
                    &op.original,
                    format!("无法重命名为临时名: {e}"),
                ));
                // 任一临时改名失败:回滚已改名的,整体中止,目录回到一致状态
                rollback_temps(dir, &tempped, &mut outcome);
                outcome.aborted = true;
                progress.warn("重命名阶段遇到错误,已回滚全部临时改名");
                return outcome;
            }
        }
        progress.current_file(&op.original_name);
        progress.tick_range((k + 1) as f64 / total as f64, 0.82, 0.91);
    }

    // ---- 阶段 8:临时名 → 最终序号名(no-replace,绝不覆盖) ----
    let mut unresolved_temps: Vec<PathBuf> = Vec::new();
    for (k, op) in plan.ops.iter().enumerate() {
        if op.is_noop {
            continue;
        }
        if progress.cancelled() {
            break;
        }
        let temp_path = dir.join(&op.temporary_name);
        let final_path = dir.join(&op.final_name);
        match platform::rename_no_replace(&temp_path, &final_path) {
            Ok(()) => outcome.renamed += 1,
            Err(e) => {
                let msg = if e.kind() == std::io::ErrorKind::AlreadyExists {
                    format!("目标名 {} 已存在,拒绝覆盖", op.final_name)
                } else {
                    format!("最终重命名失败: {e}")
                };
                outcome
                    .errors
                    .push(FileError::new("rename", &op.original, msg));
                // 还原该文件到原名(原名已被腾出);还原失败则留下临时文件
                match platform::rename_no_replace(&temp_path, &op.original) {
                    Ok(()) => {}
                    Err(_) => unresolved_temps.push(temp_path),
                }
            }
        }
        progress.tick_range((k + 1) as f64 / total as f64, 0.91, 1.0);
    }

    // 取消或错误导致的未处理临时文件:尝试全部还原
    if progress.cancelled() || !outcome.errors.is_empty() {
        for op in plan.ops.iter() {
            if op.is_noop {
                continue;
            }
            let temp_path = dir.join(&op.temporary_name);
            if temp_path.exists() {
                match platform::rename_no_replace(&temp_path, &op.original) {
                    Ok(()) => {}
                    Err(_) => unresolved_temps.push(temp_path),
                }
            }
        }
    }

    if !unresolved_temps.is_empty() {
        progress.warn(format!(
            "有 {} 个临时文件未能还原,journal 已保留,下次运行时将自动恢复",
            unresolved_temps.len()
        ));
    }
    outcome
}

/// 回滚阶段 7 的临时改名(临时 → 原名;原名已被腾出,普通 rename 足够,
/// 但仍用 no-replace 防御外部竞争)。
fn rollback_temps(dir: &Path, tempped: &[&RenameOp], outcome: &mut ExecOutcome) {
    for op in tempped {
        let temp_path = dir.join(&op.temporary_name);
        if let Err(e) = platform::rename_no_replace(&temp_path, &op.original) {
            outcome.errors.push(FileError::new(
                "rename",
                &temp_path,
                format!("回滚临时改名失败: {e}"),
            ));
        }
    }
}

/// 删除本运行的 journal(仅在重命名完全干净时调用)。
pub fn remove_journal(path: &Path) {
    let _ = std::fs::remove_file(path);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::models::FileEntry;
    use crate::core::rename_planner::{plan_renames, OccupiedNames};

    fn tmpdir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!(
            "hr_ren_{tag}_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .subsec_nanos()
        ));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn fe(dir: &Path, name: &str, content: &[u8]) -> FileEntry {
        std::fs::write(dir.join(name), content).unwrap();
        let ext = Path::new(name)
            .extension()
            .map(|e| e.to_string_lossy().to_string());
        FileEntry {
            path: dir.join(name),
            file_name: name.to_string(),
            extension: ext,
            size: content.len() as u64,
            hash: None,
        }
    }

    #[test]
    fn two_phase_rename_happy_path() {
        let dir = tmpdir("happy");
        let remaining = vec![
            fe(&dir, "zeta.jpg", b"a"),
            fe(&dir, "alpha.jpg", b"b"),
            fe(&dir, "mid.png", b"c"),
        ];
        let occ = OccupiedNames::new();
        let plan = plan_renames(&remaining, &occ, 42, "tok1");
        assert_eq!(plan.ops.len(), 3);

        let jp = write_journal(&dir, 42, "tok1", &plan).unwrap();
        let outcome = execute_plan(&dir, &plan, &Progress::default());
        assert!(!outcome.aborted);
        assert!(outcome.errors.is_empty());
        assert_eq!(outcome.renamed, 3);

        remove_journal(&jp);
        assert!(!jp.exists());
        let mut names: Vec<String> = std::fs::read_dir(&dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().to_string())
            .collect();
        names.sort();
        assert_eq!(names, vec!["001.jpg", "002.jpg", "003.png"]);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn existing_target_never_overwritten() {
        let dir = tmpdir("nooverwrite");
        // 子目录 001.txt 占用 1 号位;a.txt 被阻塞保持原名,b.jpg 占 2 号位
        let remaining = vec![fe(&dir, "a.txt", b"data-a"), fe(&dir, "b.jpg", b"data-b")];
        let mut occ = OccupiedNames::new();
        std::fs::create_dir(dir.join("001.txt")).unwrap();
        occ.insert("001.txt");
        let plan = plan_renames(&remaining, &occ, 42, "tok2");
        // a.txt → 001.txt 冲突被阻塞;b.jpg → 002.jpg
        assert_eq!(plan.blocked.len(), 1);
        assert_eq!(plan.ops.len(), 1);
        assert_eq!(plan.ops[0].final_name, "002.jpg");

        let jp = write_journal(&dir, 42, "tok2", &plan).unwrap();
        let outcome = execute_plan(&dir, &plan, &Progress::default());
        assert_eq!(outcome.renamed, 1);
        // 子目录未被动过
        assert!(dir.join("001.txt").is_dir());
        // 被阻塞文件保持原名
        assert!(dir.join("a.txt").exists());
        assert!(dir.join("002.jpg").exists());
        assert_eq!(std::fs::read(dir.join("002.jpg")).unwrap(), b"data-b");
        remove_journal(&jp);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn recovery_completes_interrupted_renames() {
        let dir = tmpdir("recover");
        // 模拟崩溃现场:部分文件处于临时名状态(在途),部分已完成,部分未开始。
        // 恢复语义:仅处理"在途"文件(补完最终重命名);未开始的下轮全量处理。
        let plan_ops = [
            ("old1.jpg", "001.jpg"),
            ("old2.jpg", "002.jpg"),
            ("old3.png", "003.png"),
        ];
        let token = "deadbeef";
        let journal = Journal {
            version: 1,
            token: token.to_string(),
            directory: dir.display().to_string(),
            entries: plan_ops
                .iter()
                .enumerate()
                .map(|(i, (o, f))| JournalEntry {
                    temp: format!(".hashrename_tmp_99_{token}_{}", i + 1),
                    original: o.to_string(),
                    final_name: f.to_string(),
                    noop: false,
                })
                .collect(),
        };
        let jp = journal_path(&dir, 99, token);
        std::fs::write(&jp, serde_json::to_vec(&journal).unwrap()).unwrap();
        for (i, (orig, _)) in plan_ops.iter().enumerate() {
            match i {
                0 => {
                    std::fs::write(dir.join(format!(".hashrename_tmp_99_{token}_1")), b"1").unwrap()
                }
                1 => std::fs::write(dir.join("002.jpg"), b"2").unwrap(), // 已完成
                _ => std::fs::write(dir.join(orig), b"3").unwrap(),      // 未开始
            }
        }

        let report = recover_pending(&dir, &Progress::default());
        assert_eq!(report.recovered_finals, 1, "只有 1 个在途文件被补完");
        assert_eq!(report.removed_journals, 1);
        assert!(report.errors.is_empty());
        // 在途文件补完;已完成保持;未开始保持原名(下轮全量处理)
        assert!(dir.join("001.jpg").exists());
        assert!(dir.join("002.jpg").exists());
        assert!(dir.join("old3.png").exists());
        assert!(!dir.join("003.png").exists());
        assert!(!jp.exists());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn recovery_restores_when_final_taken() {
        let dir = tmpdir("restore");
        let token = "feedface";
        let journal = Journal {
            version: 1,
            token: token.to_string(),
            directory: dir.display().to_string(),
            entries: vec![JournalEntry {
                temp: format!(".hashrename_tmp_7_{token}_1"),
                original: "orig.txt".to_string(),
                final_name: "001.txt".to_string(),
                noop: false,
            }],
        };
        let jp = journal_path(&dir, 7, token);
        std::fs::write(&jp, serde_json::to_vec(&journal).unwrap()).unwrap();
        std::fs::write(dir.join(format!(".hashrename_tmp_7_{token}_1")), b"x").unwrap();
        // 目标名被外部程序占用了
        std::fs::write(dir.join("001.txt"), b"other").unwrap();

        let report = recover_pending(&dir, &Progress::default());
        assert_eq!(report.restored_temps, 1);
        assert_eq!(
            std::fs::read(dir.join("orig.txt")).unwrap(),
            b"x",
            "临时文件应还原为原名,外部文件不可被覆盖"
        );
        assert_eq!(std::fs::read(dir.join("001.txt")).unwrap(), b"other");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn journal_write_failure_aborts_before_any_rename() {
        // journal 目标路径被目录占用时,write_journal 必须失败且不产生任何临时文件
        let dir = tmpdir("jfail");
        let remaining = vec![fe(&dir, "a.jpg", b"a")];
        let plan = plan_renames(&remaining, &OccupiedNames::new(), 1, "t");
        let jp = journal_path(&dir, platform::current_pid(), "t");
        std::fs::create_dir(&jp).unwrap(); // 让 journal 路径无法写入
        let r = write_journal(&dir, platform::current_pid(), "t", &plan);
        assert!(r.is_err(), "journal 写入失败必须被报告");
        // 没有产生任何临时文件
        let names: Vec<String> = std::fs::read_dir(&dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().to_string())
            .filter(|n| n.starts_with(".hashrename_tmp_"))
            .collect();
        assert!(names.is_empty());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
