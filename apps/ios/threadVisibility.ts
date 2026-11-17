export type ThreadBounds = { x: number; y: number; width: number; height: number };

export function intersectThreadBounds(left: ThreadBounds, right: ThreadBounds): ThreadBounds | undefined {
  const x = Math.max(left.x, right.x);
  const y = Math.max(left.y, right.y);
  const width = Math.min(left.x + left.width, right.x + right.width) - x;
  const height = Math.min(left.y + left.height, right.y + right.height) - y;
  return width > 0 && height > 0 ? { x, y, width, height } : undefined;
}

/** Read evidence requires two lines (the whole body, when it is shorter than two lines),
 * the entire content width and its trailing end. Metadata/author labels must not be
 * included in the measured body bounds.
 */
export function visibleThreadMessages(
  viewport: ThreadBounds,
  rows: readonly { id: string; bounds?: ThreadBounds; lineHeight?: number }[],
  limit: number,
): string[] {
  return rows.flatMap(row => {
    if (!row.bounds || !row.lineHeight || row.lineHeight <= 0) return [];
    const visible = intersectThreadBounds(viewport, row.bounds);
    return visible && visible.height >= Math.min(row.bounds.height, 2 * row.lineHeight)
      && visible.width >= row.bounds.width
      && visible.y + visible.height >= row.bounds.y + row.bounds.height ? [row.id] : [];
  }).slice(0, limit);
}
