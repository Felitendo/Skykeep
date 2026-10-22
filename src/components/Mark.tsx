import icon from "../assets/skykeep-mark.png";

/**
 * Skykeep's icon — the same drawing the dock and launcher show — so the first screen someone
 * sees is the app they will find later. Its outlines let one drawing serve light and dark alike.
 */
export function Mark({ className }: { className?: string }) {
  return <img className={className} src={icon} alt="" aria-hidden="true" draggable={false} />;
}
