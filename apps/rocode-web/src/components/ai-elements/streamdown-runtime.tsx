"use client";

import { cjk } from "@streamdown/cjk";
import type { CodeHighlighterPlugin } from "@streamdown/code";
import type { MathPlugin } from "@streamdown/math";
import { memo, useEffect, useMemo, useState } from "react";
import { Streamdown, type DiagramPlugin, type PluginConfig } from "streamdown";

const linkSafetyOff = { enabled: false } as const;

interface StreamdownRendererProps {
  children: string;
  className?: string;
  unsafeLinks?: boolean;
}

function mightNeedCode(content: string): boolean {
  return /(?:^|\n)(?:```|~~~)/.test(content);
}

function mightNeedMath(content: string): boolean {
  return /\$\$[\s\S]+?\$\$|\\\([\s\S]+?\\\)|\\\[[\s\S]+?\\\]|(^|[^\\])\$(?:[^$\n\\]|\\.)+\$/m
    .test(content);
}

function mightNeedMermaid(content: string): boolean {
  return /```mermaid\b|(?:^|\n)\s*(?:graph|flowchart|sequenceDiagram|classDiagram|erDiagram|journey|gantt|pie|gitGraph|mindmap|timeline|stateDiagram|block-beta|xychart-beta|quadrantChart|requirementDiagram|C4Context|C4Container|C4Component)\b/im
    .test(content);
}

export const StreamdownRenderer = memo(
  ({ children, className, unsafeLinks = false }: StreamdownRendererProps) => {
    const [codePlugin, setCodePlugin] = useState<CodeHighlighterPlugin | null>(null);
    const [mathPlugin, setMathPlugin] = useState<MathPlugin | null>(null);
    const [mermaidPlugin, setMermaidPlugin] = useState<DiagramPlugin | null>(null);
    const needsCode = useMemo(() => mightNeedCode(children), [children]);
    const needsMath = useMemo(() => mightNeedMath(children), [children]);
    const needsMermaid = useMemo(() => mightNeedMermaid(children), [children]);

    useEffect(() => {
      let cancelled = false;

      if (!needsCode || codePlugin) {
        return () => {
          cancelled = true;
        };
      }

      void import("../../lib/streamdownCodePlugin")
        .then((module) => {
          if (!cancelled) {
            setCodePlugin(module.rocodeCodePlugin);
          }
        })
        .catch((error) => {
          console.error("Failed to load code highlighter:", error);
        });

      return () => {
        cancelled = true;
      };
    }, [codePlugin, needsCode]);

    useEffect(() => {
      let cancelled = false;

      if (!needsMath || mathPlugin) {
        return () => {
          cancelled = true;
        };
      }

      void import("@streamdown/math")
        .then((module) => {
          if (!cancelled) {
            setMathPlugin(module.createMathPlugin({ singleDollarTextMath: true }) as MathPlugin);
          }
        })
        .catch((error) => {
          console.error("Failed to load math renderer:", error);
        });

      return () => {
        cancelled = true;
      };
    }, [mathPlugin, needsMath]);

    useEffect(() => {
      let cancelled = false;

      if (!needsMermaid || mermaidPlugin) {
        return () => {
          cancelled = true;
        };
      }

      void import("../../lib/streamdownMermaidPlugin")
        .then((module) => {
          if (!cancelled) {
            setMermaidPlugin(module.rocodeMermaidPlugin as DiagramPlugin);
          }
        })
        .catch((error) => {
          console.error("Failed to load mermaid renderer:", error);
        });

      return () => {
        cancelled = true;
      };
    }, [mermaidPlugin, needsMermaid]);

    const plugins = useMemo<PluginConfig>(
      () => ({
        cjk,
        ...(codePlugin ? { code: codePlugin } : {}),
        ...(mathPlugin ? { math: mathPlugin } : {}),
        ...(mermaidPlugin ? { mermaid: mermaidPlugin } : {}),
      }),
      [codePlugin, mathPlugin, mermaidPlugin],
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
