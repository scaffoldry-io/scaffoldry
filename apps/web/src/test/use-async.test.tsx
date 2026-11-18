import "@testing-library/jest-dom";
import { describe, it, expect, vi } from "vitest";
import { render, screen, waitFor, fireEvent } from "@testing-library/react";
import { useState } from "react";
import { useAsync } from "../ui";
import { Overview } from "../admin/Overview";
import { Workspaces } from "../admin/Workspaces";
import { apiClient } from "../api";

function Keyed({ fetcher }: { fetcher: (k: string) => Promise<string> }) {
  const [key, setKey] = useState("a");
  const { data } = useAsync(() => fetcher(key), true, [key]);
  return (
    <div>
      <span data-testid="data">{data}</span>
      <button onClick={() => setKey("b")}>change</button>
    </div>
  );
}

describe("useAsync", () => {
  it("Overview fetches once, not on every render", async () => {
    const spy = vi.spyOn(apiClient, "getAdminOverview").mockRejectedValue(new Error("down"));
    render(<Overview />);
    await new Promise((r) => setTimeout(r, 100));
    expect(spy).toHaveBeenCalledTimes(1);
  });

  it("Workspaces loads units once", async () => {
    vi.spyOn(apiClient, "adminListWorkspaces").mockResolvedValue({ rows: [], next_cursor: null });
    vi.spyOn(apiClient, "adminListApps").mockResolvedValue({ rows: [], next_cursor: null });
    const spy = vi.spyOn(apiClient, "getOrgs").mockResolvedValue([] as never);
    render(<Workspaces />);
    await new Promise((r) => setTimeout(r, 100));
    expect(spy).toHaveBeenCalledTimes(1);
  });

  it("refetches once when a listed dependency changes", async () => {
    const fetcher = vi.fn(async (k: string) => `got-${k}`);
    render(<Keyed fetcher={fetcher} />);
    await waitFor(() => expect(screen.getByTestId("data")).toHaveTextContent("got-a"));
    expect(fetcher).toHaveBeenCalledTimes(1);
    fireEvent.click(screen.getByText("change"));
    await waitFor(() => expect(screen.getByTestId("data")).toHaveTextContent("got-b"));
    expect(fetcher).toHaveBeenCalledTimes(2);
  });
});
