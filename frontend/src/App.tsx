import { useEffect, useState, useRef } from 'react'
import * as d3 from 'd3'
import './App.css'
import { getEmojiForNode } from './emojiMatcher'

interface Node {
  id: string
  label: string
  title: string
  cluster_id: number
  color: string
  size: number
  message_count?: number
  galaxy_count?: number  // For universe nodes
  pair_count?: number    // For galaxy/topic nodes
  description?: string   // For universe nodes
  sample_topics?: string[]  // For universe nodes
  zoom_level?: string    // universe, galaxy, topic, message
  created_at?: string
  keywords?: string[]
  conversation_id?: string  // For topic/message nodes
  conversation_title?: string  // For topic/message nodes
  timestamp?: string  // For topic/message nodes
  parent_galaxy?: number  // For topic/message nodes
  x?: number
  y?: number
  baseX?: number  // Home position
  baseY?: number
  fx?: number | null
  fy?: number | null
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
  
  // API Key management
  const [showApiKeyModal, setShowApiKeyModal] = useState(false)
  const [apiKey, setApiKey] = useState('')
  const [hasApiKey, setHasApiKey] = useState(false)
  const [apiKeyPreview, setApiKeyPreview] = useState('')
  const [apiKeyLoading, setApiKeyLoading] = useState(false)
  const [apiKeyError, setApiKeyError] = useState('')
  const [analyzing, setAnalyzing] = useState(false)
  const [analysisError, setAnalysisError] = useState('')
  const [regeneratingTags, setRegeneratingTags] = useState(false)
  const [analyzingMessages, setAnalyzingMessages] = useState(false)
  const [analysisStatus, setAnalysisStatus] = useState<{total: number, analyzed: number, remaining: number} | null>(null)

  // Hierarchical navigation
  const [currentZoomLevel, setCurrentZoomLevel] = useState<'universe' | 'galaxy' | 'topic' | 'message'>('universe')
  const [currentParentId, setCurrentParentId] = useState<string | null>(null)
  const [navigationStack, setNavigationStack] = useState<Array<{level: string, parentId: string | null, label: string}>>([])

  // Keep ref in sync with state
  useEffect(() => {
    focusModeRef.current = focusMode
  }, [focusMode])

  // Check API key status on load
  useEffect(() => {
    fetch('http://localhost:8000/api-key/status')
      .then(res => res.json())
      .then(data => {
        setHasApiKey(data.has_key)
        setApiKeyPreview(data.key_preview || '')
      })
      .catch(err => {
        console.error('Failed to check API key status:', err)
      })
  }, [])

  // API key management functions
  const handleSubmitApiKey = async () => {
    if (!apiKey.trim()) {
      setApiKeyError('API key is required')
      return
    }

    setApiKeyLoading(true)
    setApiKeyError('')

    try {
      const response = await fetch('http://localhost:8000/api-key', {
        method: 'POST',
        headers: {
          'Content-Type': 'application/json',
        },
        body: JSON.stringify({ api_key: apiKey }),
      })

      if (!response.ok) {
        const errorData = await response.json()
        throw new Error(errorData.detail || 'Failed to set API key')
      }

      const result = await response.json()
      setHasApiKey(true)
      setApiKeyPreview(result.key_preview)
      setShowApiKeyModal(false)
      setApiKey('')
    } catch (error) {
      setApiKeyError(error instanceof Error ? error.message : 'Failed to set API key')
    } finally {
      setApiKeyLoading(false)
    }
  }

  const handleClearApiKey = async () => {
    setApiKeyLoading(true)
    
    try {
      await fetch('http://localhost:8000/api-key', {
        method: 'DELETE',
      })
      
      setHasApiKey(false)
      setApiKeyPreview('')
      setShowApiKeyModal(false)
    } catch (error) {
      console.error('Failed to clear API key:', error)
    } finally {
      setApiKeyLoading(false)
    }
  }

  // Analysis function
  const handleRunAnalysis = async () => {
    setAnalyzing(true)
    setAnalysisError('')

    try {
      const response = await fetch('http://localhost:8000/analyze', {
        method: 'POST',
      })

      if (!response.ok) {
        throw new Error('Analysis failed')
      }

      const result = await response.json()
      console.log('Analysis completed:', result)

      // Refresh the current view data
      await fetchZoomLevelData(currentZoomLevel, currentParentId)
    } catch (error) {
      setAnalysisError(error instanceof Error ? error.message : 'Analysis failed')
    } finally {
      setAnalyzing(false)
    }
  }

  const handleRegenerateTags = async () => {
    if (!hasApiKey) {
      setAnalysisError('API key required for AI tag generation. Click the key button to set it.')
      return
    }

    setRegeneratingTags(true)
    setAnalysisError('')

    try {
      const response = await fetch('http://localhost:8000/regenerate-tags', {
        method: 'POST',
      })

      if (!response.ok) {
        const errorData = await response.json()
        throw new Error(errorData.detail || 'Tag regeneration failed')
      }

      const result = await response.json()
      console.log('Tags regenerated:', result)

      // Refresh the current view data to show new tags
      await fetchZoomLevelData(currentZoomLevel, currentParentId)
    } catch (error) {
      setAnalysisError(error instanceof Error ? error.message : 'Tag regeneration failed')
    } finally {
      setRegeneratingTags(false)
    }
  }

  // Fetch analysis status (how many messages have been analyzed)
  const fetchAnalysisStatus = async () => {
    try {
      const response = await fetch('http://localhost:8000/analyze-messages/status')
      if (response.ok) {
        const data = await response.json()
        setAnalysisStatus(data)
      }
    } catch (err) {
      console.error('Failed to fetch analysis status:', err)
    }
  }

  // Analyze all messages with AI
  const handleAnalyzeMessages = async () => {
    if (!hasApiKey) {
      setAnalysisError('API key required for AI message analysis. Click the key button to set it.')
      return
    }

    setAnalyzingMessages(true)
    setAnalysisError('')

    try {
      const response = await fetch('http://localhost:8000/analyze-messages', {
        method: 'POST',
      })

      if (!response.ok) {
        const errorData = await response.json()
        throw new Error(errorData.detail || 'Message analysis failed')
      }

      const result = await response.json()
      console.log('Messages analyzed:', result)

      // Update status
      await fetchAnalysisStatus()

      // Re-run the main analysis to pick up the new data
      await handleRunAnalysis()

      // Refresh the current view
      await fetchZoomLevelData(currentZoomLevel, currentParentId)
    } catch (error) {
      setAnalysisError(error instanceof Error ? error.message : 'Message analysis failed')
    } finally {
      setAnalyzingMessages(false)
    }
  }

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

