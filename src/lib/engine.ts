/**
 * Which engine draws the window, marked on the root for the few styles that depend on it.
 *
 * Skykeep draws in WebKitGTK on Linux and in WebView2 (Chromium) on Windows. The two differ
 * in what a live backdrop blur costs: WebView2 hands it to the GPU, WebKitGTK works it out
 * again on every frame anything under it moves, and does not always draw it. So in WebKitGTK
 * the glass is frosted instead (tokens.css, "The two tiers"). Any WebKit that is not Chromium
 * counts — the WebKit the browser checks run is WebKitGTK's sibling, and should see what the
 * app shows on Linux.
 */
export function markEngine() {
  const ua = navigator.userAgent;
  const webkitNotChromium = /AppleWebKit/.test(ua) && !/Chrome|Chromium|Edg\//.test(ua);
  if (webkitNotChromium) document.documentElement.dataset.engine = "gtk";
}
