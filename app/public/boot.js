// Runs before the interface. If the interface fails to start, the window
// says why instead of staying empty. React replaces #boot when it mounts.
(function () {
  // Under LANG=C the webview reports the language "C", which Intl rejects;
  // libraries that format with navigator.language (uPlot does at load) then
  // throw before the interface starts. Report a language Intl accepts.
  try {
    new Intl.NumberFormat(navigator.language);
  } catch (e) {
    try {
      Object.defineProperty(Navigator.prototype, "language", {
        configurable: true,
        get: function () {
          return "en-US";
        },
      });
      Object.defineProperty(Navigator.prototype, "languages", {
        configurable: true,
        get: function () {
          return ["en-US"];
        },
      });
    } catch (e2) {
      // Leave it; the error handler below explains a failed start.
    }
  }

  var errors = (window.__aispiceBootErrors = []);
  function report(text) {
    errors.push(String(text));
    var waiting = document.getElementById("boot");
    if (waiting) {
      waiting.setAttribute("role", "alert");
      waiting.textContent = "aispice could not start its interface: " + errors.join("; ");
    }
  }
  window.addEventListener(
    "error",
    function (e) {
      var target = e.target;
      if (target && target !== window && (target.src || target.href)) {
        report("could not load " + (target.src || target.href));
      } else {
        report(e.message || "unknown error");
      }
    },
    true,
  );
  window.addEventListener("unhandledrejection", function (e) {
    var r = e.reason;
    report(r && r.message ? r.message : r);
  });
})();
