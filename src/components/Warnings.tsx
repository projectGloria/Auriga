import { useState } from "react";
import type { SourceWarning } from "../bindings/SourceWarning";
import { Banner } from "./ui";

/** Collectors whose warnings belong to the Logs page; the rest are inventory. */
export const LOG_SOURCES = ["events", "setupapi", "dumps"];

export function WarningsBanner({ warnings, rest }: { warnings: SourceWarning[]; rest: string }) {
  const [open, setOpen] = useState(false);
  if (warnings.length === 0) return null;
  return (
    <Banner tone="warn">
      <button className="text-left" onClick={() => setOpen((o) => !o)}>
        Some data could not be read ({warnings.length} warning{warnings.length === 1 ? "" : "s"}). {rest}{" "}
        <span className="underline">{open ? "Hide" : "Details"}</span>
      </button>
      {open && (
        <ul className="mt-2 max-h-40 space-y-1 overflow-y-auto font-mono text-xs">
          {warnings.map((w, i) => (
            <li key={i}>
              [{w.source}] {w.message}
              {w.instanceId && <span className="text-amber-300/70"> — {w.instanceId}</span>}
            </li>
          ))}
        </ul>
      )}
    </Banner>
  );
}
