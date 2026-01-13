/**
 * Universal heat color for normalized values (0-1).
 * Red (0%) → Yellow (50%) → Cyan (100%)
 * Colorblind accessible - no green, no blue.
 *
 * Single source of truth for all value-based colors:
 * - Similarity scores
 * - Edge weights
 * - Date ranges (normalized)
 *
 * @param t - Normalized value between 0 and 1
 * @param saturation - HSL saturation (default 75%)
 * @param lightness - HSL lightness (default 65%)
 * @returns HSL color string
 */
export function getHeatColor(t: number, saturation = 75, lightness = 65): string {
  // Clamp to 0-1
  t = Math.max(0, Math.min(1, t));

  let hue: number;
  if (t < 0.5) {
    hue = t * 2 * 60; // red (0°) → yellow (60°)
  } else {
    hue = 60 + (t - 0.5) * 2 * 120; // yellow (60°) → cyan (180°)
  }
  return `hsl(${hue}, ${saturation}%, ${lightness}%)`;
}

/**
 * Get color for similarity score (0-1).
 * Alias for getHeatColor for semantic clarity.
 */
export function getSimilarityColor(similarity: number): string {
  return getHeatColor(similarity);
}

/**
 * Get color for edge weight (0-1).
 * Alias for getHeatColor for semantic clarity.
 */
export function getEdgeColor(weight: number): string {
  return getHeatColor(weight);
}

/**
 * Get color for date based on range.
 * @param timestamp - The timestamp to color
 * @param minDate - Oldest date in range
 * @param maxDate - Newest date in range
 */
export function getDateColor(timestamp: number, minDate: number, maxDate: number): string {
  const range = maxDate - minDate || 1;
  const t = (timestamp - minDate) / range;
  return getHeatColor(t);
}
