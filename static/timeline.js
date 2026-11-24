(() => {
  const root = document.querySelector("[data-timeline-more]");
  const list = document.querySelector(".timeline");
  if (!root || !list) return;

  const link = root.querySelector("[data-timeline-load]");
  let loading = false;
  let done = false;

  async function loadMore() {
    if (loading || done || !root.isConnected) return;
    loading = true;
    root.classList.add("is-loading");
    if (link) link.textContent = "加载中…";

    const params = new URLSearchParams({
      before_ts: root.dataset.beforeTs || "",
      before_id: root.dataset.beforeId || "",
      last_year: root.dataset.lastYear || "",
    });

    try {
      const response = await fetch(`/timeline/more?${params.toString()}`, {
        headers: { Accept: "text/html" },
      });
      // A redirect means the server answered with some other page; never inject it.
      if (response.redirected) throw new Error(`Unexpected redirect to ${response.url}`);
      if (!response.ok) throw new Error(`HTTP ${response.status}`);
      const contentType = response.headers.get("content-type") || "";
      if (!contentType.startsWith("text/html")) {
        throw new Error(`Unexpected content type: ${contentType || "none"}`);
      }

      const html = await response.text();
      if (html.trim()) list.insertAdjacentHTML("beforeend", html);

      const hasMore = response.headers.get("x-timeline-has-more") === "1";
      const nextTs = response.headers.get("x-timeline-next-ts");
      const nextId = response.headers.get("x-timeline-next-id");
      const lastYear = response.headers.get("x-timeline-last-year");

      if (hasMore && nextTs && nextId) {
        root.dataset.beforeTs = nextTs;
        root.dataset.beforeId = nextId;
        root.dataset.lastYear = lastYear || "";
      } else {
        done = true;
        root.remove();
      }
    } catch (error) {
      console.warn("Timeline load failed", error);
      if (link) link.textContent = "加载失败，点击重试";
    } finally {
      loading = false;
      if (root.isConnected) {
        root.classList.remove("is-loading");
        if (!done && link && link.textContent === "加载中…") {
          link.textContent = "加载更多";
        }
      }
    }
  }

  if (link) {
    link.addEventListener("click", (event) => {
      event.preventDefault();
      loadMore();
    });
  }

  if ("IntersectionObserver" in window) {
    const observer = new IntersectionObserver(
      (entries) => {
        if (entries.some((entry) => entry.isIntersecting)) loadMore();
      },
      { rootMargin: "800px 0px" },
    );
    observer.observe(root);
  }
})();
