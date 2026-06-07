import { create } from "zustand";
import type { AgendaoState } from "./types";
import { createConfigSlice } from "./config-slice";
import { createUiSlice } from "./ui-slice";
import { createSessionListSlice } from "./session-list-slice";
import { createComposerSlice } from "./composer-slice";
import { createStreamingSlice } from "./streaming-slice";
import { createTranscriptFeedSlice } from "./transcript-feed-slice";
import { createRuntimeNavigationSlice } from "./runtime-navigation-slice";
import { createWorkspaceSlice } from "./workspace-slice";

export const useAgendaoStore = create<AgendaoState>((set, get) => ({
  ...createConfigSlice(set, get),
  ...createUiSlice(set, get),
  ...createSessionListSlice(set, get),
  ...createComposerSlice(set, get),
  ...createStreamingSlice(set, get),
  ...createTranscriptFeedSlice(set, get),
  ...createRuntimeNavigationSlice(set, get),
  ...createWorkspaceSlice(set, get),
}));

export type { AgendaoState, PromptPart, StoreGet, StoreSet } from "./types";
