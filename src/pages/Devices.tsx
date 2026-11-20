import {
  createColumnHelper,
  flexRender,
  getCoreRowModel,
  getFilteredRowModel,
  getSortedRowModel,
  useReactTable,
  type SortingState,
} from "@tanstack/react-table";
import { ArrowDown, ArrowUp, Cpu, Search } from "lucide-react";
import { useCallback, useMemo, useState } from "react";
import type { DeviceRecord } from "../bindings/DeviceRecord";
import type { MatchAssessment } from "../bindings/MatchAssessment";
import { DeviceDrawer } from "../components/DeviceDrawer";
import { ScanControls } from "../components/ScanControls";
import { LOG_SOURCES, WarningsBanner } from "../components/Warnings";
import { Banner, PageHeader, StatusIcon } from "../components/ui";
import { levelIcon, levelLabel, levelTone, matchLabel } from "../lib/analysis";
import { day, deviceState, installed, searchText, signatureIcon, signatureLabel, signatureTone, stateIcon } from "../lib/device";
import { useScan } from "../lib/scan";

type StatusFilter = "all" | "problems" | "disabled" | "unsigned" | "match";

const col = createColumnHelper<DeviceRecord>();

const levelRank = { bad: 0, warn: 1, info: 2, good: 3 } as const;

const makeColumns = (matches: Map<string, MatchAssessment>) => [
  col.accessor("name", {
    header: "Device",
    cell: (c) => (
      <div className="min-w-0">
        <div className="truncate text-zinc-100" title={c.getValue()}>
          {c.getValue()}
        </div>
        {c.row.original.manufacturer && (
          <div className="truncate text-xs text-zinc-500">{c.row.original.manufacturer}</div>
        )}
      </div>
    ),
  }),
  col.accessor((d) => d.class ?? "", { id: "class", header: "Class" }),
  col.accessor((d) => deviceState(d), {
    id: "status",
    header: "Status",
    sortingFn: (a, b) => rankState(a.original) - rankState(b.original),
    cell: (c) => {
      const s = c.getValue();
      return (
        <StatusIcon
          icon={stateIcon[s.key]}
          tone={s.tone}
          label={[s.label, c.row.original.status.problemText].filter(Boolean).join(": ")}
          text={s.key === "problem" ? s.label : undefined}
        />
      );
    },
  }),
  col.accessor((d) => d.driver?.provider ?? "", { id: "provider", header: "Provider" }),
  col.accessor((d) => d.driver?.version ?? "", {
    id: "version",
    header: "Driver version",
    sortingFn: (a, b) => cmpVersion(a.original.driver?.version, b.original.driver?.version),
    cell: (c) => <span className="font-mono text-xs">{c.getValue()}</span>,
  }),
  col.accessor((d) => d.driver?.date ?? "", { id: "date", header: "Driver date", cell: (c) => <span className="tabular-nums">{c.getValue()}</span> }),
  col.accessor((d) => d.driver?.signature.status ?? null, {
    id: "signature",
    header: "Signature",
    cell: (c) => {
      const s = c.getValue();
      if (!s) return <span className="text-zinc-600">—</span>;
      return (
        <StatusIcon
          icon={signatureIcon[s]}
          tone={signatureTone[s]}
          label={[signatureLabel[s], c.row.original.driver?.signature.signer].filter(Boolean).join(" · ")}
          text={s === "valid" || s === "unknown" ? undefined : signatureLabel[s]}
        />
      );
    },
  }),
  col.accessor((d) => matches.get(d.instanceId) ?? null, {
    id: "match",
    header: "Match",
    sortingFn: (a, b) =>
      levelRank[matches.get(a.original.instanceId)?.level ?? "good"] -
      levelRank[matches.get(b.original.instanceId)?.level ?? "good"],
    cell: (c) => {
      const m = c.getValue();
      if (!m || m.kind.kind === "none") return <span className="text-zinc-600">—</span>;
      return (
        <StatusIcon icon={levelIcon[m.level]} tone={levelTone[m.level]} label={[`${levelLabel[m.level]}: ${matchLabel(m)}`, ...m.notes].join("\n")} />
      );
    },
  }),
  col.accessor((d) => day(installed(d)), {
    id: "installed",
    header: "Installed",
    cell: (c) => <span className="tabular-nums">{c.getValue()}</span>,
  }),
];

