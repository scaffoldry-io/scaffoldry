import http from "node:http";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const DIST_DIR = path.resolve(process.env.DIST_DIR || path.join(__dirname, "dist"));
const PORT = process.env.PORT || 8080;

const MIME_TYPES = {
  ".html": "text/html; charset=utf-8",
  ".js": "application/javascript; charset=utf-8",
  ".css": "text/css; charset=utf-8",
  ".json": "application/json; charset=utf-8",
  ".png": "image/png",
  ".jpg": "image/jpeg",
  ".svg": "image/svg+xml",
  ".ico": "image/x-icon",
  ".woff": "font/woff",
  ".woff2": "font/woff2",
};

// The files that may be served, listed once at startup: request path -> file on disk.
// A request only picks an entry from this list. Its text never becomes a file path.
const INDEX_PATH = path.join(DIST_DIR, "index.html");
const FILES = new Map();
for (const rel of fs.readdirSync(DIST_DIR, { recursive: true })) {
  const abs = path.join(DIST_DIR, rel);
  if (fs.statSync(abs).isFile()) {
    FILES.set("/" + rel.split(path.sep).join("/"), abs);
  }
}

const server = http.createServer((req, res) => {
  const urlPath = req.url.split("?")[0];
  // Anything that is not a listed file gets the app shell, so client-side routes work.
  const filePath = FILES.get(urlPath) ?? INDEX_PATH;

  const ext = path.extname(filePath).toLowerCase();
  const contentType = MIME_TYPES[ext] || "application/octet-stream";

  fs.readFile(filePath, (err, content) => {
    if (err) {
      res.writeHead(500, { "Content-Type": "text/plain" });
      res.end("Internal Server Error");
      return;
    }
    res.writeHead(200, { "Content-Type": contentType });
    res.end(content);
  });
});

server.listen(PORT, "0.0.0.0", () => {
  console.log(`Scaffoldry The Desk running on port ${PORT}`);
});
