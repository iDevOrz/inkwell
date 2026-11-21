(() => {
  const root = document.querySelector("#vditor");
  const textarea = document.querySelector("#body-input");
  if (!root || !textarea) return;

  // If Vditor failed to load, keep the plain textarea usable.
  if (!window.Vditor) {
    textarea.hidden = false;
    return;
  }

  const csrfInput = document.querySelector('input[name="_csrf"]');
  const csrf = csrfInput ? csrfInput.value : "";
  const form = root.closest("form");
  const isDark = document.documentElement.classList.contains("dark");

  let vditor;

  try {
    vditor = new window.Vditor(root, {
      cdn: "/static/vditor",
      lang: "zh_CN",
      icon: "ant",
      theme: isDark ? "dark" : "classic",
      mode: "sv",
      height: "calc(100vh - 280px)",
      minHeight: 600,
      value: textarea.value,
      cache: { enable: false },
      outline: { enable: false, position: "left" },
      toolbarConfig: { pin: true },
      toolbar: [
        "headings",
        "bold",
        "italic",
        "strike",
        "link",
        "|",
        "list",
        "ordered-list",
        "check",
        "|",
        "quote",
        "line",
        "code",
        "inline-code",
        "|",
        "upload",
        "table",
        "|",
        "undo",
        "redo",
        "|",
        "fullscreen",
        "outline",
        "preview",
      ],
      preview: {
        url: "/admin/vditor/preview",
        delay: 300,
        mode: "both",
        maxWidth: 800,
        actions: [],
        hljs: { enable: false },
        theme: {
          current: isDark ? "dark" : "light",
          path: "/static/vditor/dist/css/content-theme",
        },
        parse(element) {
          if (typeof window.renderMermaid === "function") {
            window.renderMermaid(element);
          }
        },
      },
      upload: {
        url: "/admin/vditor/upload",
        fieldName: "file",
        multiple: true,
        max: 10 * 1024 * 1024,
        accept:
          "image/*,application/pdf,text/plain,text/markdown,application/zip",
        headers: csrf ? { "X-CSRF-Token": csrf } : {},
      },
      input() {
        if (vditor) textarea.value = vditor.getValue();
      },
      after() {
        textarea.value = vditor.getValue();
      },
    });
  } catch (error) {
    console.warn("Vditor initialization failed", error);
    textarea.hidden = false;
    return;
  }

  window.__vditor = vditor;

  document.addEventListener("themechange", (event) => {
    const dark = event.detail.resolved === "dark";
    vditor.setTheme(dark ? "dark" : "classic", dark ? "dark" : "light");
  });

  if (form) {
    form.addEventListener("submit", () => {
      textarea.value = vditor.getValue();
    });
  }
})();
