import { BookOpen, ChevronDown, ChevronRight, CircleCheck, CircleSlash, ExternalLink, Lightbulb, ListChecks, ShieldCheck, ShieldHalf, TriangleAlert, Wrench } from "lucide-react";
import { useCallback, useEffect, useMemo, useState, type ReactNode } from "react";
import type { Confidence } from "../bindings/Confidence";
import type { Finding } from "../bindings/Finding";
import type { FindingsReport } from "../bindings/FindingsReport";
import type { FixStatus } from "../bindings/FixStatus";
import type { FixSuggestion } from "../bindings/FixSuggestion";
import type { Severity } from "../bindings/Severity";
import type { ValueText } from "../bindings/ValueText";
import { FixDialog } from "../components/FixDialog";
import { ScanControls } from "../components/ScanControls";
import { Badge, Banner, Button, PageHeader, StatusIcon } from "../components/ui";
import { api, errorText, events, inTauri } from "../lib/api";
import type { Tone } from "../lib/device";
import { useScan } from "../lib/scan";

const severityInfo: Record<Severity, { label: string; tone: Tone; group: string }> = {
  problem: { label: "Problem", tone: "bad", group: "Problems" },
  warning: { label: "Warning", tone: "warn", group: "Warnings" },
  info: { label: "Note", tone: "info", group: "Worth a look" },
};
const SEVERITIES: Severity[] = ["problem", "warning", "info"];

const confidenceInfo: Record<Confidence, { label: string; tone: Tone; hint: string }> = {
  high: { label: "High confidence", tone: "ok", hint: "Windows itself reports this, or the cause is well documented." },
  medium: { label: "Medium confidence", tone: "info", hint: "A known cause that fits the evidence; other causes are possible." },
  low: { label: "Low confidence", tone: "muted", hint: "Plausible and cheap to try, but often not the cause." },
};

function useFindings() {
  const { latest } = useScan();
  const [report, setReport] = useState<FindingsReport | null>(null);
  const [error, setError] = useState<string | null>(null);

  const load = useCallback(async () => {
    try {
      if (inTauri) setReport(await api.findings());
      else if (import.meta.env.DEV) {
        // Dev-only browser preview written by the `collect` example.
        const r = await fetch("/dev-findings.json");
        if (!r.ok) throw new Error("public/dev-findings.json missing — run the collect example.");
        setReport((await r.json()) as FindingsReport);
      }
      setError(null);
    } catch (e) {
      setError(errorText(e));
    }
  }, []);

  // Re-evaluate whenever a new scan lands.
  useEffect(() => void load(), [load, latest?.summary.id, latest?.snapshot.collectedAt]);
  // ...and whenever a rule file changes on disk.
  useEffect(() => {
    if (!inTauri) return;
    const subs = [events.kbReloaded(() => void load()), events.deviceRefreshed(() => void load())];
    return () => subs.forEach((s) => void s.then((u) => u()));
  }, [load]);

  return { report, error };
}

export function FindingsPage({ onNavigate }: { onNavigate: (page: string) => void }) {
  const { latest, loading, error: scanError, notice, dismiss } = useScan();
  const { report, error } = useFindings();
  const findings = useMemo(() => report?.findings ?? [], [report]);
  const kbErrors = report?.kb.errors ?? [];

  const counts = SEVERITIES.map((s) => [s, findings.filter((f) => f.severity === s).length] as const);

  return (
    <div className="flex h-full flex-col">
      <PageHeader
        title="Findings"
        subtitle={
          report ? (
            <>
              {report.kb.ruleCount} rules
              {report.kb.kbVersion && <> · KB {report.kb.kbVersion}</>}
              {report.scanCollectedAt && <> · scan of {new Date(report.scanCollectedAt).toLocaleString()}</>}
              {" · fully offline"}
            </>
          ) : (
            "Known problems matched against this PC, from the offline knowledge base"
          )
        }
        actions={<ScanControls />}
      />
      <div className="space-y-2 px-6 pt-3 empty:hidden">
        {scanError && (
          <Banner tone="bad" onClose={dismiss}>
            {scanError}
          </Banner>
        )}
        {notice && (
          <Banner tone="info" onClose={dismiss}>
            {notice}
          </Banner>
        )}
        {error && <Banner tone="bad">Could not evaluate rules: {error}</Banner>}
        {kbErrors.length > 0 && (
          <Banner tone="warn">
            <div className="font-medium">
              {kbErrors.length} rule file{kbErrors.length === 1 ? "" : "s"} could not be loaded and {kbErrors.length === 1 ? "was" : "were"} skipped. The other rules still apply.
            </div>
            <ul className="mt-1 space-y-0.5 font-mono text-xs opacity-90">
              {kbErrors.map((e, i) => (
                <li key={i}>
                  {e.file && <span className="font-semibold">{e.file}: </span>}
                  {e.message}
                </li>
              ))}
            </ul>
          </Banner>
        )}
      </div>

      <div className="min-h-0 flex-1 overflow-y-auto px-6 py-4">
        {!report || (loading && !latest) ? (
          <Empty>Loading…</Empty>
        ) : !report.scanCollectedAt ? (
          <Empty>Run a scan to check this PC against the knowledge base.</Empty>
        ) : findings.length === 0 ? (
          <Empty icon={<ShieldCheck className="size-8 text-emerald-400/70" />}>
            No known problems matched. {report.kb.ruleCount} rules were checked against the latest scan.
          </Empty>
        ) : (
          <div className="mx-auto max-w-5xl space-y-6">
            <div className="flex gap-2">
              {counts
                .filter(([, n]) => n > 0)
                .map(([s, n]) => (
                  <Badge key={s} tone={severityInfo[s].tone}>
                    {n} {severityInfo[s].label.toLowerCase()}
                    {n === 1 ? "" : "s"}
                  </Badge>
                ))}
            </div>
            {SEVERITIES.map((s) => {
              const group = findings.filter((f) => f.severity === s);
              if (group.length === 0) return null;
              return (
                <section key={s} className="space-y-3">
                  <h2 className="text-xs font-semibold uppercase tracking-wide text-zinc-500">{severityInfo[s].group}</h2>
                  {group.map((f) => (
                    <FindingCard key={f.key} f={f} onNavigate={onNavigate} />
                  ))}
                </section>
              );
            })}
          </div>
        )}
      </div>
    </div>
  );
}

