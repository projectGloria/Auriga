import {
  Cpu,
  Download,
  History,
  LayoutDashboard,
  ScrollText,
  Settings,
  TriangleAlert,
  type LucideIcon,
} from "lucide-react";
import { useEffect, useState, type ReactNode } from "react";
import { TitleBar } from "./components/TitleBar";
import { DevicesPage } from "./pages/Devices";
import { FindingsPage } from "./pages/Findings";
import { FixHistoryPage } from "./pages/FixHistory";
import { LogsPage } from "./pages/Logs";
import { SettingsPage } from "./pages/Settings";
import { ScanProvider, useScan } from "./lib/scan";

type PageId =
  | "dashboard"
  | "devices"
  | "findings"
  | "updates"
  | "history"
  | "logs"
  | "settings";

const NAV: { id: PageId; label: string; icon: LucideIcon; phase?: number }[] = [
  { id: "dashboard", label: "Dashboard", icon: LayoutDashboard, phase: 6 },
  { id: "devices", label: "Devices", icon: Cpu },
  { id: "findings", label: "Findings", icon: TriangleAlert },
  { id: "updates", label: "Updates", icon: Download, phase: 5 },
  { id: "history", label: "Fix History", icon: History },
  { id: "logs", label: "Logs", icon: ScrollText },
  { id: "settings", label: "Settings", icon: Settings },
];

export default function App() {
  return (
    <ScanProvider>
      <Shell />
    </ScanProvider>
  );
}

function Shell() {
  const [page, setPage] = useState<PageId>("devices");
  const { running, loading } = useScan();
  useSplash(loading);
  const current = NAV.find((n) => n.id === page)!;

  let body: ReactNode;
  if (page === "devices") body = <DevicesPage />;
  else if (page === "findings")
    body = <FindingsPage onNavigate={(p) => setPage(p as PageId)} />;
  else if (page === "history") body = <FixHistoryPage />;
  else if (page === "logs") body = <LogsPage />;
  else if (page === "settings") body = <SettingsPage />;
  else body = <Placeholder label={current.label} phase={current.phase ?? 0} />;

  return (
    <div className="flex h-screen flex-col bg-zinc-950 text-zinc-200">
      <TitleBar
        status={
          running && (
            <span className="flex items-center gap-1.5 text-xs text-sky-300">
              <span className="size-1.5 animate-pulse rounded-full bg-sky-400" />{" "}
              Scanning…
            </span>
          )
        }
      />
      <div className="flex min-h-0 flex-1">
        <nav className="flex w-52 shrink-0 flex-col border-r border-zinc-800 bg-zinc-925 px-2 py-3">
          <ul className="space-y-0.5">
            {NAV.map(({ id, label, icon: Icon, phase }) => (
              <li key={id}>
                <button
                  onClick={() => setPage(id)}
                  title={phase ? "Coming soon" : undefined}
                  className={`flex w-full items-center gap-2.5 rounded-md px-2.5 py-1.5 text-sm transition-colors ${
                    page === id
                      ? "bg-zinc-800 text-zinc-100"
                      : phase
                        ? "text-zinc-600 hover:bg-zinc-900"
                        : "text-zinc-400 hover:bg-zinc-900 hover:text-zinc-200"
                  }`}
                >
                  <Icon className="size-4" />
                  <span className="flex-1 text-left">{label}</span>
                </button>
              </li>
            ))}
          </ul>
        </nav>
        <main className="min-w-0 flex-1">{body}</main>
      </div>
    </div>
  );
}

/** Fades out the static splash in index.html once the first data is in
 * (shown at least briefly so it does not flash). */
function useSplash(loading: boolean) {
  useEffect(() => {
    if (loading) return;
    const el = document.getElementById("splash");
    if (!el) return;
    const wait = Math.max(0, 900 - performance.now());
    const t = setTimeout(() => {
      el.classList.add("hide");
      setTimeout(() => el.remove(), 350);
    }, wait);
    return () => clearTimeout(t);
  }, [loading]);
}

function Placeholder({ label, phase }: { label: string; phase: number }) {
  return (
    <div className="flex h-full items-center justify-center text-sm text-zinc-500">
      {label} arrives in phase {phase}.
    </div>
  );
}
