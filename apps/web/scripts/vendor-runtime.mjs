// Builds the three files a custom page runs on, as scripts that need no loader and no network:
// React, ReactDOM, and the Sucrase compiler. React 19 and Sucrase ship no browser build, so Vite,
// already a dev dependency, bundles each one. The output is committed under the server's ui/vendor.
import { build } from "vite";
import { statSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { resolve } from "node:path";

const here = fileURLToPath(new URL("..", import.meta.url));
const out = resolve(here, "../../crates/scaffoldry-server/src/ui/vendor");
const parts = [
  { entry: "react.js", name: "React", file: "react.js" },
  { entry: "react-dom.js", name: "ReactDOM", file: "react-dom.js", external: { react: "React" } },
  { entry: "sucrase.js", name: "Sucrase", file: "sucrase.js" },
];

for (const p of parts) {
  await build({
    configFile: false,
    publicDir: false,
    logLevel: "warn",
    define: { "process.env.NODE_ENV": '"production"' },
    build: {
      outDir: out,
      emptyOutDir: false,
      minify: true,
      lib: { entry: resolve(here, "vendor-entry", p.entry), name: p.name, formats: ["iife"], fileName: () => p.file },
      rollupOptions: { external: Object.keys(p.external ?? {}), output: { globals: p.external ?? {} } },
    },
  });
  console.log(p.file, statSync(resolve(out, p.file)).size, "bytes");
}
