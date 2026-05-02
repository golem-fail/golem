// DOM traversal evaluated inside the WebView via CDP (Android) or the
// WebKit Inspector (iOS). Returns a JSON string of `{ tree, meta }`
// matching the Android companion's hierarchy format, so the same Rust
// normalisation path can consume both the native a11y tree and this
// enriched WebView output.
//
// One async IIFE because CDP `Runtime.evaluate` with `awaitPromise: true`
// awaits exactly one Promise — splitting into multiple expressions would
// require multiple round trips.
//
// Coordinates are CSS pixels. The Rust caller scales by `meta.dpr` when
// the platform reports device pixels (Android); iOS points already match.
//
// Deliberately NOT a pure refactor of the previous inline-minified
// version. Two functional additions live here:
//   1. BUTTON / A `textContent` fallback for Svelte 5's wrapped-text
//      buttons (see `extractText` below).
//   2. CSS `env(safe-area-inset-top)` probe used by the Rust caller to
//      cancel double-counting under `viewport-fit=cover`.
// Future changes that touch text-resolution should expect to update the
// Rust normaliser in `golem-driver/src/common.rs` in tandem.
(async function () {
  const dpr = window.devicePixelRatio || 1;
  const t0 = performance.now();
  let nodeCount = 0;

  // Map from DOM element → its emitted tree node, so the IntersectionObserver
  // pass below can reach back and stamp `visible_bounds` on the right node.
  const elMap = new Map();

  // ── Text resolution ────────────────────────────────────────────────────
  //
  // Selectors fire on what the user SEES (`text`), with `accessibility_label`
  // exposed separately for screen-reader semantics. For inputs, "what the
  // user sees" is the typed value (or the placeholder when empty); for
  // everything else it's the rendered text.
  //
  // The BUTTON/A fallback exists because Svelte 5 sometimes wraps button
  // contents in a synthetic span (`<button><span>Label</span></button>`)
  // rather than a direct text child — without the fallback, those buttons
  // come through with empty `text` and tests can't match them. The 80-char
  // cap stops us swallowing an entire subtree's textContent for buttons
  // that legitimately contain rich markup.
  function extractText(el) {
    const ariaLabel = el.getAttribute('aria-label') || '';
    const placeholder = el.placeholder || '';

    let directText = '';
    for (const child of el.childNodes) {
      if (child.nodeType === 3 && child.textContent.trim()) {
        directText = child.textContent.trim();
        break;
      }
    }
    if (!directText && (el.tagName === 'BUTTON' || el.tagName === 'A')) {
      const wrapped = (el.textContent || '').trim();
      if (wrapped && wrapped.length <= 80) directText = wrapped;
    }

    const isInput =
      el.tagName === 'INPUT' || el.tagName === 'TEXTAREA' || el.tagName === 'SELECT';
    const value =
      isInput && el.type !== 'checkbox' && el.type !== 'radio' ? el.value || '' : '';

    return value || placeholder || directText || ariaLabel;
  }

  // ── Tree construction ─────────────────────────────────────────────────
  //
  // Mirrors the Android accessibility tree's filtering rules so the merged
  // output is consistent: `display:none` and `visibility:hidden` elements
  // are excluded (a11y tree wouldn't see them either), and SCRIPT/STYLE
  // are pruned so they don't contaminate text searches.
  //
  // `contentDescription` shadows the Android companion field of the same
  // name — the Rust normaliser promotes it to `accessibility_label` when
  // the element has no explicit id.
  function buildNode(el) {
    nodeCount++;
    const r = el.getBoundingClientRect();
    const ariaLabel = el.getAttribute('aria-label') || '';

    const node = {
      class: el.tagName.toLowerCase(),
      text: extractText(el),
      contentDescription: ariaLabel || el.id || '',
      bounds: {
        left: Math.round(r.left),
        top: Math.round(r.top),
        right: Math.round(r.left + r.width),
        bottom: Math.round(r.top + r.height),
      },
      clickable:
        el.tagName === 'BUTTON' || el.tagName === 'A' || el.getAttribute('role') === 'button',
      enabled: !el.disabled,
      checked: !!el.checked,
      focused: document.activeElement === el,
      scrollable: false,
      selected: false,
      children: [],
    };
    elMap.set(el, node);

    for (const child of el.children) {
      if (child.tagName === 'SCRIPT' || child.tagName === 'STYLE') continue;
      const style = window.getComputedStyle(child);
      if (style.display === 'none' || style.visibility === 'hidden') continue;
      node.children.push(buildNode(child));
    }
    return node;
  }

  const tree = buildNode(document.body);

  // ── Visible bounds via IntersectionObserver ───────────────────────────
  //
  // `getBoundingClientRect()` ignores ancestor `overflow:hidden` clipping
  // — an element nominally at (0, 1000) inside a 400px-tall scroll
  // container reports its full rect, not the slice the user can see.
  // Tests need the visible slice (so off-screen items report zero-area
  // and don't satisfy `assert_visible`). IntersectionObserver gives us
  // exactly that: the post-clip intersection rectangle, asynchronously
  // delivered after one animation frame.
  const visRects = await new Promise((resolve) => {
    const results = new Map();
    const observer = new IntersectionObserver((entries) => {
      for (const e of entries) results.set(e.target, e.intersectionRect);
      observer.disconnect();
      resolve(results);
    });
    elMap.forEach((_, el) => observer.observe(el));
  });

  visRects.forEach((rect, el) => {
    const node = elMap.get(el);
    if (!node) return;
    node.visible_bounds = {
      left: Math.round(rect.left),
      top: Math.round(rect.top),
      right: Math.round(rect.left + rect.width),
      bottom: Math.round(rect.top + rect.height),
    };
  });

  // ── Visual viewport snapshot ──────────────────────────────────────────
  //
  // Pinch-zoom and the iOS soft keyboard shift the visual viewport
  // without moving the layout viewport — coords from
  // `getBoundingClientRect()` are layout-viewport-relative, so consumers
  // need both to map a DOM coord to a real screen coord.
  const vv = window.visualViewport;
  const visualViewport = vv
    ? { scale: vv.scale, offsetLeft: vv.offsetLeft, offsetTop: vv.offsetTop }
    : null;

  // ── Safe-area inset probe ─────────────────────────────────────────────
  //
  // Pages declared with `<meta name="viewport" content="viewport-fit=cover">`
  // extend the layout viewport behind the status bar / dynamic island, so
  // `getBoundingClientRect().top = 0` aligns with the screen top — even
  // though the visible content actually starts ~54px down. The native
  // side has already added that 54px to `webview_bounds_top` (correct
  // for the non-cover case), which would double-count under cover-mode
  // and report every element ~54px below where it actually is.
  //
  // We probe `env(safe-area-inset-top)` here so the Rust caller can
  // subtract it back out, cancelling the double-count. Non-cover pages
  // return 0 from `env()`, making the subtraction a no-op.
  let cssSafeAreaInset = { top: 0, left: 0 };
  try {
    const probe = document.createElement('div');
    probe.style.cssText =
      'position:fixed;' +
      'top:env(safe-area-inset-top,0);' +
      'left:env(safe-area-inset-left,0);' +
      'width:0;height:0;visibility:hidden;pointer-events:none';
    document.body.appendChild(probe);
    const r = probe.getBoundingClientRect();
    cssSafeAreaInset = { top: Math.round(r.top), left: Math.round(r.left) };
    document.body.removeChild(probe);
  } catch (_) {
    // Probe failed (e.g. body not yet attached) — leave inset at 0.
  }

  return JSON.stringify({
    tree,
    meta: {
      elapsed_ms: Math.round(performance.now() - t0),
      node_count: nodeCount,
      dpr,
      url: location.href,
      visualViewport,
      cssSafeAreaInset,
    },
  });
})()
