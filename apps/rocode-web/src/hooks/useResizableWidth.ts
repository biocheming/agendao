import { useCallback, useEffect, useRef, useState } from "react";

/**
 * Hook for a draggable resize handle that controls a panel width.
 *
 * @param initialWidth  Default width in px
 * @param minWidth      Minimum width in px
 * @param maxWidth      Maximum width in px
 * @param side          "left" or "right" — determines drag direction
 */
export function useResizableWidth(
  initialWidth: number,
  minWidth = 200,
  maxWidth = 520,
  side: "left" | "right" = "left",
) {
  const [width, setWidth] = useState(initialWidth);
  const [isDragging, setIsDragging] = useState(false);
  const isDraggingRef = useRef(false);

  const handleMouseDown = useCallback(
    (e: React.MouseEvent) => {
      e.preventDefault();
      setIsDragging(true);
      isDraggingRef.current = true;
      document.body.style.cursor = "col-resize";
      document.body.style.userSelect = "none";
    },
    [],
  );

  useEffect(() => {
    if (!isDragging) return;

    const handleMouseMove = (e: MouseEvent) => {
      const delta = side === "left" ? e.movementX : -e.movementX;
      setWidth((prev) => Math.max(minWidth, Math.min(maxWidth, prev + delta)));
    };

    const handleMouseUp = () => {
      setIsDragging(false);
      isDraggingRef.current = false;
      document.body.style.cursor = "";
      document.body.style.userSelect = "";
    };

    document.addEventListener("mousemove", handleMouseMove);
    document.addEventListener("mouseup", handleMouseUp);

    return () => {
      document.removeEventListener("mousemove", handleMouseMove);
      document.removeEventListener("mouseup", handleMouseUp);
    };
  }, [isDragging, minWidth, maxWidth, side]);

  const handleStyle = isDragging ? "bg-primary/30" : "bg-transparent hover:bg-primary/20";
  const hitArea = "w-1.5 cursor-col-resize flex-shrink-0 transition-colors";

  return { width, isDragging, handleMouseDown, handleClassName: `${hitArea} ${handleStyle}` };
}
