// The page's only way out. Each call posts one JSON-RPC request to the host and waits for the answer with the same id.
// The host decides. This file opens no connection of its own and keeps nothing.
(function () {
  var meta = JSON.parse(document.getElementById("page-meta").textContent);
  // No prototype, so an id such as "constructor" finds no waiting call.
  var pending = Object.create(null);
  var next = 1;
  // MCP Apps (specification 2026-01-26): a view says ui/initialize, waits for the host's answer, then says
  // ui/notifications/initialized. It sends no tools/call before that, so calls made earlier wait here.
  var ready = false;
  var failed = null;
  var waiting = [];

  window.addEventListener("message", function (ev) {
    var msg = ev.data;
    if (ev.source !== window.parent || !msg || msg.jsonrpc !== "2.0" || typeof msg.id !== "number" || !pending[msg.id]) return;
    var p = pending[msg.id];
    delete pending[msg.id];
    var result = msg.result;
    if (msg.error) return p.reject(new Error(msg.error.message || "The host refused the call"));
    if (result && result.isError) {
      var first = result.content && result.content[0];
      return p.reject(new Error((first && first.text) || "The call failed"));
    }
    p.resolve(result && result.structuredContent !== undefined ? result.structuredContent : result);
  });

  function call(op, args) {
    var body = Object.assign({ app_slug: meta.app.slug, page_id: meta.app.page_id, op: op }, args || {});
    return new Promise(function (resolve, reject) {
      function send() {
        var id = next++;
        pending[id] = { resolve: resolve, reject: reject };
        window.parent.postMessage({ jsonrpc: "2.0", id: id, method: "tools/call", params: { name: "page_call", arguments: body } }, "*");
      }
      if (failed) return reject(failed);
      if (ready) return send();
      waiting.push({ send: send, reject: reject });
    });
  }

  var initId = next++;
  pending[initId] = {
    resolve: function () {
      window.parent.postMessage({ jsonrpc: "2.0", method: "ui/notifications/initialized", params: {} }, "*");
      ready = true;
      waiting.splice(0).forEach(function (w) { w.send(); });
    },
    reject: function (e) {
      failed = e;
      waiting.splice(0).forEach(function (w) { w.reject(e); });
    },
  };
  window.parent.postMessage(
    { jsonrpc: "2.0", id: initId, method: "ui/initialize", params: { protocolVersion: "2026-01-26", appCapabilities: {}, clientInfo: { name: "scaffoldry-page", version: "1" } } },
    "*"
  );

  window.scaffoldry = {
    app: meta.app,
    grants: meta.grants,
    records: {
      list: function (a) { return call("list", a); },
      get: function (a) { return call("get", a); },
      create: function (a) { return call("create", a); },
      update: function (a) { return call("update", a); },
      remove: function (a) { return call("remove", a); },
    },
    settings: {
      get: function (key) { return call("settings_get", { key: key }); },
      set: function (key, value) { return call("settings_set", { key: key, value: value }); },
    },
  };
})();
