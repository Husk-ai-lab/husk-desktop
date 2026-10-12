/**
 * Line-art robot arm with a lens over a stack of documents, in the style of the prototype's
 * Discovery screens. Strokes follow the text colour and the lens uses the accent, so it works in
 * both themes. Drawn back to front: base, column, documents, arm, lens.
 */
export function Illustration({ className }: { className?: string }) {
  const bolts: [number, number][] = [[196, 372], [228, 396], [300, 406], [372, 396], [404, 372], [372, 349], [228, 349], [300, 338]];
  return (
    <svg className={className} viewBox="0 0 520 470" fill="none" role="img" aria-label="" strokeLinejoin="round" strokeLinecap="round">
      <g stroke="currentColor" strokeWidth="2.6">
        {/* orbit rings */}
        <ellipse cx="300" cy="394" rx="218" ry="62" stroke="var(--accent)" strokeWidth="1.2" strokeDasharray="3 5" opacity=".6" />
        <ellipse cx="330" cy="392" rx="150" ry="42" stroke="var(--accent)" strokeWidth="1.2" opacity=".35" />
        {/* base plate */}
        <path d="M150 372v22c0 24 66 42 150 42s150-18 150-42v-22" fill="var(--surface)" />
        <ellipse cx="300" cy="372" rx="150" ry="42" fill="var(--surface)" />
        <ellipse cx="300" cy="372" rx="102" ry="27" />
        {bolts.map(([x, y], i) => <ellipse key={i} cx={x} cy={y} rx="9" ry="5" fill="var(--surface)" strokeWidth="2" />)}
        {/* column */}
        <path d="M232 284v76c0 14 30 22 68 22s68-8 68-22v-76" fill="var(--surface)" />
        <ellipse cx="300" cy="284" rx="68" ry="20" fill="var(--surface)" />
        <path d="M262 304c-6 20-6 40 0 58M281 308c-4 18-4 38 0 58" stroke="var(--accent)" strokeWidth="2" />

        {/* documents (three sheets) */}
        <path d="M24 438 144 408l118 30-120 30Z" fill="var(--surface)" />
        <path d="M24 424 144 394l118 30-120 30Z" fill="var(--surface)" />
        <path d="M24 410 144 380l118 30-120 30Z" fill="var(--surface)" />
        <path d="M66 412l58-14M70 422l78-19M92 430l58-14" stroke="var(--accent)" strokeWidth="2" />
        {/* light cone from the lens onto the top sheet */}
        <path d="M78 346 36 408l150 22Z" fill="var(--accent)" opacity=".16" stroke="none" />

        {/* upper arm: two parallel bars from shoulder to elbow */}
        <path d="M305 227 215 75M275 245 185 93" />
        {/* shoulder joint */}
        <circle cx="292" cy="262" r="46" fill="var(--surface)" />
        <circle cx="292" cy="262" r="26" />
        <circle cx="292" cy="262" r="8" fill="var(--accent)" stroke="var(--accent)" />
        {/* forearm: two parallel bars from elbow down to the wrist */}
        <path d="M209 106 141 256M183 94 115 244" />
        {/* elbow joint */}
        <circle cx="198" cy="82" r="38" fill="var(--surface)" />
        <circle cx="198" cy="82" r="19" />
        {/* wrist + handle */}
        <path d="M115 244l26 12" />
        <path d="M128 252l-12 22" strokeWidth="9" stroke="var(--surface)" />
        <path d="M122 250l-12 24M134 256l-12 24" />
        {/* lens */}
        <circle cx="100" cy="320" r="52" fill="var(--accent-soft)" stroke="var(--accent)" strokeWidth="3" />
        <circle cx="100" cy="320" r="42" stroke="var(--accent)" strokeWidth="1.2" />
        <path d="M84 334c10 8 22 8 32 0" stroke="currentColor" strokeWidth="2" />
      </g>
    </svg>
  );
}
