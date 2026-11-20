import type { FixState } from "../bindings/FixState";
import type { RestorePointOutcome } from "../bindings/RestorePointOutcome";
import type { ValueText } from "../bindings/ValueText";
import type { Tone } from "./device";

export function vt(v: ValueText | null | undefined): string {
  if (!v) return "unknown";
  return v.label ? `${v.label} (${v.value})` : v.value;
}

export const fixStateInfo: Record<FixState, { label: string; tone: Tone; hint: string }> = {
  pending: { label: "In progress", tone: "info", hint: "The change is being applied." },
  applied: { label: "Applied", tone: "ok", hint: "Written and read back with the new value." },
  failed: { label: "Failed", tone: "bad", hint: "The write did not happen; nothing changed." },
  verifyFailed: { label: "Not confirmed", tone: "warn", hint: "Written, but the device reports a different value afterwards." },
  reverted: { label: "Reverted", tone: "muted", hint: "The previous value was restored." },
  revertFailed: { label: "Revert failed", tone: "bad", hint: "Restoring the previous value did not work." },
  unknown: { label: "Interrupted", tone: "warn", hint: "Auriga closed before this finished; check the current value." },
};

export function restorePointText(o: RestorePointOutcome): string {
  switch (o.status) {
    case "created":
      return `Restore point created (#${o.sequence}).`;
    case "skipped":
      return "Windows did not create a restore point, most likely because one was created in the last 24 hours (Windows allows one per day by default).";
    case "disabled":
      return "System Protection is turned off, so Windows cannot create restore points.";
    case "unconfirmed":
      return `Windows accepted the request, but Auriga could not confirm a restore point exists (${o.message}).`;
    case "failed":
      return `Could not create a restore point: ${o.message}`;
  }
}

export function canRevert(revertible: boolean, s: FixState): boolean {
  return revertible && (s === "applied" || s === "verifyFailed" || s === "revertFailed" || s === "unknown");
}
