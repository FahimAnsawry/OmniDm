// SPDX-License-Identifier: GPL-3.0-or-later

// A line of recent speeds with a soft fill under it. With too few speeds yet, a dashed
// flat line.

import { useId } from "react";

const WIDTH = 200;

export function SpeedGraph({ samples, className, height = 48 }: { samples: number[]; className: string; height?: number }) {
  const id = useId();
  if (samples.length < 2) {
    return (
      <svg viewBox={`0 0 ${WIDTH} ${height}`} preserveAspectRatio="none" aria-hidden="true" className={className}>
        <path
          d={`M0 ${height * 0.86}H${WIDTH}`}
          fill="none"
          stroke="#ffffff"
          strokeOpacity=".14"
          strokeWidth="1.5"
          strokeDasharray="3 4"
          vectorEffect="non-scaling-stroke"
        />
      </svg>
    );
  }
  const max = Math.max(...samples, 1);
  const bottom = height * 0.86;
  const span = height * 0.64;
  const step = WIDTH / (samples.length - 1);
  const line = samples
    .map((s, i) => `${i ? "L" : "M"}${(i * step).toFixed(1)} ${(bottom - (s / max) * span).toFixed(1)}`)
    .join("");
  return (
    <svg viewBox={`0 0 ${WIDTH} ${height}`} preserveAspectRatio="none" aria-hidden="true" className={className}>
      <defs>
        <linearGradient id={id} x1="0" y1="0" x2="0" y2="1">
          <stop offset="0" stopColor="#6b88ff" stopOpacity=".35" />
          <stop offset="1" stopColor="#6b88ff" stopOpacity="0" />
        </linearGradient>
      </defs>
      <path d={`${line}V${height}H0Z`} fill={`url(#${id})`} />
      <path
        d={line}
        fill="none"
        stroke="#7c9bff"
        strokeWidth="2"
        strokeLinejoin="round"
        vectorEffect="non-scaling-stroke"
      />
    </svg>
  );
}
