(function () {
  "use strict";

  const RAW_ROOT = "https://raw.githubusercontent.com/fswair/vsh/main/docs/";
  const ICON = [
    '<svg viewBox="0 0 24 24" aria-hidden="true">',
    '<rect x="8" y="8" width="11" height="11" rx="2"></rect>',
    '<path d="M16 8V6a2 2 0 0 0-2-2H6a2 2 0 0 0-2 2v8a2 2 0 0 0 2 2h2"></path>',
    "</svg>",
  ].join("");

  function sourceUrl() {
    const sourceAction = document.querySelector(
      'a[href*="/raw/"][href$=".md"], a[href*="raw.githubusercontent.com"][href$=".md"]',
    );
    if (sourceAction instanceof HTMLAnchorElement) return sourceAction.href;

    const siteRoot = new URL(document.querySelector('link[rel="canonical"]')?.href || location.href);
    const configuredRoot = "/vsh/";
    let path = siteRoot.pathname;
    if (path.startsWith(configuredRoot)) path = path.slice(configuredRoot.length);
    const isDirectory = path.endsWith("/");
    path = path.replace(/^\/+|\/+$/g, "");
    if (!path) return `${RAW_ROOT}index.md`;
    if (path.endsWith(".html")) return `${RAW_ROOT}${path.slice(0, -5)}.md`;
    return `${RAW_ROOT}${path}${isDirectory ? "/index.md" : ".md"}`;
  }

  async function writeClipboard(value) {
    if (navigator.clipboard && window.isSecureContext) {
      await navigator.clipboard.writeText(value);
      return;
    }
    const field = document.createElement("textarea");
    field.value = value;
    field.setAttribute("readonly", "");
    field.style.position = "fixed";
    field.style.opacity = "0";
    document.body.append(field);
    field.select();
    const copied = document.execCommand("copy");
    field.remove();
    if (!copied) throw new Error("clipboard API is unavailable");
  }

  function setState(button, label, state) {
    const text = button.querySelector("span");
    if (text) text.textContent = label;
    button.dataset.state = state;
    window.setTimeout(() => {
      if (!button.isConnected) return;
      if (text) text.textContent = "Copy as Markdown";
      button.dataset.state = "idle";
      button.disabled = false;
    }, 1800);
  }

  async function copyPage(button) {
    button.disabled = true;
    button.dataset.state = "loading";
    try {
      const response = await fetch(sourceUrl(), { headers: { Accept: "text/plain" } });
      if (!response.ok) throw new Error(`source request failed with ${response.status}`);
      await writeClipboard(await response.text());
      setState(button, "Markdown copied", "success");
    } catch (error) {
      console.error("VSH documentation source copy failed", error);
      setState(button, "Copy failed", "error");
    }
  }

  function mount() {
    const content = document.querySelector("article.md-content__inner");
    if (!content || content.querySelector(".vsh-page-actions")) return;

    const actions = document.createElement("div");
    actions.className = "vsh-page-actions";
    actions.setAttribute("aria-label", "Page actions");

    const button = document.createElement("button");
    button.type = "button";
    button.className = "vsh-copy-markdown";
    button.dataset.state = "idle";
    button.title = "Copy this page's Markdown source";
    button.setAttribute("aria-label", "Copy this page as Markdown");
    button.innerHTML = `${ICON}<span>Copy as Markdown</span>`;
    button.addEventListener("click", () => copyPage(button));

    actions.append(button);
    content.prepend(actions);
  }

  if (typeof document$ !== "undefined" && document$?.subscribe) {
    document$.subscribe(mount);
  } else if (document.readyState === "loading") {
    document.addEventListener("DOMContentLoaded", mount, { once: true });
  } else {
    mount();
  }
})();
