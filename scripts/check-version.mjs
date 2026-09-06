#!/usr/bin/env node
/** 校验 package.json / src-tauri/Cargo.toml / src-tauri/tauri.conf.json 版本号一致。 */
import { readFileSync } from "node:fs";

const pkg = JSON.parse(readFileSync("package.json", "utf8"));
const conf = JSON.parse(readFileSync("src-tauri/tauri.conf.json", "utf8"));
const cargo = readFileSync("src-tauri/Cargo.toml", "utf8");
const cargoVersion = /^version\s*=\s*"([^"]+)"/m.exec(cargo)?.[1];

const versions = {
  "package.json": pkg.version,
  "src-tauri/Cargo.toml": cargoVersion,
  "src-tauri/tauri.conf.json": conf.version,
};

const values = new Set(Object.values(versions));
if (values.size !== 1 || [...values].some((v) => !v)) {
  console.error("版本号不一致:");
  for (const [k, v] of Object.entries(versions)) console.error(`  ${k}: ${v}`);
  process.exit(1);
}
console.log(`版本号一致: ${[...values][0]}`);
