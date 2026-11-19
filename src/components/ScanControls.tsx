import { LoaderCircle, RefreshCw, Square } from "lucide-react";
import { useScan } from "../lib/scan";
import { Button } from "./ui";

export function ScanControls() {
  const { running, progress, start, cancel, latest } = useScan();
  if (running) {
    const pct = progress && progress.total > 0 ? Math.round((progress.done / progress.total) * 100) : null;
    return (
      <div className="flex items-center gap-3">
        <div className="w-56">
          <div className="mb-1 flex items-center justify-between text-xs text-zinc-400">
            <span className="flex items-center gap-1.5">
              <LoaderCircle className="size-3.5 animate-spin" />
              {progress ? stageLabel(progress.stage) : "Starting…"}
            </span>
            {progress && progress.total > 0 && (
              <span className="tabular-nums">
                {progress.done}/{progress.total}
              </span>
            )}
          </div>
          <div className="h-1.5 overflow-hidden rounded-full bg-zinc-800">
            <div
              className={`h-full rounded-full bg-sky-500 transition-[width] duration-200 ${pct == null ? "w-1/3 animate-pulse" : ""}`}
              style={pct == null ? undefined : { width: `${pct}%` }}
            />
          </div>
        </div>
        <Button onClick={cancel} title="Stop the scan; the previous result stays">
          <Square className="size-3.5" /> Cancel
        </Button>
      </div>
    );
  }
  return (
    <Button variant="primary" onClick={start}>
      <RefreshCw className="size-4" /> {latest ? "Rescan" : "Scan now"}
    </Button>
  );
}

function stageLabel(stage: string): string {
  switch (stage) {
    case "machine":
      return "Reading system info";
    case "devices":
      return "Reading devices & drivers";
    default:
      return stage;
  }
}
