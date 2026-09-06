//! 端到端集成测试:通过 `process_directory` 完整走一遍九阶段流水线。

use hashrename_lib::core::error::HrError;
use hashrename_lib::core::hasher::Md5Hasher;
use hashrename_lib::core::models::ProcessingResult;
use hashrename_lib::core::processor::{process_directory, ProcessOptions};
use hashrename_lib::core::progress::Progress;
use hashrename_lib::core::trash::{OsTrash, TrashProvider};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

/// 测试用回收站:把文件移动到隔离目录(模拟回收站,可恢复、不污染系统)。
struct TestTrash {
    quarantine: PathBuf,
    trashed: Mutex<Vec<PathBuf>>,
}

impl TestTrash {
    fn new(root: &Path) -> Self {
        let quarantine = root.join("_quarantine(trash)");
        std::fs::create_dir_all(&quarantine).unwrap();
        TestTrash {
            quarantine,
            trashed: Mutex::new(Vec::new()),
        }
    }
}

impl TrashProvider for TestTrash {
    fn name(&self) -> &'static str {
        "test-quarantine"
    }
    fn send_to_trash(&self, path: &Path) -> Result<(), HrError> {
        let dest = self.quarantine.join(path.file_name().unwrap());
        std::fs::rename(path, &dest).map_err(|e| HrError::Trash {
            path: path.display().to_string(),
            message: e.to_string(),
        })?;
        self.trashed.lock().unwrap().push(dest);
        Ok(())
    }
}

fn tmpdir(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!(
        "hr_e2e_{tag}_{}_{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .subsec_nanos()
    ));
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn run(root: &Path) -> ProcessingResult {
    run_with(root, &root.join("work"))
}

fn run_with(root: &Path, work_dir: &Path) -> ProcessingResult {
    let trash = Arc::new(TestTrash::new(root));
    process_directory(
        work_dir,
        &ProcessOptions::default(),
        &Progress::default(),
        trash,
        Arc::new(Md5Hasher),
    )
    .expect("处理不应致命失败")
}

fn names_of(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().to_string())
        .filter(|n| !n.starts_with("_quarantine") && !n.starts_with(".hashrename"))
        .collect();
    names.sort();
    names
}

fn contents_of(dir: &Path, name: &str) -> Vec<u8> {
    std::fs::read(dir.join(name)).unwrap()
}

// ---------------- 基础去重 + 重命名 ----------------

#[test]
fn basic_dedupe_and_rename() {
    let root = tmpdir("basic");
    let work = root.join("work");
    std::fs::create_dir_all(&work).unwrap();
    // 内容相同但扩展名不同(需求 §7)
    std::fs::write(work.join("IMG_10.jpg"), b"SAME").unwrap();
    std::fs::write(work.join("IMG_2.jpg"), b"SAME").unwrap();
    std::fs::write(work.join("IMG_1.jpg"), b"SAME").unwrap();
    std::fs::write(work.join("unique.png"), b"UNIQUE").unwrap();

    let res = run(&root);
    assert_eq!(res.scanned_count, 4);
    assert_eq!(res.duplicate_count, 2);
    assert_eq!(res.trashed_count, 2);
    assert_eq!(res.kept_count, 2);
    assert_eq!(res.renamed_count, 2);
    assert_eq!(res.failed_count, 0, "errors: {:?}", res.errors);

    // 保留自然排序最靠前的 IMG_1.jpg(需求 §10)
    assert_eq!(names_of(&work), vec!["001.jpg", "002.png"]);
    assert_eq!(contents_of(&work, "001.jpg"), b"SAME");
    assert_eq!(contents_of(&work, "002.png"), b"UNIQUE");
}

#[test]
fn natural_sort_order_in_rename() {
    let root = tmpdir("sort");
    let work = root.join("work");
    std::fs::create_dir_all(&work).unwrap();
    for n in ["10.jpg", "2.jpg", "1.jpg", "20.jpg"] {
        std::fs::write(work.join(n), n.as_bytes()).unwrap();
    }
    let res = run(&root);
    assert_eq!(res.renamed_count, 4);
    assert_eq!(
        names_of(&work),
        vec!["001.jpg", "002.jpg", "003.jpg", "004.jpg"]
    );
    // 排序正确性:001 ← 1.jpg,002 ← 2.jpg,003 ← 10.jpg,004 ← 20.jpg
    assert_eq!(contents_of(&work, "001.jpg"), b"1.jpg");
    assert_eq!(contents_of(&work, "002.jpg"), b"2.jpg");
    assert_eq!(contents_of(&work, "003.jpg"), b"10.jpg");
    assert_eq!(contents_of(&work, "004.jpg"), b"20.jpg");
}

