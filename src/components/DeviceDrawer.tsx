import { Copy, X } from "lucide-react";
import { useEffect, useMemo, type ReactNode } from "react";
import type { DeviceRecord } from "../bindings/DeviceRecord";
import type { LatestSnapshot } from "../bindings/LatestSnapshot";
import type { MatchAssessment } from "../bindings/MatchAssessment";
import { eventLevel, levelLabel, levelTone, matchLabel, shortProvider, signalInfo, when } from "../lib/analysis";
import { deviceState, signatureLabel, signatureTone } from "../lib/device";
import { Badge } from "./ui";

const RECENT = 15;

export function DeviceDrawer({
  device,
  match,
  latest,
  onClose,
}: {
  device: DeviceRecord;
  match: MatchAssessment | null;
  latest: LatestSnapshot;
  onClose: () => void;
}) {
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && onClose();
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);

  const st = deviceState(device);
  const evIdx = useMemo(() => {
    const out: number[] = [];
    latest.analysis.eventDevice.forEach((d, i) => d === device.instanceId && out.push(i));
    return out;
  }, [latest, device.instanceId]);
  const dr = device.driver;
  const pw = device.power;

  return (
    <div className="fixed inset-0 z-30 flex justify-end" role="dialog" aria-modal="true" aria-label={device.name}>
      <div className="absolute inset-0 bg-black/50" onClick={onClose} />
      <aside className="relative flex h-full w-[min(560px,92vw)] flex-col border-l border-zinc-800 bg-zinc-950 shadow-2xl">
        <div className="flex items-start gap-3 border-b border-zinc-800 px-5 py-4">
          <div className="min-w-0 flex-1">
            <h2 className="truncate text-base font-semibold text-zinc-100" title={device.name}>
              {device.name}
            </h2>
            <div className="mt-1 flex flex-wrap items-center gap-2 text-xs text-zinc-400">
              <Badge tone={st.tone}>{st.label}</Badge>
              {device.class && <span>{device.class}</span>}
            </div>
          </div>
          <button onClick={onClose} className="rounded p-1 text-zinc-400 hover:bg-zinc-800 hover:text-zinc-100" aria-label="Close">
            <X className="size-5" />
          </button>
        </div>

        <div className="flex-1 space-y-5 overflow-y-auto px-5 py-4 text-sm">
          {device.status.problemCode != null && (
            <div className="rounded-lg border border-rose-500/30 bg-rose-500/10 px-3 py-2 text-rose-200">
              <b>Code {device.status.problemCode}:</b> {device.status.problemText}
            </div>
          )}

          {match && match.kind.kind !== "none" && (
            <section>
              <h3 className="mb-2 text-xs font-semibold uppercase tracking-wide text-zinc-500">Driver match</h3>
              <div className="flex flex-wrap items-center gap-2">
                <Badge tone={levelTone[match.level]}>{levelLabel[match.level]}</Badge>
                <span className="text-zinc-300">{matchLabel(match)}</span>
                {match.inbox && <Badge tone="muted">Inbox (ships with Windows)</Badge>}
              </div>
              {match.notes.length > 0 && (
                <ul className="mt-2 list-disc space-y-1 pl-5 text-zinc-300">
                  {match.notes.map((n, i) => (
                    <li key={i}>{n}</li>
                  ))}
                </ul>
              )}
            </section>
          )}

          <section>
            <h3 className="mb-2 text-xs font-semibold uppercase tracking-wide text-zinc-500">
              Recent events ({evIdx.length} in {latest.snapshot.eventWindowDays} days)
            </h3>
            {evIdx.length === 0 ? (
              <p className="text-zinc-500">No collected events are attributed to this device.</p>
            ) : (
              <ul className="space-y-1.5">
                {evIdx.slice(0, RECENT).map((i) => {
                  const e = latest.snapshot.events[i];
                  if (!e) return null;
                  const sig = latest.analysis.eventSignal[i];
                  const lv = eventLevel(e.level);
                  return (
                    <li key={i} className="rounded-md bg-zinc-900 px-2.5 py-1.5">
                      <div className="flex flex-wrap items-center gap-2 text-xs">
                        <span className="tabular-nums text-zinc-500">{when(e.time)}</span>
                        <Badge tone={lv.tone}>{lv.label}</Badge>
                        <span className="text-zinc-400">
                          {shortProvider(e.provider)} {e.eventId}
                        </span>
                        {sig && <Badge tone={signalInfo[sig].tone}>{signalInfo[sig].label}</Badge>}
                      </div>
                      {e.message && <p className="mt-0.5 line-clamp-2 text-xs text-zinc-300" title={e.message}>{e.message}</p>}
                    </li>
                  );
                })}
                {evIdx.length > RECENT && (
                  <li className="text-xs text-zinc-500">…and {evIdx.length - RECENT} more on the Logs page.</li>
                )}
              </ul>
            )}
          </section>

          <Section title="Device">
            <Row k="Instance ID" v={device.instanceId} mono copy />
            <Row k="Description" v={device.description} />
            <Row k="Manufacturer" v={device.manufacturer} />
            <Row k="Class" v={device.class} />
            <Row k="Class GUID" v={device.classGuid} mono />
            <Row k="Enumerator" v={device.enumerator} />
            <Row k="Location" v={device.location} />
            <Row k="Service" v={device.service} mono />
            <Row k="Present" v={device.present ? "Yes" : "No (ghost)"} />
            <Row k="Started" v={device.status.available ? yn(device.status.started) : "Unknown"} />
            <Row k="DevNode flags" v={`0x${device.status.devnodeFlags.toString(16).padStart(8, "0")}`} mono />
          </Section>

          <Section title="Driver">
            {dr ? (
              <>
                <Row k="Provider" v={dr.provider} />
                <Row k="Version" v={dr.version} mono />
                <Row k="Date (INF)" v={dr.date} />
                <Row k="Description" v={dr.description} />
                <Row k="INF" v={dr.infName} mono />
                <Row k="INF section" v={dr.infSection} mono />
                <Row k="Matched on" v={dr.matchingDeviceId} mono />
                <Row k="Rank" v={dr.rank != null ? `0x${dr.rank.toString(16).padStart(8, "0")}` : null} mono />
                <Row k="Driver key" v={dr.driverKey} mono />
                <Row
                  k="Signature"
                  v={
                    <span className="flex flex-wrap items-center gap-2">
                      <Badge tone={signatureTone[dr.signature.status]}>{signatureLabel[dr.signature.status]}</Badge>
                      {dr.signature.signer && <span className="text-zinc-300">{dr.signature.signer}</span>}
                    </span>
                  }
                />
                <Row k="Catalog" v={dr.signature.catalog} mono />
                <Row k="Signature detail" v={dr.signature.detail} mono />
              </>
            ) : (
              <p className="text-zinc-500">No driver installed for this device.</p>
            )}
          </Section>

          <Section title="Install dates">
            <Row k="Driver installed" v={device.install.installDate} />
            <Row k="First installed" v={device.install.firstInstallDate} />
            <Row k="Last arrival" v={device.install.lastArrivalDate} />
          </Section>

          {pw && (
            <Section title="Power">
              <Row k="Current state" v={pw.currentState} />
              <Row k="Supported" v={pw.supportedStates.join(", ") || null} />
              <Row k="Can wake from" v={pw.wakeFrom.join(", ") || null} />
              {pw.allowTurnOff != null && (
                <Row
                  k="Allow PC to turn off"
                  v={
                    <span className="flex items-center gap-2">
                      {yn(pw.allowTurnOff)}
                      {pw.pnpCapabilitiesRaw != null && (
                        <span className="font-mono text-xs text-zinc-500">PnPCapabilities={pw.pnpCapabilitiesRaw}</span>
                      )}
                    </span>
                  }
                />
              )}
            </Section>
          )}

          <IdList title="Hardware IDs" ids={device.hardwareIds} />
          <IdList title="Compatible IDs" ids={device.compatibleIds} />
        </div>
      </aside>
    </div>
  );
}

