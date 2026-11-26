(() => {
  const article = document.querySelector(".reading-prose");
  const linksByHeading = new Map();
  for (const link of document.querySelectorAll(".reading-toc a[href^='#']")) {
    const id = link.getAttribute("href").slice(1);
    const heading = document.getElementById(id);
    if (!article?.contains(heading)) continue;
    if (!linksByHeading.has(heading)) linksByHeading.set(heading, []);
    linksByHeading.get(heading).push(link);
  }
  if (linksByHeading.size === 0) return;

  const mobile = document.querySelector(".reading-mobile-toc");
  if (mobile) {
    const trigger = mobile.querySelector("summary");
    document.addEventListener("click", (event) => {
      if (!mobile.open) return;
      if (!mobile.contains(event.target) || event.target.closest(".reading-toc a")) {
        mobile.open = false;
      }
    });
    document.addEventListener("keydown", (event) => {
      if (event.key !== "Escape" || !mobile.open) return;
      mobile.open = false;
      trigger.focus();
    });
  }

  let active = null;
  let frame = null;
  function update() {
    frame = null;
    // Use a line just below the top of the viewport, so a heading near the
    // bottom does not become current while the previous section is being read.
    const readingLine = Math.min(window.innerHeight * 0.25, 144);
    let current = null;
    for (const heading of linksByHeading.keys()) {
      if (heading.getBoundingClientRect().top > readingLine) break;
      current = heading;
    }
    if (current === active) return;
    for (const link of linksByHeading.get(active) ?? []) {
      link.removeAttribute("aria-current");
    }
    for (const link of linksByHeading.get(current) ?? []) {
      link.setAttribute("aria-current", "location");
    }
    active = current;
  }

  function scheduleUpdate() {
    if (frame === null) frame = window.requestAnimationFrame(update);
  }

  window.addEventListener("scroll", scheduleUpdate, { passive: true });
  window.addEventListener("resize", scheduleUpdate);
  window.addEventListener("hashchange", scheduleUpdate);
  window.addEventListener("load", scheduleUpdate, { once: true });
  scheduleUpdate();
})();
