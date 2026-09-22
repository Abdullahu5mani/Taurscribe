import { useId } from "react";
import "./Logo.css";

/**
 * Taurscribe mark: a pen nib whose slit is a voice waveform (speech → writing).
 *
 *  - variant "full": five bars, breather hole and slit (intro, large sizes)
 *  - variant "small": three bolder bars, for 16–24 px (title bar)
 *  - animate: plays the intro (nib rises, bars grow like a voice level, the
 *    slit draws down). Changing `replayKey` replays it.
 */
export function Logo({
  size = 24,
  variant = "full",
  animate = false,
  replayKey,
  className = "",
}: {
  size?: number;
  variant?: "full" | "small";
  animate?: boolean;
  replayKey?: number | string;
  className?: string;
}) {
  const gid = `logo-grad-${useId().replace(/:/g, "")}`;
  const bars = variant === "full"
    ? [[24, 19, 23], [28, 15, 27], [32, 12, 30], [36, 15, 27], [40, 19, 23]]
    : [[26, 17, 25], [32, 12, 30], [38, 17, 25]];
  return (
    <svg
      id="logo-taurscribe"
      data-testid="logo-taurscribe"
      key={replayKey}
      className={`ts-logo${animate ? " ts-logo--animate" : ""} ${className}`}
      width={size}
      height={size}
      viewBox="0 0 64 64"
      role="img"
      aria-label="Taurscribe"
    >
      <defs>
        <linearGradient id={gid} x1="0" y1="0" x2="1" y2="1">
          <stop offset="0" stopColor="#F8C66A" />
          <stop offset="1" stopColor="#EE7F4B" />
        </linearGradient>
      </defs>
      <path
        className="ts-logo-nib"
        d="M21 6H43C45.2 6 47 7.8 47 10V25.5C47 30.5 44.2 34.2 40.6 38.6L32.9 57.6C32.6 58.4 31.4 58.4 31.1 57.6L23.4 38.6C19.8 34.2 17 30.5 17 25.5V10C17 7.8 18.8 6 21 6Z"
        fill={`url(#${gid})`}
      />
      <g stroke="#1A1411" strokeWidth={variant === "full" ? 2.7 : 4} strokeLinecap="round">
        {bars.map(([x, y1, y2], i) => (
          <line
            key={x}
            className="ts-logo-bar"
            style={{ animationDelay: `${380 + i * 70}ms`, transformOrigin: `${x}px 21px` }}
            x1={x} y1={y1} x2={x} y2={y2}
          />
        ))}
      </g>
      {variant === "full" && (
        <>
          <circle className="ts-logo-hole" cx="32" cy="37" r="2.6" fill="#1A1411" />
          <line className="ts-logo-slit" x1="32" y1="39" x2="32" y2="54" stroke="#1A1411" strokeWidth="2" strokeLinecap="round" pathLength={1} />
        </>
      )}
    </svg>
  );
}
