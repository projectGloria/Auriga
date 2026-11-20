import { ArrowRight, RotateCcw } from "lucide-react";
import { useCallback, useEffect, useState } from "react";
import type { FixHistoryEntry } from "../bindings/FixHistoryEntry";
import type { MonitorReport } from "../bindings/MonitorReport";
import { Badge, Banner, Button, PageHeader } from "../components/ui";
import { api, errorText, inTauri } from "../lib/api";
import { signalInfo, when } from "../lib/analysis";
import { canRevert, fixStateInfo, restorePointText, vt } from "../lib/fix";

export function FixHistoryPage() {
  const [entries, setEntries] = useState<FixHistoryEntry[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<{ tone: "info" | "warn" | "bad"; text: string } | null>(null);

  const load = useCallback(async () => {
    if (!inTauri) {
      setEntries([]);
      return;
    }
    try {
      setEntries(await api.fixHistory(200));
      setError(null);
    } catch (e) {
      setError(errorText(e));
    }
  }, []);
  useEffect(() => void load(), [load]);

  return (
    <div className="flex h-full flex-col">
      <PageHeader
        title="Fix History"
        subtitle="Every change Auriga made, with the exact previous value. Revert restores it."
        actions={<Button onClick={() => void load()}>Refresh</Button>}
      />
      <div className="min-h-0 flex-1 space-y-3 overflow-y-auto px-6 py-4">
      {error && <Banner tone="bad">{error}</Banner>}
      {notice && (
        <Banner tone={notice.tone} onClose={() => setNotice(null)}>
          {notice.text}
        </Banner>
      )}
      {entries === null && !error && <p className="text-sm text-zinc-500">Loading…</p>}
      {entries?.length === 0 && (
        <p className="text-sm text-zinc-500">No fixes yet. Apply one from Findings; it will appear here with a Revert button.</p>
      )}
      <ul className="space-y-3">
        {entries?.map((e) => (
          <Entry
            key={e.record.id}
            e={e}
            onReverted={(text, tone) => {
              setNotice({ text, tone });
              void load();
            }}
          />
        ))}
      </ul>
      </div>
    </div>
  );
}

function Entry({ e, onReverted }: { e: FixHistoryEntry; onReverted: (text: string, tone: "info" | "warn" | "bad") => void }) {
  const r = e.record;
  const st = fixStateInfo[r.state];
  const [confirm, setConfirm] = useState(false);
  const [busy, setBusy] = useState(false);

  const revert = async () => {
    setBusy(true);
    try {
      const out = await api.revertFix(r.id);
      if (out.kind === "refused") onReverted(out.reason, "warn");
      else {
        const s = out.record.state;
        onReverted(
          s === "reverted"
            ? `Reverted "${r.title}" to ${vt(r.from)}.`
            : `Revert of "${r.title}" did not complete: ${out.record.error ?? fixStateInfo[s].hint}`,
          s === "reverted" ? "info" : "bad",
        );
      }
    } catch (err) {
      onReverted(errorText(err), "bad");
    } finally {
      setBusy(false);
      setConfirm(false);
    }
  };

  return (
    <li className="rounded-lg border border-zinc-800 bg-zinc-900/60 px-4 py-3 text-sm">
      <div className="flex flex-wrap items-center gap-2">
        <Badge tone={st.tone} title={st.hint}>
          {st.label}
        </Badge>
        <span className="font-medium text-zinc-100">{r.title}</span>
        <span className="text-xs text-zinc-500">{when(r.appliedAt ?? r.createdAt)}</span>
      </div>
      <p className="mt-1 truncate text-zinc-400" title={r.deviceId}>
        {r.deviceName}
      </p>
      <p className="mt-1.5 font-mono text-xs text-zinc-400">
        {r.setting}: {vt(r.from)} <ArrowRight className="inline size-3 text-zinc-600" /> <span className="text-sky-300">{vt(r.to)}</span>
        {r.kind !== "rollbackDriver" && (
          <span className="ml-2 text-zinc-600">
            ({r.target}: {r.priorRaw ?? "not set"} → {r.newRaw ?? "removed"})
          </span>
        )}
      </p>
      {r.error && <p className="mt-1.5 text-red-300">{r.error}</p>}
      {r.restorePoint && <p className="mt-1 text-xs text-zinc-500">{restorePointText(r.restorePoint)}</p>}
      {r.revertedAt && <p className="mt-1 text-xs text-zinc-500">Reverted {when(r.revertedAt)}.</p>}
      {r.rebootRequired && r.state !== "reverted" && <p className="mt-1 text-xs text-amber-300">Needs a Windows restart to take full effect.</p>}
      {r.notes.length > 0 && (
        <ul className="mt-1.5 list-disc space-y-0.5 pl-5 text-xs text-zinc-500">
          {r.notes.map((n, i) => (
            <li key={i}>{n}</li>
          ))}
        </ul>
      )}
      {r.state === "applied" && <Monitor m={e.monitor} hasBaseline={r.baseline != null} />}

      {canRevert(r.revertible, r.state) && (
        <div className="mt-3 flex flex-wrap items-center gap-2">
          {!confirm ? (
            <Button onClick={() => setConfirm(true)}>
              <RotateCcw className="size-4" /> Revert…
            </Button>
          ) : (
            <>
              <span className="text-zinc-300">
                Set {r.setting} back to {vt(r.from)}? The device restarts briefly; Windows asks for admin permission.
              </span>
              <Button variant="danger" disabled={busy} onClick={() => void revert()}>
                {busy ? "Reverting…" : "Revert"}
              </Button>
              <Button disabled={busy} onClick={() => setConfirm(false)}>
                Cancel
              </Button>
            </>
          )}
        </div>
      )}
      {r.kind === "rollbackDriver" && r.state === "applied" && (
        <p className="mt-2 text-xs text-zinc-500">
          A driver rollback is undone by updating the driver again (Updates, or Device Manager); Auriga never installs drivers by itself.
        </p>
      )}
    </li>
  );
}

function Monitor({ m, hasBaseline }: { m: FixHistoryEntry["monitor"]; hasBaseline: boolean }) {
  if (!hasBaseline) return null;
  if (!m)
    return <p className="mt-2 text-xs text-zinc-500">Scan again after using the PC for a while to compare related events before and after.</p>;
  return (
    <div className="mt-2 rounded-md bg-zinc-950/50 px-3 py-2">
      <p className={m.tooEarly ? "text-zinc-400" : "text-zinc-200"}>{m.summary}</p>
      {m.trends.length > 0 && <Trends m={m} />}
    </div>
  );
}

function Trends({ m }: { m: MonitorReport }) {
  return (
    <table className="mt-1.5 text-xs text-zinc-400">
      <thead className="text-zinc-500">
        <tr>
          <th className="pr-4 text-left font-normal">Signal</th>
          <th className="pr-4 text-right font-normal">Before /day</th>
          <th className="text-right font-normal">After /day</th>
        </tr>
      </thead>
      <tbody className="tabular-nums">
        {m.trends.map((t) => (
          <tr key={t.kind}>
            <td className="pr-4">{signalInfo[t.kind]?.label ?? t.kind}</td>
            <td className="pr-4 text-right">{t.beforePerDay.toFixed(1)}</td>
            <td className="text-right">
              {t.afterPerDay.toFixed(1)} <span className="text-zinc-600">({t.afterCount})</span>
            </td>
          </tr>
        ))}
      </tbody>
    </table>
  );
}
