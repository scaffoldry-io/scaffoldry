// @vitest-environment node
import config from "../../vite.config";

// The web app calls /api/v1 on its own origin. In development the server is another process, so Vite must forward it.
describe("development server", () => {
  it("forwards /api to the API server", () => {
    const proxy = config.server?.proxy as Record<string, { target?: string } | string> | undefined;
    const rule = proxy?.["/api"];
    const target = typeof rule === "string" ? rule : rule?.target;
    expect(target).toBe("http://127.0.0.1:8080");
  });
});
