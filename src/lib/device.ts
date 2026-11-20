import { CircleAlert, CircleCheck, CircleHelp, CircleMinus, ShieldAlert, ShieldCheck, ShieldQuestion, ShieldX, Unplug, type LucideIcon } from "lucide-react";
import type { DeviceRecord } from "../bindings/DeviceRecord";
import type { SignatureStatus } from "../bindings/SignatureStatus";

export type Tone = "ok" | "warn" | "bad" | "muted" | "info";

export type DeviceState = { key: "ok" | "problem" | "disabled" | "ghost" | "unknown"; label: string; tone: Tone };

export function deviceState(d: DeviceRecord): DeviceState {
  if (!d.present) return { key: "ghost", label: "Not connected", tone: "muted" };
  if (d.status.problemCode != null)
    return { key: "problem", label: `Code ${d.status.problemCode}`, tone: "bad" };
  if (d.status.disabled) return { key: "disabled", label: "Disabled", tone: "warn" };
  if (!d.status.available) return { key: "unknown", label: "Unknown", tone: "muted" };
  // Same rule as Device Manager: no problem code and not disabled = working.
  // Resource-only devices (motherboard resources, timers…) are never
  // "started", which is normal.
  return { key: "ok", label: "Working", tone: "ok" };
}

export const signatureTone: Record<SignatureStatus, Tone> = {
  valid: "ok",
  testSigned: "warn",
  invalid: "bad",
  notSigned: "bad",
  unknown: "muted",
};

export const signatureLabel: Record<SignatureStatus, string> = {
  valid: "Signed",
  testSigned: "Test-signed",
  invalid: "Invalid",
  notSigned: "Unsigned",
  unknown: "Unknown",
};

/** YYYY-MM-DD from an RFC3339 timestamp (or passthrough). */
export const stateIcon: Record<DeviceState["key"], LucideIcon> = {
  ok: CircleCheck,
  problem: CircleAlert,
  disabled: CircleMinus,
  ghost: Unplug,
  unknown: CircleHelp,
};

export const signatureIcon: Record<SignatureStatus, LucideIcon> = {
  valid: ShieldCheck,
  testSigned: ShieldAlert,
  invalid: ShieldX,
  notSigned: ShieldX,
  unknown: ShieldQuestion,
};

export function day(s: string | null | undefined): string {
  return s ? s.slice(0, 10) : "";
}

export function installed(d: DeviceRecord): string | null {
  return d.install.installDate ?? d.install.firstInstallDate ?? null;
}

/** Lower-cased haystack used by the global search box. */
export function searchText(d: DeviceRecord): string {
  return [
    d.name,
    d.description,
    d.class,
    d.manufacturer,
    d.instanceId,
    d.service,
    d.driver?.provider,
    d.driver?.version,
    d.driver?.infName,
    ...d.hardwareIds,
  ]
    .filter(Boolean)
    .join("\n")
    .toLowerCase();
}
