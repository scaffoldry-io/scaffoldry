import { Banner } from "./Banner";

export interface PreviewBannerProps {
  /** What the proposal says it does, such as `Add a dashboard page`. */
  summary: string;
  /** Ends the preview. The desk revokes the token and goes back to the person's own session. */
  onStop: () => void;
}

/** Stays on screen for the whole of a preview, so no one mistakes a proposed app for the live one. */
export function PreviewBanner({ summary, onStop }: PreviewBannerProps) {
  return (
    <Banner kind="warning" icon="👁">
      <span>
        Previewing a proposed change: {summary}. Read-only. Nothing here is live.{" "}
      </span>
      <button type="button" onClick={onStop} className="font-semibold underline">
        Stop preview
      </button>
    </Banner>
  );
}