function yn(b: boolean) {
  return b ? "Yes" : "No";
}

function Section({ title, children }: { title: string; children: ReactNode }) {
  return (
    <section>
      <h3 className="mb-2 text-xs font-semibold uppercase tracking-wide text-zinc-500">{title}</h3>
      <dl className="grid grid-cols-[9.5rem_1fr] gap-x-3 gap-y-1.5">{children}</dl>
    </section>
  );
}

function Row({ k, v, mono, copy }: { k: string; v: ReactNode; mono?: boolean; copy?: boolean }) {
  if (v == null || v === "") return null;
  return (
    <>
      <dt className="text-zinc-500">{k}</dt>
      <dd className={`flex min-w-0 items-start gap-1.5 break-all text-zinc-200 ${mono ? "font-mono text-xs leading-5" : ""}`}>
        <span className="min-w-0">{v}</span>
        {copy && typeof v === "string" && (
          <button
            className="shrink-0 rounded p-0.5 text-zinc-500 hover:text-zinc-200"
            title="Copy"
            onClick={() => void navigator.clipboard.writeText(v)}
          >
            <Copy className="size-3.5" />
          </button>
        )}
      </dd>
    </>
  );
}

function IdList({ title, ids }: { title: string; ids: string[] }) {
  if (ids.length === 0) return null;
  return (
    <section>
      <h3 className="mb-2 text-xs font-semibold uppercase tracking-wide text-zinc-500">{title}</h3>
      <ul className="space-y-0.5 rounded-lg bg-zinc-900 px-3 py-2 font-mono text-xs text-zinc-300">
        {ids.map((id) => (
          <li key={id} className="break-all">
            {id}
          </li>
        ))}
      </ul>
    </section>
  );
}
