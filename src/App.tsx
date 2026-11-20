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
import { useState, type ReactNode } from "react";
import { DevicesPage } from "./pages/Devices";
import { LogsPage } from "./pages/Logs";
import { SettingsPage } from "./pages/Settings";
import { ScanProvider, useScan } from "./lib/scan";

type PageId = "dashboard" | "devices" | "findings" | "updates" | "history" | "logs" | "settings";

const NAV: { id: PageId; label: string; icon: LucideIcon; phase?: number }[] = [
  { id: "dashboard", label: "Dashboard", icon: LayoutDashboard, phase: 6 },
  { id: "devices", label: "Devices", icon: Cpu },
  { id: "findings", label: "Findings", icon: TriangleAlert, phase: 3 },
  { id: "updates", label: "Updates", icon: Download, phase: 5 },
  { id: "history", label: "Fix History", icon: History, phase: 4 },
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
  const { running } = useScan();
  const current = NAV.find((n) => n.id === page)!;

  let body: ReactNode;
  if (page === "devices") body = <DevicesPage />;
  else if (page === "logs") body = <LogsPage />;
  else if (page === "settings") body = <SettingsPage />;
  else body = <Placeholder label={current.label} phase={current.phase ?? 0} />;

  return (
    <div className="flex h-screen bg-zinc-950 text-zinc-200">
      <nav className="flex w-52 shrink-0 flex-col border-r border-zinc-800 bg-zinc-925 px-2 py-4">
        <div className="mb-5 flex items-center gap-2 px-2">
          <img src="/icon.svg" alt="" className="size-7" />
          <div>
            <div className="text-sm font-semibold text-zinc-100">Auriga</div>
            <div className="text-[11px] text-zinc-500">Driver Manager</div>
          </div>
        </div>
        <ul className="space-y-0.5">
          {NAV.map(({ id, label, icon: Icon, phase }) => (
            <li key={id}>
              <button
                onClick={() => setPage(id)}
                className={`flex w-full items-center gap-2.5 rounded-md px-2.5 py-1.5 text-sm transition-colors ${
                  page === id ? "bg-zinc-800 text-zinc-100" : "text-zinc-400 hover:bg-zinc-900 hover:text-zinc-200"
                }`}
              >
                <Icon className="size-4" />
                <span className="flex-1 text-left">{label}</span>
                {phase && <span className="text-[10px] text-zinc-600">soon</span>}
              </button>
            </li>
          ))}
        </ul>
        {running && (
          <div className="mt-auto flex items-center gap-2 px-2.5 text-xs text-sky-300">
            <span className="size-1.5 animate-pulse rounded-full bg-sky-400" /> Scanning…
          </div>
        )}
      </nav>
      <main className="min-w-0 flex-1">{body}</main>
    </div>
  );
}

function Placeholder({ label, phase }: { label: string; phase: number }) {
  return (
    <div className="flex h-full items-center justify-center text-sm text-zinc-500">
      {label} arrives in phase {phase}.
    </div>
  );
}
