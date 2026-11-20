// Display helpers for the analysis types (labels, tones, time formatting).
import type { Level } from "../bindings/Level";
import type { MatchAssessment } from "../bindings/MatchAssessment";
import type { SignalKind } from "../bindings/SignalKind";
import type { TimelineKind } from "../bindings/TimelineKind";
import type { Tone } from "./device";

export const signalInfo: Record<SignalKind, { label: string; tone: Tone; hint: string }> = {
  bugcheck: { label: "System crashes", tone: "bad", hint: "Blue screens (Kernel-Power 41 with a bugcheck code, or a BugCheck report)." },
  unexpectedShutdown: { label: "Unexpected shutdowns", tone: "bad", hint: "Power lost or hard reset without a bugcheck." },
  whea: { label: "Hardware errors (WHEA)", tone: "bad", hint: "CPU/PCIe/memory errors reported by the hardware." },
  liveKernelEvent: { label: "Live kernel events", tone: "warn", hint: "A driver or device hung and Windows recovered without crashing." },
  displayTdr: { label: "Display driver resets", tone: "warn", hint: "Display 4101: the GPU driver stopped responding and recovered." },
  storageReset: { label: "Storage resets/timeouts", tone: "warn", hint: "Disk/NVMe/AHCI controller resets or I/O timeouts." },
  deviceStartFailure: { label: "Device start failures", tone: "warn", hint: "Kernel-PnP: a device failed to start or was not migrated." },
  driverInstallFailure: { label: "Driver install failures", tone: "warn", hint: "Failed sections in setupapi.dev.log." },
  wlanDisconnect: { label: "Wi-Fi disconnects", tone: "warn", hint: "WLAN-AutoConfig 8003. Reason 0 is often a normal sleep/roam disconnect." },
  wifiDriverWarning: { label: "Wi-Fi driver warnings", tone: "info", hint: "Warnings/errors logged by the Intel Wi-Fi driver (Netwtw*)." },
  updateInstallFailure: { label: "Update install failures", tone: "warn", hint: "WindowsUpdateClient 20." },
};

export const timelineTone: Record<TimelineKind, Tone> = {
  driverInstall: "info",
  driverInstallFailed: "bad",
  deviceProblem: "warn",
  bugcheck: "bad",
  unexpectedShutdown: "bad",
  liveKernelEvent: "warn",
  displayTdr: "warn",
  whea: "bad",
  storageReset: "warn",
  wlanDisconnect: "warn",
  boot: "muted",
  sleep: "muted",
  wake: "muted",
  update: "ok",
  updateFailed: "warn",
};

export const levelTone: Record<Level, Tone> = { good: "ok", info: "info", warn: "warn", bad: "bad" };
export const levelLabel: Record<Level, string> = { good: "Good", info: "Note", warn: "Check", bad: "Problem" };

export function matchLabel(m: MatchAssessment): string {
  switch (m.kind.kind) {
    case "hardwareId":
      return m.kind.index === 0 ? "Exact hardware ID" : `Hardware ID #${m.kind.index + 1}`;
    case "compatibleId":
      return "Compatible ID (generic)";
    case "unlisted":
      return "Not in the device's ID lists";
    case "noDriver":
      return "No driver";
    case "none":
      return "Not applicable";
  }
}

/** Event log level → label/tone (1 critical … 4 info; 0/5 other). */
export function eventLevel(l: number): { label: string; tone: Tone } {
  if (l === 1) return { label: "Critical", tone: "bad" };
  if (l === 2) return { label: "Error", tone: "bad" };
  if (l === 3) return { label: "Warning", tone: "warn" };
  if (l === 4 || l === 0) return { label: "Info", tone: "muted" };
  return { label: `Level ${l}`, tone: "muted" };
}

const dtf = new Intl.DateTimeFormat(undefined, {
  month: "short",
  day: "numeric",
  hour: "2-digit",
  minute: "2-digit",
  second: "2-digit",
});

/** Local, short date-time from an RFC 3339 string. */
export function when(s: string | null | undefined): string {
  if (!s) return "";
  const d = new Date(s);
  return isNaN(d.getTime()) ? s : dtf.format(d);
}

/** "3 h ago" relative to a reference (the scan time, not the wall clock). */
export function ago(s: string | null | undefined, ref: string): string {
  if (!s) return "";
  const ms = new Date(ref).getTime() - new Date(s).getTime();
  if (!isFinite(ms)) return "";
  const m = Math.round(ms / 60000);
  if (m < 1) return "just before scan";
  if (m < 60) return `${m} min ago`;
  const h = Math.round(m / 60);
  if (h < 48) return `${h} h ago`;
  return `${Math.round(h / 24)} d ago`;
}

/** Short provider names: drop the Microsoft-Windows- prefix. */
export function shortProvider(p: string): string {
  return p.replace(/^Microsoft-Windows-/i, "");
}

export function shortChannel(c: string): string {
  return c.replace(/^Microsoft-Windows-/i, "").replace(/\/Operational$/i, "");
}
