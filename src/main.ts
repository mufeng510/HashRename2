/**
 * HashRename 前端(轻量 UI,需求 §17/§38):
 * - 右键菜单启动时自动开始处理,无确认对话框;
 * - 显示阶段 / 进度 / 计数 / 当前文件;
 * - 完成后显示结果统计与错误明细。
 */
import { invoke, Channel } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";
import type { LaunchInfo, ProcessingResult, ProgressEvent } from "./types";

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

function esc(s: string): string {
  const d = document.createElement("div");
  d.textContent = s;
  return d.innerHTML;
}

function shortDir(dir: string): string {
  // 过长路径中间省略
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
        <button id="pickBtn" class="primary">选择文件夹并处理</button>
        <p class="hint">也可以在文件管理器中右键文件夹 → “Hash 去重并重命名”</p>
      </div>`;
    document.getElementById("pickBtn")?.addEventListener("click", pickFolder);
    return;
  }

  if (screen === "running") {
    const filled = Math.round(percent / 2.5);
    const bar = "█".repeat(filled) + "░".repeat(40 - Math.min(filled, 40));
    app.innerHTML = `
      <div class="header">
        <div class="logo small">Hash<span>Rename</span></div>
        <div class="dir" title="${esc(currentDir ?? "")}">${shortDir(currentDir ?? "")}</div>
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
      : hasErrors
        ? "处理完成,但存在部分错误"
        : "HashRename 完成";
    app.innerHTML = `
      <div class="header">
        <div class="logo small">Hash<span>Rename</span></div>
        <div class="dir" title="${esc(r.directory)}">${shortDir(r.directory)}</div>
      </div>
      <div class="result-title ${r.cancelled ? "warn" : hasErrors ? "warn" : "ok"}">${esc(title)}</div>
      <div class="grid">
        <div class="cell"><div class="num">${r.scanned_count}</div><div class="label">扫描文件</div></div>
        <div class="cell"><div class="num warn">${r.duplicate_count}</div><div class="label">发现重复</div></div>
        <div class="cell"><div class="num warn">${r.trashed_count}</div><div class="label">移入回收站</div></div>
        <div class="cell"><div class="num ok">${r.kept_count}</div><div class="label">最终文件</div></div>
        <div class="cell"><div class="num">${r.renamed_count}</div><div class="label">重命名</div></div>
        <div class="cell"><div class="num ${hasErrors ? "bad" : ""}">${r.failed_count}</div><div class="label">失败</div></div>
      </div>
      <div class="elapsed muted">处理耗时:${(r.elapsed_ms / 1000).toFixed(1)} 秒</div>
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
    startProcessing(selected);
  }
}

function startProcessing(dir: string): void {
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

  invoke("start_processing", { dir, onEvent }).catch((e: unknown) => {
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
      // 右键菜单/命令行启动:直接开始,无确认(需求 §16)
      startProcessing(info.dir);
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
