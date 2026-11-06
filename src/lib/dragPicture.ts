/**
 * What a drag shows under the pointer: a small picture of what is carried — a row's name, as
 * the Mac shows it, or a photo — never the whole row.
 *
 * Left to itself the engine draws the dragged element, and a reminder's, a contact's or a
 * file's row is as wide as its pane. WebKitGTK (GTK 3) also hands GTK that picture as it made
 * it, at the screen's scale, without saying which, and GTK draws each of its pixels as a whole
 * point: on a screen at 2× a 664-pixel reminder row was dragged as a 1328 × 110 picture and a
 * file's row as 1852 × 78, held where it was grabbed and running off the window's edge.
 * Measured with a drag begun in WebKitGTK 2.54 itself and its drag icon read back from GTK.
 *
 * So the picture here is small, and in WebKitGTK it is drawn at one over the screen's scale,
 * which that doubling brings back to its own size: a 240 × 32 label at 2× arrives 240 × 32.
 * Chromium (WebView2) scales a drag picture itself, and on Windows a drag inside the page is
 * carried by lib/drag with no picture at all, so there it is left as it is.
 */
export function carryPicture(e: { dataTransfer: DataTransfer | null }, picture: HTMLElement, x: number, y: number) {
  const data = e.dataTransfer;
  if (!data || typeof data.setDragImage !== "function") return;
  if (document.documentElement.dataset.engine === "gtk" && window.devicePixelRatio > 1) {
    // Scaled as it is painted, not laid out smaller (`zoom`): smaller, its words met WebKit's
    // least font size and came out too large for their capsule.
    picture.style.transformOrigin = "0 0";
    picture.style.transform = `scale(${1 / window.devicePixelRatio})`;
  }
  // Drawn off-screen for the moment the engine takes its picture, then gone.
  picture.classList.add("drag-picture");
  document.body.appendChild(picture);
  data.setDragImage(picture, x, y);
  window.setTimeout(() => picture.remove(), 0);
}

/** A row's name in a small capsule, with how many go with it when it is more than one. */
export function carryLabel(e: { dataTransfer: DataTransfer | null }, text: string, count = 1) {
  const label = document.createElement("div");
  label.className = "drag-label";
  const name = document.createElement("span");
  name.textContent = text.trim() || "—";
  label.appendChild(name);
  if (count > 1) {
    const n = document.createElement("b");
    n.textContent = String(count);
    label.appendChild(n);
  }
  // Beside the pointer's tip rather than under the arrow, which covered the name's first letters.
  carryPicture(e, label, -16, -4);
}
