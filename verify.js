(function () {
  var site = window.NameholdSite;
  var filePick = document.getElementById("file-pick");
  var shellPick = document.getElementById("shell-pick");
  var fileLink = document.getElementById("file-link");
  var files = {};

  function showFile() {
    var picked = files[filePick.value];
    document.querySelectorAll(".filename").forEach(function (el) {
      el.textContent = picked ? picked.name : "<file you downloaded>";
    });
    if (picked) fileLink.href = picked.url;
  }
  function showShell() {
    document.querySelectorAll("[data-shell]").forEach(function (el) {
      el.hidden = el.getAttribute("data-shell") !== shellPick.value;
    });
  }

  var system = site.detectSystem(navigator.userAgent);
  if (system) filePick.value = system;
  if (system === "windows") shellPick.value = "windows";
  filePick.addEventListener("change", function () {
    showFile();
    shellPick.value = filePick.value === "windows" ? "windows" : "unix";
    showShell();
  });
  shellPick.addEventListener("change", showShell);
  showFile();
  showShell();

  fetch("https://api.github.com/repos/DimazzzZ/namehold-wallet/releases/latest", {
    headers: { Accept: "application/vnd.github+json" },
  })
    .then(function (r) {
      if (!r.ok) throw new Error("HTTP " + r.status);
      return r.json();
    })
    .then(function (release) {
      files = site.pickFiles(release);
      ["sums", "sumsSig"].forEach(function (key) {
        var link = document.querySelector('[data-file="' + key + '"]');
        if (files[key]) link.href = files[key].url;
      });
      showFile();
    })
    .catch(function () {
      // The links stay on the Releases page, and the commands name no version.
    });
})();
