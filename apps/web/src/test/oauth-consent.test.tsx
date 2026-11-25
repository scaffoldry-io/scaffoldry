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

  it("an AI tool's sign-in asks the person to pick what it may do, and sends the pick", async () => {
    const AGENT = {
      ...CONSENT,
      client: { ...CONSENT.client, name: "Desk assistant", kind: "agent" },
      sentences: ["Acts as you, within what you may already do. It never has more than you."],
      choices: [
        { id: "read", sentence: "Read" },
        { id: "propose", sentence: "Read and propose changes" },
        { id: "write", sentence: "Read, propose, and write records" },
      ],
      session_hours: 12,
    };
    const posts = mock(AGENT);
    render(<OAuthConsent />);
    const read = await screen.findByRole("radio", { name: "Read" });
    expect(read).toBeChecked();
    expect(screen.getAllByRole("radio")).toHaveLength(3);
    expect(screen.getByText(/up to 12 hours/)).toBeInTheDocument();
    fireEvent.click(screen.getByRole("radio", { name: "Read and propose changes" }));
    fireEvent.click(screen.getByRole("button", { name: "Allow" }));
    await waitFor(() => expect(posts[0]?.access).toBe("propose"));
  });

  describe("toolsets", () => {
    const TOOLSETS = [
      { id: "core", sentence: "Core: find workspaces, apps and datasets.", always_on: true, selected: true },
      { id: "data", sentence: "Data: read and change records.", always_on: false, selected: true },
      { id: "build", sentence: "Build: propose changes to apps.", always_on: false, selected: false },
      { id: "governance", sentence: "Governance: read the policy library and propose rules.", always_on: false, selected: false },
    ];
    const AGENT = {
      ...CONSENT,
      client: { ...CONSENT.client, name: "Desk assistant", kind: "agent" },
      choices: [{ id: "read", sentence: "Read" }],
      session_hours: 12,
      toolsets: TOOLSETS,
    };

    it("lists every toolset the server offers with its sentence, and core is always on", async () => {
      mock(AGENT);
      render(<OAuthConsent />);
      expect(await screen.findByTestId("oauth-toolsets")).toBeInTheDocument();
      for (const t of TOOLSETS) expect(screen.getByText(t.sentence)).toBeInTheDocument();
      const core = screen.getByRole("checkbox", { name: /Core:/ });
      expect(core).toBeChecked();
      expect(core).toBeDisabled();
      expect(screen.queryByRole("checkbox", { name: /Admin/ })).not.toBeInTheDocument();
    });

    it("shows the ticks the server worked out from the OAuth scope, and the person can change them", async () => {
      const ticked = TOOLSETS.map((t) => ({ ...t, selected: t.id === "core" || t.id === "governance" }));
      const posts = mock({ ...AGENT, toolsets: ticked });
      render(<OAuthConsent />);
      expect(await screen.findByRole("checkbox", { name: /Governance:/ })).toBeChecked();
      expect(screen.getByRole("checkbox", { name: /Data:/ })).not.toBeChecked();
      fireEvent.click(screen.getByRole("checkbox", { name: /Build:/ }));
      fireEvent.click(screen.getByRole("checkbox", { name: /Governance:/ }));
      fireEvent.click(screen.getByRole("button", { name: "Allow" }));
      await waitFor(() => expect(posts[0]).toBeDefined());
      expect(posts[0].toolsets).toEqual(["build"]);
    });

    it("sends the default ticks as they stand and never sends core", async () => {
      const posts = mock(AGENT);
      render(<OAuthConsent />);
      fireEvent.click(await screen.findByRole("button", { name: "Allow" }));
      await waitFor(() => expect(posts[0]).toBeDefined());
      expect(posts[0].toolsets).toEqual(["data"]);
    });

    it("says what happens when nothing is ticked", async () => {
      mock(AGENT);
      render(<OAuthConsent />);
      expect(await screen.findByTestId("oauth-toolsets")).toHaveTextContent(/nothing is ticked.*Data/i);
    });

    it("an app that is not an AI tool shows no toolsets and sends none", async () => {
      const posts = mock(CONSENT);
      render(<OAuthConsent />);
      fireEvent.click(await screen.findByRole("button", { name: "Allow" }));
      await waitFor(() => expect(posts[0]).toBeDefined());
      expect(screen.queryByTestId("oauth-toolsets")).not.toBeInTheDocument();
      expect(posts[0].toolsets).toBeUndefined();
    });
  });

  it("an app that is not an AI tool shows no choice of access", async () => {
    const posts = mock(CONSENT);
    render(<OAuthConsent />);
    await screen.findByRole("button", { name: "Allow" });
    expect(screen.queryByRole("radio")).not.toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Allow" }));
    await waitFor(() => expect(posts[0]).toBeDefined());
    expect(posts[0].access).toBeUndefined();
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
      if (url.endsWith("/settings")) return reply({ "tokens.agent_enabled": true, "tokens.max_days": 90 });
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
