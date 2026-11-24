(() => {
  const onReady = (fn) => {
    if (document.readyState === "loading") {
      document.addEventListener("DOMContentLoaded", fn, { once: true });
    } else {
      fn();
    }
  };

  const SESSION_EXPIRED_MESSAGE = "登录已过期，请刷新页面后重新登录";

  onReady(() => {
    const bodyInput = document.querySelector("#body-input");
    const previewButton = document.querySelector("#preview-button");
    const previewContent = document.querySelector("#preview-content");
    const previewStatus = document.querySelector("#preview-status");
    const csrfInput = document.querySelector('input[name="_csrf"]');

    let previewTimer = null;
    let previewController = null;

    async function renderPreview() {
      if (!bodyInput || !previewContent || !csrfInput) return;
      if (previewController) previewController.abort();
      previewController = new AbortController();

      if (previewStatus) previewStatus.textContent = "渲染中…";

      try {
        const response = await fetch("/admin/preview", {
          method: "POST",
          headers: {
            "Content-Type": "application/json",
            "X-CSRF-Token": csrfInput.value,
          },
          body: JSON.stringify({ body: bodyInput.value }),
          signal: previewController.signal,
        });
        if (response.redirected || response.status === 401 || response.status === 403) {
          // An expired session redirects to /admin/login; never inject that page.
          if (previewStatus) previewStatus.textContent = SESSION_EXPIRED_MESSAGE;
          return;
        }
        if (!response.ok) {
          throw new Error((await response.text()) || `HTTP ${response.status}`);
        }
        const contentType = response.headers.get("content-type") || "";
        if (!contentType.startsWith("text/html")) {
          throw new Error(`Unexpected content type: ${contentType || "none"}`);
        }
        previewContent.innerHTML = await response.text();
        if (previewStatus) previewStatus.textContent = "已更新";
        if (typeof window.renderMermaid === "function") {
          await window.renderMermaid(previewContent);
        }
      } catch (error) {
        if (error.name === "AbortError") return;
        if (previewStatus) previewStatus.textContent = "预览失败，请稍后重试";
        console.warn("Preview failed", error);
      }
    }

    function schedulePreview() {
      if (previewTimer) window.clearTimeout(previewTimer);
      previewTimer = window.setTimeout(renderPreview, 350);
    }

    if (bodyInput) {
      bodyInput.addEventListener("input", schedulePreview);
      if (bodyInput.value.trim()) {
        renderPreview();
      }
    }

    if (previewButton) {
      previewButton.addEventListener("click", (event) => {
        event.preventDefault();
        renderPreview();
      });
    }

    const toolbar = document.querySelector(".editor-toolbar");
    if (toolbar && bodyInput) {
      toolbar.addEventListener("click", (event) => {
        const button = event.target.closest("[data-md]");
        if (!button) return;
        event.preventDefault();
        applyMarkdownCommand(bodyInput, button.getAttribute("data-md"));
        bodyInput.dispatchEvent(new Event("input", { bubbles: true }));
        bodyInput.focus();
      });
    }

    if (bodyInput) {
      bodyInput.addEventListener("keydown", (event) => {
        if (!(event.metaKey || event.ctrlKey)) return;
        const key = event.key.toLowerCase();
        if (key === "b") {
          event.preventDefault();
          applyMarkdownCommand(bodyInput, "bold");
        } else if (key === "i") {
          event.preventDefault();
          applyMarkdownCommand(bodyInput, "italic");
        } else if (key === "k") {
          event.preventDefault();
          applyMarkdownCommand(bodyInput, "link");
        } else {
          return;
        }
        bodyInput.dispatchEvent(new Event("input", { bubbles: true }));
      });
    }

    document.querySelectorAll("[data-copy]").forEach((button) => {
      button.addEventListener("click", async () => {
        const text = button.getAttribute("data-copy") || "";
        try {
          await navigator.clipboard.writeText(text);
          const original = button.textContent;
          button.textContent = "已复制";
          window.setTimeout(() => {
            button.textContent = original;
          }, 1500);
        } catch {
          window.prompt("复制链接", text);
        }
      });
    });

    document.querySelectorAll("form[data-confirm]").forEach((form) => {
      form.addEventListener("submit", (event) => {
        const message = form.getAttribute("data-confirm") || "确定执行这个操作吗？";
        if (!window.confirm(message)) {
          event.preventDefault();
        }
      });
    });
  });

  function replaceSelection(textarea, before, after, placeholder) {
    const start = textarea.selectionStart;
    const end = textarea.selectionEnd;
    const hasSelection = end > start;
    const selected = textarea.value.slice(start, end) || placeholder;
    const replacement = `${before}${selected}${after}`;
    textarea.setRangeText(replacement, start, end, "end");
    if (!hasSelection) {
      textarea.setSelectionRange(
        start + before.length,
        start + before.length + selected.length
      );
    }
  }

  function prefixLines(textarea, prefix) {
    const start = textarea.selectionStart;
    const end = textarea.selectionEnd;
    const value = textarea.value;
    const lineStart = value.lastIndexOf("\n", start - 1) + 1;
    const lineEndIndex = value.indexOf("\n", end);
    const lineEnd = lineEndIndex === -1 ? value.length : lineEndIndex;
    const block = value.slice(lineStart, lineEnd);
    const next = block
      .split("\n")
      .map((line) => (line.startsWith(prefix) ? line : `${prefix}${line}`))
      .join("\n");
    textarea.setRangeText(next, lineStart, lineEnd, "end");
  }

  function insertBlock(textarea, block) {
    const start = textarea.selectionStart;
    const end = textarea.selectionEnd;
    const value = textarea.value;
    const needsLeading = start > 0 && value[start - 1] !== "\n";
    const needsTrailing = end < value.length && value[end] !== "\n";
    const text = `${needsLeading ? "\n" : ""}${block}${needsTrailing ? "\n" : ""}`;
    textarea.setRangeText(text, start, end, "end");
  }

  function applyMarkdownCommand(textarea, command) {
    switch (command) {
      case "bold":
        replaceSelection(textarea, "**", "**", "粗体");
        break;
      case "italic":
        replaceSelection(textarea, "*", "*", "斜体");
        break;
      case "code":
        replaceSelection(textarea, "`", "`", "code");
        break;
      case "heading":
        prefixLines(textarea, "## ");
        break;
      case "link":
        replaceSelection(textarea, "[", "](https://)", "链接文字");
        break;
      case "image":
        replaceSelection(textarea, "![", "](https://)", "图片描述");
        break;
      case "quote":
        prefixLines(textarea, "> ");
        break;
      case "ul":
        prefixLines(textarea, "- ");
        break;
      case "ol":
        prefixLines(textarea, "1. ");
        break;
      case "mermaid":
        insertBlock(
          textarea,
          "```mermaid\nflowchart LR\n  A[Note] --> B[Blog]\n```"
        );
        break;
      default:
        break;
    }
  }
})();
