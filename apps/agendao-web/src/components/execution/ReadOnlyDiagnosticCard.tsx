import type { ReactNode } from "react";
import { cn } from "@/lib/utils";

export type ReadOnlyDiagnosticTone = "good" | "warn" | "critical" | "neutral";

function diagnosticToneClass(tone: ReadOnlyDiagnosticTone) {
  switch (tone) {
    case "good":
      return "bg-(--ds-ok)/10 text-(--ds-ok)";
    case "warn":
      return "bg-(--ds-warn)/10 text-(--ds-warn)";
    case "critical":
      return "bg-(--ds-error)/10 text-(--ds-error)";
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
