//! 重命名规划(需求 §12/§13/§14/§15/§30)。
//!
//! - 剩余文件按原始文件名自然排序;
//! - 统一编号 `001` 起,位数 = max(3, 需重命名文件数的十进制位数);
//! - 扩展名原样保留(大小写不变);
//! - 目标名与"不参与重命名的条目"(子目录/符号链接/特殊文件/内部文件/
//!   被阻塞文件的原名)冲突时,该文件不编号、保持原名并报告,
//!   **绝不覆盖**;
//! - 临时名 `.hashrename_tmp_<pid>_<token>_<seq>` 由进程号 + 随机 token
//!   保证唯一,天然规避多实例并发冲突。

use crate::core::models::FileEntry;
use crate::platform;
use std::collections::HashSet;

#[derive(Debug, Clone)]
pub struct RenameOp {
    pub entry_index: usize,
    pub original: std::path::PathBuf,
    pub original_name: String,
    pub temporary_name: String,
    pub final_name: String,
    pub is_noop: bool,
}

#[derive(Debug, Clone)]
pub struct BlockedRename {
    pub entry_index: usize,
    pub original_name: String,
    pub reason: String,
}

#[derive(Debug, Default)]
pub struct RenamePlan {
    pub ops: Vec<RenameOp>,
    pub blocked: Vec<BlockedRename>,
    /// 序号位数。
    pub width: usize,
}

/// 不参与重命名的已占用名称集合(按平台大小写敏感性比较)。
#[derive(Debug, Default)]
pub struct OccupiedNames {
    names: HashSet<String>,
    ci_keys: HashSet<String>,
}

impl OccupiedNames {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&mut self, name: &str) {
        self.names.insert(name.to_string());
        if !platform::case_sensitive_fs() {
            self.ci_keys.insert(name.to_lowercase());
        }
    }

    pub(crate) fn extend_from(&mut self, other: &OccupiedNames) {
        for n in &other.names {
            self.insert(n);
        }
    }

    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.names.iter().map(|s| s.as_str())
    }

    pub fn contains(&self, name: &str) -> bool {
        if platform::case_sensitive_fs() {
            self.names.contains(name)
        } else {
            self.ci_keys.contains(&name.to_lowercase())
        }
    }
}

/// 数字序号位数(需求 §13):位数 = max(3, count 的十进制位数)。
pub fn number_width(count: usize) -> usize {
    count.to_string().len().max(3)
}

/// 生成最终序号文件名:`001` / `0001` + `.` + 扩展名(原样保留)。
pub fn format_numbered_name(number: usize, width: usize, extension: Option<&str>) -> String {
    match extension {
        Some(ext) if !ext.is_empty() => format!("{:0width$}.{}", number, ext, width = width),
        _ => format!("{:0width$}", number, width = width),
    }
}

