import { useState } from "react";
import { Ban, Braces, RefreshCw, RotateCcw, Save } from "lucide-react";
import { useI18n } from "@/i18n/I18nProvider";
import {
  DIRECT_BLUEPRINT_STARTER,
  formatBlueprintDocument,
  parseBlueprintDocument,
  type SessionBlueprintViewRecord,
} from "@/lib/blueprint";
import { useAgendaoStore } from "@/store";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Textarea } from "@/components/ui/textarea";

interface SessionBlueprintEditorProps {
  sessionId: string;
  hasBlueprint: boolean;
  apiJson: <T>(path: string, options?: RequestInit) => Promise<T>;
  onChanged: () => Promise<unknown>;
}

export function SessionBlueprintEditor({
  sessionId,
  hasBlueprint,
  apiJson,
  onChanged,
}: SessionBlueprintEditorProps) {
  const { t } = useI18n();
  const setSelectedMode = useAgendaoStore((state) => state.setSelectedMode);
  const [open, setOpen] = useState(false);
  const [loading, setLoading] = useState(false);
  const [saving, setSaving] = useState(false);
  const [view, setView] = useState<SessionBlueprintViewRecord | null>(null);
  const [document, setDocument] = useState(() => formatBlueprintDocument(DIRECT_BLUEPRINT_STARTER));
  const [error, setError] = useState<string | null>(null);

  const load = async () => {
    setError(null);
    if (!hasBlueprint) {
      setView(null);
      setDocument(formatBlueprintDocument(DIRECT_BLUEPRINT_STARTER));
      return;
    }
    setLoading(true);
    try {
      const loaded = await apiJson<SessionBlueprintViewRecord>(
        `/session/${encodeURIComponent(sessionId)}/blueprint`,
      );
      setView(loaded);
      setDocument(formatBlueprintDocument(loaded.blueprint));
    } catch (loadError) {
      setError(loadError instanceof Error ? loadError.message : t("session.unknownError"));
    } finally {
      setLoading(false);
    }
  };

  const openEditor = () => {
    setOpen(true);
    void load();
  };

  const save = async () => {
    setSaving(true);
    setError(null);
    try {
      const blueprint = parseBlueprintDocument(document);
      const saved = await apiJson<SessionBlueprintViewRecord>(
        `/session/${encodeURIComponent(sessionId)}/blueprint`,
        { method: "PUT", body: JSON.stringify({ blueprint }) },
      );
      setView(saved);
      setDocument(formatBlueprintDocument(saved.blueprint));
      setSelectedMode("scheduler:auto");
      await onChanged();
    } catch (saveError) {
      setError(saveError instanceof Error ? saveError.message : t("session.unknownError"));
    } finally {
      setSaving(false);
    }
  };

  const reject = async () => {
    setSaving(true);
    setError(null);
    try {
      await apiJson(`/session/${encodeURIComponent(sessionId)}/blueprint/reject`, {
        method: "POST",
      });
      setView(null);
      setDocument(formatBlueprintDocument(DIRECT_BLUEPRINT_STARTER));
      setOpen(false);
      await onChanged();
    } catch (rejectError) {
      setError(rejectError instanceof Error ? rejectError.message : t("session.unknownError"));
    } finally {
      setSaving(false);
    }
  };

  return (
    <>
      <Button type="button" variant="outline" size="sm" onClick={openEditor}>
        <Braces />
        {t("session.manageBlueprint")}
      </Button>
      <Dialog open={open} onOpenChange={setOpen}>
        <DialogContent className="sm:max-w-3xl" data-testid="session-blueprint-dialog">
          <DialogHeader>
            <DialogTitle>{t("session.blueprintTitle")}</DialogTitle>
            <DialogDescription>{sessionId}</DialogDescription>
          </DialogHeader>

          <div className="flex flex-wrap items-center gap-2 text-xs text-muted-foreground">
            <span>{view?.selectionSource ?? "new"}</span>
            <span className="break-all">{view?.fingerprint ?? "--"}</span>
          </div>

          <Textarea
            aria-label={t("session.blueprintDocument")}
            className="min-h-80 resize-y font-mono text-xs leading-5"
            value={document}
            onChange={(event) => setDocument(event.target.value)}
            spellCheck={false}
            disabled={loading || saving}
          />

          {view?.generatedAgents.length ? (
            <div className="grid gap-2">
              <p className="roc-section-label">{t("session.generatedAgents")}</p>
              <dl className="roc-structured-dl">
                {view.generatedAgents.map((agent) => (
                  <div className="roc-structured-row" key={agent.id}>
                    <dt className="roc-structured-key">{agent.id}</dt>
                    <dd className="text-sm text-muted-foreground">{agent.base_agent}</dd>
                  </div>
                ))}
              </dl>
            </div>
          ) : null}

          {error ? <p className="text-sm text-destructive" role="alert">{error}</p> : null}

          <DialogFooter className="flex-wrap sm:justify-between">
            <div className="flex flex-wrap gap-2">
              <Button type="button" variant="outline" onClick={() => void load()} disabled={loading || saving}>
                <RefreshCw />
                {t("session.reloadBlueprint")}
              </Button>
              <Button
                type="button"
                variant="outline"
                onClick={() => setDocument(formatBlueprintDocument(DIRECT_BLUEPRINT_STARTER))}
                disabled={loading || saving}
              >
                <RotateCcw />
                {t("session.newBlueprint")}
              </Button>
              {view?.selectionSource === "planner" ? (
                <Button type="button" variant="destructive" onClick={() => void reject()} disabled={saving}>
                  <Ban />
                  {t("session.rejectBlueprint")}
                </Button>
              ) : null}
            </div>
            <Button type="button" onClick={() => void save()} disabled={loading || saving}>
              <Save />
              {saving ? t("session.savingBlueprint") : t("session.saveBlueprint")}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </>
  );
}
