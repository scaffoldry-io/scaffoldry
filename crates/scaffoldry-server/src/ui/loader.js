// Runs the page. A module is compiled here, in the frame, and a bundle runs as written.
// Both run as a script element the loader adds. The frame's policy allows nothing else.
(function () {
  var root = document.getElementById("root");
  var meta = JSON.parse(document.getElementById("page-meta").textContent);
  var source = JSON.parse(document.getElementById("page-source").textContent);

  var failed = false;
  function fail(e) {
    if (failed) return; // the first error is the cause, whatever follows it is not
    failed = true;
    var panel = document.createElement("div");
    panel.className = "page-error";
    panel.textContent = "This page could not run.\n" + (e && e.message ? e.message : String(e));
    root.replaceChildren(panel);
  }
  window.addEventListener("error", function (ev) { fail(ev.error || ev.message); });
  window.addEventListener("unhandledrejection", function (ev) { fail(ev.reason); });

  function run(code) {
    var el = document.createElement("script");
    el.text = code;
    document.body.appendChild(el);
  }

  var allowed = { react: window.React, "react/jsx-runtime": window.React.jsxRuntime, scaffoldry: window.scaffoldry };
  window.__scaffoldryRequire = function (name) {
    if (Object.prototype.hasOwnProperty.call(allowed, name)) return allowed[name];
    throw new Error('A page may import "react", "react/jsx-runtime", and "scaffoldry". "' + name + '" is not available.');
  };

  try {
    if (meta.format === "bundle") return run(source);
    var out = window.Sucrase.transform(source, { transforms: ["jsx", "imports"], jsxRuntime: "automatic", production: true });
    window.__scaffoldryPage = { exports: {} };
    run("(function (require, module, exports) {\n" + out.code + "\n})(window.__scaffoldryRequire, window.__scaffoldryPage, window.__scaffoldryPage.exports);");
    var page = window.__scaffoldryPage.exports.default;
    if (typeof page !== "function") throw new Error("The page has no default export. Export a React component with export default.");
    window.ReactDOM.createRoot(root, { onUncaughtError: fail, onCaughtError: fail }).render(window.React.createElement(page));
  } catch (e) {
    fail(e);
  }
})();
