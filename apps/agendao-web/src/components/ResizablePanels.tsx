"use client";

import { cn } from "@/lib/utils";
import { useCallback, useEffect, useRef, useState } from "react";

// Simple two-column resizable layout
export function SimpleResizablePanels({
  leftPanel,
  rightPanel,
  leftWidth = 280,
  minLeftWidth = 200,
  maxLeftWidth = 600,
  className,
}: {
  leftPanel: React.ReactNode;
  rightPanel: React.ReactNode;
  leftWidth?: number;
  minLeftWidth?: number;
  maxLeftWidth?: number;
  className?: string;
}) {
  const [width, setWidth] = useState(leftWidth);
  const [isDragging, setIsDragging] = useState(false);

  const handleMouseDown = useCallback((e: React.MouseEvent) => {
    e.preventDefault();
    setIsDragging(true);
    document.body.style.cursor = "col-resize";
    document.body.style.userSelect = "none";
  }, []);

  useEffect(() => {
    if (!isDragging) return;

    const handleMouseMove = (e: MouseEvent) => {
      setWidth((prev) => {
        const newWidth = prev + e.movementX;
        return Math.max(minLeftWidth, Math.min(maxLeftWidth, newWidth));
      });
    };

    const handleMouseUp = () => {
      setIsDragging(false);
      document.body.style.cursor = "";
      document.body.style.userSelect = "";
    };

    document.addEventListener("mousemove", handleMouseMove);
    document.addEventListener("mouseup", handleMouseUp);

    return () => {
      document.removeEventListener("mousemove", handleMouseMove);
      document.removeEventListener("mouseup", handleMouseUp);
    };
  }, [isDragging, minLeftWidth, maxLeftWidth]);

  return (
    <div className={cn("flex h-full w-full overflow-hidden", className)}>
      <div
        style={{ width: `${width}px`, minWidth: `${minLeftWidth}px`, maxWidth: `${maxLeftWidth}px` }}
        className="flex-shrink-0 h-full overflow-hidden"
      >
        {leftPanel}
      </div>
      <div
        className={cn(
          "w-1.5 cursor-col-resize hover:w-2 transition-all flex-shrink-0",
          isDragging ? "bg-primary/30" : "bg-border hover:bg-primary/20"
        )}
        onMouseDown={handleMouseDown}
      />
      <div className="flex-1 h-full overflow-hidden min-w-0">
        {rightPanel}
      </div>
    </div>
  );
}

// Three-column resizable layout
export function ThreeColumnResizable({
  leftPanel,
  middlePanel,
  rightPanel,
  leftWidth = 240,
  middleWidth = 400,
  minLeftWidth = 180,
  maxLeftWidth = 400,
  minMiddleWidth = 300,
  maxMiddleWidth = 800,
  className,
}: {
  leftPanel: React.ReactNode;
  middlePanel: React.ReactNode;
  rightPanel: React.ReactNode;
  leftWidth?: number;
  middleWidth?: number;
  minLeftWidth?: number;
  maxLeftWidth?: number;
  minMiddleWidth?: number;
  maxMiddleWidth?: number;
  className?: string;
}) {
  const [leftW, setLeftW] = useState(leftWidth);
  const [middleW, setMiddleW] = useState(middleWidth);
  const [draggingHandle, setDraggingHandle] = useState<"left" | "middle" | null>(null);

  const handleLeftMouseDown = useCallback((e: React.MouseEvent) => {
    e.preventDefault();
    setDraggingHandle("left");
    document.body.style.cursor = "col-resize";
    document.body.style.userSelect = "none";
  }, []);

  const handleMiddleMouseDown = useCallback((e: React.MouseEvent) => {
    e.preventDefault();
    setDraggingHandle("middle");
    document.body.style.cursor = "col-resize";
    document.body.style.userSelect = "none";
  }, []);

  useEffect(() => {
    if (!draggingHandle) return;

    const handleMouseMove = (e: MouseEvent) => {
      if (draggingHandle === "left") {
        setLeftW((prev) => {
          const newWidth = prev + e.movementX;
          return Math.max(minLeftWidth, Math.min(maxLeftWidth, newWidth));
        });
      }
      if (draggingHandle === "middle") {
        setMiddleW((prev) => {
          const newWidth = prev + e.movementX;
          return Math.max(minMiddleWidth, Math.min(maxMiddleWidth, newWidth));
        });
      }
    };

    const handleMouseUp = () => {
      setDraggingHandle(null);
      document.body.style.cursor = "";
      document.body.style.userSelect = "";
    };

    document.addEventListener("mousemove", handleMouseMove);
    document.addEventListener("mouseup", handleMouseUp);

    return () => {
      document.removeEventListener("mousemove", handleMouseMove);
      document.removeEventListener("mouseup", handleMouseUp);
    };
  }, [draggingHandle, minLeftWidth, maxLeftWidth, minMiddleWidth, maxMiddleWidth]);

  return (
    <div className={cn("flex h-full w-full overflow-hidden", className)}>
      {/* Left Panel */}
      <div
        style={{ width: `${leftW}px`, minWidth: `${minLeftWidth}px` }}
        className="flex-shrink-0 h-full overflow-hidden"
      >
        {leftPanel}
      </div>

      {/* Left Resize Handle */}
      <div
        className={cn(
          "w-1 cursor-col-resize hover:bg-primary/20 transition-colors flex-shrink-0",
          draggingHandle === "left" ? "bg-primary/30" : "bg-border"
        )}
        onMouseDown={handleLeftMouseDown}
      />

      {/* Middle Panel */}
      <div
        style={{ width: `${middleW}px`, minWidth: `${minMiddleWidth}px` }}
        className="flex-shrink-0 h-full overflow-hidden"
      >
        {middlePanel}
      </div>

      {/* Middle Resize Handle */}
      <div
        className={cn(
          "w-1 cursor-col-resize hover:bg-primary/20 transition-colors flex-shrink-0",
          draggingHandle === "middle" ? "bg-primary/30" : "bg-border"
        )}
        onMouseDown={handleMiddleMouseDown}
      />

      {/* Right Panel (flexible) */}
      <div className="flex-1 h-full overflow-hidden min-w-0">
        {rightPanel}
      </div>
    </div>
  );
}
