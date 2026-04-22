"use client";

import { cjk } from "@streamdown/cjk";
import { code } from "@streamdown/code";
import { createMathPlugin } from "@streamdown/math";
import { memo, useEffect, useMemo, useState } from "react";
import { Streamdown } from "streamdown";

const linkSafetyOff = { enabled: false } as const;
const mathPlugin = createMathPlugin({ singleDollarTextMath: true });

interface StreamdownRendererProps {
  children: string;
  className?: string;
  unsafeLinks?: boolean;
}

function mightNeedMermaid(content: string): boolean {
  return /```mermaid\b|(?:^|\n)\s*(?:graph|flowchart|sequenceDiagram|classDiagram|erDiagram|journey|gantt|pie|gitGraph|mindmap|timeline|stateDiagram|block-beta|xychart-beta|quadrantChart|requirementDiagram|C4Context|C4Container|C4Component)\b/im
    .test(content);
}

export const StreamdownRenderer = memo(
  ({ children, className, unsafeLinks = false }: StreamdownRendererProps) => {
    const [mermaidPlugin, setMermaidPlugin] = useState<unknown>(null);
    const needsMermaid = useMemo(() => mightNeedMermaid(children), [children]);

    useEffect(() => {
      let cancelled = false;

      if (!needsMermaid || mermaidPlugin) {
        return () => {
          cancelled = true;
        };
      }

      void import("@streamdown/mermaid")
        .then((module) => {
          if (!cancelled) {
            setMermaidPlugin(module.mermaid);
          }
        })
        .catch((error) => {
          console.error("Failed to load mermaid renderer:", error);
        });

      return () => {
        cancelled = true;
      };
    }, [mermaidPlugin, needsMermaid]);

    const plugins = useMemo(
      () => ({
        cjk,
        code,
        math: mathPlugin,
        ...(mermaidPlugin ? { mermaid: mermaidPlugin } : {}),
      }),
      [mermaidPlugin],
    );

    return (
      <Streamdown
        className={className}
        linkSafety={unsafeLinks ? linkSafetyOff : undefined}
        plugins={plugins}
      >
        {children}
      </Streamdown>
    );
  },
  (prevProps, nextProps) =>
    prevProps.children === nextProps.children &&
    prevProps.className === nextProps.className &&
    prevProps.unsafeLinks === nextProps.unsafeLinks,
);

StreamdownRenderer.displayName = "StreamdownRenderer";