#[test]
fn unicode_and_space_names() {
    let root = tmpdir("unicode");
    let work = root.join("work");
    std::fs::create_dir_all(&work).unwrap();
    std::fs::write(work.join("照片 10.jpg"), b"a").unwrap();
    std::fs::write(work.join("照片 2.jpg"), b"b").unwrap();
    std::fs::write(work.join("写真 🌸.jpg"), b"c").unwrap();

    let res = run(&root);
    assert_eq!(res.failed_count, 0, "errors: {:?}", res.errors);
    assert_eq!(
        names_of(&work),
        vec!["001.jpg", "002.jpg", "003.jpg"],
        "中文/emoji/空格文件名必须被正确处理"
    );
}

// ---------------- 只处理当前目录 ----------------

#[test]
fn subdirectories_never_touched() {
    let root = tmpdir("subdir");
    let work = root.join("work");
    std::fs::create_dir_all(work.join("sub")).unwrap();
    std::fs::write(work.join("a.jpg"), b"A").unwrap();
    std::fs::write(work.join("sub").join("a.jpg"), b"A").unwrap(); // 子目录同名同内容
    std::fs::write(work.join("sub").join("b.jpg"), b"B").unwrap();

    let res = run(&root);
    // 子目录条目被跳过,不参与去重(需求 §5/§6)
    assert_eq!(res.scanned_count, 1);
    assert_eq!(res.skipped_count, 1);
    assert_eq!(res.duplicate_count, 0);
    // 子目录内容原样
    assert_eq!(contents_of(&work.join("sub"), "a.jpg"), b"A");
    assert_eq!(contents_of(&work.join("sub"), "b.jpg"), b"B");
}

#[cfg(unix)]
#[test]
fn symlinks_are_skipped_entirely() {
    let root = tmpdir("symlink");
    let work = root.join("work");
    let outside = root.join("outside");
    std::fs::create_dir_all(&work).unwrap();
    std::fs::create_dir_all(&outside).unwrap();
    std::fs::write(outside.join("target.jpg"), b"TARGET").unwrap();
    std::os::unix::fs::symlink(outside.join("target.jpg"), work.join("link.jpg")).unwrap();
    std::os::unix::fs::symlink(&outside, work.join("linkdir")).unwrap();
    std::fs::write(work.join("real.jpg"), b"REAL").unwrap();

    let res = run(&root);
    assert_eq!(res.scanned_count, 1);
    assert_eq!(res.skipped_count, 2, "符号链接(文件/目录)都应跳过");
    assert_eq!(contents_of(&outside, "target.jpg"), b"TARGET");
    assert_eq!(names_of(&work), vec!["001.jpg", "link.jpg", "linkdir"]);
}

// ---------------- 大小预筛选与内容判定 ----------------

#[test]
fn same_size_different_content_not_duplicates() {
    let root = tmpdir("samesize");
    let work = root.join("work");
    std::fs::create_dir_all(&work).unwrap();
    std::fs::write(work.join("x.bin"), b"AAAA").unwrap();
    std::fs::write(work.join("y.bin"), b"BBBB").unwrap();
    std::fs::write(work.join("z.bin"), b"C").unwrap(); // 不同大小

    let res = run(&root);
    assert_eq!(res.duplicate_count, 0);
    assert_eq!(res.renamed_count, 3);
    assert_eq!(names_of(&work), vec!["001.bin", "002.bin", "003.bin"]);
    assert_eq!(contents_of(&work, "001.bin"), b"AAAA");
    assert_eq!(contents_of(&work, "002.bin"), b"BBBB");
    assert_eq!(contents_of(&work, "003.bin"), b"C");
}

// ---------------- 编号位数(需求 §13) ----------------

#[test]
fn width_stays_three_up_to_999() {
    let root = tmpdir("w99");
    let work = root.join("work");
    std::fs::create_dir_all(&work).unwrap();
    for i in 0..100 {
        std::fs::write(work.join(format!("f{i:04}.dat")), format!("{i}").as_bytes()).unwrap();
    }
    let res = run(&root);
    assert_eq!(res.renamed_count, 100);
    let names = names_of(&work);
    assert_eq!(names[0], "001.dat");
    assert_eq!(names[99], "100.dat");
    assert_eq!(names.len(), 100);
}

