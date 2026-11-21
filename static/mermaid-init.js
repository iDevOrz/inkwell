(() => {
  let initialized = false;
  const pending = new WeakSet();

  function captureSources(root = document) {
    root.querySelectorAll(".mermaid").forEach((node) => {
      if (!node.dataset.mermaidSource) {
        node.dataset.mermaidSource = node.textContent;
      }
    });
  }

  // Capture Markdown sources before Mermaid's own auto-render can replace them.
  captureSources();

  const lightTheme = {
    background: "#ffffff",
    primaryColor: "#ffffff",
    primaryTextColor: "#18181b",
    primaryBorderColor: "#d4d4d8",
    secondaryColor: "#f4f4f5",
    secondaryTextColor: "#18181b",
    secondaryBorderColor: "#d4d4d8",
    tertiaryColor: "#fafafa",
    tertiaryTextColor: "#18181b",
    tertiaryBorderColor: "#e4e4e7",
    mainBkg: "#ffffff",
    secondBkg: "#f4f4f5",
    tertiaryBkg: "#fafafa",
    nodeBorder: "#d4d4d8",
    nodeTextColor: "#18181b",
    lineColor: "#6366f1",
    textColor: "#18181b",
    titleColor: "#18181b",
    edgeLabelBackground: "#ffffff",
    clusterBkg: "#fafafa",
    clusterBorder: "#e4e4e7",
    actorBkg: "#eef2ff",
    actorBorder: "#c7d2fe",
    actorTextColor: "#1e1b4b",
    actorLineColor: "#c7d2fe",
    signalColor: "#4f46e5",
    signalTextColor: "#18181b",
    labelBoxBkgColor: "#eef2ff",
    labelBoxBorderColor: "#c7d2fe",
    labelTextColor: "#1e1b4b",
    loopTextColor: "#18181b",
    noteBkgColor: "#fef9c3",
    noteBorderColor: "#facc15",
    noteTextColor: "#713f12",
    activationBkgColor: "#e4e4e7",
    activationBorderColor: "#a1a1aa",
    sequenceNumberColor: "#18181b",
    classText: "#18181b",
  };

  const darkTheme = {
    background: "#18181b",
    primaryColor: "#27272a",
    primaryTextColor: "#fafafa",
    primaryBorderColor: "#52525b",
    secondaryColor: "#232326",
    secondaryTextColor: "#fafafa",
    secondaryBorderColor: "#3f3f46",
    tertiaryColor: "#1f1f22",
    tertiaryTextColor: "#e4e4e7",
    tertiaryBorderColor: "#3f3f46",
    mainBkg: "#27272a",
    secondBkg: "#232326",
    tertiaryBkg: "#1f1f22",
    nodeBorder: "#52525b",
    nodeTextColor: "#fafafa",
    lineColor: "#818cf8",
    textColor: "#e4e4e7",
    titleColor: "#fafafa",
    edgeLabelBackground: "#18181b",
    clusterBkg: "#1f1f22",
    clusterBorder: "#3f3f46",
    actorBkg: "#1e1b4b",
    actorBorder: "#4338ca",
    actorTextColor: "#e0e7ff",
    actorLineColor: "#4338ca",
    signalColor: "#a5b4fc",
    signalTextColor: "#f8fafc",
    labelBoxBkgColor: "#1e1b4b",
    labelBoxBorderColor: "#4338ca",
    labelTextColor: "#e0e7ff",
    loopTextColor: "#fafafa",
    noteBkgColor: "#422006",
    noteBorderColor: "#a16207",
    noteTextColor: "#fef3c7",
    activationBkgColor: "#3f3f46",
    activationBorderColor: "#71717a",
    sequenceNumberColor: "#fafafa",
    classText: "#fafafa",
  };

  function isDark() {
    return document.documentElement.classList.contains("dark");
  }

  const diagramFont =
    'ui-sans-serif, system-ui, -apple-system, "PingFang SC", "Hiragino Sans GB", "Microsoft YaHei", sans-serif';

  function initialize() {
    if (initialized || !window.mermaid) return;
    const dark = isDark();
    window.mermaid.initialize({
      startOnLoad: false,
      securityLevel: "strict",
      theme: "base",
      darkMode: dark,
      fontFamily: diagramFont,
      themeVariables: dark ? darkTheme : lightTheme,
      sequence: {
        useMaxWidth: false,
        mirrorActors: false,
        wrap: true,
        diagramMarginX: 24,
        diagramMarginY: 16,
        actorMargin: 48,
        boxMargin: 10,
        noteMargin: 10,
        messageMargin: 32,
        activationWidth: 8,
        actorFontFamily: diagramFont,
        messageFontFamily: diagramFont,
        noteFontFamily: diagramFont,
        actorFontSize: 14,
        messageFontSize: 14,
        noteFontSize: 13,
        actorFontWeight: 600,
        messageFontWeight: 500,
      },
      class: {
        useMaxWidth: false,
        htmlLabels: true,
      },
      flowchart: {
        useMaxWidth: false,
        curve: "basis",
      },
    });
    initialized = true;

    // If Mermaid auto-rendered before our config was applied, restore the
    // source and render again with the configured theme.
    document.querySelectorAll(".mermaid").forEach((node) => {
      if (node.dataset.mermaidSource) {
        node.textContent = node.dataset.mermaidSource;
        delete node.dataset.processed;
      }
    });
  }

  async function renderMermaid(root = document, attempt = 0) {
    if (!window.mermaid) return;
    initialize();

    const nodes = Array.from(root.querySelectorAll(".mermaid")).filter(
      (node) => !node.dataset.processed && !pending.has(node),
    );
    if (!nodes.length) return;

    for (const node of nodes) {
      if (!node.dataset.mermaidSource) {
        node.dataset.mermaidSource = node.textContent;
      }
      pending.add(node);
    }

    try {
      await Promise.race([
        window.mermaid.run({ nodes, suppressErrors: true }),
        new Promise((_, reject) =>
          window.setTimeout(() => reject(new Error("Mermaid render timed out")), 5000),
        ),
      ]);
    } catch (error) {
      console.warn("Mermaid render failed", error);
      if (attempt < 2) {
        window.setTimeout(() => renderMermaid(root, attempt + 1), 400);
      }
    } finally {
      for (const node of nodes) pending.delete(node);
    }
  }

  function rerenderForTheme() {
    captureSources();
    document.querySelectorAll(".mermaid").forEach((node) => {
      if (node.dataset.mermaidSource) {
        node.textContent = node.dataset.mermaidSource;
        delete node.dataset.processed;
      }
    });
    initialized = false;
    renderMermaid(document, 0);
  }

  window.renderMermaid = (root = document) => renderMermaid(root, 0);
  document.addEventListener("themechange", rerenderForTheme);

  const start = () => renderMermaid(document, 0);
  if (document.readyState === "complete") {
    window.setTimeout(start, 0);
  } else {
    window.addEventListener("load", start, { once: true });
  }
})();
