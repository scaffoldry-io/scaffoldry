import React from "react";
import { describe, it, expect, vi, afterEach } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import "@testing-library/jest-dom";

import { ApiError, apiClient } from "../api";
import { DataTable, ErrorState, Toast, explainError } from "../ui";

const policy = { id: "policy11", description: "Only members and central administrators can open restricted workspaces." };

describe("UX standards phase 2: errors that explain", () => {
  afterEach(() => {
    vi.restoreAllMocks();
  });

  describe("ApiError", () => {
    it("carries the code, policy, fields, and remedy from the response body", async () => {
      global.fetch = vi.fn().mockResolvedValue({
        ok: false,
        status: 403,
        statusText: "Forbidden",
        json: async () => ({
          error: "403 Forbidden: Cedar Policy restricts access to this workspace",
          code: "forbidden",
          policy,
          fields: { reason: "required" },
          remedy: "Ask the owner.",
        }),
      });
      try {
        await apiClient.getWorkspace("ws-bio-lab");
        expect.unreachable("the call must fail");
      } catch (e) {
        expect(e).toBeInstanceOf(ApiError);
        const err = e as ApiError;
        expect(err.status).toBe(403);
        expect(err.code).toBe("forbidden");
        expect(err.policy).toEqual(policy);
        expect(err.fields).toEqual({ reason: "required" });
        expect(err.remedy).toBe("Ask the owner.");
      }
    });

    it("is status 0 when the server cannot be reached", async () => {
      global.fetch = vi.fn().mockRejectedValue(new TypeError("Failed to fetch"));
      await expect(apiClient.getWorkspace("ws-bio-lab")).rejects.toMatchObject({ name: "ApiError", status: 0 });
    });

    it("still works with an older server that sends only an error sentence", async () => {
      global.fetch = vi.fn().mockResolvedValue({
        ok: false,
        status: 404,
        statusText: "Not Found",
        json: async () => ({ error: "Workspace not found" }),
      });
      await expect(apiClient.getWorkspace("x")).rejects.toMatchObject({ status: 404, message: "Workspace not found" });
    });
  });

  describe("explainError", () => {
    const cases: Array<[string, ApiError, { message: string; next?: string }]> = [
      [
        "unauthorized",
        new ApiError(401, "Unauthorized", { code: "unauthorized" }),
        { message: "Your session has ended.", next: "Sign in again." },
      ],
      [
        "forbidden with a policy",
        new ApiError(403, "403 Forbidden", { code: "forbidden", policy }),
        {
          message: policy.description,
          next: "Ask a workspace owner or an administrator if you need access.",
        },
      ],
      [
        "forbidden without a policy",
        new ApiError(403, "403 Forbidden", { code: "forbidden" }),
        {
          message: "You do not have permission to do this.",
          next: "Ask a workspace owner or an administrator if you need access.",
        },
      ],
      [
        "version_conflict",
        new ApiError(409, "Record changed", { code: "version_conflict" }),
        { message: "Someone else changed this first.", next: "Your view has been refreshed. Review it and try again." },
      ],
      [
        "no_approver",
        new ApiError(409, "No approver", { code: "no_approver" }),
        {
          message: "No one is assigned to decide this step.",
          next: "Ask an administrator to assign a position holder.",
        },
      ],
      [
        "too_large",
        new ApiError(413, "The result has more than 10,000 rows", { code: "too_large" }),
        { message: "The result has more than 10,000 rows", next: "Narrow the filter." },
      ],
      [
        "status 0",
        new ApiError(0, "Failed to fetch"),
        { message: "The server is not reachable.", next: "Check the connection and try again." },
      ],
      [
        "any other code uses the server's sentence and the body's remedy",
        new ApiError(422, "The file type is blocked", { code: "file_type_blocked", remedy: "Save it as a PDF." }),
        { message: "The file type is blocked", next: "Save it as a PDF." },
      ],
    ];

    it.each(cases)("%s", (_name, error, expected) => {
      expect(explainError(error)).toEqual(expected);
    });

    it("has no next step for another code with no remedy", () => {
      const explained = explainError(new ApiError(500, "Something broke", { code: "internal" }));
      expect(explained).toEqual({ message: "Something broke" });
    });

    it("infers the code from the status for a server that sends none", () => {
      expect(explainError(new ApiError(401, "Unauthorized")).message).toBe("Your session has ended.");
      expect(explainError(new ApiError(409, "Changed")).message).toBe("Someone else changed this first.");
    });

    it("passes a plain Error's message through", () => {
      expect(explainError(new Error("Boom"))).toEqual({ message: "Boom" });
      expect(explainError("just text")).toEqual({ message: "just text" });
    });
  });

  describe("the kit shows them", () => {
    it("a DataTable whose load rejects with a forbidden error shows the policy's description", async () => {
      const load = vi.fn().mockRejectedValue(new ApiError(403, "403 Forbidden", { code: "forbidden", policy }));
      render(<DataTable columns={[{ key: "id", header: "Id" } as any]} load={load} rowKey="id" />);
      await waitFor(() => {
        expect(screen.getByText(policy.description)).toBeInTheDocument();
      });
      expect(screen.getByText("Ask a workspace owner or an administrator if you need access.")).toBeInTheDocument();
      expect(screen.getByRole("alert")).toBeInTheDocument();
    });

    it("an ErrorState given an error explains it and keeps the code in its details", () => {
      render(<ErrorState error={new ApiError(409, "Changed", { code: "version_conflict" })} />);
      expect(screen.getByText("Someone else changed this first.")).toBeInTheDocument();
      expect(screen.getByText("Your view has been refreshed. Review it and try again.")).toBeInTheDocument();
    });

    it("an error Toast given an error shows the explanation and the next step", () => {
      render(<Toast tone="error" error={new ApiError(0, "Failed to fetch")} onClose={() => {}} />);
      const toast = screen.getByRole("alert");
      expect(toast).toHaveTextContent("The server is not reachable.");
      expect(toast).toHaveTextContent("Check the connection and try again.");
    });
  });
});
