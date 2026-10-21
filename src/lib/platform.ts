// Which system the window runs on, for the few places that differ: WebView2 says "Windows NT"
// in its user agent, WebKitGTK says "Linux".
export const onWindows = typeof navigator !== "undefined" && /Windows/.test(navigator.userAgent);
