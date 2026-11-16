export type ThreadBounds = { x: number; y: number; width: number; height: number };

export function intersectThreadBounds(left: ThreadBounds, right: ThreadBounds): ThreadBounds | undefined {
  const x = Math.max(left.x, right.x);
  const y = Math.max(left.y, right.y);
  const width = Math.min(left.x + left.width, right.x + right.width) - x;
  const height = Math.min(left.y + left.height, right.y + right.height) - y;
  return width > 0 && height > 0 ? { x, y, width, height } : undefined;
}

/** At least a line of message content, not a one-pixel edge or an author label.
 * Short bodies must be fully visible. Rows arrive in canonical newest-first order.
 */
export function visibleThreadMessages(
  viewport: ThreadBounds,
  rows: readonly { id: string; bounds?: ThreadBounds }[],
  limit: number,
): string[] {
  return rows.flatMap(row => {
    if (!row.bounds) return [];
    const visible = intersectThreadBounds(viewport, row.bounds);
    return visible && visible.height >= Math.min(24, row.bounds.height)
      && visible.width >= Math.min(24, row.bounds.width) ? [row.id] : [];
  }).slice(0, limit);
}
