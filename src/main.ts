/**
 * HashRename 前端(轻量 UI,需求 §17/§38):
 * - 右键菜单启动时自动开始处理,无确认对话框;
 * - 显示阶段 / 进度 / 计数 / 当前文件;
 * - 完成后显示结果统计与错误明细;
 * - 可配置项(v0.2.0):哈希算法、预览模式。
 */
import { invoke, Channel } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";
import type {
  LaunchInfo,
  ProcessingOptions,
  ProcessingResult,
  ProgressEvent,
} from "./types";

const app = document.getElementById("app")!;

type Screen = "loading" | "pick" | "running" | "done" | "error";

let screen: Screen = "loading";
let currentDir: string | null = null;
let stageMessage = "";
let percent = 0;
let currentFile = "";
let counts = { scanned: 0, duplicates: 0, kept: 0 };
let result: ProcessingResult | null = null;
let errorMessage = "";

// 可配置选项(v0.2.0):算法与预览模式
const ALGORITHMS = [
  { value: "md5", label: "MD5(默认,快)" },
  { value: "sha256", label: "SHA-256(更安全,稍慢)" },
  { value: "xxh3", label: "xxHash3(最快,非加密)" },
];
let selectedAlgorithm = "md5";
let dryRun = false;

function esc(s: string): string {
  const d = document.createElement("div");
  d.textContent = s;
  return d.innerHTML;
}

function shortDir(dir: string): string {
  if (dir.length <= 52) return esc(dir);
  return esc(dir.slice(0, 24)) + "…" + esc(dir.slice(-26));
}

function render(): void {
  if (screen === "loading") {
    app.innerHTML = `
      <div class="center">
        <div class="logo">Hash<span>Rename</span></div>
        <div class="muted">正在启动…</div>
      </div>`;
    return;
  }

  if (screen === "pick") {
    app.innerHTML = `
      <div class="center">
        <div class="logo">Hash<span>Rename</span></div>
        <p class="muted">选择一个文件夹:扫描其中的文件,把内容重复的文件
        移入回收站,其余文件按自然顺序重命名为 001、002、003……</p>
        <div class="options">
          <label class="option-row">
            <span class="option-label">哈希算法</span>
            <select id="algoSelect">
              ${ALGORITHMS.map(
                (a) =>
                  `<option value="${a.value}" ${a.value === selectedAlgorithm ? "selected" : ""}>${a.label}</option>`,
              ).join("")}
            </select>
          </label>
          <label class="option-row check">
            <input type="checkbox" id="dryRunCheck" ${dryRun ? "checked" : ""}/>
            <span>预览模式(不修改任何文件,只显示计划)</span>
          </label>
        </div>
        <button id="pickBtn" class="primary">选择文件夹并处理</button>
        <p class="hint">也可以在文件管理器中右键文件夹 → “Hash 去重并重命名”<br/>
        右键菜单启动时按上次选择执行,无确认对话框</p>
      </div>`;
    document.getElementById("pickBtn")?.addEventListener("click", pickFolder);
    document.getElementById("algoSelect")?.addEventListener("change", (e) => {
      selectedAlgorithm = (e.target as HTMLSelectElement).value;
    });
    document.getElementById("dryRunCheck")?.addEventListener("change", (e) => {
      dryRun = (e.target as HTMLInputElement).checked;
    });
    return;
  }

  if (screen === "running") {
    const filled = Math.round(percent / 2.5);
    const bar = "█".repeat(filled) + "░".repeat(40 - Math.min(filled, 40));
    app.innerHTML = `
      <div class="header">
        <div class="logo small">Hash<span>Rename</span></div>
        <div class="dir" title="${esc(currentDir ?? "")}">${shortDir(currentDir ?? "")}</div>
        <div class="dir">${dryRun ? "预览模式 · " : ""}${esc(selectedAlgorithm.toUpperCase())}</div>
      </div>
      <div class="stage">
        <div class="stage-msg">${esc(stageMessage)}</div>
        <div class="bar mono">${bar}</div>
        <div class="pct mono">${percent.toFixed(1)}%</div>
      </div>
      <div class="grid">
        <div class="cell"><div class="num" id="cScanned">${counts.scanned}</div><div class="label">扫描文件</div></div>
        <div class="cell"><div class="num warn" id="cDup">${counts.duplicates}</div><div class="label">重复文件</div></div>
        <div class="cell"><div class="num ok" id="cKept">${counts.kept}</div><div class="label">保留文件</div></div>
      </div>
      <div class="current muted mono">${currentFile ? "正在处理:" + esc(currentFile) : ""}</div>
      <div class="footer">
        <button id="cancelBtn" class="secondary">取消</button>
      </div>`;
    document.getElementById("cancelBtn")?.addEventListener("click", () => {
      invoke("cancel_processing");
    });
    return;
  }

  if (screen === "done" && result) {
    const r = result;
    const hasErrors = r.failed_count > 0;
    const title = r.cancelled
      ? "已取消"
      : r.dry_run
        ? "预览完成(未修改任何文件)"
        : hasErrors
          ? "处理完成,但存在部分错误"
          : "HashRename 完成";
    app.innerHTML = `
      <div class="header">
        <div class="logo small">Hash<span>Rename</span></div>
        <div class="dir" title="${esc(r.directory)}">${shortDir(r.directory)}</div>
        <div class="dir">${r.dry_run ? "预览模式 · " : ""}${esc(r.hash_algorithm.toUpperCase())}</div>
      </div>
      <div class="result-title ${r.cancelled ? "warn" : hasErrors ? "warn" : r.dry_run ? "info" : "ok"}">${esc(title)}</div>
      <div class="grid">
        <div class="cell"><div class="num">${r.scanned_count}</div><div class="label">扫描文件</div></div>
        <div class="cell"><div class="num warn">${r.duplicate_count}</div><div class="label">发现重复</div></div>
        ${
          r.dry_run
            ? `<div class="cell"><div class="num warn">${r.planned_trashes.length}</div><div class="label">将移入回收站</div></div>
               <div class="cell"><div class="num">${r.planned_renames.length}</div><div class="label">将重命名</div></div>`
            : `<div class="cell"><div class="num warn">${r.trashed_count}</div><div class="label">移入回收站</div></div>
               <div class="cell"><div class="num ok">${r.kept_count}</div><div class="label">最终文件</div></div>
               <div class="cell"><div class="num">${r.renamed_count}</div><div class="label">重命名</div></div>
               <div class="cell"><div class="num ${hasErrors ? "bad" : ""}">${r.failed_count}</div><div class="label">失败</div></div>`
        }
      </div>
      <div class="elapsed muted">处理耗时:${(r.elapsed_ms / 1000).toFixed(1)} 秒</div>
      ${
        r.dry_run && (r.planned_renames.length || r.planned_trashes.length)
          ? `<div class="problems">
              <details open><summary>重命名计划(${r.planned_renames.length})</summary>
                <div class="preview-list mono">
                  ${r.planned_renames
                    .map((p) => `<div>${esc(p.from)} → ${esc(p.to)}</div>`)
                    .join("")}
                </div>
              </details>
              ${
                r.planned_trashes.length
                  ? `<details><summary>将移入回收站(${r.planned_trashes.length})</summary>
                     <div class="preview-list mono">${r.planned_trashes.map((t) => `<div>${esc(t)}</div>`).join("")}</div>
                     </details>`
                  : ""
              }
             </div>`
          : ""
      }
      ${
        r.warnings.length || hasErrors
          ? `<div class="problems">
              ${r.warnings.map((w) => `<div class="warning">⚠ ${esc(w)}</div>`).join("")}
              ${
                hasErrors
                  ? `<details open><summary>错误明细(${r.errors.length})</summary>
                     <div class="error-list">${r.errors
                       .map(
                         (e) =>
                           `<div class="error-item"><span class="op mono">[${esc(e.operation)}]</span> ${esc(e.path)}<div class="msg">${esc(e.message)}</div></div>`,
                       )
                       .join("")}</div></details>`
                  : ""
              }
            </div>`
          : ""
      }
      <div class="footer">
        <button id="againBtn" class="secondary">处理其他文件夹</button>
      </div>`;
    document.getElementById("againBtn")?.addEventListener("click", () => {
      result = null;
      counts = { scanned: 0, duplicates: 0, kept: 0 };
      percent = 0;
      currentFile = "";
      screen = "pick";
      render();
    });
    return;
  }

  // error
  app.innerHTML = `
    <div class="center">
      <div class="logo">Hash<span>Rename</span></div>
      <div class="result-title bad">出现错误</div>
      <p class="muted">${esc(errorMessage)}</p>
      <button id="retryBtn" class="primary">选择其他文件夹</button>
    </div>`;
  document.getElementById("retryBtn")?.addEventListener("click", () => {
    screen = "pick";
    render();
  });
}

