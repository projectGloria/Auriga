// Typed wrapper over Tauri commands/events. Payload types are generated from
// Rust by ts-rs (src/bindings) — never hand-edit those.
import { invoke, isTauri } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type { AppInfo } from "../bindings/AppInfo";
import type { ApplyOptions } from "../bindings/ApplyOptions";
import type { ApplyOutcome } from "../bindings/ApplyOutcome";
import type { FixHistoryEntry } from "../bindings/FixHistoryEntry";
import type { FixPreview } from "../bindings/FixPreview";
import type { MinidumpReport } from "../bindings/MinidumpReport";
import type { RevertOutcome } from "../bindings/RevertOutcome";
import type { ValueText } from "../bindings/ValueText";
import type { FindingsReport } from "../bindings/FindingsReport";
import type { KbStatus } from "../bindings/KbStatus";
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
  findings: () => invoke<FindingsReport>("get_findings"),
  // Fixes: the backend rebuilds each fix from the latest scan; the UI only names it.
  previewFix: (findingKey: string, fixId: string) => invoke<FixPreview>("preview_fix", { findingKey, fixId }),
  applyFix: (findingKey: string, fixId: string, expectedFrom: ValueText | null, options: ApplyOptions) =>
    invoke<ApplyOutcome>("apply_fix", { findingKey, fixId, expectedFrom, options }),
  revertFix: (id: number) => invoke<RevertOutcome>("revert_fix", { id }),
  fixHistory: (limit: number) => invoke<FixHistoryEntry[]>("list_fix_history", { limit }),
  readMinidumps: () => invoke<MinidumpReport>("read_minidumps"),
};

export const events = {
  scanProgress: (cb: (p: ScanProgress) => void): Promise<UnlistenFn> =>
    listen<ScanProgress>("scan://progress", (e) => cb(e.payload)),
  scanFinished: (cb: (p: ScanFinished) => void): Promise<UnlistenFn> =>
    listen<ScanFinished>("scan://finished", (e) => cb(e.payload)),
  kbReloaded: (cb: (k: KbStatus) => void): Promise<UnlistenFn> =>
    listen<KbStatus>("kb://reloaded", (e) => cb(e.payload)),
  /** A fix or revert re-read a device; scan-derived views should reload. */
  deviceRefreshed: (cb: (deviceId: string) => void): Promise<UnlistenFn> =>
    listen<string>("devices://refreshed", (e) => cb(e.payload)),
};

export function errorText(e: unknown): string {
  if (typeof e === "string") return e;
  if (e instanceof Error) return e.message;
  return JSON.stringify(e);
}
