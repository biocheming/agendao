# ROCode Web Visual Refactor

## Why

The original design system suffered from "AI slop" — excessive decoration that obscures hierarchy:
- 9+ shadow levels, 12 border opacity tiers, 14 bg opacity tiers, 7 border-radius values
- 14 backdrop-blur instances on nearly every surface
- 3-layer radial gradients on app shell
- translateY(-1px) hover on 4 interactive elements
- Every element competing for visual attention equally

## What Changed

Following the "restraint is power" principle from cc-design:

| Dimension | Before | After |
|---|---|---|
| roc-* classes | ~90 | ~20 |
| Shadow levels | 9+ | 2 (subtle + float) |
| Border opacity tiers | 12 | 2 (/50, /75) |
| border-radius values | 7 | 3 (8px, 12px, 20px) |
| backdrop-blur | 14 | 3 (overlay/popover/dialog only) |
| Radial gradients | 3 layers | 0 |
| hover translateY | 4 | 0 |

## Core Design Tokens

- `--roc-shadow`: subtle card shadow
- `--roc-shadow-float`: elevated surface shadow (popover, dialog)
- All surfaces are **opaque** — no backdrop-blur except overlays
- Visual hierarchy through bg color difference, not shadow/blur depth

## Preserved

- 4 oklch themes (Daylight, Sunset, Graphite, Midnight)
- IBM Plex font family
- `roc-markdown-flow` typography system (unchanged)
- All animation keyframes
- shadcn/ui components
