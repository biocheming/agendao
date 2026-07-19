"use client";

import type { CommandApiSpec } from "../../lib/command";
import { useI18n } from "@/i18n/I18nProvider";
import { cn } from "@/lib/utils";

interface SlashCommandMenuProps {
  items: CommandApiSpec[];
  activeIndex: number;
  onActiveIndexChange: (index: number) => void;
  onSelect: (command: CommandApiSpec) => void;
}

/**
 * Slash command picker anchored above the composer textarea. Keyboard
 * navigation lives in ComposerPanel (the textarea keeps focus); this menu is
 * the visual list plus pointer selection.
 */
export function SlashCommandMenu({
  items,
  activeIndex,
  onActiveIndexChange,
  onSelect,
}: SlashCommandMenuProps) {
  const { t } = useI18n();
  return (
    <div
      className="roc-floating-surface absolute inset-x-0 bottom-full z-50 mb-2 max-h-[16rem] overflow-y-auto p-2"
      data-side="top"
      data-testid="slash-command-menu"
      role="listbox"
      aria-label={t("composer.slashCommands")}
    >
      <div className="px-3 pb-1.5 pt-2 text-[10px] font-medium uppercase tracking-[0.14em] text-muted-foreground/70">
        {t("composer.commands")}
      </div>
      {items.map((command, index) => {
        const active = index === activeIndex;
        const aliases = command.aliases ?? [];
        return (
          <button
            key={command.name}
            type="button"
            role="option"
            aria-selected={active}
            data-active={active ? "true" : "false"}
            data-testid={`slash-command-item-${command.name}`}
            className={cn(
              "flex w-full flex-col gap-0.5 rounded-2xl px-3 py-2 text-left transition-colors",
              active ? "bg-accent" : "hover:bg-accent/60",
            )}
            // Select on mouse down (before blur) so the textarea keeps focus.
            onMouseDown={(event) => {
              event.preventDefault();
              onSelect(command);
            }}
            onMouseEnter={() => onActiveIndexChange(index)}
          >
            <div className="flex min-w-0 items-center gap-2">
              <span className="truncate text-sm font-medium text-foreground">
                /{command.name}
              </span>
              {aliases.length > 0 ? (
                <span className="truncate text-[11px] text-muted-foreground">
                  {aliases.map((alias) => `/${alias}`).join(" ")}
                </span>
              ) : null}
            </div>
            {command.description ? (
              <span className="truncate text-[11px] leading-[1.35] text-muted-foreground">
                {command.description}
              </span>
            ) : null}
          </button>
        );
      })}
    </div>
  );
}
