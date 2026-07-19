import { cn } from "@/lib/utils";
import { useI18n } from "../../i18n/I18nProvider";
import { compactionContinuitySourceLabel } from "../../lib/contextClosureDiagnostics";
import { ReadOnlyDiagnosticCard } from "./ReadOnlyDiagnosticCard";

interface CompactionContinuityLike {
  source: string;
  summary_text?: string | null;
  eligible_message_count?: number | null;
  exact_recent_tail_count?: number | null;
  omitted_older_turns?: number | null;
  has_working_ledger: boolean;
  has_memory_anchors: boolean;
  recall_policy?: string | null;
}

interface CompactionContinuityCardProps {
  continuity: CompactionContinuityLike;
  title?: string;
  className?: string;
}

function compactionContinuityToneClass(source: string) {
  return source === "continuity_packet" ? "good" : "warn";
}

export function CompactionContinuityCard({
  continuity,
  title,
  className,
}: CompactionContinuityCardProps) {
  const { t } = useI18n();
  return (
    <ReadOnlyDiagnosticCard
      title={title ?? t("execution.continuity.title")}
      statusLabel={compactionContinuitySourceLabel(continuity)}
      statusTone={compactionContinuityToneClass(continuity.source)}
      className={cn("grid gap-2", className)}
    >
      <p className="text-xs text-muted-foreground">
        {t("execution.continuity.tail", {
          tail: typeof continuity.exact_recent_tail_count === "number"
            ? continuity.exact_recent_tail_count
            : "--",
          omitted: typeof continuity.omitted_older_turns === "number"
            ? continuity.omitted_older_turns
            : "--",
          ledger: continuity.has_working_ledger ? t("session.yes") : t("session.no"),
          anchors: continuity.has_memory_anchors ? t("session.yes") : t("session.no"),
        })}
      </p>
      <p className="text-xs text-muted-foreground">
        {t("execution.continuity.eligible", {
          eligible: typeof continuity.eligible_message_count === "number"
            ? continuity.eligible_message_count
            : "--",
          recall: continuity.recall_policy || "--",
        })}
      </p>
      <p className="text-xs text-muted-foreground">
        {continuity.summary_text || t("execution.continuity.noSummary")}
      </p>
    </ReadOnlyDiagnosticCard>
  );
}
