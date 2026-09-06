/** 与 Rust core::progress / core::models 对应的类型定义。 */

export type Stage =
  | "recover"
  | "scan"
  | "hash"
  | "verify"
  | "plan"
  | "trash"
  | "rename"
  | "done";

export interface RenamePreview {
  from: string;
  to: string;
}

export interface ProcessingResult {
  directory: string;
  scanned_count: number;
  skipped_count: number;
  duplicate_groups: number;
  duplicate_count: number;
  trashed_count: number;
  kept_count: number;
  renamed_count: number;
  failed_count: number;
  cancelled: boolean;
  dry_run: boolean;
  planned_trashes: string[];
  planned_renames: RenamePreview[];
  hash_algorithm: string;
  recovered_finals: number;
  restored_temps: number;
  orphan_temps: number;
  elapsed_ms: number;
  errors: FileError[];
  warnings: string[];
}

export interface FileError {
  path: string;
  operation: string;
  message: string;
}

export type ProgressEvent =
  | { type: "stage"; stage: Stage; message: string }
  | { type: "counts"; scanned: number; duplicates: number; kept: number }
  | { type: "current_file"; file: string }
  | { type: "progress"; percent: number }
  | { type: "warning"; message: string }
  | { type: "finished"; result: ProcessingResult };

export interface LaunchInfo {
  dir: string | null;
  version: string;
}

/** 与 Rust ProcessOptions 对应的处理选项(功能可配置)。 */
export interface ProcessingOptions {
  /** 哈希算法:md5 / sha256 / xxh3 */
  hashAlgorithm: string;
  /** 预览模式:只输出计划,不修改任何文件 */
  dryRun: boolean;
}
