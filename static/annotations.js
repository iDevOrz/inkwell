(() => {
  const root = document.querySelector("#annotator");
  const article = document.querySelector("#annotatable");
  const toolbar = document.querySelector("#annotation-toolbar");
  const form = document.querySelector("#inline-annotation-form");
  if (!root || !article || !toolbar || !form) return;

  const quoteInput = form.querySelector("#annotation-quote");
  const prefixInput = form.querySelector("#annotation-prefix");
  const suffixInput = form.querySelector("#annotation-suffix");
  const startInput = form.querySelector("#annotation-start");
  const endInput = form.querySelector("#annotation-end");
  const quotePreview = form.querySelector("#selected-quote");
  const commentInput = form.querySelector("#annotation-comment");
  const cancelButton = form.querySelector("#annotation-cancel");
  const CONTEXT = 40;
  const MAX_QUOTE = 2000;

  function hideToolbar() {
    toolbar.classList.add("is-hidden");
  }

  function hideForm() {
    form.classList.add("is-hidden");
    quoteInput.value = "";
    prefixInput.value = "";
    suffixInput.value = "";
    startInput.value = "";
    endInput.value = "";
    quotePreview.textContent = "";
    commentInput.value = "";
  }

  function isInsideForbidden(node) {
    const element = node.nodeType === Node.ELEMENT_NODE ? node : node.parentElement;
    return Boolean(element && element.closest("pre, code, .mermaid, script, style"));
  }

  function textOffset(container, offset) {
    const range = document.createRange();
    range.selectNodeContents(article);
    range.setEnd(container, offset);
    return range.toString().length;
  }

  function selectionPayload() {
    const selection = window.getSelection();
    if (!selection || selection.rangeCount === 0 || selection.isCollapsed) return null;
    const range = selection.getRangeAt(0);
    if (!article.contains(range.commonAncestorContainer)) return null;
    if (isInsideForbidden(range.startContainer) || isInsideForbidden(range.endContainer)) {
      return { forbidden: true };
    }

    // range.toString() concatenates text nodes exactly like textContent, so the quote
    // lines up with the offsets below; selection.toString() uses innerText-style
    // separators ("\n\n" between paragraphs) and would break cross-paragraph quotes.
    const quote = range.toString();
    if (!quote.trim()) return null;
    if (quote.length > MAX_QUOTE) return { tooLong: true };

    const text = article.textContent || "";
    const start = textOffset(range.startContainer, range.startOffset);
    const end = start + quote.length;
    return {
      quote,
      start,
      end,
      prefix: text.slice(Math.max(0, start - CONTEXT), start),
      suffix: text.slice(end, end + CONTEXT),
      rect: range.getBoundingClientRect(),
    };
  }

  function showToolbar(rect) {
    toolbar.classList.remove("is-hidden");
    const top = Math.min(window.innerHeight - 44, rect.bottom + 8);
    const left = Math.max(8, Math.min(window.innerWidth - 120, rect.left));
    toolbar.style.top = `${top}px`;
    toolbar.style.left = `${left}px`;
  }

  article.addEventListener("mouseup", () => {
    window.setTimeout(() => {
      const payload = selectionPayload();
      if (!payload) {
        hideToolbar();
        return;
      }
      if (payload.forbidden) {
        hideToolbar();
        return;
      }
      if (payload.tooLong) {
        hideToolbar();
        window.alert(`选中内容太长了，请控制在 ${MAX_QUOTE} 字以内。`);
        return;
      }
      toolbar.dataset.quote = payload.quote;
      toolbar.dataset.prefix = payload.prefix;
      toolbar.dataset.suffix = payload.suffix;
      toolbar.dataset.start = String(payload.start);
      toolbar.dataset.end = String(payload.end);
      showToolbar(payload.rect);
    }, 0);
  });

  toolbar.addEventListener("click", () => {
    quoteInput.value = toolbar.dataset.quote || "";
    prefixInput.value = toolbar.dataset.prefix || "";
    suffixInput.value = toolbar.dataset.suffix || "";
    startInput.value = toolbar.dataset.start || "";
    endInput.value = toolbar.dataset.end || "";
    quotePreview.textContent = quoteInput.value;
    form.classList.remove("is-hidden");
    hideToolbar();
    form.scrollIntoView({ behavior: "smooth", block: "nearest" });
    commentInput.focus();
  });

  cancelButton.addEventListener("click", hideForm);

  document.addEventListener("mousedown", (event) => {
    if (event.target.closest("#annotation-toolbar")) return;
    if (!event.target.closest(".annotatable")) hideToolbar();
  });

  function rangeFromOffsets(start, end) {
    const walker = document.createTreeWalker(article, NodeFilter.SHOW_TEXT);
    let position = 0;
    let startNode = null;
    let startOffset = 0;
    let endNode = null;
    let endOffset = 0;
    let node = walker.nextNode();
    while (node) {
      const length = node.nodeValue.length;
      if (!startNode && start <= position + length) {
        startNode = node;
        startOffset = Math.max(0, start - position);
      }
      if (!endNode && end <= position + length) {
        endNode = node;
        endOffset = Math.max(0, end - position);
        break;
      }
      position += length;
      node = walker.nextNode();
    }
    if (!startNode || !endNode) return null;
    const range = document.createRange();
    range.setStart(startNode, startOffset);
    range.setEnd(endNode, endOffset);
    return range;
  }

  function locate(item) {
    const quote = item.dataset.quote || "";
    if (!quote) return null;
    const text = article.textContent || "";
    const startHint = Number(item.dataset.start);
    if (Number.isFinite(startHint) && text.slice(startHint, startHint + quote.length) === quote) {
      return rangeFromOffsets(startHint, startHint + quote.length);
    }

    const prefix = item.dataset.prefix || "";
    const suffix = item.dataset.suffix || "";
    let index = -1;
    let searchFrom = 0;
    while (true) {
      const found = text.indexOf(quote, searchFrom);
      if (found === -1) break;
      const before = text.slice(Math.max(0, found - prefix.length), found);
      const after = text.slice(found + quote.length, found + quote.length + suffix.length);
      if ((!prefix || before.endsWith(prefix)) && (!suffix || after.startsWith(suffix))) {
        index = found;
        break;
      }
      if (index === -1) index = found;
      searchFrom = found + 1;
    }
    if (index === -1) return null;
    return rangeFromOffsets(index, index + quote.length);
  }

  const ranges = new Map();
  const items = [...document.querySelectorAll(".annotation-item[data-kind='inline']")];
  for (const item of items) {
    const range = locate(item);
    if (range) ranges.set(item.dataset.annotationId, range);
  }

  if ("Highlight" in window && "highlights" in CSS) {
    const highlight = new Highlight(...ranges.values());
    CSS.highlights.set("annotation-highlight", highlight);
  }

  for (const item of items) {
    item.addEventListener("click", (event) => {
      if (event.target.closest("form, textarea, button, a")) return;
      const range = ranges.get(item.dataset.annotationId);
      if (!range) return;
      const rect = range.getBoundingClientRect();
      window.scrollTo({ top: window.scrollY + rect.top - 140, behavior: "smooth" });
    });
  }
})();
