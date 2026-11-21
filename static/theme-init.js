(() => {
  const STORAGE_KEY = "typecho-theme";
  const media = window.matchMedia("(prefers-color-scheme: dark)");

  function readMode() {
    try {
      const value = localStorage.getItem(STORAGE_KEY);
      if (value === "light" || value === "dark" || value === "system") {
        return value;
      }
    } catch {
      // localStorage can be unavailable in private contexts.
    }
    return "system";
  }

  function resolveMode(mode) {
    return mode === "dark" || (mode === "system" && media.matches)
      ? "dark"
      : "light";
  }

  function apply(mode) {
    const resolved = resolveMode(mode);
    const root = document.documentElement;
    root.dataset.theme = mode;
    root.dataset.resolvedTheme = resolved;
    root.classList.toggle("dark", resolved === "dark");
    root.style.colorScheme = resolved;
    return resolved;
  }

  function setMode(mode) {
    if (mode !== "light" && mode !== "dark" && mode !== "system") {
      mode = "system";
    }
    try {
      localStorage.setItem(STORAGE_KEY, mode);
    } catch {
      // Ignore storage errors; the mode still applies for this page.
    }
    const resolved = apply(mode);
    document.dispatchEvent(
      new CustomEvent("themechange", { detail: { mode, resolved } })
    );
  }

  apply(readMode());

  media.addEventListener("change", () => {
    if (readMode() === "system") {
      const resolved = apply("system");
      document.dispatchEvent(
        new CustomEvent("themechange", {
          detail: { mode: "system", resolved },
        })
      );
    }
  });

  window.TypechoTheme = {
    get: readMode,
    set: setMode,
    resolve: resolveMode,
  };

  function updateMenuState() {
    const mode = readMode();
    document.querySelectorAll("[data-theme-set]").forEach((button) => {
      button.dataset.active = String(button.dataset.themeSet === mode);
    });
  }

  function closeMenus() {
    document.querySelectorAll(".theme-menu[open]").forEach((menu) => {
      menu.removeAttribute("open");
    });
  }

  function setupMenus() {
    document.querySelectorAll("[data-theme-set]").forEach((button) => {
      button.addEventListener("click", (event) => {
        event.preventDefault();
        setMode(button.dataset.themeSet);
        closeMenus();
      });
    });

    document.addEventListener("click", (event) => {
      document.querySelectorAll(".theme-menu[open]").forEach((menu) => {
        if (!menu.contains(event.target)) {
          menu.removeAttribute("open");
        }
      });
    });

    document.addEventListener("keydown", (event) => {
      if (event.key === "Escape") closeMenus();
    });

    updateMenuState();
  }

  document.addEventListener("themechange", updateMenuState);

  if (document.readyState === "loading") {
    document.addEventListener("DOMContentLoaded", setupMenus, { once: true });
  } else {
    setupMenus();
  }
})();
