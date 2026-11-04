import { onWindows } from "./platform";

/**
 * Dragging inside a window — a reminder to another place or list, a contact onto a list, photos
 * onto an album — on Windows.
 *
 * There Tauri takes the window's drop target from WebView2 to hear of files dropped from
 * Explorer (Drive, Notes and Photos take them), and with it every drop of the page's own: a
 * drag started in the page never got a `dragover` or a `drop` anywhere, so nothing dragged
 * inside a window ever landed. Here a drag the page starts from one of its own draggable
 * elements is carried by the mouse instead: the engine's drag is stopped before it starts, and
 * the same events — `dragstart`, `dragenter`, `dragover`, `dragleave`, `drop`, `dragend`, with
 * one DataTransfer through them all — are sent to the elements under the pointer as it moves.
 * Every handler in the page takes them as it takes the engine's. Files dragged in from outside
 * never start in the page, and still arrive as before; text, links and pictures are left to the
 * engine. Linux (WebKitGTK) keeps the engine's own drag.
 */
export function carryDragsInPage(on = onWindows) {
  if (!on || typeof window === "undefined") return;

  type Drag = {
    source: Element;
    data: DataTransfer;
    over: Element | null;
    /** Whether the element under the pointer said it takes the drop (its `dragover` was cancelled). */
    takes: boolean;
    x: number;
    y: number;
    /** Its dragstart has been sent: until then the pointer is only followed. */
    begun?: boolean;
  };
  let drag: Drag | null = null;

  const send = (type: string, to: Element, d: Drag, related: Element | null = null) => {
    const event = new DragEvent(type, {
      bubbles: true,
      cancelable: type !== "dragleave" && type !== "dragend",
      composed: true,
      clientX: d.x,
      clientY: d.y,
      screenX: d.x + window.screenX,
      screenY: d.y + window.screenY,
      relatedTarget: related,
      dataTransfer: d.data,
    });
    to.dispatchEvent(event);
    return event;
  };

  /** The pointer at (x, y): leaving what it was over, entering what it is over now. */
  const move = (d: Drag, x: number, y: number) => {
    d.x = x;
    d.y = y;
    const at = document.elementFromPoint(x, y);
    if (at !== d.over) {
      const was = d.over;
      if (at) send("dragenter", at, d, was);
      if (was) send("dragleave", was, d, at);
      d.over = at;
    }
    send("drag", d.source, d);
    d.takes = d.over ? send("dragover", d.over, d).defaultPrevented : false;
  };

  const finish = (dropped: boolean) => {
    const d = drag;
    if (!d) return;
    drag = null;
    stopListening();
    if (!d.begun) return;
    if (d.over) {
      if (dropped && d.takes) send("drop", d.over, d);
      else send("dragleave", d.over, d);
    }
    if (!(dropped && d.takes)) d.data.dropEffect = "none";
    send("dragend", d.source, d);
    // The press began on one element and ended on another: no click for whatever holds both.
    const swallow = (e: MouseEvent) => {
      e.preventDefault();
      e.stopImmediatePropagation();
    };
    window.addEventListener("click", swallow, true);
    window.setTimeout(() => window.removeEventListener("click", swallow, true), 0);
  };

  function stopListening() {
    window.removeEventListener("mousemove", onMove, true);
    window.removeEventListener("mouseup", onUp, true);
    window.removeEventListener("keydown", onKey, true);
    window.removeEventListener("blur", onCancel);
    document.documentElement.style.removeProperty("cursor");
    document.documentElement.style.removeProperty("user-select");
  }

  const onMove = (e: MouseEvent) => {
    if (!drag) return;
    if ((e.buttons & 1) === 0) {
      // The button came up somewhere the page did not hear of (outside the window).
      finish(false);
      return;
    }
    e.preventDefault();
    window.getSelection()?.removeAllRanges();
    if (!drag.begun) {
      drag.x = e.clientX;
      drag.y = e.clientY;
      return;
    }
    move(drag, e.clientX, e.clientY);
    document.documentElement.style.cursor = drag.takes ? "default" : "no-drop";
  };
  const onUp = (e: MouseEvent) => {
    if (!drag) return;
    e.preventDefault();
    e.stopPropagation();
    if (drag.begun) move(drag, e.clientX, e.clientY);
    finish(true);
  };
  const onKey = (e: KeyboardEvent) => {
    if (!drag || e.key !== "Escape") return;
    e.preventDefault();
    e.stopImmediatePropagation();
    finish(false);
  };
  const onCancel = () => finish(false);

  window.addEventListener(
    "dragstart",
    (e) => {
      // Only the engine's own, from one of the page's draggable elements: the ones sent here
      // are let through to the page, and so is a drag of text, a link or a picture.
      if (!e.isTrusted || drag) return;
      const source = (e.target as Element | null)?.closest?.('[draggable="true"]');
      if (!source) return;
      // Kept from the page, which hears the one sent below instead.
      e.preventDefault();
      e.stopPropagation();
      const d: Drag = { source, data: new DataTransfer(), over: null, takes: false, x: e.clientX, y: e.clientY };
      drag = d;
      window.addEventListener("mousemove", onMove, true);
      window.addEventListener("mouseup", onUp, true);
      window.addEventListener("keydown", onKey, true);
      window.addEventListener("blur", onCancel);
      document.documentElement.style.userSelect = "none";
      // Sent once the engine's own has gone by — never inside it, where anything else on the
      // window still waiting for "the" dragstart would take this one for it.
      window.setTimeout(() => {
        if (drag !== d) return;
        if (send("dragstart", source, d).defaultPrevented) {
          drag = null;
          stopListening();
          return;
        }
        d.begun = true;
        move(d, d.x, d.y);
      }, 0);
    },
    true,
  );
}
