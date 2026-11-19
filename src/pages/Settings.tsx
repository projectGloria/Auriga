import { LoaderCircle, ShieldCheck } from "lucide-react";
import { useState } from "react";
import type { HelperTestResult } from "../bindings/HelperTestResult";
import { Banner, Button, PageHeader } from "../components/ui";
import { api, errorText } from "../lib/api";
import { useScan } from "../lib/scan";

export function SettingsPage() {
  const { info } = useScan();
  const [busy, setBusy] = useState(false);
  const [result, setResult] = useState<HelperTestResult | null>(null);
  const [err, setErr] = useState<string | null>(null);

  const test = async () => {
    setBusy(true);
    setErr(null);
    setResult(null);
    try {
      setResult(await api.helperTest());
    } catch (e) {
      setErr(errorText(e));
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="flex h-full flex-col">
      <PageHeader title="Settings" subtitle={`Auriga ${info?.version ?? ""}`} />
      <div className="max-w-3xl space-y-6 px-6 py-5">
        {info?.storeWarning && <Banner tone="warn">{info.storeWarning}</Banner>}

        <section className="rounded-xl border border-zinc-800 bg-zinc-900/40 p-4">
          <h2 className="font-medium text-zinc-100">Elevated helper</h2>
          <p className="mt-1 text-sm text-zinc-400">
            Auriga runs as a normal user. Changes that need administrator rights go through a small separate helper
            started on demand with a UAC prompt. It accepts only a fixed list of typed operations and exits after 2
            minutes idle. This test starts it, sends one ping, and closes it — nothing is changed.
          </p>
          <div className="mt-3 flex items-center gap-3">
            <Button onClick={() => void test()} disabled={busy}>
              {busy ? <LoaderCircle className="size-4 animate-spin" /> : <ShieldCheck className="size-4" />}
              Test helper connection (UAC)
            </Button>
            {busy && <span className="text-sm text-zinc-400">Waiting for the UAC prompt…</span>}
          </div>
          {result && (
            <div className="mt-3">
              <Banner tone={result.elevated ? "info" : "warn"}>
                Helper PID {result.pid} answered in {result.roundTripMs.toFixed(1)} ms ·{" "}
                {result.elevated ? "running elevated" : "NOT elevated"} · exit code {result.exitCode ?? "still running"}
              </Banner>
            </div>
          )}
          {err && (
            <div className="mt-3">
              <Banner tone="bad">{err}</Banner>
            </div>
          )}
        </section>

        <section className="rounded-xl border border-zinc-800 bg-zinc-900/40 p-4 text-sm text-zinc-400">
          <h2 className="font-medium text-zinc-100">Privacy</h2>
          <p className="mt-1">
            Diagnosis is fully offline. No telemetry. Network is used only for update checks (added in a later
            phase), and only against allow-listed official domains.
          </p>
        </section>
      </div>
    </div>
  );
}
