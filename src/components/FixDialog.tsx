import { ArrowRight, LoaderCircle, ShieldAlert, X } from "lucide-react";
import { useCallback, useEffect, useState, type ReactNode } from "react";
import type { ApplyOutcome } from "../bindings/ApplyOutcome";
import type { FixPreview } from "../bindings/FixPreview";
import type { RestorePointOutcome } from "../bindings/RestorePointOutcome";
import { api, errorText } from "../lib/api";
import { fixStateInfo, restorePointText, vt } from "../lib/fix";
import { Badge, Banner, Button } from "./ui";

type Step =
  | { s: "loading" }
  | { s: "error"; message: string }
  | { s: "preview"; p: FixPreview }
  | { s: "applying"; p: FixPreview }
  | { s: "noRestorePoint"; p: FixPreview; outcome: RestorePointOutcome }
  | { s: "result"; p: FixPreview; outcome: ApplyOutcome };

/// Preview → explicit confirm → (restore point) → apply → verify result.
export function FixDialog({
  findingKey,
  fixId,
  onClose,
  onOpenHistory,
}: {
  findingKey: string;
  fixId: string;
  onClose: () => void;
  onOpenHistory: () => void;
}) {
  const [step, setStep] = useState<Step>({ s: "loading" });
  const [restorePoint, setRestorePoint] = useState(false);

  const load = useCallback(async () => {
    setStep({ s: "loading" });
    try {
      const p = await api.previewFix(findingKey, fixId);
      if (p.preview.kind === "ready") setRestorePoint(p.preview.plan.restorePointDefault);
      setStep({ s: "preview", p });
    } catch (e) {
      setStep({ s: "error", message: errorText(e) });
    }
  }, [findingKey, fixId]);
  useEffect(() => void load(), [load]);

  const apply = async (p: FixPreview, continueWithout: boolean) => {
    if (p.preview.kind !== "ready") return;
    setStep({ s: "applying", p });
    try {
      const outcome = await api.applyFix(findingKey, fixId, p.preview.plan.from, {
        restorePoint,
        continueWithoutRestorePoint: continueWithout,
      });
      if (outcome.kind === "restorePointNotCreated") setStep({ s: "noRestorePoint", p, outcome: outcome.outcome });
      else setStep({ s: "result", p, outcome });
    } catch (e) {
      setStep({ s: "error", message: errorText(e) });
    }
  };

  const busy = step.s === "loading" || step.s === "applying";
  const title = "p" in step ? step.p.target.title : "Change a setting";

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/60 p-6" role="dialog" aria-modal="true">
      <div className="flex max-h-full w-full max-w-xl flex-col rounded-xl border border-zinc-800 bg-zinc-900 shadow-2xl">
        <header className="flex items-start gap-3 border-b border-zinc-800 px-5 py-4">
          <div className="min-w-0 flex-1">
            <h2 className="font-semibold text-zinc-100">{title}</h2>
            {"p" in step && <p className="mt-0.5 truncate text-sm text-zinc-400">{step.p.target.deviceName}</p>}
          </div>
          <button onClick={onClose} disabled={busy} className="rounded p-1 text-zinc-400 hover:text-zinc-100 disabled:opacity-40" aria-label="Close">
            <X className="size-4" />
          </button>
        </header>

        <div className="min-h-0 flex-1 space-y-3 overflow-y-auto px-5 py-4 text-sm">
          {step.s === "loading" && <Spinner>Reading the device's current settings…</Spinner>}
          {step.s === "error" && <Banner tone="bad">{step.message}</Banner>}
          {(step.s === "preview" || step.s === "applying") && <PreviewBody p={step.p} restorePoint={restorePoint} setRestorePoint={setRestorePoint} disabled={busy} />}
          {step.s === "applying" && (
            <Spinner>
              Applying… Windows asks for administrator permission the first time.
              {restorePoint && " Creating the restore point can take a minute."}
            </Spinner>
          )}
          {step.s === "noRestorePoint" && (
            <Banner tone="warn">
              <p className="font-medium">No restore point was created. Nothing has been changed.</p>
              <p className="mt-1">{restorePointText(step.outcome)}</p>
              <p className="mt-1">You can apply the change without one. The previous value is still saved in Fix History for Revert.</p>
            </Banner>
          )}
          {step.s === "result" && <ResultBody outcome={step.outcome} />}
        </div>

        <footer className="flex justify-end gap-2 border-t border-zinc-800 px-5 py-3">
          {step.s === "preview" && step.p.preview.kind === "ready" && (
            <>
              <Button onClick={onClose}>Cancel</Button>
              <Button variant="primary" onClick={() => void apply(step.p, false)}>
                <ShieldAlert className="size-4" /> Apply change
              </Button>
            </>
          )}
          {step.s === "preview" && step.p.preview.kind !== "ready" && <Button onClick={onClose}>Close</Button>}
          {step.s === "noRestorePoint" && (
            <>
              <Button onClick={onClose}>Cancel</Button>
              <Button variant="primary" onClick={() => void apply(step.p, true)}>
                Apply without restore point
              </Button>
            </>
          )}
          {step.s === "result" && (
            <>
              {step.outcome.kind === "changedSincePreview" && <Button onClick={() => void load()}>Preview again</Button>}
              {step.outcome.kind === "done" && <Button onClick={onOpenHistory}>Open Fix History</Button>}
              <Button variant="primary" onClick={onClose}>
                Close
              </Button>
            </>
          )}
          {step.s === "error" && (
            <>
              <Button onClick={() => void load()}>Try again</Button>
              <Button onClick={onClose}>Close</Button>
            </>
          )}
        </footer>
      </div>
    </div>
  );
}

