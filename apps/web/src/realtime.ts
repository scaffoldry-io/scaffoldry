export interface LiveEvent {
  event: string;
  data: any;
}

export function parseEvents(_buffer: string): { events: LiveEvent[]; rest: string } {
  return { events: [], rest: "" };
}

export function subscribe(_appSlug: string, _onEvent: (e: LiveEvent) => void, _options: { firstDelayMs?: number } = {}): () => void {
  return () => {};
}