function Empty({ children, icon }: { children: ReactNode; icon?: ReactNode }) {
  return (
    <div className="flex h-full flex-col items-center justify-center gap-3 text-center text-sm text-zinc-500">
      {icon}
      <p className="max-w-md">{children}</p>
    </div>
  );
}

function FindingCard({ f, onNavigate }: { f: Finding; onNavigate: (page: string) => void }) {
  const [open, setOpen] = useState(f.severity !== "info");
  const sev = severityInfo[f.severity];
  const conf = confidenceInfo[f.confidence];
  const required = f.fixes.filter((x) => !x.optional);
  const optional = f.fixes.filter((x) => x.optional);
  const pending = f.fixes.filter((x) => x.status.state === "applicable").length;

  return (
    <article className="rounded-lg border border-zinc-800 bg-zinc-900/60">
      <button onClick={() => setOpen(!open)} className="flex w-full items-start gap-3 px-4 py-3 text-left">
        {open ? <ChevronDown className="mt-0.5 size-4 shrink-0 text-zinc-500" /> : <ChevronRight className="mt-0.5 size-4 shrink-0 text-zinc-500" />}
        <div className="min-w-0 flex-1">
          <div className="flex flex-wrap items-center gap-2">
            <Badge tone={sev.tone}>
              <TriangleAlert className="size-3" />
              {sev.label}
            </Badge>
            <Badge tone={conf.tone} title={conf.hint}>
              {conf.label}
            </Badge>
            <span className="text-xs text-zinc-500">{f.category}</span>
            {pending > 0 && <span className="text-xs text-sky-300">{pending} suggested change{pending === 1 ? "" : "s"}</span>}
          </div>
          <h3 className="mt-1.5 font-medium text-zinc-100">{f.title}</h3>
          <p className="mt-0.5 text-sm text-zinc-400">{f.summary}</p>
        </div>
      </button>

      {open && (
        <div className="space-y-4 border-t border-zinc-800 px-4 py-4 pl-11 text-sm">
          {f.evidence.length > 0 && (
            <Section title="Seen on this PC">
              <ul className="space-y-1">
                {f.evidence.map((e, i) => (
                  <li key={i} className="flex gap-2 text-zinc-300">
                    <span className="mt-2 size-1 shrink-0 rounded-full bg-zinc-500" />
                    {e}
                  </li>
                ))}
              </ul>
            </Section>
          )}

          <Section title="What this means">
            <p className="whitespace-pre-line text-zinc-400">{f.explanation.trim()}</p>
            {f.confidenceNote && (
              <p className="mt-2 rounded-md bg-zinc-800/50 px-3 py-2 text-xs text-zinc-400">
                <span className="font-medium text-zinc-300">{conf.label}: </span>
                {f.confidenceNote}
              </p>
            )}
          </Section>

          {required.length > 0 && (
            <Section title={optional.length ? "Suggested changes, in order — try one, then watch" : "Suggested fix"}>
              <FixList findingKey={f.key} fixes={required} onNavigate={onNavigate} />
            </Section>
          )}
          {optional.length > 0 && (
            <Section title="Optional, if the above does not help">
              <FixList findingKey={f.key} fixes={optional} onNavigate={onNavigate} />
            </Section>
          )}

          {f.sources.length > 0 && (
            <Section title="Sources">
              <ul className="space-y-1">
                {f.sources.map((s, i) => (
                  <li key={i} className="flex items-center gap-2 text-zinc-400">
                    <BookOpen className="size-3.5 shrink-0 text-zinc-500" />
                    {s.url ? (
                      <a href={s.url} target="_blank" rel="noreferrer" className="inline-flex items-center gap-1 text-sky-300 hover:underline">
                        {s.title}
                        <ExternalLink className="size-3" />
                      </a>
                    ) : (
                      <span>{s.title}</span>
                    )}
                    {s.kind && <span className="text-xs text-zinc-600">{s.kind}</span>}
                  </li>
                ))}
              </ul>
            </Section>
          )}

          <p className="font-mono text-[11px] text-zinc-600">
            {f.ruleId} v{f.ruleVersion} · {f.sourceFile}
            {f.deviceId && <> · {f.deviceId}</>}
          </p>
        </div>
      )}
    </article>
  );
}

