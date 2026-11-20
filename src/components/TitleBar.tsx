import { getCurrentWindow } from "@tauri-apps/api/window";
import { Copy, Minus, Square, X } from "lucide-react";
import { useEffect, useState, type ReactNode } from "react";
import { inTauri } from "../lib/api";

/** Replaces the native frame (decorations are off). The bar itself drags the
 * window and double-click maximizes (Tauri handles both via the attribute). */
export function TitleBar({ status }: { status?: ReactNode }) {
  const [maximized, setMaximized] = useState(false);

  useEffect(() => {
    if (!inTauri) return;
    const w = getCurrentWindow();
    let un: (() => void) | undefined;
    const sync = () => void w.isMaximized().then(setMaximized, () => {});
    sync();
    void w.onResized(sync).then((f) => (un = f));
    return () => un?.();
  }, []);

  const win = () => (inTauri ? getCurrentWindow() : null);

  return (
    <header data-tauri-drag-region className="flex h-9 shrink-0 select-none items-center border-b border-zinc-800/80 bg-zinc-925 pl-3">
      <img src="/auriga.png" alt="" draggable={false} className="pointer-events-none size-5 rounded" />
      <span className="pointer-events-none ml-2 text-xs font-semibold tracking-wide text-zinc-200">Auriga</span>
      <span className="pointer-events-none ml-1.5 text-xs text-zinc-500">Driver Manager</span>
      <div className="pointer-events-none ml-4 flex-1">{status}</div>
      <div className="flex h-full">
        <CaptionButton label="Minimize" onClick={() => void win()?.minimize()}>
          <Minus className="size-4" strokeWidth={1.5} />
        </CaptionButton>
        <CaptionButton label={maximized ? "Restore" : "Maximize"} onClick={() => void win()?.toggleMaximize()}>
          {maximized ? <Copy className="size-3.5 -scale-x-100" strokeWidth={1.5} /> : <Square className="size-3.5" strokeWidth={1.5} />}
        </CaptionButton>
        <CaptionButton label="Close" danger onClick={() => void win()?.close()}>
          <X className="size-4" strokeWidth={1.5} />
        </CaptionButton>
      </div>
    </header>
  );
}

function CaptionButton({ label, danger, onClick, children }: { label: string; danger?: boolean; onClick: () => void; children: ReactNode }) {
  return (
    <button
      type="button"
      title={label}
      aria-label={label}
      onClick={onClick}
      className={`flex h-full w-11 items-center justify-center text-zinc-400 transition-colors ${
        danger ? "hover:bg-[#c42b1c] hover:text-white" : "hover:bg-zinc-800 hover:text-zinc-100"
      }`}
    >
      {children}
    </button>
  );
}
