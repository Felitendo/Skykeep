import icon from "../assets/skykeep-mark.png";

/**
 * Skykeep's mark — the tower in its cloud, without the tile, the same drawing the dock and every
 * alert show inside theirs — so the first screen someone sees is the app they will find later.
 * One mark serves light and dark alike: it reads on white and on near-black.
 */
export function Mark({ className }: { className?: string }) {
  return <img className={className} src={icon} alt="" aria-hidden="true" draggable={false} />;
}
