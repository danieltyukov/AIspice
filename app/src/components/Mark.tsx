/** The aispice mark: a step response traced from a probe dot, on a teal tile. */
export function Mark({ size = 20, className }: { size?: number; className?: string }) {
  return (
    <svg viewBox="0 0 32 32" width={size} height={size} className={className} aria-hidden="true" focusable="false">
      <rect width="32" height="32" rx="7.5" fill="var(--accent)" />
      <path
        d="M5.75 22.25H10.25C12.4 22.25 12.6 8.75 15.1 8.75C17.5 8.75 17.4 14.6 19.7 14.6C21.6 14.6 21.7 11.35 23.4 11.35C24.8 11.35 25.2 12.1 26.5 12.1"
        fill="none"
        stroke="#fff"
        strokeWidth="2.3"
        strokeLinecap="round"
        strokeLinejoin="round"
      />
      <circle cx="5.75" cy="22.25" r="2.15" fill="#fff" />
    </svg>
  );
}
