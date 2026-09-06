# HashRename

**跨平台文件哈希去重与序号重命名工具**(Windows / macOS / Linux)

在文件管理器中右键一个文件夹,HashRename 会自动:扫描其中的文件 → 按内容(MD5)找出重复文件 → 把重复文件移入系统回收站 → 将剩余文件按自然顺序重命名为 `001.xxx`、`002.xxx`、`003.xxx`…… **只处理当前文件夹,绝不递归子目录。**

```text
右键文件夹 → Hash 去重并重命名 → 等待 → 完成
```

---

## 目录

- [功能](#功能)
- [支持平台](#支持平台)
- [安装](#安装)
  - [Windows 安装](#windows-安装)
  - [macOS 安装](#macos-安装)
  - [Linux 安装](#linux-安装)
- [右键菜单使用方式](#右键菜单使用方式)
- [CLI 使用方式](#cli-使用方式)
- [工作原理](#工作原理)
  - [MD5 去重规则](#md5-去重规则)
  - [文件保留规则](#文件保留规则)
  - [重命名规则](#重命名规则)
  - [回收站规则](#回收站规则)
- [开发环境](#开发环境)
- [本地开发](#本地开发)
- [测试](#测试)
- [构建](#构建)
- [GitHub Actions](#github-actions)
- [常见问题](#常见问题)
- [已知限制](#已知限制)

---

## 功能

- **内容去重**:相同大小 + 相同 MD5 + 逐字节二次验证,三重确认才算重复
- **安全删除**:重复文件一律移入系统回收站/废纸篓,**绝不永久删除**
- **序号重命名**:剩余文件按原始文件名自然排序,统一重命名为 `001`、`002`……
- **不递归**:只处理右键的那个文件夹本身,子目录一律不动
- **零确认直接执行**:右键后立即开始,实时显示进度,完成后显示统计与错误
- **防冲突**:两阶段重命名 + 目标名冲突检测,绝不覆盖已有文件
- **防并发**:同一目录同时只允许一个任务(目录锁)
- **可恢复**:任意时刻崩溃/被杀,下次运行自动恢复中断的重命名

## 支持平台

| 平台 | 版本 | 右键菜单机制 | 回收站机制 |
| --- | --- | --- | --- |
| Windows | Windows 10/11(x64) | Explorer 注册表右键菜单(HKCU,无需管理员) | 回收站(Recycle Bin) |
| macOS | macOS 10.14+(Apple Silicon / Intel) | Automator 快速操作(Quick Action) | 废纸篓(Trash) |
| Linux | 主流发行版(x64) | Nautilus / Dolphin / Nemo / Thunar | FreeDesktop Trash |

核心业务逻辑(扫描/哈希/去重/重命名计划)三端共用同一份 Rust 代码;平台差异(回收站、no-replace 重命名、右键菜单、进程探测)通过 `platform` 模块抽象。

---

## 安装

从 [GitHub Releases](../../releases) 下载对应平台的安装包。

### Windows 安装

1. 下载 `HashRename_<版本>_x64-setup.exe`(NSIS 安装程序)。
2. 双击安装(当前用户安装,不需要管理员权限)。
3. 默认不自动注册右键菜单,运行一次:
   ```powershell
   & "C:\Users\<你>\AppData\Local\HashRename\hashrename.exe" --install-context-menu
   ```
4. 卸载程序会自动清理右键菜单;也可手动执行 `--uninstall-context-menu`。

### macOS 安装

1. 下载 `HashRename_<版本>_universal.dmg`(Apple Silicon + Intel 通用)。
2. 打开 DMG,把 **HashRename.app** 拖入「应用程序」。
3. 首次打开若提示「无法验证开发者」,在 **系统设置 → 隐私与安全性** 中点击「仍要打开」。
4. 安装 Finder 快速操作:
   ```bash
   /Applications/HashRename.app/Contents/MacOS/hashrename --install-context-menu
   ```

> macOS 权限说明:首次处理「桌面 / 文稿 / 下载」等受保护目录时,系统会弹出授权对话框,点击「允许」一次即可;Quick Action 机制本身无需额外授权。

### Linux 安装

**AppImage(推荐,免安装):**

```bash
chmod +x HashRename_<版本>_amd64.AppImage
./HashRename_<版本>_amd64.AppImage --install-context-menu
```

**deb(Debian/Ubuntu):**

```bash
sudo dpkg -i HashRename_<版本>_amd64.deb
hashrename --install-context-menu
```

> AppImage 运行需要 FUSE(libfuse2);Ubuntu 22.04+ 如提示缺少,执行 `sudo apt install libfuse2`。
> 依赖系统 glibc ≥ 2.28(基于 ubuntu-22.04 构建)。

## 右键菜单使用方式

安装命令会在当前用户级别注册右键菜单(不需要管理员/root):

| 平台 | 安装后位置 |
| --- | --- |
| Windows | 右键任意文件夹 → **Hash 去重并重命名**(文件夹内空白处右键同样支持) |
| macOS | Finder 右键文件夹 → **快速操作(或 服务)→ Hash 去重并重命名**;若未立即出现,注销重登或在「系统设置 → 隐私与安全性 → 扩展 → Finder」中勾选 |
| Linux | 见下表 |

Linux 各文件管理器的集成方式(由同一条 `--install-context-menu` 命令自动全部安装):

| 文件管理器 | 机制 | 位置 |
| --- | --- | --- |
| Nautilus(GNOME) | 用户脚本 | `~/.local/share/nautilus/scripts/` → 右键 → Scripts |
| Dolphin(KDE Plasma 5/6) | 服务菜单 | `~/.local/share/kio/servicemenus/` |
| Nemo(Cinnamon) | 动作 | `~/.local/share/nemo/actions/` |
| Thunar(XFCE) | 自定义动作 | `~/.config/Thunar/uca.xml`(合并写入) |

卸载一律使用:

```bash
hashrename --uninstall-context-menu
```

Windows 的 NSIS 卸载程序也会自动清理注册表中的右键菜单项(HKCU,通过 NSIS 卸载钩子)。

## CLI 使用方式

CLI 与右键菜单/GUI 使用**完全相同**的核心逻辑:

```bash
hashrename "/path/to/folder"             # 处理目录(终端中运行时直接执行)
hashrename --cli "/path/to/folder"       # 强制命令行模式(脚本/自动化)
hashrename --gui "/path/to/folder"       # 打开图形窗口处理
hashrename                               # 打开图形窗口(选择文件夹)
hashrename --install-context-menu        # 安装右键菜单
hashrename --uninstall-context-menu      # 卸载右键菜单
hashrename --verbose "/path/to/folder"   # 详细输出(逐文件进度)
hashrename --help
hashrename --version
```

模式判定:带目录参数且标准输出是终端 → CLI;从文件管理器/双击启动(无终端)→ 图形窗口。脚本中请显式使用 `--cli`。

退出码:`0` 成功;`1` 完成但有失败或被取消;`2` 致命错误(目录不存在、被锁等)。

## 工作原理

执行流程严格按九个阶段进行(先建立完整计划,再执行):

```text
扫描(快照,不递归)
  → 过滤(只保留普通文件;子目录/符号链接/特殊文件跳过)
  → 文件大小预筛选(只有大小相同的文件才进入哈希)
  → 并行流式 MD5(限制线程数,不整文件载入内存)
  → 大小+MD5 分组
  → 逐字节二次验证
  → 确定重复组并选择保留文件
  → 生成完整操作计划(journal)
  → 重复文件移入回收站
  → 临时重命名(原文件 → 唯一临时名)
  → 最终重命名(临时名 → 001.xxx,no-replace)
  → 报告结果
```

### MD5 去重规则

- **判定依据只有文件内容**,与文件名、路径、扩展名无关:`a.jpg`、`b.png`、`c.webp` 内容相同即为重复。
- 三重确认:文件大小相同 → MD5 相同 → **逐字节内容完全一致**。即使发生理论上的 MD5 碰撞,内容不同的文件也绝不会被误删。
- 无法读取/验证的文件(权限、被占用等)一律**保留**,只记录错误。
- 第一版仅内置 MD5;`Hasher` trait 已抽象,可扩展 SHA-256 等算法(代码内已附带 Sha256Hasher 实现与测试)。

### 文件保留规则

同一组重复文件中,保留 **原始文件名自然排序最靠前** 的一个,其余移入回收站:

```text
IMG_2.jpg、IMG_10.jpg、IMG_1.jpg  →  自然排序 IMG_1 < IMG_2 < IMG_10
保留 IMG_1.jpg,其余两个进入回收站
```

### 重命名规则

- 去重后的剩余文件,按**原始文件名自然排序**(`1, 2, 10, 20`,不是字典序 `1, 10, 2, 20`)。
- 文件名主体统一为数字序号,从 `001` 开始;**扩展名原样保留(包括大小写)**。
- 序号位数 = `max(3, 剩余文件数的十进制位数)`:1~999 个 → 3 位;1000~9999 → 4 位;10000 以上 → 5 位。所有文件统一位数。
- 多段扩展名按标准语义取最后一段:`archive.tar.gz` → `001.gz`。
- 与排序中的平局(如 `2.jpg` 与 `02.jpg`)处理:数值相等时前导零少者在前;文本段忽略大小写比较,平局按原始码点——全序确定,结果可复现。

### 回收站规则

- 重复文件通过系统机制移入回收站:Windows(IFileOperation / Recycle Bin)、macOS(NSWorkspace / Trash)、Linux(FreeDesktop Trash,`~/.local/share/Trash` 或挂载点 `.Trash-<uid>`)。
- Linux 环境没有可用 Trash 机制时**安全失败**:不删除任何文件,报告原因后停止移动其余重复文件。
- 从回收站中可以随时还原文件。

---

## 开发环境

- Rust ≥ 1.77(含 cargo)
- Node.js ≥ 18 + npm
- Tauri CLI:`cargo install tauri-cli --version "^2"`
- Linux 额外系统依赖(Debian/Ubuntu):

```bash
sudo apt install libwebkit2gtk-4.1-dev libgtk-3-dev \
  libayatana-appindicator3-dev librsvg2-dev
```

- macOS:`xcode-select --install`
- Windows:MSVC Build Tools + WebView2(Win10/11 自带)

## 本地开发

```bash
npm install          # 安装前端依赖
npm run tauri dev    # 启动开发模式(热重载)
```

## 测试

```bash
npm run build          # 前端类型检查 + 构建(GUI 资源在编译期嵌入,先于 cargo)
cargo test             # 全部测试:48 个单元测试 + 21 个集成测试
cargo clippy --all-targets -- -D warnings
cargo fmt --all -- --check
node scripts/check-version.mjs   # 版本号一致性检查
```

测试覆盖(对应需求 §31):MD5 已知向量(空文件/abc/大文件流式/二进制)、去重(同内容异扩展名、同大小异内容、哈希碰撞模拟与二次验证防护)、自然排序(数字/中文/emoji/空格/大数防溢出)、编号位数(1/9/10/99/100/999/1000/10000)、扩展名(大小写/无扩展名/多段)、冲突(预存在 001.jpg、名为 001.jpg 的子目录)、回收站(真实系统回收站 + 失败安全停止)、权限(chmod 000,root 自动跳过)、并发(目录锁拒绝第二实例)、崩溃恢复(journal 补完/还原)。

## 构建

```bash
npm install
npm run tauri build
# Linux  → src-tauri/target/release/bundle/{appimage,deb}/
# Windows→ src-tauri/target/release/bundle/nsis/HashRename_*_x64-setup.exe
# macOS → src-tauri/target/release/bundle/{dmg}/
```

指定平台目标:

```bash
npm run tauri build -- --bundles appimage,deb            # Linux
npm run tauri build -- --bundles nsis                    # Windows
npm run tauri build -- --target universal-apple-darwin   # macOS 通用二进制
```

## GitHub Actions

- `.github/workflows/build.yml`:push / PR 触发——版本号一致性检查、rustfmt、clippy(`-D warnings`)、全量测试,然后在 Windows(x64/NSIS)、macOS(universal/App+DMG)、Linux(x64/AppImage+deb)三端构建并上传 Artifacts。
- `.github/workflows/release.yml`:推送 `v*` 标签触发——三端构建并通过 `tauri-action` 自动创建 GitHub Release、上传全部安装包。

版本号统一管理:`package.json`、`src-tauri/Cargo.toml`、`src-tauri/tauri.conf.json` 三处必须一致,CI 中由 `scripts/check-version.mjs` 强制校验。

---

## 常见问题

**Q: 会不会误删我的文件?**
重复文件只移入回收站,可随时还原。去重需要「大小相同 + MD5 相同 + 逐字节一致」三重确认;无法确认的一律保留。所有重命名都走「不覆盖」语义。

**Q: 子目录会被处理吗?**
不会。只处理你右键的那个文件夹里的普通文件,子目录与其内容原样不动。

**Q: 文件夹正在被处理时又点了一次右键?**
同一目录同时只允许一个任务,第二个任务会直接报错退出,不会产生并发冲突。

**Q: 程序中途崩溃/被杀,文件会乱吗?**
不会。重命名先写 journal,再按「原文件 → 临时名 → 最终名」两阶段执行;任意时刻中断,下次运行会自动补完或还原,目录不会留下无法识别的临时文件。

**Q: 为什么目录里已经叫 `001.jpg` 的文件没有变成序号?**
若 `001.jpg` 是一个普通文件,它会参与统一的重新编号(先腾名、再改名);若它是子目录/符号链接等不参与重命名的条目,对应的文件会**保持原名并报告冲突**,绝不覆盖。

**Q: 重命名后编号有间断?**
若个别文件因目标名冲突被阻塞,它们的号码空缺、文件保持原名,其余文件编号连续。这是不覆盖原则的代价,错误面板中会说明。

## 已知限制

1. **Windows 回收站策略**:如果用户/组策略关闭了某分区的回收站("不将文件移入回收站"),系统 API 的行为由 OS 决定,程序无法绕过。
2. **Windows 极长路径**:核心文件操作基于 Rust 标准库(内部使用 `\\?\` 完整路径),通常无 260 限制;但 Explorer 右键菜单的 `%V` 参数本身可能受 Explorer 长路径策略影响。
3. **Linux 大小写敏感性**按 `cfg!(target_os = "linux")` 判定,极少数大小写不敏感文件系统(如某些 FAT/NTFS 挂载)下的冲突检测可能多报或少报;最终重命名始终使用 `renameat2(RENAME_NOREPLACE)` 兜底,绝不会覆盖文件。内核不支持 `renameat2`(Linux < 3.15)时回退到「先检查后改名」,存在理论上的竞态窗口。
4. **macOS Quick Action 为生成式安装**:由 `--install-context-menu` 写入 `~/Library/Services/HashRename.workflow`(Automator Run Shell Script 格式)。该机制是 macOS 10.14+ 的主流方案,但未经实体 macOS 设备实测;若快速操作未出现,可按 README「macOS 安装」一节在 Automator 中手动创建等效 Quick Action(`Run Shell Script`,输入作为参数,调用 `.../MacOS/hashrename --gui "$@"`)。
5. **Linux 集成覆盖范围**:Nautilus/Dolphin/Nemo/Thunar 四种主流文件管理器;其他文件管理器(PCManFM-Qt 等)不在第一版范围,可直接用 CLI。
6. **多段扩展名**按标准语义取最后一段:`archive.tar.gz` 重命名为 `001.gz`。
7. **隐藏文件参与处理**(除 `.hashrename` 内部文件外):`.gitignore` 等点开头的普通文件同样会被去重/重命名。
8. **运行期间目录变化**:只处理启动扫描时的快照;运行中新增的文件不会被处理(避免无限重扫)。
9. **未在实体设备上验证的平台**:Windows 与 macOS 仅通过 CI 构建产物验证打包流程,核心逻辑由跨平台测试覆盖;Linux 为开发实测平台。

---

## License

MIT
