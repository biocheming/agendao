export interface BrowserSpeechRecognitionAlternative {
  transcript: string;
}

export interface BrowserSpeechRecognitionResult {
  isFinal: boolean;
  length: number;
  item(index: number): BrowserSpeechRecognitionAlternative;
  [index: number]: BrowserSpeechRecognitionAlternative;
}

export interface BrowserSpeechRecognitionResultList {
  length: number;
  item(index: number): BrowserSpeechRecognitionResult;
  [index: number]: BrowserSpeechRecognitionResult;
}

export interface BrowserSpeechRecognitionEvent extends Event {
  resultIndex: number;
  results: BrowserSpeechRecognitionResultList;
}

export interface BrowserSpeechRecognitionErrorEvent extends Event {
  error: string;
}

export interface BrowserSpeechRecognition extends EventTarget {
  continuous: boolean;
  interimResults: boolean;
  lang: string;
  onresult: ((event: BrowserSpeechRecognitionEvent) => void) | null;
  onerror: ((event: BrowserSpeechRecognitionErrorEvent) => void) | null;
  onend: (() => void) | null;
  start(): void;
  stop(): void;
}

export interface BrowserSpeechRecognitionConstructor {
  new (): BrowserSpeechRecognition;
}

declare global {
  interface Window {
    SpeechRecognition?: BrowserSpeechRecognitionConstructor;
    webkitSpeechRecognition?: BrowserSpeechRecognitionConstructor;
  }
}

export function browserSpeechRecognitionConstructor(
  targetWindow: Window,
): BrowserSpeechRecognitionConstructor | null {
  return targetWindow.SpeechRecognition ?? targetWindow.webkitSpeechRecognition ?? null;
}
