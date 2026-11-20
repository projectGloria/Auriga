import { ChevronDown, ChevronRight, ScrollText, Search } from "lucide-react";
import { Fragment, useMemo, useState, type ReactNode } from "react";
import type { EventRecord } from "../bindings/EventRecord";
import type { LatestSnapshot } from "../bindings/LatestSnapshot";
import type { Signal } from "../bindings/Signal";
import type { SignalKind } from "../bindings/SignalKind";
import { ScanControls } from "../components/ScanControls";
import { LOG_SOURCES, WarningsBanner } from "../components/Warnings";
import { Badge, Banner, PageHeader } from "../components/ui";
import { ago, eventLevel, shortChannel, shortProvider, signalInfo, timelineTone, when } from "../lib/analysis";
import { useScan } from "../lib/scan";

type Tab = "timeline" | "events" | "groups" | "setup";
const PAGE = 300;

export function LogsPage() {
  const { latest, loading, error, notice, dismiss } = useScan();
  const [tab, setTab] = useState<Tab>("timeline");
  const [signalFilter, setSignalFilter] = useState<SignalKind | "">("");

  const snap = latest?.snapshot;
  const warnings = (snap?.warnings ?? []).filter((w) => LOG_SOURCES.includes(w.source));
  const names = useMemo(() => new Map((snap?.devices ?? []).map((d) => [d.instanceId, d.name])), [snap]);

  const tabs: { id: Tab; label: string; count?: number }[] = latest
    ? [
        { id: "timeline", label: "Timeline", count: latest.analysis.timeline.length },
        { id: "events", label: "Events", count: latest.snapshot.events.length },
        { id: "groups", label: "By event ID", count: latest.analysis.groups.length },
        { id: "setup", label: "Setup log", count: latest.snapshot.setupapi.length },
      ]
    : [];

  return (
    <div className="flex h-full flex-col">
      <PageHeader
        title="Logs"
        subtitle={
          snap
            ? `Last ${snap.eventWindowDays} days of System, WLAN, Kernel-PnP and crash-report events · scanned ${new Date(snap.collectedAt).toLocaleString()}`
            : "Event Log, setupapi.dev.log and crash dumps"
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
        <WarningsBanner warnings={warnings} rest="Other log sources were read normally." />
      </div>

      {!latest || !snap ? (
        <div className="flex flex-1 flex-col items-center justify-center gap-3 text-zinc-500">
          <ScrollText className="size-10 text-zinc-700" />
          <p className="text-sm">{loading ? "Loading last scan…" : "No scan yet. Run a scan to read the logs."}</p>
        </div>
      ) : (
        <div className="min-h-0 flex-1 overflow-y-auto px-6 pb-6">
          <SignalCards
            signals={latest.analysis.signals}
            names={names}
            reference={snap.collectedAt}
            active={signalFilter}
            onPick={(k) => {
              setSignalFilter((cur) => (cur === k ? "" : k));
              setTab("events");
            }}
          />
          <DumpsNote latest={latest} />

          <div className="sticky top-0 z-10 -mx-6 mt-4 flex gap-1 border-b border-zinc-800 bg-zinc-950 px-6">
            {tabs.map((t) => (
              <button
                key={t.id}
                onClick={() => setTab(t.id)}
                className={`-mb-px border-b-2 px-3 py-2 text-sm ${
                  tab === t.id ? "border-sky-500 text-zinc-100" : "border-transparent text-zinc-400 hover:text-zinc-200"
                }`}
              >
                {t.label}
                {t.count != null && <span className="ml-1.5 text-xs tabular-nums text-zinc-500">{t.count}</span>}
              </button>
            ))}
          </div>

          <div className="pt-3">
            {tab === "timeline" && <Timeline latest={latest} names={names} />}
            {tab === "events" && (
              <Events latest={latest} names={names} signal={signalFilter} setSignal={setSignalFilter} />
            )}
            {tab === "groups" && <Groups latest={latest} names={names} />}
            {tab === "setup" && <SetupLog latest={latest} names={names} />}
          </div>
        </div>
      )}
    </div>
  );
}

function SignalCards({
  signals,
  names,
  reference,
  active,
  onPick,
}: {
  signals: Signal[];
  names: Map<string, string>;
  reference: string;
  active: SignalKind | "";
  onPick: (k: SignalKind) => void;
}) {
  if (signals.length === 0)
    return (
      <p className="mt-4 rounded-lg border border-zinc-800 bg-zinc-900/50 px-4 py-3 text-sm text-zinc-400">
        No crashes, resets, disconnects or install failures in this window.
      </p>
    );
  return (
    <div className="mt-4 grid grid-cols-[repeat(auto-fill,minmax(15rem,1fr))] gap-3">
      {signals.map((s) => {
        const info = signalInfo[s.kind];
        const dev = s.deviceIds[0] ? names.get(s.deviceIds[0]) : undefined;
        return (
          <button
            key={s.kind}
            onClick={() => onPick(s.kind)}
            title={info.hint}
            className={`rounded-lg border bg-zinc-900/60 px-3.5 py-3 text-left transition-colors hover:bg-zinc-900 ${
              active === s.kind ? "border-sky-600" : "border-zinc-800"
            }`}
          >
            <div className="flex items-center justify-between gap-2">
              <span className="text-sm font-medium text-zinc-200">{info.label}</span>
              <Badge tone={info.tone}>{s.count}</Badge>
            </div>
            <div className="mt-1.5 text-xs text-zinc-400">
              {s.last24h} in 24 h · {s.last7d} in 7 d · peak {s.peakPerHour}/h
            </div>
            <div className="mt-0.5 text-xs text-zinc-500">Last seen {ago(s.last, reference)}</div>
            {s.breakdown.length > 0 && (
              <div className="mt-1.5 truncate font-mono text-[11px] text-zinc-400">
                {s.breakdown.map(([v, n]) => `${v} ×${n}`).join(" · ")}
              </div>
            )}
            {dev && <div className="mt-1 truncate text-xs text-zinc-500">{dev}</div>}
          </button>
        );
      })}
    </div>
  );
}

function DumpsNote({ latest }: { latest: LatestSnapshot }) {
  const d = latest.snapshot.dumps;
  if (d.length === 0) return null;
  const newest = d.reduce((a, b) => ((a.modified ?? "") > (b.modified ?? "") ? a : b));
  return (
    <p className="mt-3 text-xs text-zinc-400">
      {d.length} crash dump file{d.length === 1 ? "" : "s"} on disk (newest {when(newest.modified)}:{" "}
      <span className="font-mono">{newest.path}</span>). Reading their contents needs admin and arrives with the
      opt-in helper analysis.
    </p>
  );
}

function DeviceName({ id, names }: { id: string | null; names: Map<string, string> }) {
  if (!id) return null;
  return (
    <span className="truncate text-xs text-zinc-500" title={id}>
      {names.get(id) ?? id}
    </span>
  );
}

function Timeline({ latest, names }: { latest: LatestSnapshot; names: Map<string, string> }) {
  const [hideRoutine, setHideRoutine] = useState(true);
  const [limit, setLimit] = useState(PAGE);
  const routine = new Set(["boot", "sleep", "wake"]);
  const items = latest.analysis.timeline.filter((t) => !hideRoutine || !routine.has(t.kind));
  return (
    <>
      <label className="mb-2 flex select-none items-center gap-2 text-sm text-zinc-300">
        <input type="checkbox" checked={hideRoutine} onChange={(e) => setHideRoutine(e.target.checked)} className="accent-sky-500" />
        Hide boot / sleep / wake
      </label>
      {items.length === 0 && <Empty>Nothing on the timeline.</Empty>}
      <ol className="relative ml-2 border-l border-zinc-800">
        {items.slice(0, limit).map((t, i) => (
          <li key={i} className="relative py-1.5 pl-5">
            <span
              className={`absolute -left-[5px] top-3 size-2.5 rounded-full ring-2 ring-zinc-950 ${dot[timelineTone[t.kind]]}`}
            />
            <div className="grid grid-cols-[9rem_1fr] items-baseline gap-x-3">
              <span className="text-xs tabular-nums text-zinc-500">{when(t.time)}</span>
              <div className="flex min-w-0 flex-col">
                <span className="text-sm text-zinc-200">
                  {t.title}
                  {t.count > 1 && (
                    <span className="ml-2 align-middle">
                      <Badge tone="muted" title={`From ${when(t.firstTime)} to ${when(t.time)}`}>
                        ×{t.count} within 10 min
                      </Badge>
                    </span>
                  )}
                </span>
                {t.detail && (
                  <span className="truncate text-xs text-zinc-400" title={t.detail}>
                    {t.detail}
                  </span>
                )}
                <DeviceName id={t.deviceId} names={names} />
              </div>
            </div>
          </li>
        ))}
      </ol>
      <More shown={Math.min(limit, items.length)} total={items.length} onMore={() => setLimit((l) => l + PAGE)} />
    </>
  );
}

const dot: Record<string, string> = {
  ok: "bg-emerald-400",
  warn: "bg-amber-400",
  bad: "bg-rose-500",
  muted: "bg-zinc-600",
  info: "bg-sky-400",
};

function Events({
  latest,
  names,
  signal,
  setSignal,
}: {
  latest: LatestSnapshot;
  names: Map<string, string>;
  signal: SignalKind | "";
  setSignal: (s: SignalKind | "") => void;
}) {
  const { events } = latest.snapshot;
  const { eventDevice, eventSignal } = latest.analysis;
  const [channel, setChannel] = useState("");
  const [level, setLevel] = useState("");
  const [device, setDevice] = useState("");
  const [query, setQuery] = useState("");
  const [open, setOpen] = useState<number | null>(null);
  const [limit, setLimit] = useState(PAGE);

  const channels = useMemo(() => [...new Set(events.map((e) => e.channel))].sort(), [events]);
  const devices = useMemo(() => {
    const ids = [...new Set(eventDevice.filter((d): d is string => d != null))];
    return ids.map((id) => [id, names.get(id) ?? id] as const).sort((a, b) => a[1].localeCompare(b[1]));
  }, [eventDevice, names]);
  const signals = useMemo(() => [...new Set(eventSignal.filter((s): s is SignalKind => s != null))], [eventSignal]);

  const rows = useMemo(() => {
    const q = query.trim().toLowerCase();
    const out: { i: number; e: EventRecord; dev: string | null; sig: SignalKind | null }[] = [];
    events.forEach((e, i) => {
      const dev = eventDevice[i] ?? null;
      const sig = eventSignal[i] ?? null;
      if (channel && e.channel !== channel) return;
      if (level === "err" && !(e.level === 1 || e.level === 2)) return;
      if (level === "warn" && e.level !== 3) return;
      if (signal && sig !== signal) return;
      if (device && dev !== device) return;
      if (q && !`${e.provider} ${e.eventId} ${e.message ?? ""}`.toLowerCase().includes(q)) return;
      out.push({ i, e, dev, sig });
    });
    return out;
  }, [events, eventDevice, eventSignal, channel, level, signal, device, query]);

  return (
    <>
      <div className="mb-2 flex flex-wrap items-center gap-2">
        <label className="relative">
          <Search className="pointer-events-none absolute left-2.5 top-1/2 size-4 -translate-y-1/2 text-zinc-500" />
          <input
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            placeholder="Provider, event ID, message…"
            className="w-64 rounded-md border border-zinc-800 bg-zinc-900 py-1.5 pl-8 pr-3 text-sm text-zinc-100 placeholder:text-zinc-500 focus:border-sky-600 focus:outline-none"
          />
        </label>
        <select value={channel} onChange={(e) => setChannel(e.target.value)} className="select">
          <option value="">All channels</option>
          {channels.map((c) => (
            <option key={c} value={c}>
              {shortChannel(c)}
            </option>
          ))}
        </select>
        <select value={level} onChange={(e) => setLevel(e.target.value)} className="select">
          <option value="">Any level</option>
          <option value="err">Critical / Error</option>
          <option value="warn">Warning</option>
        </select>
        <select value={signal} onChange={(e) => setSignal(e.target.value as SignalKind | "")} className="select">
          <option value="">Any signal</option>
          {signals.map((s) => (
            <option key={s} value={s}>
              {signalInfo[s].label}
            </option>
          ))}
        </select>
        <select value={device} onChange={(e) => setDevice(e.target.value)} className="select max-w-64">
          <option value="">Any device</option>
          {devices.map(([id, n]) => (
            <option key={id} value={id}>
              {n}
            </option>
          ))}
        </select>
        <span className="ml-auto text-xs tabular-nums text-zinc-500">{rows.length} shown</span>
      </div>

      <table className="w-full table-fixed border-separate border-spacing-0 text-sm">
        <colgroup>
          <col className="w-36" />
          <col className="w-20" />
          <col className="w-[22%]" />
          <col className="w-16" />
          <col />
        </colgroup>
        <thead>
          <tr className="text-left text-xs uppercase tracking-wide text-zinc-500">
            <Th>Time</Th>
            <Th>Level</Th>
            <Th>Provider</Th>
            <Th>ID</Th>
            <Th>Message / device</Th>
          </tr>
        </thead>
        <tbody>
          {rows.slice(0, limit).map(({ i, e, dev, sig }) => {
            const lv = eventLevel(e.level);
            const isOpen = open === i;
            return (
              <Fragment key={`${e.channel}:${e.recordId}`}>
                <tr
                  onClick={() => setOpen(isOpen ? null : i)}
                  className="cursor-pointer align-top hover:bg-zinc-900"
                >
                  <Td>
                    <span className="flex items-center gap-1 tabular-nums text-zinc-400">
                      {isOpen ? <ChevronDown className="size-3" /> : <ChevronRight className="size-3" />}
                      {when(e.time)}
                    </span>
                  </Td>
                  <Td>
                    <Badge tone={lv.tone}>{lv.label}</Badge>
                  </Td>
                  <Td>
                    <span className="block truncate text-zinc-300" title={e.provider}>
                      {shortProvider(e.provider)}
                    </span>
                  </Td>
                  <Td>
                    <span className="font-mono text-xs text-zinc-300">{e.eventId}</span>
                  </Td>
                  <Td>
                    <span className={`block text-zinc-300 ${isOpen ? "whitespace-pre-wrap" : "truncate"}`}>
                      {e.message ?? <span className="text-zinc-500">{dataSummary(e)}</span>}
                    </span>
                    <DeviceName id={dev} names={names} />
                  </Td>
                </tr>
                {isOpen && (
                  <tr>
                    <td colSpan={5} className="border-b border-zinc-900 bg-zinc-900/40 px-4 py-2">
                      <EventDetail e={e} device={dev} signal={sig} />
                    </td>
                  </tr>
                )}
              </Fragment>
            );
          })}
        </tbody>
      </table>
      {rows.length === 0 && <Empty>No events match the filters.</Empty>}
      <More shown={Math.min(limit, rows.length)} total={rows.length} onMore={() => setLimit((l) => l + PAGE)} />
    </>
  );
}

function dataSummary(e: EventRecord): string {
  return e.data
    .slice(0, 4)
    .map((f) => (f.name ? `${f.name}=${f.value}` : f.value))
    .join(" · ");
}

function EventDetail({ e, device, signal }: { e: EventRecord; device: string | null; signal: SignalKind | null }) {
  return (
    <dl className="grid grid-cols-[9rem_1fr] gap-x-3 gap-y-1 text-xs">
      <dt className="text-zinc-500">Channel</dt>
      <dd className="font-mono text-zinc-300">{e.channel}</dd>
      <dt className="text-zinc-500">Provider</dt>
      <dd className="font-mono text-zinc-300">{e.provider}</dd>
      <dt className="text-zinc-500">Record</dt>
      <dd className="font-mono text-zinc-300">
        {e.recordId} · {e.time}
      </dd>
      {signal && (
        <>
          <dt className="text-zinc-500">Signal</dt>
          <dd className="text-zinc-300">{signalInfo[signal].label}</dd>
        </>
      )}
      {device && (
        <>
          <dt className="text-zinc-500">Device</dt>
          <dd className="break-all font-mono text-zinc-300">{device}</dd>
        </>
      )}
      {e.data.map((f, j) => (
        <Fragment key={j}>
          <dt className="truncate text-zinc-500" title={f.name ?? undefined}>
            {f.name ?? `Data ${j + 1}`}
          </dt>
          <dd className="break-all font-mono text-zinc-300">{f.value || <span className="text-zinc-600">(empty)</span>}</dd>
        </Fragment>
      ))}
    </dl>
  );
}

function Groups({ latest, names }: { latest: LatestSnapshot; names: Map<string, string> }) {
  const ref = latest.snapshot.collectedAt;
  return (
    <table className="w-full table-fixed border-separate border-spacing-0 text-sm">
      <colgroup>
        <col className="w-[24%]" />
        <col className="w-16" />
        <col className="w-20" />
        <col className="w-28" />
        <col className="w-28" />
        <col />
      </colgroup>
      <thead>
        <tr className="text-left text-xs uppercase tracking-wide text-zinc-500">
          <Th>Provider</Th>
          <Th>ID</Th>
          <Th>Level</Th>
          <Th>Count (24 h / 7 d)</Th>
          <Th>Last seen</Th>
          <Th>Sample message / device</Th>
        </tr>
      </thead>
      <tbody>
        {latest.analysis.groups.map((g) => {
          const lv = eventLevel(g.level);
          return (
            <tr key={`${g.channel}|${g.provider}|${g.eventId}`} className="align-top hover:bg-zinc-900">
              <Td>
                <span className="block truncate text-zinc-300" title={`${g.provider} (${g.channel})`}>
                  {shortProvider(g.provider)}
                </span>
                <span className="block truncate text-xs text-zinc-500">{shortChannel(g.channel)}</span>
              </Td>
              <Td>
                <span className="font-mono text-xs">{g.eventId}</span>
              </Td>
              <Td>
                <Badge tone={lv.tone}>{lv.label}</Badge>
              </Td>
              <Td>
                <span className="tabular-nums text-zinc-200">{g.count}</span>{" "}
                <span className="text-xs tabular-nums text-zinc-500">
                  ({g.last24h} / {g.last7d}) · peak {g.peakPerHour}/h
                </span>
              </Td>
              <Td>
                <span className="text-xs text-zinc-400" title={when(g.last)}>
                  {ago(g.last, ref)}
                </span>
              </Td>
              <Td>
                <span className="block truncate text-zinc-400" title={g.sampleMessage ?? undefined}>
                  {g.sampleMessage ?? ""}
                </span>
                {g.deviceIds[0] && <DeviceName id={g.deviceIds[0]} names={names} />}
              </Td>
            </tr>
          );
        })}
      </tbody>
    </table>
  );
}

function SetupLog({ latest, names }: { latest: LatestSnapshot; names: Map<string, string> }) {
  const [onlyProblems, setOnlyProblems] = useState(false);
  const secs = latest.snapshot.setupapi.filter((s) => !onlyProblems || s.status === "failure" || s.errorCount > 0);
  return (
    <>
      <label className="mb-2 flex select-none items-center gap-2 text-sm text-zinc-300">
        <input type="checkbox" checked={onlyProblems} onChange={(e) => setOnlyProblems(e.target.checked)} className="accent-sky-500" />
        Only failures and sections with errors
      </label>
      {secs.length === 0 && <Empty>No setupapi.dev.log sections in this window.</Empty>}
      <ul className="space-y-2">
        {secs.map((s, i) => (
          <li key={i} className="rounded-lg border border-zinc-800 bg-zinc-900/40 px-3 py-2">
            <div className="flex flex-wrap items-center gap-2">
              <Badge tone={s.status === "failure" ? "bad" : s.status === "success" ? "ok" : "muted"}>
                {s.status === "failure" ? `Failed${s.statusCode ? ` ${s.statusCode}` : ""}` : s.status === "success" ? "Success" : "Unknown"}
              </Badge>
              {s.errorCount > 0 && <Badge tone="bad">{s.errorCount} error{s.errorCount === 1 ? "" : "s"}</Badge>}
              {s.warningCount > 0 && <Badge tone="warn">{s.warningCount} warning{s.warningCount === 1 ? "" : "s"}</Badge>}
              <span className="text-xs tabular-nums text-zinc-500">{when(s.start)}</span>
            </div>
            <div className="mt-1 break-all text-sm text-zinc-200">{s.title}</div>
            {s.instanceId && <DeviceName id={s.instanceId} names={names} />}
            {s.infs.length > 0 && <div className="mt-0.5 font-mono text-xs text-zinc-400">{s.infs.join(", ")}</div>}
            {s.errors.length > 0 && (
              <ul className="mt-1 space-y-0.5 font-mono text-xs text-rose-300/90">
                {s.errors.map((e, j) => (
                  <li key={j} className="break-all">
                    {e}
                  </li>
                ))}
              </ul>
            )}
          </li>
        ))}
      </ul>
    </>
  );
}

function Th({ children }: { children: ReactNode }) {
  return <th className="sticky top-[41px] border-b border-zinc-800 bg-zinc-950 px-2 py-2 font-medium">{children}</th>;
}
function Td({ children }: { children: ReactNode }) {
  return <td className="min-w-0 overflow-hidden border-b border-zinc-900 px-2 py-1.5">{children}</td>;
}
function Empty({ children }: { children: ReactNode }) {
  return <p className="py-8 text-center text-sm text-zinc-500">{children}</p>;
}
function More({ shown, total, onMore }: { shown: number; total: number; onMore: () => void }) {
  if (shown >= total) return null;
  return (
    <div className="py-3 text-center">
      <button onClick={onMore} className="text-sm text-sky-400 hover:text-sky-300">
        Show more ({shown} of {total})
      </button>
    </div>
  );
}
