/**
 * Build an SVG polyline path for a sparkline, normalized to the viewport.
 * Mirrors the path math from design/design-marathon.html.
 */
export function sparkPathD(values: number[], w: number, h: number): string {
  if (values.length < 2) return '';
  const mx = Math.max(...values, 1e-9);
  const st = w / (values.length - 1);
  return values
    .map((v, i) => {
      const x = (i * st).toFixed(1);
      const y = (h - (v / mx) * h * 0.9).toFixed(1);
      return `${i ? 'L' : 'M'}${x} ${y}`;
    })
    .join(' ');
}
