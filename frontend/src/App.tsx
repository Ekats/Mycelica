import { useEffect, useState, useRef } from 'react'
import * as d3 from 'd3'
import './App.css'

interface Node {
  id: string
  label: string
  title: string
  cluster_id: number
  color: string
  size: number
  message_count: number
  created_at: string
  keywords: string[]
  x?: number
  y?: number
  baseX?: number  // Home position
  baseY?: number
  fx?: number | null
  fy?: number | null
}

// Emoji mapping for common topics
const topicEmojis: { [key: string]: string } = {
  // Programming & Tech
  'code': '💻', 'python': '🐍', 'javascript': '🟨', 'react': '⚛️', 'api': '🔌',
  'database': '🗄️', 'server': '🖥️', 'web': '🌐', 'app': '📱', 'bug': '🐛',
  'error': '❌', 'function': '⚡', 'class': '📦', 'test': '🧪', 'deploy': '🚀',
  'git': '📝', 'docker': '🐳', 'cloud': '☁️', 'security': '🔒', 'data': '📊',

  // AI & ML
  'ai': '🤖', 'machine': '🤖', 'learning': '🧠', 'model': '🎯', 'neural': '🧠',
  'gpt': '🤖', 'claude': '🧠', 'prompt': '💬', 'chat': '💭', 'llm': '🤖',

  // Writing & Content
  'write': '✍️', 'writing': '✍️', 'story': '📖', 'book': '📚', 'blog': '📝',
  'article': '📰', 'essay': '📄', 'poem': '🎭', 'script': '🎬', 'creative': '🎨',

  // Business & Work
  'business': '💼', 'work': '👔', 'project': '📋', 'meeting': '🤝', 'email': '📧',
  'resume': '📄', 'interview': '🎤', 'presentation': '📊', 'strategy': '♟️', 'plan': '📅',

  // Science & Math
  'math': '🔢', 'science': '🔬', 'physics': '⚛️', 'chemistry': '🧪', 'biology': '🧬',
  'research': '🔍', 'experiment': '🧫', 'analysis': '📈', 'statistics': '📉',

  // Education
  'learn': '📚', 'study': '📖', 'course': '🎓', 'tutorial': '👨‍🏫', 'explain': '💡',
  'question': '❓', 'answer': '✅', 'help': '🆘', 'understand': '🤔',

  // Design & Creative
  'design': '🎨', 'ui': '🖼️', 'ux': '👤', 'color': '🌈', 'image': '🖼️',
  'logo': '🏷️', 'icon': '⭐', 'font': '🔤', 'layout': '📐',

  // Communication
  'translate': '🌍', 'language': '🗣️', 'english': '🇬🇧', 'spanish': '🇪🇸',
  'french': '🇫🇷', 'german': '🇩🇪', 'chinese': '🇨🇳', 'japanese': '🇯🇵',

  // Life & Personal
  'health': '❤️', 'food': '🍽️', 'recipe': '👨‍🍳', 'travel': '✈️', 'home': '🏠',
  'money': '💰', 'finance': '💵', 'fitness': '💪', 'meditation': '🧘',

  // Entertainment
  'game': '🎮', 'music': '🎵', 'movie': '🎬', 'video': '📹', 'art': '🎨',

  // Misc
  'idea': '💡', 'problem': '🔧', 'solution': '✨', 'list': '📋', 'compare': '⚖️',
  'review': '⭐', 'summary': '📝', 'debug': '🔍', 'fix': '🔧', 'create': '✨',
  'build': '🏗️', 'make': '🛠️', 'generate': '⚡', 'convert': '🔄', 'format': '📋'
}

function getEmojiForNode(node: Node): string {
  const searchText = (node.title + ' ' + node.keywords.join(' ')).toLowerCase()

  for (const [keyword, emoji] of Object.entries(topicEmojis)) {
    if (searchText.includes(keyword)) {
      return emoji
    }
  }

  return '💭' // Default thinking emoji
}

interface Edge {
  source: string | Node
  target: string | Node
  weight: number
}

interface Cluster {
  id: number
  keywords: string[]
  color: string
  count: number
  name: string
}

interface GraphData {
  nodes: Node[]
  edges: Edge[]
  clusters: Cluster[]
}

