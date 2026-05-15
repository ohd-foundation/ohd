// Minimal theme toggle. Persists choice in localStorage; respects prefers-color-scheme on first visit.
(function () {
  var STORAGE_KEY = "ohd-theme";
  var root = document.documentElement;

  function apply(theme) {
    root.setAttribute("data-theme", theme);
  }

  function current() {
    return root.getAttribute("data-theme") || "light";
  }

  // Initial: stored preference > system preference > light default.
  try {
    var stored = localStorage.getItem(STORAGE_KEY);
    if (stored === "light" || stored === "dark") {
      apply(stored);
    } else if (window.matchMedia && window.matchMedia("(prefers-color-scheme: dark)").matches) {
      apply("dark");
    }
  } catch (_) {}

  var btn = document.getElementById("theme-toggle");
  if (!btn) return;

  btn.addEventListener("click", function () {
    var next = current() === "dark" ? "light" : "dark";
    apply(next);
    try { localStorage.setItem(STORAGE_KEY, next); } catch (_) {}
  });
})();
