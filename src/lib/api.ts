// Typed wrapper over Tauri commands/events. Payload types are generated from
// Rust by ts-rs (src/bindings) — never hand-edit those.
import { invoke, isTauri } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type { AppInfo } from "../bindings/AppInfo";
import type { HelperTestResult } from "../bindings/HelperTestResult";
import type { LatestSnapshot } from "../bindings/LatestSnapshot";
import type { ScanFinished } from "../bindings/ScanFinished";
import type { ScanProgress } from "../bindings/ScanProgress";
import type { ScanSummary } from "../bindings/ScanSummary";

export const inTauri = isTauri();

export const api = {
  appInfo: () => invoke<AppInfo>("app_info"),
  startScan: () => invoke<number>("start_scan"),
  cancelScan: () => invoke<boolean>("cancel_scan"),
  latestSnapshot: () => invoke<LatestSnapshot | null>("get_latest_snapshot"),
  listScans: (limit: number) => invoke<ScanSummary[]>("list_scans", { limit }),
  helperTest: () => invoke<HelperTestResult>("helper_test"),
};

export const events = {
  scanProgress: (cb: (p: ScanProgress) => void): Promise<UnlistenFn> =>
    listen<ScanProgress>("scan://progress", (e) => cb(e.payload)),
  scanFinished: (cb: (p: ScanFinished) => void): Promise<UnlistenFn> =>
    listen<ScanFinished>("scan://finished", (e) => cb(e.payload)),
};

export function errorText(e: unknown): string {
  if (typeof e === "string") return e;
  if (e instanceof Error) return e.message;
  return JSON.stringify(e);
}