#[test]
fn width_grows_to_four_at_1000() {
    let root = tmpdir("w1000");
    let work = root.join("work");
    std::fs::create_dir_all(&work).unwrap();
    for i in 0..1000 {
        std::fs::write(work.join(format!("f{i:05}.dat")), format!("{i}").as_bytes()).unwrap();
    }
    let res = run(&root);
    assert_eq!(res.renamed_count, 1000);
    let names = names_of(&work);
    assert_eq!(names[0], "0001.dat", "1000 个文件 → 4 位编号");
    assert_eq!(names[999], "1000.dat");
}

// ---------------- 冲突与不覆盖(需求 §15/§30) ----------------

#[test]
fn existing_numbered_names_are_handled_safely() {
    let root = tmpdir("conflict");
    let work = root.join("work");
    std::fs::create_dir_all(&work).unwrap();
    // 目录里已经有 001.jpg / 002.jpg(参与重命名的普通文件,会先腾名)
    std::fs::write(work.join("001.jpg"), b"old1").unwrap();
    std::fs::write(work.join("002.jpg"), b"old2").unwrap();
    std::fs::write(work.join("ABC.jpg"), b"abc").unwrap();

    let res = run(&root);
    assert_eq!(res.renamed_count, 3, "errors: {:?}", res.errors);
    assert_eq!(names_of(&work), vec!["001.jpg", "002.jpg", "003.jpg"]);
    assert_eq!(contents_of(&work, "001.jpg"), b"old1");
    assert_eq!(contents_of(&work, "002.jpg"), b"old2");
    assert_eq!(contents_of(&work, "003.jpg"), b"abc");
}

#[test]
fn directory_named_001_is_never_overwritten() {
    let root = tmpdir("dirconflict");
    let work = root.join("work");
    std::fs::create_dir_all(work.join("001.jpg")).unwrap(); // 名为 001.jpg 的子目录!
    std::fs::write(work.join("001.jpg").join("inner.txt"), b"inner").unwrap();
    std::fs::write(work.join("photo.jpg"), b"photo").unwrap();

    let res = run(&root);
    // photo.jpg 占 1 号位(001.jpg)→ 冲突阻塞,保持原名,绝不覆盖
    assert!(
        res.errors.iter().any(|e| e.operation == "rename"),
        "应报告冲突: {:?}",
        res.errors
    );
    assert!(work.join("001.jpg").is_dir(), "子目录必须原样保留");
    assert_eq!(contents_of(&work.join("001.jpg"), "inner.txt"), b"inner");
    assert!(work.join("photo.jpg").exists(), "原文件保持原名");
    assert_eq!(contents_of(&work, "photo.jpg"), b"photo");
}

// ---------------- 回收站(需求 §11/§41) ----------------

#[test]
fn duplicates_go_to_quarantine_not_permanently_deleted() {
    let root = tmpdir("quarantine");
    let work = root.join("work");
    std::fs::create_dir_all(&work).unwrap();
    std::fs::write(work.join("keep.bin"), b"CONTENT-X").unwrap();
    std::fs::write(work.join("dup.bin"), b"CONTENT-X").unwrap();

    let trash = Arc::new(TestTrash::new(&root));
    let res = process_directory(
        &work,
        &ProcessOptions::default(),
        &Progress::default(),
        trash.clone(),
        Arc::new(Md5Hasher),
    )
    .unwrap();

    assert_eq!(res.trashed_count, 1);
    assert_eq!(trash.trashed.lock().unwrap().len(), 1);
    // 被移走的文件在隔离区中内容完好(未永久删除,可恢复)
    let trashed_path = trash.trashed.lock().unwrap()[0].clone();
    assert_eq!(
        std::fs::read(&trashed_path).unwrap(),
        b"CONTENT-X",
        "回收站中的文件内容必须完好"
    );
    assert!(!work.join("dup.bin").exists());
    assert!(work.join("001.bin").exists());
}

