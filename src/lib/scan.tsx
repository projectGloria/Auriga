import { createContext, useCallback, useContext, useEffect, useState, type ReactNode } from "react";
import type { AppInfo } from "../bindings/AppInfo";
import type { LatestSnapshot } from "../bindings/LatestSnapshot";
import type { ScanProgress } from "../bindings/ScanProgress";
import { api, errorText, events, inTauri } from "./api";

type ScanState = {
  info: AppInfo | null;
  latest: LatestSnapshot | null;
  loading: boolean;
  running: boolean;
  progress: ScanProgress | null;
  error: string | null;
  notice: string | null;
  start: () => void;
  cancel: () => void;
  dismiss: () => void;
};

const Ctx = createContext<ScanState | null>(null);

export function ScanProvider({ children }: { children: ReactNode }) {
  const [info, setInfo] = useState<AppInfo | null>(null);
  const [latest, setLatest] = useState<LatestSnapshot | null>(null);
  const [loading, setLoading] = useState(true);
  const [running, setRunning] = useState(false);
  const [progress, setProgress] = useState<ScanProgress | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);

  const reload = useCallback(async () => {
    try {
      setLatest(await api.latestSnapshot());
    } catch (e) {
      setError(`Could not load last scan: ${errorText(e)}`);
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    if (!inTauri) {
      setLoading(false);
      setError("Not running inside Tauri — start with `npm run tauri dev`.");
      return;
    }
    api.appInfo().then((i) => {
      setInfo(i);
      setRunning(i.scanRunning);
    }, (e) => setError(errorText(e)));
    void reload();
    const subs = [
      events.scanProgress((p) => setProgress(p)),
      events.scanFinished((f) => {
        setRunning(false);
        setProgress(null);
        if (f.status === "completed") void reload();
        else if (f.status === "cancelled") setNotice("Scan cancelled — showing the previous result.");
        else setError(`Scan failed: ${f.error ?? "unknown error"}`);
      }),
    ];
    return () => subs.forEach((s) => void s.then((u) => u()));
  }, [reload]);

  const start = useCallback(() => {
    setError(null);
    setNotice(null);
    setProgress(null);
    setRunning(true);
    api.startScan().catch((e) => {
      setRunning(false);
      setError(errorText(e));
    });
  }, []);

  const cancel = useCallback(() => void api.cancelScan(), []);
  const dismiss = useCallback(() => {
    setError(null);
    setNotice(null);
  }, []);

  return (
    <Ctx.Provider value={{ info, latest, loading, running, progress, error, notice, start, cancel, dismiss }}>
      {children}
    </Ctx.Provider>
  );
}

export function useScan(): ScanState {
  const c = useContext(Ctx);
  if (!c) throw new Error("useScan outside ScanProvider");
  return c;
}
