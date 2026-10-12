/** The Husk mark. Exports follow the design system: cropped to the mark, 1x/2x srcset. */
export function Logo({ size = 28, withName = true }: { size?: number; withName?: boolean }) {
  const px = size <= 32 ? 32 : size <= 64 ? 64 : 128;
  const px2 = px === 32 ? 64 : px === 64 ? 128 : 256;
  return (
    <span className="logo">
      <img
        src={`/brand/husk-logo-${px}.png`}
        srcSet={`/brand/husk-logo-${px}.png 1x, /brand/husk-logo-${px2}.png 2x`}
        width={size}
        height={size}
        alt=""
      />
      {withName && <span className="logo-name">Husk AI</span>}
    </span>
  );
}