#[test]
#[cfg(target_os = "linux")]
fn real_system_trash_receives_duplicates() {
    // 真实回收站验证:重复文件进入 ~/.local/share/Trash(未永久删除)
    let root = tmpdir("realtash");
    let work = root.join("work");
    std::fs::create_dir_all(&work).unwrap();
    let marker: &[u8] = b"HASHRENAME-REAL-TRASH-MARKER-9f3a";
    std::fs::write(work.join("a1.txt"), marker).unwrap();
    std::fs::write(work.join("a2.txt"), marker).unwrap();

    let res = process_directory(
        &work,
        &ProcessOptions::default(),
        &Progress::default(),
        Arc::new(OsTrash),
        Arc::new(Md5Hasher),
    )
    .unwrap();

    if res.trashed_count == 0 {
        // 环境无可用回收站:必须安全失败(文件保留)
        assert!(
            work.join("a1.txt").exists() && work.join("a2.txt").exists(),
            "回收站不可用时文件必须原样保留"
        );
        eprintln!("环境无可用回收站,仅验证了安全失败路径");
        return;
    }

    assert_eq!(res.trashed_count, 1);
    assert!(!work.join("a2.txt").exists() || work.join("001.txt").exists());
    // 在系统回收站中找到该文件(内容一致)
    let home = PathBuf::from(std::env::var("HOME").unwrap());
    let trash_files = home.join(".local/share/Trash/files");
    let mut found = false;
    if let Ok(rd) = std::fs::read_dir(&trash_files) {
        for e in rd.flatten() {
            if std::fs::read(e.path())
                .map(|c| c == marker)
                .unwrap_or(false)
            {
                found = true;
                let _ = trash::delete(e.path()); // 清理测试垃圾
                break;
            }
        }
    }
    assert!(found, "重复文件应真实进入系统回收站且内容完好");
}

#[test]
fn trash_failure_stops_and_preserves_files() {
    // 回收站失败 → 安全停止,文件保持原状(需求 §41)
    struct FailingTrash;
    impl TrashProvider for FailingTrash {
        fn name(&self) -> &'static str {
            "failing"
        }
        fn send_to_trash(&self, _: &Path) -> Result<(), HrError> {
            Err(HrError::Trash {
                path: String::new(),
                message: "模拟:环境无回收站".to_string(),
            })
        }
    }

    let root = tmpdir("trashfail");
    let work = root.join("work");
    std::fs::create_dir_all(&work).unwrap();
    std::fs::write(work.join("a.txt"), b"DUP").unwrap();
    std::fs::write(work.join("b.txt"), b"DUP").unwrap();
    std::fs::write(work.join("c.txt"), b"OTHER").unwrap();

    let res = process_directory(
        &work,
        &ProcessOptions::default(),
        &Progress::default(),
        Arc::new(FailingTrash),
        Arc::new(Md5Hasher),
    )
    .unwrap();

    assert_eq!(res.trashed_count, 0);
    assert!(
        res.errors.iter().any(|e| e.operation == "trash"),
        "应记录回收站失败: {:?}",
        res.errors
    );
    assert!(res.warnings.iter().any(|w| w.contains("回收站")));
    // 重命名阶段照常进行(文件未被删除),三个文件内容全部完好
    assert_eq!(names_of(&work), vec!["001.txt", "002.txt", "003.txt"]);
    assert_eq!(contents_of(&work, "001.txt"), b"DUP");
    assert_eq!(contents_of(&work, "002.txt"), b"DUP");
    assert_eq!(contents_of(&work, "003.txt"), b"OTHER");
}

// ---------------- 并发锁(需求 §19) ----------------

#[test]
fn second_concurrent_run_is_rejected() {
    let root = tmpdir("lock");
    let work = root.join("work");
    std::fs::create_dir_all(&work).unwrap();
    std::fs::write(work.join("a.txt"), b"A").unwrap();

    let _lock = hashrename_lib::core::lock::acquire(&work).unwrap();
    let trash = Arc::new(TestTrash::new(&root));
    let err = process_directory(
        &work,
        &ProcessOptions::default(),
        &Progress::default(),
        trash,
        Arc::new(Md5Hasher),
    )
    .unwrap_err();
    match err {
        HrError::Locked { .. } => {}
        other => panic!("应为 Locked 错误: {other}"),
    }
    // 文件未被动过
    assert!(work.join("a.txt").exists());
    drop(_lock);

    // 释放后可正常处理
    let res = process_directory(
        &work,
        &ProcessOptions::default(),
        &Progress::default(),
        Arc::new(TestTrash::new(&root)),
        Arc::new(Md5Hasher),
    )
    .unwrap();
    assert_eq!(res.renamed_count, 1);
}

