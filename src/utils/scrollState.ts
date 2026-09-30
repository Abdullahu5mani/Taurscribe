/**
 * Mirrors a scroll container's state onto `host` as data attributes, for CSS:
 *   data-scrolled  — scrolled away from the top (top edge fade)
 *   data-more      — more content below (bottom edge fade)
 *   data-scrolling — being scrolled right now (rows ignore the pointer, so
 *                    they don't flash their hover state passing under it)
 * Writes at most once per frame. Returns a function that stops tracking.
 */
export function trackScrollState(host: HTMLElement, scroller: HTMLElement = host): () => void {
    let frame = 0;
    let settle: ReturnType<typeof setTimeout> | undefined;

    const update = () => {
        frame = 0;
        const { scrollTop, scrollHeight, clientHeight } = scroller;
        host.toggleAttribute("data-scrolled", scrollTop > 1);
        host.toggleAttribute("data-more", scrollTop + clientHeight < scrollHeight - 1);
    };
    const schedule = () => {
        if (!frame) frame = requestAnimationFrame(update);
    };
    const onScroll = () => {
        if (!host.hasAttribute("data-scrolling")) host.setAttribute("data-scrolling", "");
        clearTimeout(settle);
        settle = setTimeout(() => host.removeAttribute("data-scrolling"), 160);
        schedule();
    };

    scroller.addEventListener("scroll", onScroll, { passive: true });
    // Content growing or shrinking (a new transcript, a view switch) changes
    // whether there is more below without any scroll.
    const resize = new ResizeObserver(schedule);
    resize.observe(scroller);
    const observeChildren = () => {
        for (const child of Array.from(scroller.children)) resize.observe(child);
    };
    observeChildren();
    const mutations = new MutationObserver(() => {
        observeChildren();
        schedule();
    });
    mutations.observe(scroller, { childList: true });
    update();

    return () => {
        scroller.removeEventListener("scroll", onScroll);
        resize.disconnect();
        mutations.disconnect();
        cancelAnimationFrame(frame);
        clearTimeout(settle);
        for (const name of ["data-scrolled", "data-more", "data-scrolling"]) host.removeAttribute(name);
    };
}
