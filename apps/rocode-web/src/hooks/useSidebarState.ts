import { useState } from "react";
import { useResizableWidth } from "./useResizableWidth";

export interface SidebarState {
  leftOpen: boolean;
  rightOpen: boolean;
  leftWidth: number;
  rightWidth: number;
  leftResize: ReturnType<typeof useResizableWidth>;
  rightResize: ReturnType<typeof useResizableWidth>;
  toggleLeft: () => void;
  toggleRight: () => void;
}

export function useSidebarState(): SidebarState {
  const [leftOpen, setLeftOpen] = useState(true);
  const [rightOpen, setRightOpen] = useState(true);
  const leftResize = useResizableWidth(312, 220, 520, "left");
  const rightResize = useResizableWidth(420, 320, 880, "right");

  return {
    leftOpen,
    rightOpen,
    leftWidth: leftResize.width,
    rightWidth: rightResize.width,
    leftResize,
    rightResize,
    toggleLeft: () => setLeftOpen((v) => !v),
    toggleRight: () => setRightOpen((v) => !v),
  };
}
