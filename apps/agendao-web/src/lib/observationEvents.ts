// P2-4: Unified debug observation events for the block processing pipeline.
//
// A low-cost debug hook. emitObservationEvent() accepts a lazy factory so
// that Date.now() and object construction are skipped entirely when no sink
// is registered. Tests register a collector sink, run pipeline operations,
// then assert the event sequence.
//
// BOUNDARY: This is a debug facility only. Do not instrument production
// monitoring or metrics through this channel.

// -- event types -------------------------------------------------------------

export type ObservationEventKind =
  | "block_received"
  | "block_normalized"
  | "block_routed"
  | "block_accumulated"
  | "block_committed"
  | "history_rebuilt";

export interface ObservationEvent {
  /** Wall-clock timestamp (Date.now()). */
  ts: number;
  /** Pipeline stage that emitted this event. */
  kind: ObservationEventKind;
  /** Block kind (message, reasoning, tool, status, etc.). */
  blockKind: string;
  /** Block phase (start, delta, full, end, snapshot) or undefined. */
  phase?: string;
  /** Normalized stable block ID (undefined before normalization). */
  blockId?: string;
  /** Route assigned by liveTranscriptRoute(). */
  route?: "transcript" | "non_transcript_live";
  /** For history_rebuilt: number of history messages processed. */
  historyMessageCount?: number;
}

export type ObservationSink = (event: ObservationEvent) => void;

// -- sink registration -------------------------------------------------------

let sink: ObservationSink | null = null;

export function registerObservationSink(nextSink: ObservationSink | null): void {
  sink = nextSink;
}

export function emitObservationEvent(factory: () => ObservationEvent): void {
  if (sink !== null) {
    sink(factory());
  }
}