export function DevicesPage() {
  const { latest, loading, error, notice, dismiss } = useScan();
  const [query, setQuery] = useState("");
  const [cls, setCls] = useState("");
  const [status, setStatus] = useState<StatusFilter>("all");
  const [showGhosts, setShowGhosts] = useState(false);
  const [sorting, setSorting] = useState<SortingState>([{ id: "status", desc: false }]);
  const [selected, setSelected] = useState<DeviceRecord | null>(null);
  const closeDrawer = useCallback(() => setSelected(null), []);

  const snap = latest?.snapshot;
  const all = useMemo(() => snap?.devices ?? [], [snap]);
  const matches = useMemo(
    () => new Map((latest?.analysis.matches ?? []).map((m) => [m.instanceId, m])),
    [latest],
  );
  const columns = useMemo(() => makeColumns(matches), [matches]);
  const haystacks = useMemo(() => new Map(all.map((d) => [d.instanceId, searchText(d)])), [all]);
  const classes = useMemo(
    () => [...new Set(all.filter((d) => showGhosts || d.present).map((d) => d.class ?? ""))].filter(Boolean).sort(),
    [all, showGhosts],
  );

  const rows = useMemo(() => {
    const q = query.trim().toLowerCase();
    return all.filter((d) => {
      if (!showGhosts && !d.present) return false;
      if (cls && d.class !== cls) return false;
      if (status === "problems" && d.status.problemCode == null) return false;
      if (status === "disabled" && !d.status.disabled) return false;
      if (status === "unsigned" && (!d.driver || d.driver.signature.status === "valid")) return false;
      if (status === "match" && !["warn", "bad"].includes(matches.get(d.instanceId)?.level ?? "good")) return false;
      return !q || (haystacks.get(d.instanceId) ?? "").includes(q);
    });
  }, [all, showGhosts, cls, status, query, haystacks, matches]);

  const table = useReactTable({
    data: rows,
    columns,
    state: { sorting },
    onSortingChange: setSorting,
    getCoreRowModel: getCoreRowModel(),
    getSortedRowModel: getSortedRowModel(),
    getFilteredRowModel: getFilteredRowModel(),
    getRowId: (d) => d.instanceId,
  });

  const present = all.filter((d) => d.present).length;
  const problems = all.filter((d) => d.present && d.status.problemCode != null).length;
  const warnings = (snap?.warnings ?? []).filter((w) => !LOG_SOURCES.includes(w.source));

  return (
    <div className="flex h-full flex-col">
      <PageHeader
        title="Devices"
        subtitle={
          snap ? (
            <>
              {present} connected · {all.length - present} not connected · {problems} with problems · scanned{" "}
              {new Date(snap.collectedAt).toLocaleString()}
            </>
          ) : (
            "Inventory of devices and their drivers"
          )
        }
        actions={<ScanControls />}
      />

      <div className="space-y-2 px-6 pt-3 empty:hidden">
        {error && (
          <Banner tone="bad" onClose={dismiss}>
            {error}
          </Banner>
        )}
        {notice && (
          <Banner tone="info" onClose={dismiss}>
            {notice}
          </Banner>
        )}
        <WarningsBanner warnings={warnings} rest="The rest of the inventory is complete." />
      </div>

      {!snap ? (
        <EmptyState loading={loading} />
      ) : (
        <>
          <div className="flex flex-wrap items-center gap-2 px-6 py-3">
            <label className="relative">
              <Search className="pointer-events-none absolute left-2.5 top-1/2 size-4 -translate-y-1/2 text-zinc-500" />
              <input
                value={query}
                onChange={(e) => setQuery(e.target.value)}
                placeholder="Search name, hardware ID, INF, provider…"
                className="w-80 rounded-md border border-zinc-800 bg-zinc-900 py-1.5 pl-8 pr-3 text-sm text-zinc-100 placeholder:text-zinc-500 focus:border-sky-600 focus:outline-none"
              />
            </label>
            <select value={cls} onChange={(e) => setCls(e.target.value)} className="select">
              <option value="">All classes</option>
              {classes.map((c) => (
                <option key={c} value={c}>
                  {c}
                </option>
              ))}
            </select>
            <select value={status} onChange={(e) => setStatus(e.target.value as StatusFilter)} className="select">
              <option value="all">Any status</option>
              <option value="problems">Has a problem code</option>
              <option value="disabled">Disabled</option>
              <option value="unsigned">Signature not valid</option>
              <option value="match">Driver match needs a look</option>
            </select>
            <label className="ml-1 flex select-none items-center gap-2 text-sm text-zinc-300">
              <input type="checkbox" checked={showGhosts} onChange={(e) => setShowGhosts(e.target.checked)} className="accent-sky-500" />
              Show not-connected devices
            </label>
            <span className="ml-auto text-xs tabular-nums text-zinc-500">{rows.length} shown</span>
          </div>

          <div className="min-h-0 flex-1 overflow-auto px-6 pb-6">
            <table className="w-full table-fixed border-separate border-spacing-0 text-sm">
              <colgroup>
                <col className="w-[25%]" />
                <col className="w-[9%]" />
                <col className="w-[8.5%]" />
                <col className="w-[12.5%]" />
                <col className="w-[12.5%]" />
                <col className="w-[9%]" />
                <col className="w-[8%]" />
                <col className="w-[7%]" />
                <col className="w-[8.5%]" />
              </colgroup>
              <thead className="sticky top-0 z-10 bg-zinc-950">
                {table.getHeaderGroups().map((hg) => (
                  <tr key={hg.id}>
                    {hg.headers.map((h) => {
                      const dir = h.column.getIsSorted();
                      return (
                        <th
                          key={h.id}
                          onClick={h.column.getToggleSortingHandler()}
                          className="cursor-pointer select-none border-b border-zinc-800 px-2 py-2 text-left text-xs font-medium uppercase tracking-wide text-zinc-500 hover:text-zinc-300"
                        >
                          <span className="inline-flex items-center gap-1">
                            {flexRender(h.column.columnDef.header, h.getContext())}
                            {dir === "asc" && <ArrowUp className="size-3" />}
                            {dir === "desc" && <ArrowDown className="size-3" />}
                          </span>
                        </th>
                      );
                    })}
                  </tr>
                ))}
              </thead>
              <tbody>
                {table.getRowModel().rows.map((r) => (
                  <tr
                    key={r.id}
                    tabIndex={0}
                    onClick={() => setSelected(r.original)}
                    onKeyDown={(e) => e.key === "Enter" && setSelected(r.original)}
                    className={`cursor-pointer outline-none hover:bg-zinc-900 focus-visible:bg-zinc-900 ${r.original.present ? "" : "opacity-55"}`}
                  >
                    {r.getVisibleCells().map((c) => (
                      <td key={c.id} className="truncate border-b border-zinc-900 px-2 py-1.5 text-zinc-300">
                        {flexRender(c.column.columnDef.cell, c.getContext())}
                      </td>
                    ))}
                  </tr>
                ))}
              </tbody>
            </table>
            {rows.length === 0 && <p className="py-10 text-center text-sm text-zinc-500">No devices match the filters.</p>}
          </div>
        </>
      )}

      {selected && latest && (
        <DeviceDrawer
          device={selected}
          match={matches.get(selected.instanceId) ?? null}
          latest={latest}
          onClose={closeDrawer}
        />
      )}
    </div>
  );
}

function EmptyState({ loading }: { loading: boolean }) {
  return (
    <div className="flex flex-1 flex-col items-center justify-center gap-3 text-zinc-500">
      <Cpu className="size-10 text-zinc-700" />
      <p className="text-sm">{loading ? "Loading last scan…" : "No scan yet. Run a scan to list devices and drivers."}</p>
      {!loading && <ScanControls />}
    </div>
  );
}

function rankState(d: DeviceRecord): number {
  return ["problem", "disabled", "unknown", "ok", "ghost"].indexOf(deviceState(d).key);
}

/** Numeric dotted-version compare ("31.0.101.5186" vs "9.1.2"). */
function cmpVersion(a?: string | null, b?: string | null): number {
  const pa = (a ?? "").split(".").map((x) => parseInt(x, 10) || 0);
  const pb = (b ?? "").split(".").map((x) => parseInt(x, 10) || 0);
  for (let i = 0; i < Math.max(pa.length, pb.length); i++) {
    const d = (pa[i] ?? 0) - (pb[i] ?? 0);
    if (d) return d;
  }
  return 0;
}