/// 为剩余文件制定两阶段重命名计划。
///
/// `remaining` 必须已按自然排序(扫描器保证)。
///
/// 编号语义:**按排序位置占号** —— 第 i 个文件的目标名固定为
/// `(i+1).扩展名`,这样最终编号 001..N 与排序一一对应、完全确定
/// (N = 剩余文件数,位数 = max(3, digits(N)))。
///
/// 若某文件的目标名与"不参与重命名的条目"(子目录/符号链接/特殊文件/
/// 内部文件)冲突,该文件被阻塞(保持原名并报告,绝不覆盖),
/// 其号码空缺。级联检查:被阻塞文件的原名若恰好是另一个文件的
/// 目标名,则后者同样阻塞,循环至稳定(每轮至少阻塞 1 个,必然终止)。
pub fn plan_renames(
    remaining: &[FileEntry],
    occupied_base: &OccupiedNames,
    pid: u32,
    token: &str,
) -> RenamePlan {
    let width = number_width(remaining.len());
    let mut occupied = OccupiedNames::new();
    occupied.extend_from(occupied_base);

    // 第一遍:按位置占号,目标名冲突者阻塞
    let mut slots: Vec<Option<RenameOp>> = Vec::with_capacity(remaining.len());
    let mut blocked: Vec<BlockedRename> = Vec::new();

    for (i, fe) in remaining.iter().enumerate() {
        let final_name = format_numbered_name(i + 1, width, fe.extension.as_deref());
        if occupied.contains(&final_name) {
            blocked.push(BlockedRename {
                entry_index: i,
                original_name: fe.file_name.clone(),
                reason: format!(
                    "目标名 {final_name} 与目录中不参与重命名的条目冲突,为避免覆盖已跳过该文件"
                ),
            });
            occupied.insert(&fe.file_name);
            slots.push(None);
            continue;
        }
        let temp_name = format!("{}_tmp_{pid}_{token}_{}", platform::INTERNAL_PREFIX, i + 1);
        slots.push(Some(RenameOp {
            entry_index: i,
            original: fe.path.clone(),
            original_name: fe.file_name.clone(),
            temporary_name: temp_name,
            is_noop: fe.file_name == final_name,
            final_name,
        }));
    }

    // 级联收敛:被阻塞文件的原名可能与某 op 的最终名相同
    loop {
        let conflict = slots.iter().position(|s| match s {
            Some(op) => !op.is_noop && occupied.contains(&op.final_name),
            None => false,
        });
        let Some(i) = conflict else { break };
        let op = slots[i].take().expect("position 指向存在的 op");
        blocked.push(BlockedRename {
            entry_index: op.entry_index,
            original_name: op.original_name.clone(),
            reason: format!(
                "目标名 {} 与其他保留文件的原名冲突,为避免覆盖已跳过该文件",
                op.final_name
            ),
        });
        occupied.insert(&op.original_name);
    }

    RenamePlan {
        ops: slots.into_iter().flatten().collect(),
        blocked,
        width,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fe(name: &str) -> FileEntry {
        let ext = std::path::Path::new(name)
            .extension()
            .map(|e| e.to_string_lossy().to_string());
        FileEntry {
            path: std::path::PathBuf::from(name),
            file_name: name.to_string(),
            extension: ext,
            size: 1,
            hash: None,
        }
    }

    fn plan_for(names: &[&str], extra_occupied: &[&str]) -> RenamePlan {
        let mut remaining: Vec<FileEntry> = names.iter().map(|n| fe(n)).collect();
        crate::core::sorter::sort_names(&mut remaining, |f| &f.file_name);
        let mut occ = OccupiedNames::new();
        for e in extra_occupied {
            occ.insert(e);
        }
        plan_renames(&remaining, &occ, 1234, "aabbccdd")
    }

    #[test]
    fn basic_numbering() {
        let plan = plan_for(
            &["Apple.jpg", "IMG_002.jpg", "IMG_100.png", "test.webp"],
            &[],
        );
        let finals: Vec<&str> = plan.ops.iter().map(|o| o.final_name.as_str()).collect();
        assert_eq!(finals, vec!["001.jpg", "002.jpg", "003.png", "004.webp"]);
        assert_eq!(plan.width, 3);
    }

    #[test]
    fn width_grows_with_count() {
        assert_eq!(number_width(1), 3);
        assert_eq!(number_width(9), 3);
        assert_eq!(number_width(10), 3);
        assert_eq!(number_width(99), 3);
        assert_eq!(number_width(100), 3);
        assert_eq!(number_width(999), 3);
        assert_eq!(number_width(1000), 4);
        assert_eq!(number_width(9999), 4);
        assert_eq!(number_width(10000), 5);
    }

    #[test]
    fn preserves_extension_case() {
        let plan = plan_for(&["a.JPG", "b.Tar.gz"], &[]);
        assert_eq!(plan.ops[0].final_name, "001.JPG");
        assert_eq!(plan.ops[1].final_name, "002.gz");
    }

    #[test]
    fn no_extension_no_dot() {
        let plan = plan_for(&["Makefile", "README"], &[]);
        assert_eq!(plan.ops[0].final_name, "001");
        assert_eq!(plan.ops[1].final_name, "002");
    }

    #[test]
    fn conflict_with_directory_blocks_file() {
        // 目录里已有子目录 `001.jpg`(不参与重命名、永不腾出名字)
        // 位置占号:A.jpg 占 001(冲突阻塞),B.jpg 占 002 正常改名
        let plan = plan_for(&["A.jpg", "B.jpg"], &["001.jpg"]);
        assert_eq!(plan.ops.len(), 1);
        assert_eq!(plan.ops[0].original_name, "B.jpg");
        assert_eq!(plan.ops[0].final_name, "002.jpg");
        assert_eq!(plan.blocked.len(), 1);
        assert_eq!(plan.blocked[0].original_name, "A.jpg");
    }

    #[test]
    fn conflict_cascades_converge() {
        // remaining 排序:002.jpg(数字段) < c.jpg(文本段)。
        // 002.jpg 占 001 → 与子目录 001.jpg 冲突 → 阻塞,其原名 "002.jpg"
        // 进入占用集;c.jpg 占 002 → 与之冲突 → 级联阻塞,全部安全跳过。
        let plan = plan_for(&["c.jpg", "002.jpg"], &["001.jpg"]);
        assert_eq!(plan.ops.len(), 0);
        assert_eq!(plan.blocked.len(), 2);
    }

    #[test]
    fn blocked_original_name_guards_later_ops() {
        // 与上一测试同型:阻塞文件的原名不得被其他文件的目标名覆盖。
        let mut remaining = vec![fe("002.jpg"), fe("a.jpg")];
        crate::core::sorter::sort_names(&mut remaining, |f| &f.file_name);
        let mut occ = OccupiedNames::new();
        occ.insert("001.jpg"); // 目录已存在子目录 001.jpg → "002.jpg" 被阻塞
        let plan = plan_renames(&remaining, &occ, 1, "tok");
        assert!(
            plan.ops.is_empty(),
            "a.jpg 不应获得会覆盖阻塞文件原名的名字"
        );
        assert_eq!(plan.blocked.len(), 2);
    }

    #[test]
    fn temp_names_unique_and_prefixed() {
        let plan = plan_for(&["a.jpg", "b.jpg", "c.jpg"], &[]);
        let temps: HashSet<&str> = plan.ops.iter().map(|o| o.temporary_name.as_str()).collect();
        assert_eq!(temps.len(), plan.ops.len());
        for t in temps {
            assert!(t.starts_with(".hashrename_tmp_"));
            assert!(!t.ends_with(".jpg"), "临时名不能带原扩展名");
        }
    }

    #[test]
    fn noop_when_already_numbered() {
        let plan = plan_for(&["001.jpg", "002.jpg"], &[]);
        assert!(plan.ops[0].is_noop);
        assert!(plan.ops[1].is_noop);
    }
}