function App() {
  const svgRef = useRef<SVGSVGElement>(null)
  const zoomRef = useRef<d3.ZoomBehavior<SVGSVGElement, unknown> | null>(null)
  const focusModeRef = useRef(false)
  const [graphData, setGraphData] = useState<GraphData | null>(null)
  const [selectedNode, setSelectedNode] = useState<Node | null>(null)
  const [loading, setLoading] = useState(true)
  const [searchQuery, setSearchQuery] = useState('')
  const [hoveredNode, setHoveredNode] = useState<Node | null>(null)
  const [expandedCluster, setExpandedCluster] = useState<number | null>(null)
  const [focusMode, setFocusMode] = useState(false)

  // Keep ref in sync with state
  useEffect(() => {
    focusModeRef.current = focusMode
  }, [focusMode])

  // Get connected node IDs for focus mode
  const getConnectedNodeIds = (nodeId: string): Set<string> => {
    if (!graphData) return new Set()
    const connected = new Set<string>([nodeId])
    graphData.edges.forEach(edge => {
      const sourceId = typeof edge.source === 'string' ? edge.source : edge.source.id
      const targetId = typeof edge.target === 'string' ? edge.target : edge.target.id
      if (sourceId === nodeId) connected.add(targetId)
      if (targetId === nodeId) connected.add(sourceId)
    })
    return connected
  }

  // Zoom to cluster function - use BASE positions for targeting
  const zoomToCluster = (clusterId: number) => {
    if (!graphData || !svgRef.current || !zoomRef.current) return

    const clusterNodes = graphData.nodes.filter(n => n.cluster_id === clusterId)
    if (clusterNodes.length === 0) return

    const hasPositions = clusterNodes.every(n => n.baseX !== undefined && n.baseY !== undefined)
    if (!hasPositions) return

    // Use BASE positions for consistent targeting
    const xs = clusterNodes.map(n => n.baseX!)
    const ys = clusterNodes.map(n => n.baseY!)

    const minX = Math.min(...xs)
    const maxX = Math.max(...xs)
    const minY = Math.min(...ys)
    const maxY = Math.max(...ys)

    const centerX = (minX + maxX) / 2
    const centerY = (minY + maxY) / 2

    const clusterWidth = Math.max(maxX - minX, 100)
    const clusterHeight = Math.max(maxY - minY, 100)

    const width = window.innerWidth
    const height = window.innerHeight

    const scale = Math.min(
      (width * 0.6) / clusterWidth,
      (height * 0.6) / clusterHeight,
      2.5
    )

    const svg = d3.select(svgRef.current)
    svg.transition()
      .duration(750)
      .call(
        zoomRef.current.transform,
        d3.zoomIdentity
          .translate(width / 2, height / 2)
          .scale(scale)
          .translate(-centerX, -centerY)
      )
  }

  useEffect(() => {
    // Fetch graph data
    fetch('http://localhost:8000/graph/analyzed')
      .then(res => res.json())
      .then(data => {
        setGraphData(data)
        setLoading(false)
      })
      .catch(err => {
        console.error('Failed to load graph:', err)
        setLoading(false)
      })
  }, [])

  useEffect(() => {
    if (!graphData || !svgRef.current) return

    const svg = d3.select(svgRef.current)
    const width = window.innerWidth
    const height = window.innerHeight

    // Clear previous
    svg.selectAll('*').remove()

    // Create container for zoom
    const container = svg.append('g')

    // Initialize node positions in spiral by cluster
    const clusterGroups: { [key: number]: Node[] } = {}
    graphData.nodes.forEach(node => {
      if (!clusterGroups[node.cluster_id]) {
        clusterGroups[node.cluster_id] = []
      }
      clusterGroups[node.cluster_id].push(node)
    })

    const clusterIds = Object.keys(clusterGroups)
      .map(Number)
      .sort((a, b) => clusterGroups[b].length - clusterGroups[a].length)

    // Initial positions in spiral - store as base positions
    clusterIds.forEach((clusterId, i) => {
      const angle = i * 2.4
      const radius = 100 + i * 30
      const clusterX = width / 2 + radius * Math.cos(angle)
      const clusterY = height / 2 + radius * Math.sin(angle)

      const nodes = clusterGroups[clusterId]
      nodes.forEach((node, j) => {
        const nodeAngle = (2 * Math.PI * j) / nodes.length
        const nodeRadius = Math.min(15 + nodes.length * 3, 50)
        node.baseX = clusterX + nodeRadius * Math.cos(nodeAngle)
        node.baseY = clusterY + nodeRadius * Math.sin(nodeAngle)
        node.x = node.baseX
        node.y = node.baseY
      })
    })

    // Function to update positions based on zoom
    const updatePositions = (transform: d3.ZoomTransform) => {
      const scale = transform.k
      const viewCenterX = (width / 2 - transform.x) / scale
      const viewCenterY = (height / 2 - transform.y) / scale

      // Spread factor increases with zoom
      const spreadFactor = Math.max(0, (scale - 1) * 1.5)

      graphData.nodes.forEach(node => {
        if (spreadFactor === 0) {
          // At base zoom, use home positions
          node.x = node.baseX!
          node.y = node.baseY!
        } else {
          // Calculate distance from view center
          const dx = node.baseX! - viewCenterX
          const dy = node.baseY! - viewCenterY
          const dist = Math.sqrt(dx * dx + dy * dy)

          // Nodes near center spread outward, far nodes pushed further
          const pushAmount = spreadFactor * 20
          const angle = Math.atan2(dy, dx)

          if (dist < 200) {
            // Near center: spread out
            node.x = node.baseX! + Math.cos(angle) * pushAmount * (1 + dist / 100)
            node.y = node.baseY! + Math.sin(angle) * pushAmount * (1 + dist / 100)
          } else {
            // Far from center: push away more
            node.x = node.baseX! + Math.cos(angle) * pushAmount * 2
            node.y = node.baseY! + Math.sin(angle) * pushAmount * 2
          }
        }
      })

      // Update DOM
      container.selectAll<SVGCircleElement, Node>('.nodes circle')
        .attr('cx', d => d.x!)
        .attr('cy', d => d.y!)

      container.selectAll<SVGGElement, Node>('.labels g')
        .attr('transform', d => `translate(${d.x}, ${d.y! + d.size + 8})`)

      container.selectAll<SVGTextElement, Node>('.node-emojis text')
        .attr('x', d => d.x!)
        .attr('y', d => d.y!)

      container.selectAll<SVGLineElement, Edge>('.links line')
        .attr('x1', d => (typeof d.source === 'string' ? graphData.nodes.find(n => n.id === d.source)?.x : (d.source as Node).x) || 0)
        .attr('y1', d => (typeof d.source === 'string' ? graphData.nodes.find(n => n.id === d.source)?.y : (d.source as Node).y) || 0)
        .attr('x2', d => (typeof d.target === 'string' ? graphData.nodes.find(n => n.id === d.target)?.x : (d.target as Node).x) || 0)
        .attr('y2', d => (typeof d.target === 'string' ? graphData.nodes.find(n => n.id === d.target)?.y : (d.target as Node).y) || 0)

      // Update cluster labels
      const clusterCentersUpdate: { [key: number]: { x: number, y: number, count: number } } = {}
      graphData.nodes.forEach(node => {
        if (!clusterCentersUpdate[node.cluster_id]) {
          clusterCentersUpdate[node.cluster_id] = { x: 0, y: 0, count: 0 }
        }
        clusterCentersUpdate[node.cluster_id].x += node.x!
        clusterCentersUpdate[node.cluster_id].y += node.y!
        clusterCentersUpdate[node.cluster_id].count++
      })

      container.selectAll<SVGTextElement, { id: number }>('.cluster-labels text')
        .attr('x', d => clusterCentersUpdate[d.id] ? clusterCentersUpdate[d.id].x / clusterCentersUpdate[d.id].count : 0)
        .attr('y', d => clusterCentersUpdate[d.id] ? clusterCentersUpdate[d.id].y / clusterCentersUpdate[d.id].count : 0)
    }

    // Add zoom behavior with semantic zoom levels and dynamic spreading
    const zoom = d3.zoom<SVGSVGElement, unknown>()
      .scaleExtent([0.1, 4])
      .on('zoom', (event) => {
        container.attr('transform', event.transform)

        const scale = event.transform.k

        // Semantic zoom: show/hide labels based on zoom level
        container.selectAll('.labels g')
          .style('opacity', scale > 1.3 ? 1 : 0)

        // Only adjust cluster label opacity if not in focus mode
        if (!focusModeRef.current) {
          container.selectAll('.cluster-labels text')
            .style('opacity', scale < 1.2 ? 1 : 0.3)
        }
        // Counter-scale text so it stays readable when zoomed out
        container.selectAll('.cluster-labels text')
          .attr('font-size', `${24 / scale}px`)

        // Counter-scale emojis too
        container.selectAll('.node-emojis text')
          .attr('font-size', (d: Node) => `${Math.max(d.size * 0.8, 8) / scale}px`)

        // Update positions based on zoom (Prezi-style spreading)
        updatePositions(event.transform)
      })

    svg.call(zoom)
    zoomRef.current = zoom

    // Fit to view - no physics, static layout
    const padding = 50
    const minX = Math.min(...graphData.nodes.map(n => n.x! - n.size)) - padding
    const maxX = Math.max(...graphData.nodes.map(n => n.x! + n.size)) + padding
    const minY = Math.min(...graphData.nodes.map(n => n.y! - n.size)) - padding
    const maxY = Math.max(...graphData.nodes.map(n => n.y! + n.size)) + padding

    const scale = Math.min(width / (maxX - minX), height / (maxY - minY), 1) * 0.9
    const graphCenterX = (minX + maxX) / 2
    const graphCenterY = (minY + maxY) / 2

    svg.call(
      zoom.transform,
      d3.zoomIdentity
        .translate(width / 2, height / 2)
        .scale(scale)
        .translate(-graphCenterX, -graphCenterY)
    )

    // Draw edges with static positions
    const links = container.append('g')
      .attr('class', 'links')
      .selectAll('line')
      .data(graphData.edges)
      .join('line')
      .attr('stroke', '#555')
      .attr('stroke-opacity', d => Math.min(d.weight * 0.8, 0.6))
      .attr('stroke-width', d => Math.max(d.weight * 2, 0.5))
      .attr('x1', d => {
        const source = graphData.nodes.find(n => n.id === (typeof d.source === 'string' ? d.source : d.source.id))
        return source?.x || 0
      })
      .attr('y1', d => {
        const source = graphData.nodes.find(n => n.id === (typeof d.source === 'string' ? d.source : d.source.id))
        return source?.y || 0
      })
      .attr('x2', d => {
        const target = graphData.nodes.find(n => n.id === (typeof d.target === 'string' ? d.target : d.target.id))
        return target?.x || 0
      })
      .attr('y2', d => {
        const target = graphData.nodes.find(n => n.id === (typeof d.target === 'string' ? d.target : d.target.id))
        return target?.y || 0
      })

    // Draw nodes with static positions
    const nodes = container.append('g')
      .attr('class', 'nodes')
      .selectAll('circle')
      .data(graphData.nodes)
      .join('circle')
      .attr('r', d => d.size)
      .attr('fill', d => d.color)
      .attr('stroke', '#fff')
      .attr('stroke-width', 1.5)
      .attr('cursor', 'pointer')
      .attr('cx', d => d.x!)
      .attr('cy', d => d.y!)
      .on('click', function(_, d) {
        setSelectedNode(d)

        // Highlight selected node
        container.selectAll('.nodes circle')
          .attr('stroke', '#fff')
          .attr('stroke-width', 1.5)
          .attr('filter', null)

        d3.select(this)
          .attr('stroke', '#fff')
          .attr('stroke-width', 3)
          .attr('filter', 'drop-shadow(0 0 10px rgba(255,255,255,0.8))')

        // Zoom to clicked node - use BASE position
        if (zoomRef.current) {
          svg.transition()
            .duration(500)
            .call(
              zoomRef.current.transform,
              d3.zoomIdentity
                .translate(width / 2, height / 2)
                .scale(2.5)
                .translate(-d.baseX!, -d.baseY!)
            )
        }
      })
      .on('mouseenter', (_, d) => setHoveredNode(d))
      .on('mouseleave', () => setHoveredNode(null))

    // Add emojis to nodes
    container.append('g')
      .attr('class', 'node-emojis')
      .selectAll('text')
      .data(graphData.nodes)
      .join('text')
      .attr('x', d => d.x!)
      .attr('y', d => d.y!)
      .text(d => getEmojiForNode(d))
      .attr('font-size', d => Math.max(d.size * 0.8, 8) + 'px')
      .attr('text-anchor', 'middle')
      .attr('dominant-baseline', 'central')
      .attr('pointer-events', 'none')

    // Add cluster labels (visible when zoomed out)
    const clusterCenters: { [key: number]: { x: number, y: number, count: number } } = {}
    graphData.nodes.forEach(node => {
      if (!clusterCenters[node.cluster_id]) {
        clusterCenters[node.cluster_id] = { x: 0, y: 0, count: 0 }
      }
      clusterCenters[node.cluster_id].x += node.x!
      clusterCenters[node.cluster_id].y += node.y!
      clusterCenters[node.cluster_id].count++
    })

    const clusterLabelData = graphData.clusters.map(cluster => ({
      id: cluster.id,
      name: cluster.keywords[0] || `Cluster ${cluster.id}`,
      x: clusterCenters[cluster.id] ? clusterCenters[cluster.id].x / clusterCenters[cluster.id].count : 0,
      y: clusterCenters[cluster.id] ? clusterCenters[cluster.id].y / clusterCenters[cluster.id].count : 0,
      color: cluster.color
    }))

    const clusterLabels = container.append('g')
      .attr('class', 'cluster-labels')
      .selectAll('text')
      .data(clusterLabelData)
      .join('text')
      .attr('x', d => d.x)
      .attr('y', d => d.y)
      .text(d => d.name)
      .attr('font-size', '24px')
      .attr('font-weight', 'bold')
      .attr('fill', d => d.color)
      .attr('text-anchor', 'middle')
      .attr('dominant-baseline', 'middle')
      .attr('pointer-events', 'none')
      .attr('stroke', '#000')
      .attr('stroke-width', '3px')
      .attr('paint-order', 'stroke')
      .style('text-shadow', '0 0 10px rgba(0,0,0,0.8), 0 2px 4px rgba(0,0,0,0.9)')

    // Add labels for ALL nodes (visible when zoomed in)
    const labels = container.append('g')
      .attr('class', 'labels')
      .selectAll('g')
      .data(graphData.nodes)
      .join('g')
      .attr('transform', d => `translate(${d.x}, ${d.y! + d.size + 8})`)
      .attr('pointer-events', 'none')
      .style('opacity', 0) // Start hidden, show when zoomed in

    // Background for readability
    labels.append('rect')
      .attr('fill', 'rgba(10, 10, 15, 0.85)')
      .attr('rx', 3)
      .attr('x', d => {
        const text = d.label.length > 25 ? d.label.slice(0, 22) + '...' : d.label
        return -text.length * 3.2
      })
      .attr('y', -8)
      .attr('width', d => {
        const text = d.label.length > 25 ? d.label.slice(0, 22) + '...' : d.label
        return text.length * 6.4
      })
      .attr('height', 16)

    // Text
    labels.append('text')
      .text(d => d.label.length > 25 ? d.label.slice(0, 22) + '...' : d.label)
      .attr('font-size', '10px')
      .attr('fill', '#fff')
      .attr('text-anchor', 'middle')
      .attr('dominant-baseline', 'middle')

  }, [graphData])

  // Apply focus mode filtering
  useEffect(() => {
    if (!graphData || !svgRef.current) return

    const svg = d3.select(svgRef.current)
    const container = svg.select('g')

    if (focusMode && selectedNode) {
      const connectedIds = getConnectedNodeIds(selectedNode.id)

      // Fade unconnected nodes
      container.selectAll<SVGCircleElement, Node>('.nodes circle')
        .style('opacity', d => connectedIds.has(d.id) ? 1 : 0.1)

      container.selectAll<SVGTextElement, Node>('.node-emojis text')
        .style('opacity', d => connectedIds.has(d.id) ? 1 : 0.1)

      container.selectAll<SVGGElement, Node>('.labels g')
        .style('opacity', d => connectedIds.has(d.id) ? 1 : 0)

      // Fade unconnected edges
      container.selectAll<SVGLineElement, Edge>('.links line')
        .style('opacity', d => {
          const sourceId = typeof d.source === 'string' ? d.source : d.source.id
          const targetId = typeof d.target === 'string' ? d.target : d.target.id
          return (sourceId === selectedNode.id || targetId === selectedNode.id) ? 0.8 : 0.05
        })

      // Fade cluster labels for unconnected clusters
      const connectedClusterIds = new Set<number>()
      graphData.nodes.forEach(node => {
        if (connectedIds.has(node.id)) {
          connectedClusterIds.add(node.cluster_id)
        }
      })
      container.selectAll<SVGTextElement, { id: number }>('.cluster-labels text')
        .style('opacity', d => connectedClusterIds.has(d.id) ? 0.8 : 0.1)
    } else {
      // Reset to normal
      container.selectAll('.nodes circle')
        .style('opacity', 1)

      container.selectAll('.node-emojis text')
        .style('opacity', 1)

      container.selectAll('.links line')
        .style('opacity', d => Math.min((d as Edge).weight * 0.8, 0.6))

      container.selectAll('.cluster-labels text')
        .style('opacity', 1)
    }
  }, [focusMode, selectedNode, graphData])

  if (loading) {
    return (
      <div className="loading">
        <h1>Loading Mycelica...</h1>
        <p>Analyzing your conversations</p>
      </div>
    )
  }

  return (
    <div className="app">
      <svg ref={svgRef} className="graph" />

      {/* Search bar */}
      <div className="search-bar">
        <input
          type="text"
          placeholder="Search conversations..."
          value={searchQuery}
          onChange={(e) => setSearchQuery(e.target.value)}
        />
      </div>

      {/* Hover tooltip */}
      {hoveredNode && !selectedNode && (
        <div className="tooltip">
          <h3>{hoveredNode.title}</h3>
          <p>{hoveredNode.message_count} messages</p>
          <p className="keywords">
            {hoveredNode.keywords.slice(0, 3).join(', ')}
          </p>
        </div>
      )}

      {/* Cluster legend */}
      {graphData && (
        <div className="legend">
          <h3>Clusters</h3>
          {graphData.clusters.map(cluster => (
            <div key={cluster.id} className="cluster-group">
              <div
                className="legend-item clickable"
                onClick={() => setExpandedCluster(expandedCluster === cluster.id ? null : cluster.id)}
              >
                <span className="expand-icon">
                  {expandedCluster === cluster.id ? '▼' : '▶'}
                </span>
                <span
                  className="color-dot"
                  style={{ backgroundColor: cluster.color }}
                />
                <span className="cluster-name">
                  {cluster.name.slice(0, 20)} ({cluster.count})
                </span>
              </div>
              {expandedCluster === cluster.id && (
                <div className="cluster-nodes">
                  {graphData.nodes
                    .filter(n => n.cluster_id === cluster.id)
                    .sort((a, b) => b.message_count - a.message_count)
                    .map(node => (
                      <div
                        key={node.id}
                        className="node-item clickable"
                        onClick={() => {
                          setSelectedNode(node)
                          zoomToCluster(cluster.id)
                        }}
                      >
                        <span className="node-title">
                          {node.label.slice(0, 30)}
                        </span>
                        <span className="node-count">
                          {node.message_count}
                        </span>
                      </div>
                    ))
                  }
                </div>
              )}
            </div>
          ))}
        </div>
      )}

      {/* Selected node panel */}
      {selectedNode && (
        <div className="detail-panel">
          <button className="close-btn" onClick={() => { setSelectedNode(null); setFocusMode(false) }}>x</button>
          <h2>{selectedNode.title}</h2>
          <div className="meta">
            <span>{selectedNode.message_count} messages</span>
            <span>{new Date(selectedNode.created_at).toLocaleDateString()}</span>
          </div>
          <div className="keywords">
            <strong>Topics:</strong> {selectedNode.keywords.join(', ')}
          </div>
          <button
            className={`action-btn ${focusMode ? 'primary' : ''}`}
            onClick={() => setFocusMode(!focusMode)}
          >
            {focusMode ? 'Exit Focus Mode' : 'Focus Mode'}
          </button>
          <button className="action-btn primary" onClick={() => {
            window.open(`https://claude.ai/chat/${selectedNode.id}`, '_blank')
          }} style={{ marginTop: '8px' }}>
            Open in Claude
          </button>
        </div>
      )}

      {/* Stats */}
      {graphData && (
        <div className="stats">
          <span>{graphData.nodes.length} conversations</span>
          <span>{graphData.edges.length} connections</span>
          <span>{graphData.clusters.length} clusters</span>
        </div>
      )}
    </div>
  )
}

export default App