// ---------------- 异常恢复(需求 §29) ----------------

#[test]
fn interrupted_rename_is_recovered_on_next_run() {
    let root = tmpdir("recover_e2e");
    let work = root.join("work");
    std::fs::create_dir_all(&work).unwrap();
    // 三个文件,z.jpg 处于"已临时改名"状态(模拟崩溃),其余未处理
    std::fs::write(work.join("y.jpg"), b"Y").unwrap();
    std::fs::write(work.join("x.jpg"), b"X").unwrap();
    let token = "cafe1234";
    let temp_name = format!(".hashrename_tmp_777_{token}_1");
    std::fs::write(work.join(&temp_name), b"Z").unwrap(); // z.jpg 的在途状态

    // 构造与真实运行一致的 journal(z.jpg 排在最后 → 3 号位)
    let mut remaining: Vec<hashrename_lib::core::models::FileEntry> = ["y.jpg", "x.jpg"]
        .iter()
        .map(|n| hashrename_lib::core::models::FileEntry {
            path: work.join(n),
            file_name: n.to_string(),
            extension: Some("jpg".to_string()),
            size: 1,
            hash: None,
        })
        .collect();
    hashrename_lib::core::sorter::sort_names(&mut remaining, |f| &f.file_name);
    let mut entries = vec![hashrename_lib::core::renamer::JournalEntry {
        temp: temp_name.clone(),
        original: "z.jpg".to_string(),
        final_name: "003.jpg".to_string(),
        noop: false,
    }];
    for (i, fe) in remaining.iter().enumerate() {
        entries.push(hashrename_lib::core::renamer::JournalEntry {
            temp: String::new(),
            original: fe.file_name.clone(),
            final_name: format!("{:03}.jpg", i + 1),
            noop: true,
        });
    }
    let journal = hashrename_lib::core::renamer::Journal {
        version: 1,
        token: token.to_string(),
        directory: work.display().to_string(),
        entries,
    };
    std::fs::write(
        work.join(format!(".hashrename_journal_777_{token}.json")),
        serde_json::to_vec(&journal).unwrap(),
    )
    .unwrap();

    let res = run(&root);
    // 恢复:z.jpg 的在途重命名被补完(003.jpg)
    assert_eq!(res.recovered_finals, 1, "errors: {:?}", res.errors);
    // 新一轮运行对全部文件重新编号:恢复出来的 003.jpg(数字名)排最前,
    // x.jpg、y.jpg 依次跟上 —— 内容无丢失,编号确定
    assert_eq!(names_of(&work), vec!["001.jpg", "002.jpg", "003.jpg"]);
    assert_eq!(contents_of(&work, "001.jpg"), b"Z", "恢复的 z.jpg 内容完好");
    assert_eq!(contents_of(&work, "002.jpg"), b"X");
    assert_eq!(contents_of(&work, "003.jpg"), b"Y");
}

// ---------------- 特殊情况 ----------------

#[test]
fn empty_dir_is_handled() {
    let root = tmpdir("empty");
    let work = root.join("work");
    std::fs::create_dir_all(&work).unwrap();
    let res = run(&root);
    assert_eq!(res.scanned_count, 0);
    assert_eq!(res.renamed_count, 0);
    assert_eq!(res.failed_count, 0);
}

#[test]
fn single_file_renamed() {
    let root = tmpdir("single");
    let work = root.join("work");
    std::fs::create_dir_all(&work).unwrap();
    std::fs::write(work.join("only.txt"), b"hello").unwrap();
    let res = run(&root);
    assert_eq!(res.renamed_count, 1);
    assert_eq!(names_of(&work), vec!["001.txt"]);
}

#[test]
fn nonexistent_dir_is_fatal_error() {
    let err = process_directory(
        Path::new("/nonexistent/hashrename/e2e/dir"),
        &ProcessOptions::default(),
        &Progress::default(),
        Arc::new(OsTrash),
        Arc::new(Md5Hasher),
    )
    .unwrap_err();
    assert!(matches!(err, HrError::Directory { .. }));
}

