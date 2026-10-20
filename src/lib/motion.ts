/**
 * An animation that can never flash.
 *
 * WebKitGTK hands an animation to its compositor a frame after it is asked for, and takes it
 * back a frame after it ends. For that frame the element is drawn as its own style says, not
 * as the animation does: a backdrop fading in drew fully black for one frame before its fade
 * began, and a viewer fading out drew fully there for one frame after it had gone — the
 * whole-window flicker on every open and close, frame by frame in his recordings.
 *
 * So the element's own style is always where the animation is: its first frame before the
 * animation starts, its last frame once the engine says it is running (`ready`). Whenever the
 * engine draws the element without the animation, it draws the right thing.
 */
export function glide(
  el: HTMLElement,
  frames: [Keyframe, Keyframe],
  options: KeyframeAnimationOptions,
): Animation {
  const [from, to] = frames;
  set(el, from);
  const a = el.animate(frames, { ...options, fill: "both" });
  a.ready.then(
    () => {
      if (a.playState !== "idle") set(el, to);
    },
    () => {},
  );
  // Done, it lets go: its last frame is the element's own style by then, so nothing moves —
  // and nothing held over that style (a zoom writes the photo's transform itself).
  a.finished.then(
    () => {
      set(el, to);
      a.cancel();
    },
    () => {},
  );
  return a;
}

function set(el: HTMLElement, frame: Keyframe) {
  for (const [k, v] of Object.entries(frame)) {
    if (k === "offset" || k === "easing" || k === "composite") continue;
    el.style.setProperty(k.replace(/[A-Z]/g, (c) => `-${c.toLowerCase()}`), v == null ? "" : String(v));
  }
}