function Section({ title, children }: { title: string; children: ReactNode }) {
  return (
    <div>
      <h4 className="mb-1.5 text-xs font-semibold uppercase tracking-wide text-zinc-500">{title}</h4>
      {children}
    </div>
  );
}

function vt(v: ValueText | null): string {
  if (!v) return "unknown";
  return v.label ? `${v.label} (${v.value})` : v.value;
}

function statusBadge(s: FixStatus): ReactNode {
  switch (s.state) {
    case "applicable":
      return <StatusIcon icon={Wrench} tone="info" label="Can be changed on this PC" />;
    case "alreadySet":
      return <StatusIcon icon={CircleCheck} tone="ok" label="Already set on this PC" />;
    case "unavailable":
      return <StatusIcon icon={CircleSlash} tone="muted" label="Not available on this PC" />;
    case "manual":
      return <StatusIcon icon={ListChecks} tone="muted" label="Manual steps" />;
  }
}

const pageLabel: Record<string, string> = { updates: "Updates", devices: "Devices", logs: "Logs" };

function GuidedSteps({ steps, page, onNavigate }: { steps: string[]; page: string | null; onNavigate: (page: string) => void }) {
  return (
    <div className="mt-2">
      <ol className="list-decimal space-y-0.5 pl-5 text-zinc-400">
        {steps.map((s, i) => (
          <li key={i}>{s}</li>
        ))}
      </ol>
      {page && (
        <div className="mt-2">
          <Button onClick={() => onNavigate(page)}>Open {pageLabel[page] ?? page}</Button>
        </div>
      )}
    </div>
  );
}

function FixList({
  findingKey,
  fixes,
  onNavigate,
}: {
  findingKey: string;
  fixes: FixSuggestion[];
  onNavigate: (page: string) => void;
}) {
  const [open, setOpen] = useState<string | null>(null);
  return (
    <>
    {open && (
      <FixDialog
        findingKey={findingKey}
        fixId={open}
        onClose={() => setOpen(null)}
        onOpenHistory={() => {
          setOpen(null);
          onNavigate("history");
        }}
      />
    )}
    <ol className="space-y-2">
      {fixes.map((x) => (
        <li key={x.id} className="rounded-md border border-zinc-800 bg-zinc-950/40 px-3 py-2.5">
          <div className="flex flex-wrap items-center gap-2">
            {statusBadge(x.status)}
            <span className="font-medium text-zinc-200">{x.title}</span>
            {x.risk !== "low" && <Badge tone={x.risk === "high" ? "bad" : "warn"}>{x.risk} risk</Badge>}
            {x.requiresAdmin && x.status.state === "applicable" && <StatusIcon icon={ShieldHalf} tone="muted" label="Needs administrator permission (Windows asks via UAC)" />}
          </div>
          {x.status.state === "applicable" && (
            <div className="mt-1 font-mono text-xs text-zinc-400">
              {vt(x.status.from)} <span className="text-zinc-600">→</span> <span className="text-sky-300">{vt(x.status.to)}</span>
            </div>
          )}
          {x.status.state === "alreadySet" && <div className="mt-1 font-mono text-xs text-emerald-300/80">Current: {vt(x.status.value)}</div>}
          {x.status.state === "unavailable" && <div className="mt-1 text-xs text-zinc-500">{x.status.reason}</div>}
          {x.why && <p className="mt-1.5 text-zinc-400">{x.why}</p>}
          {x.tradeoff && (
            <p className="mt-1 flex gap-1.5 text-xs text-zinc-500">
              <Lightbulb className="mt-px size-3.5 shrink-0" />
              {x.tradeoff}
            </p>
          )}
          {"guided" in x.action && <GuidedSteps {...x.action.guided} onNavigate={onNavigate} />}
          {x.status.state === "applicable" && (
            <div className="mt-2">
              <Button
                variant="primary"
                disabled={!inTauri || "guided" in x.action}
                title={inTauri ? "Shows exactly what changes; nothing is written until you confirm." : "Only in the desktop app."}
                onClick={() => setOpen(x.id)}
              >
                Preview & apply…
              </Button>
            </div>
          )}
        </li>
      ))}
    </ol>
    </>
  );
}
