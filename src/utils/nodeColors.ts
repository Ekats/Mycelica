/**
 * Shared color utilities for graph nodes and connections
 */

import { getHeatColor } from './similarityColor';

// Type for node data in rendering context
export interface RenderNode {
  id: string;
  renderClusterId: number;
}

/**
 * Generate consistent color for a cluster using golden angle distribution
 */
export const generateClusterColor = (clusterId: number): string => {
  const hue = (clusterId * 137.508) % 360; // Golden angle for good color distribution
  return `hsl(${hue}, 55%, 35%)`;
};

/**
 * Direct connection color: uses shared heat color formula
 * Darker than default for better visibility on graph nodes
 */
export const getDirectConnectionColor = (weight: number): string => {
  return getHeatColor(weight, 80, 40); // Darker for graph visibility
};

/**
 * Chain connection color: darker red tint for indirect connections
 */
export const getChainConnectionColor = (hopDistance: number): string => {
  // Further = darker/more faded red
  const lightness = Math.max(25, 35 - hopDistance * 3); // 35% -> 25% as distance increases
  return `hsl(0, 60%, ${lightness}%)`; // Red hue, moderate saturation
};

/**
 * Calculate structural depth for shadow stacking
 * Items: 0 (no stack, just subtle shadow)
 * Topics: 1-4 (violet base + 0-3 cluster shadows)
 */
export const getStructuralDepth = (childCount: number, isItem: boolean): number => {
  if (isItem) return 0;
  if (childCount >= 16) return 4;  // violet + 3 cluster
  if (childCount >= 6) return 3;   // violet + 2 cluster
  if (childCount >= 2) return 2;   // violet + 1 cluster
  return 1;  // just violet (all topics)
};

/**
 * Get muted cluster color (gray with hint of cluster hue)
 */
export const getMutedClusterColor = (node: RenderNode): string => {
  if (node.renderClusterId < 0) return '#374151';
  const hue = (node.renderClusterId * 137.508) % 360;
  return `hsl(${hue}, 12%, 28%)`;
};
