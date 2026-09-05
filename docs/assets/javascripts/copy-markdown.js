(function () {
  "use strict";

  // Fetch only on demand. Sources ship with this site, including local previews.
  let sources;

  async function pageSource(button) {
    if (!sources) {
      const controller = new AbortController();
      const timeout = window.setTimeout(() => controller.abort(), 8000);
      sources = fetch(button.dataset.sourceUrl, {
        credentials: "same-origin",
        signal: controller.signal,
      })
        .then((response) => {
          if (!response.ok) throw new Error(`Markdown request failed: ${response.status}`);
          return response.json();
        })
        .catch((error) => {
          sources = undefined;
          throw error;
        })
        .finally(() => window.clearTimeout(timeout));
    }
    const pages = await sources;
    const value = pages[button.dataset.sourceKey];
    if (typeof value !== "string") throw new Error("Markdown source is unavailable");
    return value;
  }

  async function writeClipboard(value) {
    if (navigator.clipboard && window.isSecureContext) {
      try {
        await navigator.clipboard.writeText(value);
        return;
      } catch {
        // Restricted clipboard permissions can still allow the selection fallback.
      }
    }
    const previousFocus = document.activeElement;
    const field = document.createElement("textarea");
    field.value = value;
    field.setAttribute("readonly", "");
    field.setAttribute("aria-label", "Page Markdown source");
    field.className = "vsh-clipboard-buffer";
    document.body.append(field);
    field.select();
    try {
      if (!document.execCommand("copy")) throw new Error("Clipboard is unavailable");
    } finally {
      field.remove();
      if (previousFocus instanceof HTMLElement) previousFocus.focus({ preventScroll: true });
    }
  }

  async function copyPage(button) {
    const label = button.querySelector("span");
    const status = button.parentElement.querySelector(".vsh-copy-status");
    const wasFocused = document.activeElement === button;
    button.disabled = true;
    button.dataset.state = "loading";
    button.setAttribute("aria-busy", "true");
    label.textContent = "Copying…";
    try {
      await writeClipboard(await pageSource(button));
      button.dataset.state = "success";
      label.textContent = "Markdown copied";
      status.textContent = "Page Markdown copied to clipboard.";
    } catch {
      button.dataset.state = "error";
      label.textContent = "Try copying again";
      status.textContent = "Could not copy this page. Please try again.";
    } finally {
      button.removeAttribute("aria-busy");
      button.disabled = false;
      if (wasFocused && button.isConnected && document.activeElement === document.body) {
        button.focus({ preventScroll: true });
      }
      window.clearTimeout(button.resetTimer);
      button.resetTimer = window.setTimeout(() => {
        if (!button.isConnected || button.disabled) return;
        label.textContent = "Copy as Markdown";
        button.dataset.state = "idle";
        status.textContent = "";
      }, 2400);
    }
  }

  function mount() {
    document.querySelectorAll(".vsh-copy-markdown").forEach((button) => {
      if (button.dataset.mounted) return;
      button.dataset.mounted = "true";
      button.hidden = false;
      button.addEventListener("click", () => copyPage(button));
    });
  }

  if (typeof document$ !== "undefined" && document$?.subscribe) {
    document$.subscribe(mount);
  } else if (document.readyState === "loading") {
    document.addEventListener("DOMContentLoaded", mount, { once: true });
  } else {
    mount();
  }
})();
