import { Banner } from "./Banner";

export interface ViewAsBannerProps {
  /** The app's title, such as `Applicants`. */
  appTitle: string;
  /** The person being viewed, such as `Dr. Rivera`. */
  person: string;
  /** Ends the view. The desk revokes the token and goes back to the viewer's own session. */
  onExit: () => void;
}

/** Stays on screen for the whole of a view-as session, so no one mistakes it for their own. */
export function ViewAsBanner({ appTitle, person, onExit }: ViewAsBannerProps) {
  return (
    <Banner kind="warning" icon="👁">
      <span>
        You are viewing {appTitle} as {person}. This is read-only.{" "}
      </span>
      <button type="button" onClick={onExit} className="font-semibold underline">
        Exit
      </button>
    </Banner>
  );
}