function Spinner({ children }: { children: ReactNode }) {
  return (
    <p className="flex items-center gap-2 text-zinc-400">
      <LoaderCircle className="size-4 animate-spin" />
      {children}
    </p>
  );
}

function Row({ k, children }: { k: string; children: ReactNode }) {
  return (
    <div className="grid grid-cols-[8rem_1fr] gap-3">
      <dt className="text-zinc-500">{k}</dt>
      <dd className="min-w-0 text-zinc-200">{children}</dd>
    </div>
  );
}

function PreviewBody({
  p,
  restorePoint,
  setRestorePoint,
  disabled,
}: {
  p: FixPreview;
  restorePoint: boolean;
  setRestorePoint: (b: boolean) => void;
  disabled: boolean;
}) {
  const pr = p.preview;
  if (pr.kind === "refused") return <Banner tone="warn">{pr.reason}</Banner>;
  if (pr.kind === "alreadySet")
    return <Banner tone="info">This device already has {vt(pr.value)}. Nothing to change. (Scan again to refresh Findings.)</Banner>;
  const plan = pr.plan;
  return (
    <>
      <p className="text-zinc-400">Read from the device just now. Nothing is changed until you click Apply.</p>
      <dl className="space-y-1.5 rounded-lg border border-zinc-800 bg-zinc-950/50 px-4 py-3">
        <Row k="Setting">{plan.setting}</Row>
        <Row k="Change">
          <span className="font-mono text-xs">
            {vt(plan.from)} <ArrowRight className="inline size-3 text-zinc-500" /> <span className="text-sky-300">{vt(plan.to)}</span>
          </span>
        </Row>
        <Row k="Writes">
          <span className="font-mono text-xs text-zinc-400">
            {plan.kind === "rollbackDriver" ? "Driver rollback (Windows' previous driver)" : `${plan.target} = ${plan.newRaw ?? "(remove, use default)"}`}
          </span>
        </Row>
        <Row k="Risk">
          <Badge tone={p.target.risk === "low" ? "ok" : p.target.risk === "medium" ? "warn" : "bad"}>{p.target.risk}</Badge>
        </Row>
        <Row k="Undo">{plan.revertible ? "Revert from Fix History restores the previous value exactly." : "Not revertible from Auriga."}</Row>
      </dl>
      {plan.notes.length > 0 && (
        <ul className="list-disc space-y-0.5 pl-5 text-zinc-400">
          {plan.notes.map((n, i) => (
            <li key={i}>{n}</li>
          ))}
        </ul>
      )}
      <label className="flex select-none items-start gap-2 text-zinc-300">
        <input type="checkbox" className="mt-0.5 accent-sky-500" checked={restorePoint} disabled={disabled} onChange={(e) => setRestorePoint(e.target.checked)} />
        <span>
          Create a system restore point first
          <span className="block text-xs text-zinc-500">
            {plan.restorePointDefault ? "Recommended for driver changes." : "Optional for a single setting; Revert covers it."}
          </span>
        </span>
      </label>
      <p className="text-xs text-zinc-500">Needs administrator rights: Windows shows a UAC prompt for Auriga's helper (not for the whole app).</p>
    </>
  );
}

function ResultBody({ outcome }: { outcome: ApplyOutcome }) {
  if (outcome.kind === "refused") return <Banner tone="warn">{outcome.reason}</Banner>;
  if (outcome.kind === "changedSincePreview") return <Banner tone="warn">{outcome.message}</Banner>;
  if (outcome.kind === "restorePointNotCreated") return <Banner tone="warn">{restorePointText(outcome.outcome)}</Banner>;
  const r = outcome.record;
  const st = fixStateInfo[r.state];
  return (
    <div className="space-y-3">
      <div className="flex items-center gap-2">
        <Badge tone={st.tone}>{st.label}</Badge>
        <span className="text-zinc-400">{st.hint}</span>
      </div>
      {r.error && <Banner tone="bad">{r.error}</Banner>}
      {r.restorePoint && <p className="text-zinc-400">{restorePointText(r.restorePoint)}</p>}
      {r.rebootRequired && <Banner tone="warn">Restart Windows to finish this change.</Banner>}
      {r.notes.length > 0 && (
        <ul className="list-disc space-y-0.5 pl-5 text-zinc-400">
          {r.notes.map((n, i) => (
            <li key={i}>{n}</li>
          ))}
        </ul>
      )}
      {r.state === "applied" && (
        <p className="text-zinc-400">
          Now use the PC normally for a couple of days before changing anything else, then scan again. Fix History compares related
          events before and after this change.
        </p>
      )}
    </div>
  );
}