async function pickFolder(): Promise<void> {
  const selected = await open({
    directory: true,
    multiple: false,
    title: "选择要处理的文件夹",
  });
  if (typeof selected === "string") {
    startProcessing(selected, {
      hashAlgorithm: selectedAlgorithm,
      dryRun,
    });
  }
}

function startProcessing(dir: string, options: ProcessingOptions): void {
  currentDir = dir;
  screen = "running";
  render();

  const onEvent = new Channel<ProgressEvent>();
  onEvent.onmessage = (event: ProgressEvent) => {
    switch (event.type) {
      case "stage":
        stageMessage = event.message;
        break;
      case "counts":
        counts = {
          scanned: event.scanned,
          duplicates: event.duplicates,
          kept: event.kept,
        };
        break;
      case "current_file":
        currentFile = event.file;
        break;
      case "progress":
        percent = Math.max(percent, event.percent);
        break;
      case "warning":
        // 警告在结果页汇总展示
        break;
      case "finished":
        result = event.result;
        screen = "done";
        break;
    }
    render();
  };

  invoke("start_processing", {
    dir,
    onEvent,
    hashAlgorithm: options.hashAlgorithm,
    dryRun: options.dryRun,
  }).catch((e: unknown) => {
    errorMessage = typeof e === "string" ? e : String(e);
    screen = "error";
    render();
  });
}

async function boot(): Promise<void> {
  screen = "loading";
  render();
  try {
    const info = await invoke<LaunchInfo>("get_launch_info");
    if (info.dir) {
      // 右键菜单/命令行启动:直接开始,无确认(需求 §16),按当前选项执行
      startProcessing(info.dir, { hashAlgorithm: selectedAlgorithm, dryRun });
    } else {
      screen = "pick";
      render();
    }
  } catch (e) {
    errorMessage = String(e);
    screen = "error";
    render();
  }
}

boot();
