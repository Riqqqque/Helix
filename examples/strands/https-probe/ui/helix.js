(function (root) {
  "use strict";
  var pending = Object.create(null);
  var nextId = 1;
  function call(method, params) {
    return new Promise(function (resolve, reject) {
      var id = String(nextId++);
      var timer = root.setTimeout(function () {
        if (!pending[id]) return;
        delete pending[id];
        reject(new Error("Strand host call timed out"));
      }, 30000);
      pending[id] = {
        resolve: function (value) { root.clearTimeout(timer); resolve(value); },
        reject: function (error) { root.clearTimeout(timer); reject(error); }
      };
      root.parent.postMessage({ type: "helix-strand", id: id, method: method, params: params || {} }, "*");
    });
  }
  root.addEventListener("message", function (event) {
    if (event.source !== root.parent) return;
    var msg = event.data;
    if (!msg || msg.type !== "helix-strand-result" || !pending[msg.id]) return;
    var job = pending[msg.id];
    delete pending[msg.id];
    if (msg.ok) job.resolve(msg.result);
    else job.reject(new Error(msg.error || "Strand host call failed"));
  });
  root.helix = {
    call: call,
    metrics: { snapshot: function () { return call("metrics.snapshot"); } },
    storage: {
      get: function (key) { return call("storage.get", { key: key }); },
      set: function (key, value) { return call("storage.set", { key: key, value: value }); },
      remove: function (key) { return call("storage.delete", { key: key }); },
      list: function () { return call("storage.list"); }
    },
    net: { fetch: function (request) { return call("net.fetch", request); } }
  };
})(window);
