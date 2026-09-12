/** The Quince mark: a pome, a stem, a leaf. Monoline, so the chrome stays quiet. */
export function Mark({ className }: { className?: string }) {
  return (
    <svg
      className={className}
      viewBox="0 0 48 48"
      fill="none"
      stroke="currentColor"
      strokeWidth="2.4"
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
    >
      <path d="M24 18c-7 0-12.4 5.2-12.4 12.2C11.6 37.1 17 42.5 24 42.5s12.4-5.4 12.4-12.3C36.4 23.2 31 18 24 18Z" />
      <path d="M24 18c-.4-3.6.3-6.2 2.1-8.4" />
      <path d="M26.6 9.1c2-2.6 5.4-3.6 8.6-2.6.2 3.6-2.3 6.9-6 7.5" />
    </svg>
  );
}
