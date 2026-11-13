export type ThreadBounds = { x: number; y: number; width: number; height: number };

export function intersectThreadBounds(left: ThreadBounds, right: ThreadBounds): ThreadBounds | undefined {
  const x = Math.max(left.x, right.x);
  const y = Math.max(left.y, right.y);
  const width = Math.min(left.x + left.width, right.x + right.width) - x;
  const height = Math.min(left.y + left.height, right.y + right.height) - y;
  return width > 0 && height > 0 ? { x, y, width, height } : undefined;
}

/** Rows arrive in canonical newest-first order. A clipped or unknown row is never acknowledged. */
export function visibleThreadMessages(
  viewport: ThreadBounds,
  rows: readonly { id: string; bounds?: ThreadBounds }[],
  limit: number,
): string[] {
  return rows.flatMap(row => row.bounds && intersectThreadBounds(viewport, row.bounds) ? [row.id] : []).slice(0, limit);
}
