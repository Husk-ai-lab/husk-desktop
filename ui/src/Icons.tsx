import type { SVGProps } from "react";

const base = (p: SVGProps<SVGSVGElement>) => ({
  width: 18,
  height: 18,
  viewBox: "0 0 24 24",
  fill: "none",
  stroke: "currentColor",
  strokeWidth: 1.6,
  strokeLinecap: "round" as const,
  strokeLinejoin: "round" as const,
  "aria-hidden": true,
  ...p,
});

export const Moon = (p: SVGProps<SVGSVGElement>) => (
  <svg {...base(p)}><path d="M20 14.5A8 8 0 0 1 9.5 4 8 8 0 1 0 20 14.5Z" /></svg>
);
export const Sun = (p: SVGProps<SVGSVGElement>) => (
  <svg {...base(p)}>
    <circle cx="12" cy="12" r="4" />
    <path d="M12 3v2M12 19v2M3 12h2M19 12h2M5.6 5.6 7 7M17 17l1.4 1.4M5.6 18.4 7 17M17 7l1.4-1.4" />
  </svg>
);
export const Check = (p: SVGProps<SVGSVGElement>) => (
  <svg {...base(p)}><path d="m5 12.5 4.5 4.5L19 7.5" /></svg>
);
export const Layers = (p: SVGProps<SVGSVGElement>) => (
  <svg {...base(p)}><path d="m12 3 9 5-9 5-9-5 9-5Z" /><path d="m3 13 9 5 9-5" /></svg>
);
export const Refresh = (p: SVGProps<SVGSVGElement>) => (
  <svg {...base(p)}><path d="M20 11a8 8 0 0 0-14.5-4M4 4v4h4" /><path d="M4 13a8 8 0 0 0 14.5 4M20 20v-4h-4" /></svg>
);
export const FileText = (p: SVGProps<SVGSVGElement>) => (
  <svg {...base(p)}><path d="M6 3h8l4 4v14H6z" /><path d="M14 3v4h4M9 12h6M9 16h6" /></svg>
);
export const Sliders = (p: SVGProps<SVGSVGElement>) => (
  <svg {...base(p)}><path d="M4 7h10M18 7h2M4 17h2M10 17h10" /><circle cx="16" cy="7" r="2" /><circle cx="8" cy="17" r="2" /></svg>
);
export const Shield = (p: SVGProps<SVGSVGElement>) => (
  <svg {...base(p)}><path d="M12 3 5 6v6c0 4.2 2.8 7.3 7 9 4.2-1.7 7-4.8 7-9V6l-7-3Z" /><path d="m9 12 2 2 4-4" /></svg>
);
export const Download = (p: SVGProps<SVGSVGElement>) => (
  <svg {...base(p)}><path d="M12 4v11M7.5 11 12 15.5 16.5 11M5 20h14" /></svg>
);
export const Arrow = (p: SVGProps<SVGSVGElement>) => (
  <svg {...base(p)}><path d="M5 12h14M13 6l6 6-6 6" /></svg>
);

function gear(cx: number, cy: number, r: number, teeth: number, depth: number): string {
  const pts: string[] = [];
  const step = (Math.PI * 2) / teeth;
  for (let i = 0; i < teeth; i++) {
    const a = i * step;
    const q = step / 4;
    for (const [da, rr] of [[-q * 1.5, r], [-q * 0.6, r + depth], [q * 0.6, r + depth], [q * 1.5, r]] as const) {
      pts.push(`${(cx + Math.cos(a + da) * rr).toFixed(1)} ${(cy + Math.sin(a + da) * rr).toFixed(1)}`);
    }
  }
  return `M${pts.join("L")}Z`;
}

/** Large outline icon for the Studio card: two meshing gears, like the prototype's "Tecnico". */
export const GearsOutline = (p: SVGProps<SVGSVGElement>) => (
  <svg viewBox="0 0 96 96" width="84" height="84" fill="none" stroke="currentColor" strokeWidth="2.4" strokeLinejoin="round" aria-hidden {...p}>
    <path d={gear(36, 38, 17, 8, 7)} />
    <circle cx="36" cy="38" r="7" />
    <path d={gear(68, 66, 10, 6, 5)} />
    <circle cx="68" cy="66" r="4" />
  </svg>
);
