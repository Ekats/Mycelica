/**
 * Get color for similarity score.
 * Red (≤25%) → Yellow (50%) → [step] → Blue (65%) → Cyan (100%)
 * Colorblind accessible - no green.
 *
 * @param similarity - Value between 0 and 1
 * @returns HSL color string
 */
export function getSimilarityColor(similarity: number): string {
  let hue: number;

  if (similarity <= 0.25) {
    // Solid red
    hue = 0;
  } else if (similarity < 0.50) {
    // Red (0°) → Yellow (50°)
    const t = (similarity - 0.25) / 0.25;
    hue = t * 50;
  } else if (similarity < 0.65) {
    // Yellow range (50° - 60°)
    const t = (similarity - 0.50) / 0.15;
    hue = 50 + t * 10;
  } else {
    // Blue (220°) → Cyan (180°)
    const t = (similarity - 0.65) / 0.35;
    hue = 220 - t * 40;
  }

  return `hsl(${hue}, 80%, 50%)`;
}