  // Zoom to cluster function - use current positions for targeting
  const zoomToCluster = (clusterId: number) => {
    if (!graphData || !svgRef.current || !zoomRef.current) return

    const clusterNodes = graphData.nodes.filter(n => n.cluster_id === clusterId)
    if (clusterNodes.length === 0) return

    const hasPositions = clusterNodes.every(n => n.x !== undefined && n.y !== undefined)
    if (!hasPositions) return

    // Use current positions for targeting
    const xs = clusterNodes.map(n => n.x!)
    const ys = clusterNodes.map(n => n.y!)

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

  // Hierarchical data fetching function
  const fetchZoomLevelData = async (level: string, parentId: string | null = null) => {
    setLoading(true)
    try {
      let url = `http://localhost:8000/graph/zoom/${level}`
      if (parentId) {
        url += `?parent_id=${parentId}`
      }
      
      const response = await fetch(url)
      if (!response.ok) {
        const errorData = await response.json()
        throw new Error(errorData.detail || 'Failed to fetch data')
      }
      
      const data = await response.json()
      
      // Calculate similarity-based edges between nodes
      const nodes = data.nodes || []
      const similarityEdges: Edge[] = []

      // Compare each pair of nodes for similarity
      for (let i = 0; i < nodes.length; i++) {
        for (let j = i + 1; j < nodes.length; j++) {
          const nodeA = nodes[i]
          const nodeB = nodes[j]

          let similarity = 0

          // Same cluster = strong connection
          if (nodeA.cluster_id === nodeB.cluster_id) {
            similarity += 0.6
          }

          // Shared keywords = additional connection strength
          const keywordsA = new Set(nodeA.keywords || [])
          const keywordsB = nodeB.keywords || []
          const sharedKeywords = keywordsB.filter((k: string) => keywordsA.has(k)).length
          if (sharedKeywords > 0) {
            similarity += Math.min(sharedKeywords * 0.2, 0.6)
          }

          // Only create edge if similarity is above threshold
          if (similarity >= 0.25) {
            similarityEdges.push({
              source: nodeA.id,
              target: nodeB.id,
              weight: similarity
            })
          }
        }
      }

      // Transform API format to match existing interface
      const transformedData = {
        nodes: nodes,
        edges: similarityEdges,
        clusters: nodes.map((node: Node) => ({
          id: node.cluster_id,
          name: node.label,
          color: node.color,
          count: level === 'universe' ? node.galaxy_count : node.pair_count,
          keywords: node.keywords || []
        }))
      }
      
      setGraphData(transformedData)
      setLoading(false)
    } catch (err) {
      console.error('Failed to load graph data:', err)
      setGraphData({ nodes: [], edges: [], clusters: [] })
      setLoading(false)
    }
  }

  // Navigation functions
  const drillDown = (node: Node) => {
    // Add current level to navigation stack with meaningful label
    setNavigationStack(prev => [...prev, {
      level: currentZoomLevel,
      parentId: currentParentId,
      label: node.label || currentZoomLevel.charAt(0).toUpperCase() + currentZoomLevel.slice(1)
    }])

    // Determine next zoom level and parent ID
    if (currentZoomLevel === 'universe') {
      setCurrentZoomLevel('galaxy')
      // Use cluster_ids array if available (merged universes), otherwise single cluster_id
      const clusterIds = (node as any).cluster_ids || [node.cluster_id]
      setCurrentParentId(clusterIds.join(',')) // Filter galaxies by all merged universe cluster_ids
    } else if (currentZoomLevel === 'galaxy') {
      setCurrentZoomLevel('topic')
      setCurrentParentId(node.cluster_id.toString())
    } else if (currentZoomLevel === 'topic') {
      setCurrentZoomLevel('message')
      setCurrentParentId(node.cluster_id.toString())
    }
  }

  const navigateBack = () => {
    if (navigationStack.length > 0) {
      const lastLevel = navigationStack[navigationStack.length - 1]
      setNavigationStack(prev => prev.slice(0, -1))
      setCurrentZoomLevel(lastLevel.level as any)
      setCurrentParentId(lastLevel.parentId)
    }
  }

  // Fetch data when zoom level or parent changes
  useEffect(() => {
    fetchZoomLevelData(currentZoomLevel, currentParentId)
  }, [currentZoomLevel, currentParentId])

  useEffect(() => {
    if (!graphData || !svgRef.current) return

    const svg = d3.select(svgRef.current)
    const width = window.innerWidth
    const height = window.innerHeight

    // Set SVG dimensions to prevent D3 zoom errors
    svg
      .attr('width', width)
      .attr('height', height)

    // Clear previous
    svg.selectAll('*').remove()

    // Create container for zoom
    const container = svg.append('g')

    // Initialize node positions in spiral by cluster
    // For topic level, use conversation_id for spreading (since all have same parent_galaxy)
    // Otherwise use cluster_id or parent_galaxy
    const clusterGroups: { [key: string]: Node[] } = {}
    graphData.nodes.forEach((node, index) => {
      let groupId: string
      if (currentZoomLevel === 'topic') {
        // At topic level, use conversation_id or unique index to spread cards
        groupId = node.conversation_id || `topic_${index}`
      } else {
        groupId = String(node.cluster_id ?? node.parent_galaxy ?? 0)
      }
      if (!clusterGroups[groupId]) {
        clusterGroups[groupId] = []
      }
      clusterGroups[groupId].push(node)
    })

    const clusterIds = Object.keys(clusterGroups)
      .sort((a, b) => clusterGroups[b].length - clusterGroups[a].length)

    // Ensure we have valid dimensions
    if (!width || !height || width <= 0 || height <= 0) {
      console.warn('Invalid dimensions, using defaults')
      return
    }

    // Sort clusters by total size (larger clusters go closer to center)
    const clusterSizes = clusterIds.map(clusterId => {
      const nodes = clusterGroups[clusterId] || []
      const totalSize = nodes.reduce((sum, n) => sum + (n.size || 0), 0)
      return { clusterId, totalSize, nodes }
    }).sort((a, b) => b.totalSize - a.totalSize)

    // Initial positions in spiral - store as base positions
    clusterSizes.forEach(({ nodes }, i) => {
      // Sort nodes within cluster by size (larger nodes closer to cluster center)
      nodes.sort((a, b) => (b.size || 0) - (a.size || 0))

      const angle = i * 2.4
      // Larger clusters closer to center (smaller radius)
      const radius = 120 + i * 40  // Increased spacing between clusters
      const clusterX = width / 2 + radius * Math.cos(angle)
      const clusterY = height / 2 + radius * Math.sin(angle)

      // Initial positioning: place nodes in circle around cluster center
      // Account for node sizes to create more spacing
      const baseRadius = Math.min(60 + nodes.length * 5, 180)
      nodes.forEach((node, j) => {
        const nodeAngle = (j / nodes.length) * 2 * Math.PI
        const nodeRadius = baseRadius + (node.size || 1) * 0.5

        const initialX = clusterX + nodeRadius * Math.cos(nodeAngle)
        const initialY = clusterY + nodeRadius * Math.sin(nodeAngle)

        node.baseX = isNaN(initialX) ? width / 2 : initialX
        node.baseY = isNaN(initialY) ? height / 2 : initialY
        node.x = node.baseX
        node.y = node.baseY
      })
    })

    // Use D3 force simulation for proper collision detection across ALL nodes
    const allNodes = graphData.nodes
    const baseGap = 15 // Minimum gap between node edges

    // Create a force simulation for collision detection
    const simulation = d3.forceSimulation(allNodes as d3.SimulationNodeDatum[])
      .force('collision', d3.forceCollide<Node>()
        .radius(d => (d.size || 10) + baseGap) // Node radius + gap
        .strength(1) // Full strength collision
        .iterations(4) // Multiple iterations per tick for accuracy
      )
      .force('cluster', d3.forceRadial<Node>(
        // Keep nodes near their cluster center
        (d, i) => {
          let groupId: string
          if (currentZoomLevel === 'topic') {
            groupId = d.conversation_id || `topic_${i}`
          } else {
            groupId = String(d.cluster_id ?? d.parent_galaxy ?? 0)
          }
          const clusterInfo = clusterSizes.find(c => c.clusterId === groupId)
          const clusterIndex = clusterInfo ? clusterSizes.indexOf(clusterInfo) : 0
          return 120 + clusterIndex * 40
        },
        width / 2,
        height / 2
      ).strength(0.05)) // Gentle pull toward cluster position
      .stop() // Don't auto-run

    // Run simulation synchronously for stable initial positions
    const numIterations = 300
    for (let i = 0; i < numIterations; i++) {
      simulation.tick()
    }

    // Store final positions as base positions
    allNodes.forEach(node => {
      node.baseX = node.x!
      node.baseY = node.y!
    })

    // Calculate graph center from base positions (set by force simulation)
    const graphCenterX = graphData.nodes.reduce((sum, n) => sum + (n.baseX || 0), 0) / graphData.nodes.length
    const graphCenterY = graphData.nodes.reduce((sum, n) => sum + (n.baseY || 0), 0) / graphData.nodes.length

    // Find min/max node sizes for normalization (constant, doesn't change with zoom)
    const sizes = graphData.nodes.map(n => n.size || 10)
    const minSize = Math.min(...sizes)
    const maxSize = Math.max(...sizes)
    const sizeRange = maxSize - minSize || 1

    // Group nodes by importance tier (3 tiers - large center area + 2 outer rings)
    const numTiers = 3
    const tiers: Node[][] = Array.from({ length: numTiers }, () => [])
    graphData.nodes.forEach(node => {
      const normalizedSize = ((node.size || 10) - minSize) / sizeRange
      // Top 50% goes to center (tier 0), rest distributed across 2 rings
      let tier
      if (normalizedSize >= 0.5) {
        tier = 0 // Large nodes fill the center
      } else if (normalizedSize >= 0.2) {
        tier = 1 // Medium nodes in middle ring
      } else {
        tier = 2 // Small nodes in outer ring
      }
      tiers[tier].push(node)
    })

    // Sort nodes within each tier by cluster_id to group similar topics together
    // Then by size within cluster for visual hierarchy
    tiers.forEach(tierNodes => {
      tierNodes.sort((a, b) => {
        // Primary sort: by cluster/galaxy or conversation
        let aGroup: string, bGroup: string
        if (currentZoomLevel === 'topic') {
          aGroup = a.conversation_id || ''
          bGroup = b.conversation_id || ''
        } else {
          aGroup = String(a.cluster_id ?? a.parent_galaxy ?? 0)
          bGroup = String(b.cluster_id ?? b.parent_galaxy ?? 0)
        }
        if (aGroup !== bGroup) {
          return aGroup.localeCompare(bGroup)
        }
        // Secondary sort: by size within cluster (larger first)
        return (b.size || 0) - (a.size || 0)
      })
    })

    // Target ring radii - 2 rings for universe view, 3 rings for galaxy
    const isUniverse = currentZoomLevel === 'universe'
    const centerRadius = isUniverse ? 150 : 800  // Moderately larger center
    const tierBaseRadii = isUniverse
      ? [centerRadius, 300, 300]  // 2 rings: center + outer
      : [centerRadius, 1100, 1500]  // 3 rings for galaxy - expanded outer ring

    // Pre-calculate base angles for each node (constant, doesn't change with zoom)
    const nodeAngles = new Map<string, number>()
    const goldenAngle = Math.PI * (3 - Math.sqrt(5))

    tiers.forEach((tierNodes, tierIndex) => {
      tierNodes.forEach((node, nodeIndex) => {
        if (tierIndex === 0) {
          nodeAngles.set(node.id, nodeIndex * goldenAngle)
        } else {
          const angleOffset = tierIndex * 0.3
          nodeAngles.set(node.id, (nodeIndex / tierNodes.length) * 2 * Math.PI + angleOffset)
        }
      })
    })

    // Function to update positions based on zoom
    const updatePositions = (transform: d3.ZoomTransform) => {
      const scale = transform.k

      // Base spread factor from zoom level
      let baseSpread = 1
      if (scale < 1) {
        const rawSpread = Math.sqrt(1 / scale)
        baseSpread = Math.min(rawSpread, 2.5)
      } else if (scale > 1) {
        baseSpread = 1 + (scale - 1) * 0.15
      }

      // Sticky notes - counter-scale to maintain constant visual size
      // 0.7/scale keeps notes at 70% of full compensation (smaller but still readable)
      // This ensures notes NEVER move relative to each other
      const noteScale = 0.7 / scale

      // Position each tier's nodes
      tiers.forEach((tierNodes, tierIndex) => {
        if (tierNodes.length === 0) return

        const ringRadius = tierBaseRadii[tierIndex] * baseSpread

        if (tierIndex === 0) {
          // Center tier: uniform distribution using expanding concentric rings
          // Ring 0 = center (1 node), Ring 1 = 6 nodes, Ring 2 = 12 nodes, etc.
          // This fills the area evenly from center outward

          const rings: Node[][] = []
          let nodeIndex = 0
          let ringNum = 0

          while (nodeIndex < tierNodes.length) {
            // Ring 0 has 1 node, subsequent rings have 6 * ringNum nodes
            const nodesInRing = ringNum === 0 ? 1 : Math.min(6 * ringNum, tierNodes.length - nodeIndex)
            rings.push(tierNodes.slice(nodeIndex, nodeIndex + nodesInRing))
            nodeIndex += nodesInRing
            ringNum++
          }

          // Position each ring
          const totalRings = rings.length
          rings.forEach((ringNodes, ringIdx) => {
            if (ringIdx === 0 && ringNodes.length === 1) {
              // Center node
              ringNodes[0].x = graphCenterX
              ringNodes[0].y = graphCenterY
            } else {
              // Distribute evenly around the ring
              // Radius scales linearly from center to edge
              const radiusFraction = totalRings > 1 ? ringIdx / (totalRings - 1) : 0.5
              const radius = ringRadius * radiusFraction * 0.95

              ringNodes.forEach((node, posInRing) => {
                const angleOffset = ringIdx * 0.15  // Slight rotation per ring
                const angle = (posInRing / ringNodes.length) * 2 * Math.PI + angleOffset
                node.x = graphCenterX + Math.cos(angle) * radius
                node.y = graphCenterY + Math.sin(angle) * radius
              })
            }
          })
        } else {
          // Outer tiers: rings
          tierNodes.forEach((node) => {
            const angle = nodeAngles.get(node.id) || 0
            const idHash = node.id.split('').reduce((a, c) => a + c.charCodeAt(0), 0)
            const radiusVariation = (idHash % 30) - 15
            const nodeRadius = ringRadius + radiusVariation

            node.x = graphCenterX + Math.cos(angle) * nodeRadius
            node.y = graphCenterY + Math.sin(angle) * nodeRadius
          })
        }
      })

      // Collision detection for topic view - prevent note containers from overlapping
      if (isTopicLevel) {
        // Group nodes by conversation to find anchor nodes and container sizes
        const convMap = new Map<string, Node[]>()
        graphData.nodes.forEach(node => {
          const convId = node.conversation_id || node.id
          if (!convMap.has(convId)) convMap.set(convId, [])
          convMap.get(convId)!.push(node)
        })

        // Build container bounds for each conversation
        const noteW = 320, noteH = 140, noteGap = 10, padding = 15
        const containers: { id: string, anchor: Node, x: number, y: number, w: number, h: number }[] = []

        convMap.forEach((nodes, convId) => {
          // Sort by some consistent order to find anchor
          const anchor = nodes[0]
          const n = nodes.length
          const containerW = (noteW + padding * 2) * noteScale
          const containerH = (n * noteH + (n - 1) * noteGap + padding * 2) * noteScale
          containers.push({
            id: convId,
            anchor,
            x: anchor.x ?? 0,
            y: anchor.y ?? 0,
            w: containerW,
            h: containerH
          })
        })

        // Iterative separation - push overlapping containers apart
        const margin = 20 * noteScale  // Extra spacing between containers
        for (let iter = 0; iter < 50; iter++) {
          let moved = false
          for (let i = 0; i < containers.length; i++) {
            for (let j = i + 1; j < containers.length; j++) {
              const a = containers[i]
              const b = containers[j]

              // Check rectangular overlap (centered on anchor)
              const aLeft = a.x - a.w / 2, aRight = a.x + a.w / 2
              const aTop = a.y - a.h / 2, aBottom = a.y + a.h / 2
              const bLeft = b.x - b.w / 2, bRight = b.x + b.w / 2
              const bTop = b.y - b.h / 2, bBottom = b.y + b.h / 2

              const overlapX = Math.min(aRight, bRight) - Math.max(aLeft, bLeft) + margin
              const overlapY = Math.min(aBottom, bBottom) - Math.max(aTop, bTop) + margin

              if (overlapX > 0 && overlapY > 0) {
                // Overlap detected - push apart along the smaller overlap axis
                moved = true
                const pushX = overlapX < overlapY
                const dx = b.x - a.x
                const dy = b.y - a.y

                if (pushX) {
                  const push = overlapX / 2 + 1
                  const dir = dx >= 0 ? 1 : -1
                  a.x -= push * dir
                  b.x += push * dir
                } else {
                  const push = overlapY / 2 + 1
                  const dir = dy >= 0 ? 1 : -1
                  a.y -= push * dir
                  b.y += push * dir
                }
              }
            }
          }
          if (!moved) break
        }

        // Apply separated positions back to anchor nodes
        containers.forEach(c => {
          c.anchor.x = c.x
          c.anchor.y = c.y
        })
      }

      // Update DOM - handle both bubbles and sticky notes
      container.selectAll<SVGCircleElement, Node>('.nodes circle')
        .attr('cx', d => d.x!)
        .attr('cy', d => d.y!)

      // Update conversation container positions (notes are children, so they move with container)
      // Scale is applied to container, which scales all children proportionally
      container.selectAll<SVGGElement, any>('.conversation-container')
        .attr('transform', d => {
          const x = d.anchorNode?.x ?? 0
          const y = d.anchorNode?.y ?? 0
          // Apply scale to container - all child notes scale together
          return `translate(${x}, ${y}) scale(${noteScale})`
        })
        .each(function(cardData: any) {
          // Update topic node positions for accurate connection lines
          // Each topic's world position = container position + local offset (scaled)
          const anchorX = cardData.anchorNode?.x ?? 0
          const anchorY = cardData.anchorNode?.y ?? 0
          cardData.topics?.forEach((topic: Node, idx: number) => {
            const localY = idx * (140 + 10) + 15  // noteHeight + noteSpacing + containerPadding
            topic.x = anchorX
            topic.y = anchorY + localY * noteScale
          })
        })

      container.selectAll<SVGGElement, Node>('.labels g')
        .attr('transform', d => `translate(${d.x}, ${d.y! + d.size + 8})`)

      container.selectAll<SVGTextElement, Node>('.node-emojis text')
        .attr('x', d => d.x || 0)
        .attr('y', d => d.y || 0)

      // Update detective string paths
      container.selectAll<SVGPathElement, Edge>('.links path')
        .attr('d', (d, i) => {
          const source = typeof d.source === 'string'
            ? graphData.nodes.find(n => n.id === d.source)
            : d.source as Node
          const target = typeof d.target === 'string'
            ? graphData.nodes.find(n => n.id === d.target)
            : d.target as Node
          if (!source || !target) return ''
          const curvature = 0.1 + (i % 5) * 0.05 * (i % 2 === 0 ? 1 : -1)
          const midX = (source.x! + target.x!) / 2
          const midY = (source.y! + target.y!) / 2
          const dx = target.x! - source.x!
          const dy = target.y! - source.y!
          const ctrlX = midX + (-dy * curvature)
          const ctrlY = midY + (dx * curvature)
          return `M ${source.x} ${source.y} Q ${ctrlX} ${ctrlY} ${target.x} ${target.y}`
        })

    }

    // Track previous scale for zoom direction detection
    let prevScale = 1

    // Add zoom behavior with semantic zoom levels and dynamic spreading
    // Cap zoom at 1.5 for topic level (sticky notes)
    const maxZoom = currentZoomLevel === 'topic' ? 1.5 : 3
    const zoom = d3.zoom<SVGSVGElement, unknown>()
      .scaleExtent([0.1, maxZoom])
      .wheelDelta((event) => -event.deltaY * (event.deltaMode === 1 ? 0.05 : event.deltaMode ? 1 : 0.002) * 0.8)  // Smoother wheel zoom
      .on('zoom', (event) => {
        const scale = event.transform.k
        const isZoomingIn = scale > prevScale

        // Enhanced cursor-following: when zooming in, drift viewport towards cursor
        if (isZoomingIn && event.sourceEvent && svgRef.current) {
          // Get SVG's actual position and size on screen
          const svgRect = svgRef.current.getBoundingClientRect()

          // Get mouse position relative to SVG element
          const clientX = event.sourceEvent.clientX ?? event.sourceEvent.touches?.[0]?.clientX
          const clientY = event.sourceEvent.clientY ?? event.sourceEvent.touches?.[0]?.clientY

          if (clientX !== undefined && clientY !== undefined) {
            const mouseX = clientX - svgRect.left
            const mouseY = clientY - svgRect.top
            const viewportCenterX = svgRect.width / 2
            const viewportCenterY = svgRect.height / 2

            // Calculate drift towards cursor (subtle effect)
            const driftStrength = 0.15  // How much to pull towards cursor (0-1)
            const dx = (mouseX - viewportCenterX) * driftStrength * (scale - prevScale)
            const dy = (mouseY - viewportCenterY) * driftStrength * (scale - prevScale)

            // Apply modified transform with drift
            const modifiedTransform = event.transform.translate(-dx / scale, -dy / scale)
            container.attr('transform', modifiedTransform)
          } else {
            container.attr('transform', event.transform)
          }
        } else {
          container.attr('transform', event.transform)
        }

        prevScale = scale

        // Semantic zoom: show/hide elements based on zoom level and current view
        const _showIndividualNodes = scale > 0.5
        const showTextLabels = scale > 0.2  // Show text when zoomed in past 0.2
        const _showEmojis = scale <= 0.6    // Show emojis when zoomed out (scale ≤ 0.8)

        console.log(`Zoom scale: ${scale.toFixed(2)}, Level: ${currentZoomLevel}`)

        // Emojis/bubbles - growth from 0.2 to 1.0, then capped
        // Below 0.2: compensate fully to stay readable
        // 0.2 to 1.0: growth (1.0x to 2x visual)
        // Above 1.0: cap visual size (compensate for zoom scale)
        let sizeMultiplier: number
        if (scale < 0.2) {
          // Very zoomed out: full compensation to stay readable
          sizeMultiplier = 1 / scale
        } else if (scale < 1) {
          // Zoomed out but not extreme: noticeable growth
          // At 0.2: visual = 1x base
          // At 1.0: visual = 2x base
          const t = (scale - 0.2) / 0.8  // 0 to 1 as scale goes 0.2 to 1.0
          const visualBoost = 1 + t  // 1.0 to 2.0
          sizeMultiplier = (1 / scale) * visualBoost
        } else {
          // Past 1.0: cap visual size by compensating for zoom
          sizeMultiplier = 2 / scale
        }
        // Use sqrt for gentler size curve - compresses differences between small and large nodes
        // Base 14 + sqrt(size) * 3.5 gives: size 10→25px, size 25→32px, size 50→39px, size 100→49px
        const emojiFontSize = (d: Node) => (14 + Math.sqrt(d.size) * 3.5) * sizeMultiplier
        container.selectAll('.node-emojis text')
          .style('opacity', 1)
          .style('font-size', d => `${emojiFontSize(d as Node)}px`)

        // Node circles - always visible, sized to just contain the emoji
        container.selectAll('.nodes circle')
          .style('opacity', 1)
          .attr('r', d => {
            // Bubble radius = emoji font size * 0.6 (emoji is roughly square, radius needs slight padding)
            return emojiFontSize(d as Node) * 0.6
          })

        // Text labels - only when zoomed in, with uniform readable size
        const labelFontSize = 14 / scale  // Counter-scale to stay uniform at 14px visual
        const labelPadding = 12 / scale
        const labelRadius = 8 / scale
        container.selectAll('.labels g')
          .style('opacity', showTextLabels ? 1 : 0)
        container.selectAll('.labels text')
          .attr('font-size', `${labelFontSize}px`)
          .attr('letter-spacing', `${1 / scale}px`)
        container.selectAll<SVGRectElement, Node>('.labels rect')
          .attr('height', labelFontSize * 2)
          .attr('y', -labelFontSize)
          .attr('rx', labelRadius)
          .attr('ry', labelRadius)
          .each(function(this: SVGRectElement) {
            // Get sibling text element to measure width
            const textEl = this.parentElement?.querySelector('text') as SVGTextElement
            if (textEl) {
              const bbox = textEl.getBBox()
              d3.select(this)
                .attr('x', -bbox.width / 2 - labelPadding)
                .attr('width', bbox.width + labelPadding * 2)
            }
          })

        // Detective strings - counter-scale to stay visible when zoomed out
        const lineScale = Math.max(1, 1 / scale)  // Scale up when zoomed out
        container.selectAll('.links path')
          .attr('stroke-width', (d: any) => (0.3 + Math.pow(d.weight, 2.5) * 28) * lineScale)

        // Counter-scale universe notes to stay readable
        const universeNoteScale = 0.7 / scale  // Counter-scale to maintain consistent visual size
        container.selectAll('.universe-notes .sticky-note')
          .attr('transform', (d: any) => {
            const noteWidth = currentZoomLevel === 'universe' ? 280 : 240
            const noteHeight = currentZoomLevel === 'universe' ? 160 : 120
            const x = (d.x || 0) - noteWidth / 2
            const y = (d.y || 0) - noteHeight / 2
            return `translate(${x}, ${y}) scale(${universeNoteScale})`
          })

        // Update positions based on zoom (Prezi-style spreading)
        // Note: Sticky note transforms are handled in updatePositions to support both
        // Node data (old style) and ConversationCard data (topic level)
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

    const initialScale = Math.min(width / (maxX - minX), height / (maxY - minY), 1) * 0.9
    const fitCenterX = (minX + maxX) / 2
    const fitCenterY = (minY + maxY) / 2

    // Create SVG defs for gradients and filters
    const defs = svg.append('defs')

    // Glow filter for bubbles
    const glowFilter = defs.append('filter')
      .attr('id', 'bubble-glow')
      .attr('x', '-50%')
      .attr('y', '-50%')
      .attr('width', '200%')
      .attr('height', '200%')

    glowFilter.append('feGaussianBlur')
      .attr('in', 'SourceAlpha')
      .attr('stdDeviation', '3')
      .attr('result', 'blur')

    glowFilter.append('feFlood')
      .attr('flood-color', 'rgba(255,255,255,0.4)')
      .attr('result', 'color')

    glowFilter.append('feComposite')
      .attr('in', 'color')
      .attr('in2', 'blur')
      .attr('operator', 'in')
      .attr('result', 'glow')

    const glowMerge = glowFilter.append('feMerge')
    glowMerge.append('feMergeNode').attr('in', 'glow')
    glowMerge.append('feMergeNode').attr('in', 'SourceGraphic')

    // Create radial gradients for each unique color
    const uniqueColors = [...new Set(graphData.nodes.map(n => n.color))]
    uniqueColors.forEach((color, i) => {
      const gradient = defs.append('radialGradient')
        .attr('id', `bubble-gradient-${i}`)
        .attr('cx', '35%')
        .attr('cy', '35%')
        .attr('r', '60%')
        .attr('fx', '25%')
        .attr('fy', '25%')

      // Highlight at top-left
      gradient.append('stop')
        .attr('offset', '0%')
        .attr('stop-color', 'rgba(255,255,255,0.9)')

      gradient.append('stop')
        .attr('offset', '20%')
        .attr('stop-color', 'rgba(255,255,255,0.3)')

      // Main color in middle
      gradient.append('stop')
        .attr('offset', '50%')
        .attr('stop-color', color)

      // Darker edge
      gradient.append('stop')
        .attr('offset', '100%')
        .attr('stop-color', d3.color(color)?.darker(0.8)?.toString() || color)
    })

    // Create a color-to-gradient-id map
    const colorToGradient: Record<string, string> = {}
    uniqueColors.forEach((color, i) => {
      colorToGradient[color] = `url(#bubble-gradient-${i})`
    })

    // Draw detective-style connection lines between similar topics
    // Helper to create curved path between two points
    const createCurvedPath = (x1: number, y1: number, x2: number, y2: number, curvature: number = 0.2) => {
      const midX = (x1 + x2) / 2
      const midY = (y1 + y2) / 2
      const dx = x2 - x1
      const dy = y2 - y1
      // Perpendicular offset for curve
      const offsetX = -dy * curvature
      const offsetY = dx * curvature
      const ctrlX = midX + offsetX
      const ctrlY = midY + offsetY
      return `M ${x1} ${y1} Q ${ctrlX} ${ctrlY} ${x2} ${y2}`
    }

    const linksGroup = container.append('g')
      .attr('class', 'links')

    // Draw the string lines
    linksGroup.selectAll('path')
      .data(graphData.edges)
      .join('path')
      .attr('class', 'detective-string')
      .attr('fill', 'none')
      .attr('stroke', d => {
        // Red tones: thinner = very dark, thicker = glowing bright
        const weight = d.weight
        if (weight > 0.9) return '#fca5a5'  // Very strong - pale glowing red
        if (weight > 0.7) return '#f87171'  // Strong - bright coral
        if (weight > 0.5) return '#ef4444'  // Medium - bright red
        if (weight > 0.35) return '#991b1b' // Light - dark red
        return '#450a0a'  // Weak - almost black burgundy
      })
      .attr('stroke-opacity', d => 0.15 + Math.pow(d.weight, 1.5) * 0.75)  // 0.15 to 0.9 opacity
      .attr('stroke-width', d => 0.3 + Math.pow(d.weight, 2.5) * 28)  // Extreme contrast: 0.3px to ~28px
      .attr('stroke-linecap', 'round')
      .attr('d', d => {
        const source = graphData.nodes.find(n => n.id === (typeof d.source === 'string' ? d.source : d.source.id))
        const target = graphData.nodes.find(n => n.id === (typeof d.target === 'string' ? d.target : d.target.id))
        if (!source || !target) return ''
        // Slight curve based on edge index for variety
        const idx = graphData.edges.indexOf(d)
        const curvature = 0.1 + (idx % 5) * 0.05 * (idx % 2 === 0 ? 1 : -1)
        return createCurvedPath(source.x!, source.y!, target.x!, target.y!, curvature)
      })

    // Helper to convert hex to darker rgba
    const hexToDarkerRgba = (hex: string, alpha: number, darkenFactor: number = 0.5) => {
      const r = Math.round(parseInt(hex.slice(1, 3), 16) * darkenFactor)
      const g = Math.round(parseInt(hex.slice(3, 5), 16) * darkenFactor)
      const b = Math.round(parseInt(hex.slice(5, 7), 16) * darkenFactor)
      return `rgba(${r}, ${g}, ${b}, ${alpha})`
    }

    // Check if we're at topic level for sticky notes
    const isTopicLevel = currentZoomLevel === 'topic'

    // Set initial positions on all nodes BEFORE drawing sticky notes
    // This ensures anchorNode.x/y have values when sticky notes are created
    const preDrawScale = 0.3  // Approximate initial zoom scale for topic level
    const preDrawSpread = Math.min(Math.sqrt(1 / preDrawScale), 2.5)

    tiers.forEach((tierNodes, tierIndex) => {
      if (tierNodes.length === 0) return
      const ringRadius = tierBaseRadii[tierIndex] * preDrawSpread

      if (tierIndex === 0) {
        // Center tier: concentric rings
        const rings: Node[][] = []
        let nodeIndex = 0
        let ringNum = 0
        while (nodeIndex < tierNodes.length) {
          const nodesInRing = ringNum === 0 ? 1 : Math.min(6 * ringNum, tierNodes.length - nodeIndex)
          rings.push(tierNodes.slice(nodeIndex, nodeIndex + nodesInRing))
          nodeIndex += nodesInRing
          ringNum++
        }
        const totalRings = rings.length
        rings.forEach((ringNodes, ringIdx) => {
          if (ringIdx === 0 && ringNodes.length === 1) {
            ringNodes[0].x = graphCenterX
            ringNodes[0].y = graphCenterY
          } else {
            const radiusFraction = totalRings > 1 ? ringIdx / (totalRings - 1) : 0.5
            const radius = ringRadius * radiusFraction * 0.95
            ringNodes.forEach((node, posInRing) => {
              const angleOffset = ringIdx * 0.15
              const angle = (posInRing / ringNodes.length) * 2 * Math.PI + angleOffset
              node.x = graphCenterX + Math.cos(angle) * radius
              node.y = graphCenterY + Math.sin(angle) * radius
            })
          }
        })
      } else {
        // Outer tiers
        tierNodes.forEach((node) => {
          const angle = nodeAngles.get(node.id) || 0
          const idHash = node.id.split('').reduce((a, c) => a + c.charCodeAt(0), 0)
          const radiusVariation = (idHash % 30) - 15
          const nodeRadius = ringRadius + radiusVariation
          node.x = graphCenterX + Math.cos(angle) * nodeRadius
          node.y = graphCenterY + Math.sin(angle) * nodeRadius
        })
      }
    })

    if (isTopicLevel) {
      // Group nodes by conversation
      const conversationGroups = new Map<string, Node[]>()
      graphData.nodes.forEach(node => {
        const convId = node.conversation_id || 'unknown'
        if (!conversationGroups.has(convId)) {
          conversationGroups.set(convId, [])
        }
        conversationGroups.get(convId)!.push(node)
      })

      // Sort each conversation's topics by timestamp
      conversationGroups.forEach(nodes => {
        nodes.sort((a, b) => {
          const timeA = a.timestamp ? new Date(a.timestamp).getTime() : 0
          const timeB = b.timestamp ? new Date(b.timestamp).getTime() : 0
          return timeA - timeB
        })
      })

      // Create combined conversation data for rendering
      interface ConversationCard {
        id: string
        title: string
        topics: Node[]
        anchorNode: Node  // Use first topic's position
        color: string
      }

      const conversationCards: ConversationCard[] = Array.from(conversationGroups.entries()).map(([convId, topics]) => ({
        id: convId,
        title: topics[0]?.conversation_title || 'Untitled Conversation',
        topics: topics,
        anchorNode: topics[0],
        color: topics[0]?.color || '#4a5568'
      }))

      // Debug: log cards with multiple topics
      const multiTopicCards = conversationCards.filter(c => c.topics.length > 1)
      const singleTopicCards = conversationCards.filter(c => c.topics.length === 1)
      console.log(`ConversationCards: ${singleTopicCards.length} single-topic, ${multiTopicCards.length} multi-topic`)
      if (multiTopicCards.length > 0) {
        console.log('Multi-topic cards:', multiTopicCards.map(c => ({
          id: c.id.slice(0, 8),
          title: c.title.slice(0, 30),
          topics: c.topics.length,
          anchorX: c.anchorNode?.x,
          anchorY: c.anchorNode?.y
        })))
      }

      // Individual sticky note dimensions
      const noteWidth = 320
      const noteHeight = 140
      const noteSpacing = 10  // Gap between notes in same conversation
      const containerPadding = 15  // Padding inside container

      // Flatten topics with position info for VERTICAL stacked layout
      interface TopicNote {
        topic: Node
        anchorNode: Node  // Reference to first topic (for position updates)
        conversationId: string
        conversationTitle: string
        indexInConv: number  // Position within conversation
        totalInConv: number  // Total topics in this conversation
        offsetX: number  // Offset from anchor node (always 0 for vertical)
        offsetY: number  // Vertical offset from anchor
      }

      const topicNotes: TopicNote[] = []
      conversationCards.forEach(card => {
        card.topics.forEach((topic, idx) => {
          // VERTICAL layout: stack notes below each other
          const offsetX = 0  // No horizontal offset
          const offsetY = idx * (noteHeight + noteSpacing) + containerPadding

          // Update the topic node's position to match its sticky note visual position
          // This ensures connection lines point to the correct locations
          topic.x = (card.anchorNode.x ?? 0) + offsetX
          topic.y = (card.anchorNode.y ?? 0) + offsetY

          topicNotes.push({
            topic,
            anchorNode: card.anchorNode,  // Keep reference for position updates
            conversationId: card.id,
            conversationTitle: card.title,
            indexInConv: idx,
            totalInConv: card.topics.length,
            offsetX,
            offsetY
          })
        })
      })

      console.log(`Rendering ${topicNotes.length} individual topic notes`)

      // Dark colors for sticky notes
      const darkColors = ['#5c4033', '#4a5568', '#2d3748', '#744210', '#553c9a', '#285e61', '#4a5568', '#2c5282', '#2d3748', '#276749', '#5c4033', '#4a235a']

      // Create container groups for ALL conversations (notes will be nested inside)
      // This ensures notes in the same conversation scale together
      const containerGroups = container.append('g')
        .attr('class', 'conversation-containers')
        .selectAll('g')
        .data(conversationCards)
        .join('g')
        .attr('class', 'conversation-container')
        .attr('data-conv-id', d => d.id)
        .attr('transform', d => {
          const x = d.anchorNode?.x ?? 0
          const y = d.anchorNode?.y ?? 0
          return `translate(${x}, ${y})`
        })

      // Add background rect only for multi-topic conversations
      containerGroups.filter(d => d.topics.length > 1)
        .each(function(d) {
          const n = d.topics.length
          const containerTop = -noteHeight / 2
          const containerHeight = n * noteHeight + (n - 1) * noteSpacing + containerPadding * 2
          const containerWidth = noteWidth + containerPadding * 2

          d3.select(this).append('rect')
            .attr('class', 'container-bg')
            .attr('x', -containerWidth / 2)
            .attr('y', containerTop)
            .attr('width', containerWidth)
            .attr('height', containerHeight)
            .attr('rx', 12)
            .attr('ry', 12)
            .attr('fill', 'rgba(255,255,255,0.03)')
            .attr('stroke', 'rgba(255,255,255,0.08)')
            .attr('stroke-width', 1)
            .attr('stroke-dasharray', '8,4')
        })

      // Draw sticky notes INSIDE their container groups (local positioning)
      containerGroups.each(function(cardData) {
        const containerEl = d3.select(this)

        // Get the topics for this conversation
        const topics = topicNotes.filter(tn => tn.conversationId === cardData.id)

        // Create sticky notes inside this container
        const stickyNotes = containerEl.selectAll('.sticky-note')
          .data(topics)
          .join('g')
          .attr('class', 'sticky-note')
          .attr('transform', d => {
            // Local positioning: offset from container origin (0,0)
            // offsetX is 0, offsetY is the vertical stack position
            return `translate(${-noteWidth/2}, ${d.offsetY - noteHeight/2})`
          })
          .attr('cursor', 'pointer')
          .on('click', function(_, d) {
            setSelectedNode(d.topic)
            container.selectAll('.sticky-note rect.note-bg')
              .attr('stroke', 'rgba(255,255,255,0.15)')
              .attr('stroke-width', 2)
            d3.select(this).select('rect.note-bg')
              .attr('stroke', '#fbbf24')
              .attr('stroke-width', 4)
          })
          .on('dblclick', (_, d) => drillDown(d.topic))
          .on('mouseenter', (_, d) => setHoveredNode(d.topic))
          .on('mouseleave', () => setHoveredNode(null))

        // Add note elements to each sticky note

        // Sticky note shadow
        stickyNotes.append('rect')
          .attr('class', 'note-shadow')
          .attr('x', 6)
          .attr('y', 6)
          .attr('width', noteWidth)
          .attr('height', noteHeight)
          .attr('rx', 6)
          .attr('ry', 6)
          .attr('fill', 'rgba(0,0,0,0.3)')

        // Sticky note background
        stickyNotes.append('rect')
          .attr('class', 'note-bg')
          .attr('x', 0)
          .attr('y', 0)
          .attr('width', noteWidth)
          .attr('height', noteHeight)
          .attr('rx', 6)
          .attr('ry', 6)
          .attr('fill', d => {
            const colorIndex = d.topic.parent_galaxy ?? d.topic.cluster_id ?? 0
            return darkColors[colorIndex % darkColors.length]
          })
          .attr('stroke', 'rgba(255,255,255,0.15)')
          .attr('stroke-width', 2)

        // Fold corner effect
        stickyNotes.append('path')
          .attr('class', 'note-fold')
          .attr('d', `M ${noteWidth - 20} 0 L ${noteWidth} 20 L ${noteWidth} 0 Z`)
          .attr('fill', 'rgba(0,0,0,0.15)')

        // Topic title (main text)
        stickyNotes.append('text')
          .attr('class', 'note-title')
          .attr('x', 15)
          .attr('y', 35)
          .text(d => {
            const label = d.topic.label || 'Untitled'
            return label.length > 35 ? label.slice(0, 33) + '...' : label
          })
          .attr('font-size', '18px')
          .attr('font-weight', '600')
          .attr('font-family', 'Georgia, "Times New Roman", serif')
          .attr('fill', '#fff')

        // Keywords
        stickyNotes.append('text')
          .attr('x', 15)
          .attr('y', 60)
          .text(d => d.topic.keywords?.slice(0, 4).join(' • ') || '')
          .attr('font-size', '12px')
          .attr('font-family', '-apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, sans-serif')
          .attr('fill', 'rgba(255,255,255,0.6)')

        // Conversation title (smaller, at bottom)
        stickyNotes.append('text')
          .attr('x', 15)
          .attr('y', noteHeight - 35)
          .text(d => {
            const title = d.conversationTitle || 'Untitled'
            return title.length > 40 ? title.slice(0, 38) + '...' : title
          })
          .attr('font-size', '11px')
          .attr('font-family', '-apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, sans-serif')
          .attr('fill', 'rgba(255,255,255,0.5)')

        // Position badge (e.g., "2 of 5")
        stickyNotes.append('text')
          .attr('x', noteWidth - 15)
          .attr('y', noteHeight - 15)
          .attr('text-anchor', 'end')
          .text(d => d.totalInConv > 1 ? `${d.indexInConv + 1} of ${d.totalInConv}` : '')
          .attr('font-size', '11px')
          .attr('font-family', '-apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, sans-serif')
          .attr('fill', 'rgba(255,255,255,0.4)')
      })

    } else {
      // NEW UX: Draw sticky notes for ALL levels (universe, galaxy, cluster)
      // Each note has: emoji titlebar, label, stats bar, preview of what's inside

      const noteWidth = currentZoomLevel === 'universe' ? 280 : 240
      const noteHeight = currentZoomLevel === 'universe' ? 160 : 120

      // Dark colors for sticky notes
      const darkColors = ['#5c4033', '#4a5568', '#2d3748', '#744210', '#553c9a', '#285e61', '#4a5568', '#2c5282', '#2d3748', '#276749', '#5c4033', '#4a235a']

      // Calculate initial scale for counter-scaling
      const preDrawScale = 0.5
      const noteScale = 0.7 / preDrawScale

      const notesGroup = container.append('g')
        .attr('class', 'universe-notes')

      const stickyNotes = notesGroup.selectAll('.sticky-note')
        .data(graphData.nodes)
        .join('g')
        .attr('class', 'sticky-note')
        .attr('transform', d => {
          const x = (d.x || 0) - noteWidth / 2
          const y = (d.y || 0) - noteHeight / 2
          return `translate(${x}, ${y}) scale(${noteScale})`
        })
        .attr('cursor', 'pointer')
        .on('click', function(_, d) {
          setSelectedNode(d)
          // Highlight selected
          notesGroup.selectAll('.sticky-note rect.note-bg')
            .attr('stroke', 'rgba(255,255,255,0.15)')
            .attr('stroke-width', 2)
          d3.select(this).select('rect.note-bg')
            .attr('stroke', '#fbbf24')
            .attr('stroke-width', 4)
        })
        .on('dblclick', (_, d) => {
          if (currentZoomLevel !== 'message') {
            drillDown(d)
          }
        })
        .on('mouseenter', (_, d) => setHoveredNode(d))
        .on('mouseleave', () => setHoveredNode(null))

      // Shadow
      stickyNotes.append('rect')
        .attr('class', 'note-shadow')
        .attr('x', 6)
        .attr('y', 6)
        .attr('width', noteWidth)
        .attr('height', noteHeight)
        .attr('rx', 8)
        .attr('ry', 8)
        .attr('fill', 'rgba(0,0,0,0.3)')

      // Background
      stickyNotes.append('rect')
        .attr('class', 'note-bg')
        .attr('x', 0)
        .attr('y', 0)
        .attr('width', noteWidth)
        .attr('height', noteHeight)
        .attr('rx', 8)
        .attr('ry', 8)
        .attr('fill', d => darkColors[(d.cluster_id || 0) % darkColors.length])
        .attr('stroke', 'rgba(255,255,255,0.15)')
        .attr('stroke-width', 2)

      // Fold corner
      stickyNotes.append('path')
        .attr('class', 'note-fold')
        .attr('d', `M ${noteWidth - 18} 0 L ${noteWidth} 18 L ${noteWidth} 0 Z`)
        .attr('fill', 'rgba(0,0,0,0.15)')

      // Emoji titlebar background
      stickyNotes.append('rect')
        .attr('class', 'emoji-titlebar')
        .attr('x', 0)
        .attr('y', 0)
        .attr('width', noteWidth)
        .attr('height', 36)
        .attr('rx', 8)
        .attr('ry', 8)
        .attr('fill', 'rgba(0,0,0,0.2)')

      // Square off bottom corners of titlebar
      stickyNotes.append('rect')
        .attr('x', 0)
        .attr('y', 18)
        .attr('width', noteWidth)
        .attr('height', 18)
        .attr('fill', 'rgba(0,0,0,0.2)')

      // Emoji in titlebar
      stickyNotes.append('text')
        .attr('class', 'note-emoji')
        .attr('x', 14)
        .attr('y', 24)
        .text(d => getEmojiForNode(d))
        .attr('font-size', '20px')
        .attr('dominant-baseline', 'middle')

      // Title text (after emoji)
      stickyNotes.append('text')
        .attr('class', 'note-title')
        .attr('x', 42)
        .attr('y', 24)
        .text(d => {
          const label = d.label || 'Untitled'
          const maxLen = currentZoomLevel === 'universe' ? 28 : 22
          return label.length > maxLen ? label.slice(0, maxLen - 2) + '...' : label
        })
        .attr('font-size', '14px')
        .attr('font-weight', '600')
        .attr('font-family', '-apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, sans-serif')
        .attr('fill', '#fff')
        .attr('dominant-baseline', 'middle')

      // Stats bar - colorful indicators
      stickyNotes.append('g')
        .attr('class', 'stats-bar')
        .attr('transform', `translate(12, 50)`)
        .each(function(d) {
          const statsGroup = d3.select(this)
          let xOffset = 0

          // Item count (always show)
          const itemCount = d.galaxy_count || d.pair_count || d.message_count || 0
          if (itemCount > 0) {
            // Blue dot for items
            statsGroup.append('circle')
              .attr('cx', xOffset + 6)
              .attr('cy', 0)
              .attr('r', 5)
              .attr('fill', '#3b82f6')
            statsGroup.append('text')
              .attr('x', xOffset + 16)
              .attr('y', 0)
              .text(`${itemCount}`)
              .attr('font-size', '11px')
              .attr('fill', 'rgba(255,255,255,0.7)')
              .attr('dominant-baseline', 'middle')
            xOffset += 40
          }

          // Keywords/topics preview (show a few)
          const keywords = d.keywords || d.sample_topics || []
          if (keywords.length > 0) {
            // Green dot for keywords
            statsGroup.append('circle')
              .attr('cx', xOffset + 6)
              .attr('cy', 0)
              .attr('r', 5)
              .attr('fill', '#10b981')
            statsGroup.append('text')
              .attr('x', xOffset + 16)
              .attr('y', 0)
              .text(keywords.slice(0, 2).join(', '))
              .attr('font-size', '10px')
              .attr('fill', 'rgba(255,255,255,0.6)')
              .attr('dominant-baseline', 'middle')
          }
        })

      // Preview text - what's inside (next level preview)
      stickyNotes.append('text')
        .attr('class', 'preview-text')
        .attr('x', 12)
        .attr('y', noteHeight - 35)
        .text(d => {
          if (currentZoomLevel === 'universe') {
            return d.description || `Contains ${d.galaxy_count || 0} topic clusters`
          } else {
            const topics = d.sample_topics || d.keywords || []
            if (topics.length > 0) {
              return topics.slice(0, 3).join(' • ')
            }
            return `${d.pair_count || d.message_count || 0} conversations`
          }
        })
        .attr('font-size', '10px')
        .attr('font-family', '-apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, sans-serif')
        .attr('fill', 'rgba(255,255,255,0.5)')

      // Level indicator badge
      stickyNotes.append('text')
        .attr('x', noteWidth - 12)
        .attr('y', noteHeight - 12)
        .attr('text-anchor', 'end')
        .text(() => {
          if (currentZoomLevel === 'universe') return '🌌'
          if (currentZoomLevel === 'galaxy') return '🌟'
          return '💫'
        })
        .attr('font-size', '14px')
        .attr('fill', 'rgba(255,255,255,0.4)')
    }

    // Apply initial zoom transform AFTER all elements are drawn
    // This triggers the zoom handler which applies correct scaling
    svg.call(
      zoom.transform,
      d3.zoomIdentity
        .translate(width / 2, height / 2)
        .scale(initialScale)
        .translate(-fitCenterX, -fitCenterY)
    )

  }, [graphData, currentZoomLevel])

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

      // Fade unconnected edges (detective strings)
      container.selectAll<SVGPathElement, Edge>('.links path')
        .style('opacity', d => {
          const sourceId = typeof d.source === 'string' ? d.source : d.source.id
          const targetId = typeof d.target === 'string' ? d.target : d.target.id
          return (sourceId === selectedNode.id || targetId === selectedNode.id) ? 0.8 : 0.05
        })

    } else {
      // Reset to normal
      container.selectAll('.nodes circle')
        .style('opacity', 1)

      container.selectAll('.node-emojis text')
        .style('opacity', 1)

      container.selectAll('.links path')
        .style('opacity', d => 0.15 + Math.pow((d as Edge).weight, 1.5) * 0.75)
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

      {/* Navigation breadcrumbs */}
      <div className="navigation-bar">
        <div className="breadcrumbs">
          <button 
            className={`breadcrumb-item ${currentZoomLevel === 'universe' ? 'active' : ''}`}
            onClick={() => {
              setCurrentZoomLevel('universe')
              setCurrentParentId(null)
              setNavigationStack([])
            }}
          >
            🌌 Universe
          </button>
          
          {navigationStack.map((item, index) => (
            <span key={index} className="breadcrumb-separator">
              &gt;
              <button
                className="breadcrumb-item"
                onClick={() => {
                  // Navigate to this level
                  const newStack = navigationStack.slice(0, index + 1)
                  const targetLevel = newStack[newStack.length - 1]
                  setNavigationStack(newStack.slice(0, -1))
                  setCurrentZoomLevel(targetLevel.level as any)
                  setCurrentParentId(targetLevel.parentId)
                }}
              >
                {item.level === 'galaxy' ? '🌟' : item.level === 'topic' ? '💫' : '💭'} {item.label}
              </button>
            </span>
          ))}
          
          {currentZoomLevel !== 'universe' && (
            <span className="breadcrumb-separator">
              &gt;
              <span className="breadcrumb-current">
                {currentZoomLevel === 'galaxy' ? '🌟 Galaxies' : 
                 currentZoomLevel === 'topic' ? '💫 Topics' : 
                 '💭 Messages'}
              </span>
            </span>
          )}
        </div>

        <div className="zoom-controls">
          <button
            className="zoom-btn back-btn"
            onClick={navigateBack}
            disabled={navigationStack.length === 0}
            title="Go back one level"
          >
            ← Back
          </button>
          
          <span className="current-level">
            {currentZoomLevel.charAt(0).toUpperCase() + currentZoomLevel.slice(1)} Level
          </span>
        </div>
      </div>

      {/* Hover tooltip */}
      {hoveredNode && !selectedNode && (
        <div className="tooltip">
          <h3>{hoveredNode.label}</h3>
          {hoveredNode.zoom_level === 'universe' ? (
            <div>
              <p>{hoveredNode.galaxy_count} galaxies</p>
              <p className="description">{hoveredNode.description}</p>
              {hoveredNode.sample_topics && (
                <p className="keywords">
                  Examples: {hoveredNode.sample_topics.slice(0, 2).join(', ')}
                </p>
              )}
              <p className="instruction">Double-click to drill down</p>
            </div>
          ) : (
            <div>
              <p>{hoveredNode.message_count} messages</p>
              {hoveredNode.keywords && (
                <p className="keywords">
                  {hoveredNode.keywords.slice(0, 3).join(', ')}
                </p>
              )}
              <p className="instruction">Double-click to drill down</p>
            </div>
          )}
        </div>
      )}

      {/* Cluster legend */}
      {graphData && (
        <div className="legend">
          <h3>Clusters</h3>
          {graphData.clusters.map((cluster, idx) => (
            <div key={`cluster-${cluster.id ?? idx}`} className="cluster-group">
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
                    .sort((a, b) => (b.message_count ?? 0) - (a.message_count ?? 0))
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
            {selectedNode.created_at && (
              <span>{new Date(selectedNode.created_at).toLocaleDateString()}</span>
            )}
          </div>
          {selectedNode.keywords && selectedNode.keywords.length > 0 && (
            <div className="keywords">
              <strong>Topics:</strong> {selectedNode.keywords.join(', ')}
            </div>
          )}
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

      {/* Control Buttons */}
      <div className="control-buttons">
        <button
          className="control-btn analyze-btn"
          onClick={handleRunAnalysis}
          disabled={analyzing}
          title="Run conversation analysis with clustering"
        >
          {analyzing ? '⏳' : '🔬'} {analyzing ? 'Analyzing...' : 'Analyze'}
        </button>

        <button
          className="control-btn regenerate-btn"
          onClick={handleRegenerateTags}
          disabled={regeneratingTags || !hasApiKey}
          title={hasApiKey ? 'Regenerate tags using AI for better quality' : 'Set API key first to enable AI tags'}
        >
          {regeneratingTags ? '⏳' : '🏷️'} {regeneratingTags ? 'Generating...' : 'AI Tags'}
        </button>

        <button
          className="control-btn analyze-messages-btn"
          onClick={handleAnalyzeMessages}
          disabled={analyzingMessages || !hasApiKey}
          title={hasApiKey
            ? `AI analyze all messages (${analysisStatus ? `${analysisStatus.remaining} remaining` : 'click to check'})`
            : 'Set API key first to enable AI analysis'}
        >
          {analyzingMessages ? '⏳' : '🧠'} {analyzingMessages ? 'Analyzing...' : 'AI Analyze'}
          {analysisStatus && analysisStatus.remaining > 0 && !analyzingMessages && (
            <span className="badge">{analysisStatus.remaining}</span>
          )}
        </button>

        <button
          className="control-btn api-key-btn"
          onClick={() => setShowApiKeyModal(true)}
          title={hasApiKey ? `API Key: ${apiKeyPreview}` : 'Set API Key for AI naming'}
        >
          🔑 {hasApiKey ? '✓' : '✗'}
        </button>
      </div>
      
      {/* Analysis Error */}
      {analysisError && (
        <div className="error-banner">
          ❌ {analysisError}
        </div>
      )}

      {/* API Key Modal */}
      {showApiKeyModal && (
        <div className="modal-overlay" onClick={() => setShowApiKeyModal(false)}>
          <div className="modal-content" onClick={(e) => e.stopPropagation()}>
            <div className="modal-header">
              <h3>API Key Management</h3>
              <button 
                className="modal-close" 
                onClick={() => setShowApiKeyModal(false)}
              >
                ×
              </button>
            </div>
            
            <div className="modal-body">
              {hasApiKey ? (
                <div className="api-key-status">
                  <p className="status-text success">
                    ✓ API Key is configured: {apiKeyPreview}
                  </p>
                  <p className="status-description">
                    AI-powered cluster naming is enabled.
                  </p>
                  <button
                    className="btn btn-danger"
                    onClick={handleClearApiKey}
                    disabled={apiKeyLoading}
                  >
                    {apiKeyLoading ? 'Clearing...' : 'Clear API Key'}
                  </button>
                </div>
              ) : (
                <div className="api-key-input">
                  <p className="input-description">
                    Enter your Anthropic API key to enable AI-powered cluster naming.
                    The key is stored in memory only and not persisted to disk.
                  </p>
                  
                  <input
                    type="password"
                    className="api-key-field"
                    placeholder="sk-ant-api03-..."
                    value={apiKey}
                    onChange={(e) => setApiKey(e.target.value)}
                    onKeyDown={(e) => e.key === 'Enter' && handleSubmitApiKey()}
                  />
                  
                  {apiKeyError && (
                    <p className="error-text">{apiKeyError}</p>
                  )}
                  
                  <div className="modal-actions">
                    <button
                      className="btn btn-secondary"
                      onClick={() => setShowApiKeyModal(false)}
                    >
                      Cancel
                    </button>
                    <button
                      className="btn btn-primary"
                      onClick={handleSubmitApiKey}
                      disabled={apiKeyLoading || !apiKey.trim()}
                    >
                      {apiKeyLoading ? 'Setting...' : 'Set API Key'}
                    </button>
                  </div>
                </div>
              )}
            </div>
          </div>
        </div>
      )}
    </div>
  )
}

export default App