#[test]
fn internal_files_are_excluded_from_processing() {
    let root = tmpdir("internal");
    let work = root.join("work");
    std::fs::create_dir_all(&work).unwrap();
    std::fs::write(work.join("a.txt"), b"A").unwrap();
    // 残留的孤儿临时文件:应被排除并警告,不参与去重/重命名
    std::fs::write(work.join(".hashrename_tmp_999_aabb_7"), b"orphan").unwrap();

    let res = run(&root);
    assert_eq!(res.scanned_count, 1);
    assert_eq!(res.orphan_temps, 1);
    assert!(
        res.warnings.iter().any(|w| w.contains("临时文件")),
        "应警告孤儿临时文件"
    );
    assert!(work.join(".hashrename_tmp_999_aabb_7").exists());
    assert_eq!(names_of(&work), vec!["001.txt"]);
}

#[cfg(unix)]
#[test]
fn unreadable_file_does_not_break_the_run() {
    // 以 root 运行时 chmod 000 仍可读,跳过
    if unsafe { libc::geteuid() } == 0 {
        eprintln!("以 root 运行,跳过权限测试");
        return;
    }
    use std::os::unix::fs::PermissionsExt;
    let root = tmpdir("perm");
    let work = root.join("work");
    std::fs::create_dir_all(&work).unwrap();
    // 三个文件同大小,确保 secret.dat 进入哈希阶段
    std::fs::write(work.join("secret.dat"), b"SECR").unwrap();
    std::fs::set_permissions(
        work.join("secret.dat"),
        std::fs::Permissions::from_mode(0o000),
    )
    .unwrap();
    std::fs::write(work.join("normal.dat"), b"NNNN").unwrap();
    std::fs::write(work.join("copy.dat"), b"NNNN").unwrap(); // 与 normal 重复

    let res = run(&root);
    // 无法读取的文件:记录错误但保留并照常重命名
    assert!(
        res.errors.iter().any(|e| e.operation == "hash"),
        "应记录哈希失败: {:?}",
        res.errors
    );
    // normal/copy 内容相同 → 1 组重复;normal.dat(排序在后)被移入回收站,
    // secret.dat(无法哈希)与 copy.dat 保留并重新编号
    assert_eq!(res.duplicate_count, 1);
    assert_eq!(res.trashed_count, 1);
    let names = names_of(&work);
    assert_eq!(names, vec!["001.dat", "002.dat"], "两个文件保留并重命名");
    // 先恢复可读权限(测试需要读取),再验证 secret.dat 内容完好
    for n in &names {
        if n.ends_with(".dat") {
            let _ = std::fs::set_permissions(work.join(n), std::fs::Permissions::from_mode(0o644));
        }
    }
    let contents = [contents_of(&work, "001.dat"), contents_of(&work, "002.dat")];
    assert!(
        contents.contains(&b"SECR".to_vec()),
        "无法哈希的文件必须被保留且内容完好"
    );
    assert!(
        contents.contains(&b"NNNN".to_vec()),
        "被保留的重复代表内容完好"
    );
}

#[test]
fn duplicate_groups_across_sizes_and_names() {
    // 综合场景:多组重复 + 唯一文件 + 不同扩展名
    let root = tmpdir("multi");
    let work = root.join("work");
    std::fs::create_dir_all(&work).unwrap();
    let big = vec![7u8; 4096];
    std::fs::write(work.join("big1.bin"), &big).unwrap();
    std::fs::write(work.join("big2.bin"), &big).unwrap();
    std::fs::write(work.join("big3.bin"), &big).unwrap();
    std::fs::write(work.join("solo.txt"), b"unique-content").unwrap();
    std::fs::write(work.join("photo.jpeg"), b"JPEGDATA").unwrap();
    std::fs::write(work.join("copy.jpg"), b"JPEGDATA").unwrap();

    let res = run(&root);
    assert_eq!(res.duplicate_groups, 2);
    assert_eq!(res.duplicate_count, 3);
    assert_eq!(res.trashed_count, 3);
    assert_eq!(res.kept_count, 3);
    assert_eq!(res.renamed_count, 3);
    // 自然排序:big1/2/3 中保留 big1;copy.jpg ('c') 在 photo.jpeg ('p')
    // 之前 → 保留 copy.jpg;剩余文件重新连续编号
    let names = names_of(&work);
    assert_eq!(names, vec!["001.bin", "002.jpg", "003.txt"]);
    assert_eq!(contents_of(&work, "001.bin"), vec![7u8; 4096]);
    assert_eq!(contents_of(&work, "002.jpg"), b"JPEGDATA");
    assert_eq!(contents_of(&work, "003.txt"), b"unique-content");
}
