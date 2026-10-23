import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import "@testing-library/jest-dom";
import { OAuthConsent } from "../OAuthConsent";
import { UserSettingsModal } from "../UserSettingsModal";

const CONSENT = {
  client: { client_id: "scc_1", name: "Grant tracker", operator: "Physics Department", callback_origin: "https://tracker.example.edu" },
  sentences: ["Read the title of Proposals in Grants Office.", "Create Proposals in Grants Office."],
  remembered: false,
  user: { eppn: "einstein@physics.state.edu", name: "Albert Einstein" },
};

function setAddress(search: string) {
  window.history.pushState({}, "", `/oauth/authorize${search}`);
}

describe("External apps phase 2: the consent screen", () => {
  const assign = vi.fn();
  beforeEach(() => {
    vi.clearAllMocks();
    localStorage.setItem("scaffoldry_token", "test-token");
    setAddress("?client_id=scc_1&redirect_uri=https://tracker.example.edu/callback&response_type=code&state=xyz&code_challenge=abc&code_challenge_method=S256&resource=https://api.test.edu");
    Object.defineProperty(window, "location", { configurable: true, value: { ...window.location, assign, search: window.location.search, pathname: "/oauth/authorize" } });
  });
  afterEach(() => localStorage.clear());

  function mock(consent: unknown, status = 200) {
    const posts: any[] = [];
    global.fetch = vi.fn().mockImplementation((url: string, options?: RequestInit) => {
      if (options?.method === "POST") {
        posts.push(JSON.parse(options.body as string));
        return Promise.resolve({ ok: true, status: 200, json: async () => ({ redirect_to: "https://tracker.example.edu/callback?code=c1&state=xyz" }) });
      }
      return Promise.resolve({ ok: status < 400, status, json: async () => consent });
    });
    return posts;
  }

  it("names the app, who runs it, where the person returns, and what it may use", async () => {
    mock(CONSENT);
    render(<OAuthConsent />);
    expect(await screen.findByRole("heading", { name: /Allow Grant tracker/ })).toBeInTheDocument();
    expect(screen.getByText("Physics Department")).toBeInTheDocument();
    expect(screen.getByText("https://tracker.example.edu")).toBeInTheDocument();
    expect(screen.getByText("Read the title of Proposals in Grants Office.")).toBeInTheDocument();
    expect(screen.getByText("Create Proposals in Grants Office.")).toBeInTheDocument();
  });

  it("Allow sends the request back with the answer and goes where the server says", async () => {
    const posts = mock(CONSENT);
    render(<OAuthConsent />);
    fireEvent.click(await screen.findByRole("button", { name: "Allow" }));
    await waitFor(() => expect(assign).toHaveBeenCalledWith("https://tracker.example.edu/callback?code=c1&state=xyz"));
    expect(posts[0]).toMatchObject({ client_id: "scc_1", state: "xyz", code_challenge_method: "S256", decision: "allow" });
  });

  it("Deny answers deny", async () => {
    const posts = mock(CONSENT);
    render(<OAuthConsent />);
    fireEvent.click(await screen.findByRole("button", { name: "Deny" }));
    await waitFor(() => expect(posts[0]?.decision).toBe("deny"));
  });

  it("a person who already agreed to exactly this is not asked again", async () => {
    const posts = mock({ ...CONSENT, remembered: true });
    render(<OAuthConsent />);
    await waitFor(() => expect(posts[0]?.decision).toBe("allow"));
    await waitFor(() => expect(assign).toHaveBeenCalled());
  });

  it("a request the server refuses is an error page and never a redirect", async () => {
    mock({ error: "The return address does not match one this app registered.", code: "bad_request" }, 400);
    render(<OAuthConsent />);
    expect(await screen.findByTestId("oauth-error")).toHaveTextContent("The return address does not match");
    expect(screen.queryByRole("button", { name: "Allow" })).not.toBeInTheDocument();
    expect(assign).not.toHaveBeenCalled();
  });
});

describe("External apps phase 2: connected apps", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    localStorage.setItem("scaffoldry_token", "test-token");
  });

  it("lists the apps a person agreed to and disconnects one", async () => {
    let connected = [{ client_id: "scc_1", name: "Grant tracker", operator: "Physics Department", status: "Active", sentences: ["Read the title of Proposals in Grants Office."], connected_at: "2026-10-23" }];
    const deleted: string[] = [];
    global.fetch = vi.fn().mockImplementation((url: string, options?: RequestInit) => {
      const reply = (body: unknown) => Promise.resolve({ ok: true, status: 200, json: async () => body });
      if (url.endsWith("/me/connections") ) return reply(connected);
      if (url.includes("/me/connections/") && options?.method === "DELETE") {
        deleted.push(url);
        connected = [];
        return reply({ status: "disconnected" });
      }
      if (url.endsWith("/settings")) return reply({ settings: { "tokens.agent_enabled": true, "tokens.max_days": 90 } });
      if (url.endsWith("/auth/tokens")) return reply([]);
      return reply({});
    });
    render(<UserSettingsModal isOpen onClose={() => {}} eppn="einstein@physics.state.edu" />);
    const section = await screen.findByTestId("connected-apps");
    expect(await screen.findByText("Grant tracker")).toBeInTheDocument();
    expect(section).toHaveTextContent("run by Physics Department");
    expect(section).toHaveTextContent("Read the title of Proposals in Grants Office.");
    fireEvent.click(screen.getByRole("button", { name: "Disconnect" }));
    await waitFor(() => expect(deleted).toHaveLength(1));
    expect(await screen.findByText(/No outside app is connected/)).toBeInTheDocument();
  });
});
