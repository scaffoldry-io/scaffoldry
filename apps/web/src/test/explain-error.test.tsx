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
        "position_full",
        new ApiError(409, "Full", { code: "position_full" }),
        {
          message: "This position already has its holders.",
          next: "Replace a current holder, or raise the limit on the position type.",
        },
      ],
      [
        "too_large",
        new ApiError(413, "The result has more than 10,000 rows", { code: "too_large" }),
        { message: "The result has more than 10,000 rows", next: "Narrow the filter." },
      ],
      [
        "undo_conflict",
        new ApiError(409, "raw server text", { code: "undo_conflict" }),
        { message: "Someone changed this since. Undo stopped." },
      ],
      [
        "index_building",
        new ApiError(409, "This field is still being prepared for sorting. It is 40% done.", { code: "index_building" }),
        { message: "This field is still being prepared for sorting. It is 40% done.", next: "Try again in a few minutes." },
      ],
      [
        "field_hidden shows the rule's reason and who to ask",
        new ApiError(403, "You cannot filter or sort on 'gpa' because you cannot see it.", {
          code: "field_hidden",
          reason: "Financial details are visible to the scholarship office only.",
          remedy: "Ask the scholarship office, or an owner of this workspace.",
        }),
        { message: "Financial details are visible to the scholarship office only.", next: "Ask the scholarship office, or an owner of this workspace." },
      ],
      [
        "field_read_only without a reason falls back to the sentence and the access step",
        new ApiError(403, "You cannot change 'name'.", { code: "field_read_only" }),
        { message: "You cannot change 'name'.", next: "Ask a workspace owner or an administrator if you need access." },
      ],
      [
        "constraint_violated",
        new ApiError(400, "field 'score' violates constraint 'min'", { code: "constraint_violated" }),
        { message: "field 'score' violates constraint 'min'", next: "Check the field value and try again." },
      ],
      [
        "calculating",
        new ApiError(409, "This field is still being calculated. It is 63% done.", { code: "calculating" }),
        { message: "This field is still being calculated. It is 63% done.", next: "Try again when it has finished." },
      ],
      [
        "quota_exceeded",
        new ApiError(429, "You already have 5 jobs waiting or running. Wait for one to finish, or cancel one.", { code: "quota_exceeded" }),
        {
          message: "You already have 5 jobs waiting or running. Wait for one to finish, or cancel one.",
          next: "Wait for your background work to finish, or cancel some of it, then try again.",
        },
      ],
      [
        "too_many_indexes",
        new ApiError(400, "This table already indexes 20 fields.", { code: "too_many_indexes", unused: ["kept01", "kept02", "kept03", "kept04"] }),
        {
          message: "This table already indexes 20 fields.",
          next: "Ask an admin to stop indexing a field no view uses, such as kept01, kept02, kept03.",
        },
      ],
      [
        "grant_conflict",
        new ApiError(409, "Dean and auditor may not be held together.", { code: "grant_conflict" }),
        { message: "Dean and auditor may not be held together.", next: "Give this to someone else, or ask a Platform Admin whether the conflict should stand." },
      ],
      [
        "impersonation_not_synthetic",
        new ApiError(403, "That is a real person.", { code: "impersonation_not_synthetic" }),
        { message: "Only synthetic people can be acted as.", next: "Use view-as to see what this person sees. View-as is read-only." },
      ],
      [
        "view_as_read_only",
        new ApiError(403, "You are viewing as someone else. This view is read-only.", { code: "view_as_read_only" }),
        { message: "You are viewing as someone else. This view is read-only.", next: "Exit the view to make changes." },
      ],
      [
        "status 0",
        new ApiError(0, "Failed to fetch"),
        { message: "The server is not reachable.", next: "Check the connection and try again." },
      ],
      [
        "file_too_large",
        new ApiError(413, "That file is 60 MB. The limit is 50 MB.", { code: "file_too_large" }),
        { message: "That file is 60 MB. The limit is 50 MB.", next: "Choose a smaller file, or ask an admin to raise the file size limit." },
      ],
      [
        "file_type_blocked",
        new ApiError(400, "Programs and web pages cannot be uploaded.", { code: "file_type_blocked" }),
        { message: "Programs and web pages cannot be uploaded.", next: "Save it as a PDF, an image, or a plain document, then upload that." },
      ],
      [
        "storage_unavailable",
        new ApiError(503, "The server cannot read its database: connection refused", { code: "storage_unavailable" }),
        { message: "The server cannot reach its database right now.", next: "Try again in a minute. If it keeps happening, tell an administrator." },
      ],
      [
        "scanner_unavailable",
        new ApiError(503, "Files cannot be uploaded now. The virus scanner is not answering.", { code: "scanner_unavailable" }),
        { message: "Files cannot be uploaded right now.", next: "Try again in a few minutes, or ask an admin to check the virus scanner." },
      ],
      [
        "any other code uses the server's sentence and the body's remedy",
        new ApiError(422, "The vault is sealed", { code: "master_key_missing", remedy: "Try again later." }),
        { message: "The vault is sealed", next: "Try again later." },
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
