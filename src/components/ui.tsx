import { Info, TriangleAlert, X, type LucideIcon } from "lucide-react";
import type { ReactNode } from "react";
import type { Tone } from "../lib/device";

const toneClass: Record<Tone, string> = {
  ok: "bg-emerald-500/12 text-emerald-300 ring-emerald-500/25",
  warn: "bg-amber-500/12 text-amber-300 ring-amber-500/25",
  bad: "bg-rose-500/12 text-rose-300 ring-rose-500/30",
  muted: "bg-zinc-500/10 text-zinc-400 ring-zinc-500/20",
  info: "bg-sky-500/12 text-sky-300 ring-sky-500/25",
};

export function Badge({ tone, children, title }: { tone: Tone; children: ReactNode; title?: string }) {
  return (
    <span
      title={title}
      className={`inline-flex items-center gap-1 whitespace-nowrap rounded-md px-1.5 py-0.5 text-xs font-medium ring-1 ring-inset ${toneClass[tone]}`}
    >
      {children}
    </span>
  );
}

const toneText: Record<Tone, string> = {
  ok: "text-emerald-400",
  warn: "text-amber-400",
  bad: "text-rose-400",
  muted: "text-zinc-500",
  info: "text-sky-400",
};

/** Compact status: a coloured icon with its meaning in the tooltip; `text`
 * only where the detail matters (e.g. a problem code). */
export function StatusIcon({ icon: Icon, tone, label, text }: { icon: LucideIcon; tone: Tone; label: string; text?: string }) {
  return (
    <span title={label} aria-label={label} role="img" className={`inline-flex items-center gap-1 whitespace-nowrap text-xs font-medium ${toneText[tone]}`}>
      <Icon className="size-4 shrink-0" strokeWidth={2} />
      {text}
    </span>
  );
}

export function Banner({
  tone,
  children,
  onClose,
}: {
  tone: "warn" | "bad" | "info";
  children: ReactNode;
  onClose?: () => void;
}) {
  const Icon = tone === "info" ? Info : TriangleAlert;
  const cls =
    tone === "bad"
      ? "border-rose-500/30 bg-rose-500/10 text-rose-200"
      : tone === "warn"
        ? "border-amber-500/30 bg-amber-500/10 text-amber-100"
        : "border-sky-500/30 bg-sky-500/10 text-sky-100";
  return (
    <div className={`flex items-start gap-2 rounded-lg border px-3 py-2 text-sm ${cls}`} role="status">
      <Icon className="mt-0.5 size-4 shrink-0" />
      <div className="min-w-0 flex-1">{children}</div>
      {onClose && (
        <button onClick={onClose} className="rounded p-0.5 opacity-70 hover:opacity-100" aria-label="Dismiss">
          <X className="size-4" />
        </button>
      )}
    </div>
  );
}

export function Button({
  children,
  onClick,
  variant = "secondary",
  disabled,
  title,
}: {
  children: ReactNode;
  onClick?: () => void;
  variant?: "primary" | "secondary" | "danger";
  disabled?: boolean;
  title?: string;
}) {
  const cls =
    variant === "primary"
      ? "bg-sky-600 text-white hover:bg-sky-500"
      : variant === "danger"
        ? "bg-rose-600/90 text-white hover:bg-rose-500"
        : "bg-zinc-800 text-zinc-200 ring-1 ring-inset ring-zinc-700 hover:bg-zinc-700";
  return (
    <button
      onClick={onClick}
      disabled={disabled}
      title={title}
      className={`inline-flex items-center gap-1.5 rounded-md px-3 py-1.5 text-sm font-medium transition-colors disabled:cursor-not-allowed disabled:opacity-50 ${cls}`}
    >
      {children}
    </button>
  );
}

export function PageHeader({ title, subtitle, actions }: { title: string; subtitle?: ReactNode; actions?: ReactNode }) {
  return (
    <header className="flex flex-wrap items-end justify-between gap-3 border-b border-zinc-800 px-6 pb-4 pt-5">
      <div>
        <h1 className="text-lg font-semibold text-zinc-100">{title}</h1>
        {subtitle && <p className="mt-0.5 text-sm text-zinc-400">{subtitle}</p>}
      </div>
      {actions && <div className="flex items-center gap-2">{actions}</div>}
    </header>
  );
}
