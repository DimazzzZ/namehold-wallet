(function (root, factory) {
  if (typeof module === "object" && module.exports) module.exports = factory();
  else root.NameholdSite = factory();
})(this, function () {
  var FILES = {
    mac: /_universal\.dmg$/,
    windows: /_x64-setup\.exe$/,
    appimage: /_amd64\.AppImage$/,
    deb: /_amd64\.deb$/,
    rpm: /\.x86_64\.rpm$/,
    sums: /^SHA256SUMS$/,
    sumsSig: /^SHA256SUMS\.sig$/,
  };
  // These names end up in commands people paste into a terminal.
  var SAFE_NAME = /^[\w.-]+$/;
  var RELEASE_FILES = "https://github.com/DimazzzZ/namehold-wallet/releases/download/";

  function detectSystem(userAgent) {
    var ua = userAgent || "";
    if (/iPhone|iPad|iPod|Android/i.test(ua)) return null;
    if (/Macintosh|Mac OS X/i.test(ua)) return "mac";
    if (/Windows/i.test(ua)) return "windows";
    if (/Linux|X11/i.test(ua)) return "appimage";
    return null;
  }

  // A wallet's download link: only ever this repository's own release files.
  function pickFiles(release) {
    var picked = {};
    var assets = release && Array.isArray(release.assets) ? release.assets : [];
    Object.keys(FILES).forEach(function (key) {
      var found = assets.filter(function (a) {
        return (
          a && typeof a.name === "string" && SAFE_NAME.test(a.name) && FILES[key].test(a.name) &&
          typeof a.browser_download_url === "string" &&
          a.browser_download_url.indexOf(RELEASE_FILES) === 0
        );
      })[0];
      if (found) picked[key] = { name: found.name, url: found.browser_download_url, size: found.size };
    });
    return picked;
  }

  function formatSize(bytes) {
    return typeof bytes === "number" ? Math.round(bytes / 1048576) + " MB" : "";
  }

  var LABELS = { mac: "macOS", windows: "Windows", appimage: "Linux" };

  function wireCopy() {
    document.querySelectorAll("[data-copy]").forEach(function (button) {
      button.addEventListener("click", function () {
        var text = document.getElementById(button.getAttribute("data-copy"));
        var done = function (label) {
          button.textContent = label;
          setTimeout(function () { button.textContent = "Copy"; }, 1800);
        };
        var select = function () {
          var range = document.createRange();
          range.selectNodeContents(text);
          var selection = window.getSelection();
          selection.removeAllRanges();
          selection.addRange(range);
          done("Press Cmd/Ctrl+C");
        };
        if (navigator.clipboard) {
          navigator.clipboard.writeText(text.textContent).then(function () { done("Copied"); }, select);
        } else {
          select();
        }
      });
    });
  }

  function start() {
    wireCopy();
    if (!document.getElementById("dl-hero")) return;
    var hero = document.getElementById("dl-hero");
    var heroMeta = document.getElementById("dl-hero-meta");
    var status = document.getElementById("dl-status");
    var system = detectSystem(navigator.userAgent);

    if (system) {
      var row = document.querySelector('[data-file="' + system + '"]');
      if (row) row.classList.add("mine");
    }

    fetch("https://api.github.com/repos/DimazzzZ/namehold-wallet/releases/latest", {
      headers: { Accept: "application/vnd.github+json" },
    })
      .then(function (r) {
        if (!r.ok) throw new Error("HTTP " + r.status);
        return r.json();
      })
      .then(function (release) {
        var files = pickFiles(release);
        Object.keys(files).forEach(function (key) {
          document.querySelectorAll('[data-file="' + key + '"]').forEach(function (el) {
            var link = el.tagName === "A" ? el : el.querySelector("a");
            if (link) link.href = files[key].url;
            var name = el.querySelector(".file-name");
            if (name) name.textContent = files[key].name;
            var size = el.querySelector(".file-size");
            if (size) size.textContent = formatSize(files[key].size);
          });
        });
        var when = new Date(release.published_at);
        var date = isNaN(when) ? "" : ", published " + when.toLocaleDateString("en", { year: "numeric", month: "short", day: "numeric" });
        var tag = String(release.tag_name || "").replace(/[^\w.-]/g, "");
        status.textContent = "Latest release: " + tag + date + ".";
        if (system && files[system]) {
          hero.href = files[system].url;
          hero.textContent = "Download for " + LABELS[system];
          heroMeta.textContent = tag + " · " + formatSize(files[system].size) + " · other systems below";
        }
      })
      .catch(function () {
        status.textContent = "Couldn't load the latest release from GitHub. These links open the Releases page, where every file is listed.";
      });

    var steps = document.querySelectorAll(".track button");
    var panel = document.getElementById("step-detail");
    function choose(index) {
      steps.forEach(function (step, i) {
        step.setAttribute("aria-pressed", String(i === index));
        step.parentNode.classList.toggle("here", i <= index);
      });
      panel.textContent = steps[index].getAttribute("data-detail");
    }
    steps.forEach(function (step, i) {
      step.addEventListener("click", function () { choose(i); });
    });
    panel.hidden = false;
    choose(0);

    var box = document.getElementById("zoom");
    var boxImage = box.querySelector("img");
    var boxCaption = box.querySelector("figcaption");
    document.querySelectorAll("a.zoom").forEach(function (link) {
      link.addEventListener("click", function (event) {
        if (!box.showModal) return;
        event.preventDefault();
        var image = link.querySelector("img");
        boxImage.src = link.getAttribute("href");
        boxImage.alt = image.alt;
        boxCaption.textContent = image.alt;
        box.showModal();
      });
    });
    box.addEventListener("click", function (event) {
      if (event.target === box || event.target.closest("button")) box.close();
    });
  }

  if (typeof document !== "undefined") {
    if (document.readyState === "loading") document.addEventListener("DOMContentLoaded", start);
    else start();
  }
  return { detectSystem: detectSystem, pickFiles: pickFiles, formatSize: formatSize };
});
