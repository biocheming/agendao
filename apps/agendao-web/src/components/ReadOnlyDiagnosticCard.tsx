import type { ReactNode } from "react";
import { cn } from "@/lib/utils";

export type ReadOnlyDiagnosticTone = "good" | "warn" | "critical" | "neutral";

function diagnosticToneClass(tone: ReadOnlyDiagnosticTone) {
  switch (tone) {
    case "good":
      return "bg-green-500/10 text-green-700 dark:text-green-300";
    case "warn":
      return "bg-amber-500/10 text-amber-700 dark:text-amber-300";
    case "critical":
      return "bg-rose-500/10 text-rose-700 dark:text-rose-300";
    default:
      return "bg-muted text-muted-foreground";
  }
}

interface ReadOnlyDiagnosticCardProps {
  title: string;
  statusLabel?: string | null;
  statusTone?: ReadOnlyDiagnosticTone;
  badges?: string[];
  className?: string;
  children: ReactNode;
}

export function ReadOnlyDiagnosticCard({
  title,
  statusLabel = null,
  statusTone = "neutral",
  badges = [],
  className,
  children,
}: ReadOnlyDiagnosticCardProps) {
  return (
    <div className={cn("roc-rail-item grid gap-2 bg-card/45 p-4", className)}>
      <div className="flex flex-wrap items-center gap-2">
        <strong>{title}</strong>
        {statusLabel ? (
          <span
            className={cn(
              "roc-badge px-2.5 py-1 text-xs",
              diagnosticToneClass(statusTone),
            )}
          >
            {statusLabel}
          </span>
        ) : null}
        {badges.map((badge) => (
          <span key={badge} className="roc-badge px-2.5 py-1 text-xs">
            {badge}
          </span>
        ))}
      </div>
      {children}
    </div>
  );
}
