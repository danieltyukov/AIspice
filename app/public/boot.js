// Runs before the interface. If the interface fails to start, the window
// says why instead of staying empty. React replaces #boot when it mounts.
(function () {
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
