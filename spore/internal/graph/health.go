package graph

import "math"

// HealthBreakdown shows the sub-scores of the health formula
type HealthBreakdown struct {
	Connectivity float64 `json:"connectivity"`
	Components   float64 `json:"components"`
	Staleness    float64 `json:"staleness"`
	Fragility    float64 `json:"fragility"`
}

// AnalysisReport is the full analysis result
type AnalysisReport struct {
	HealthScore     float64          `json:"health_score"`
	HealthBreakdown HealthBreakdown  `json:"health_breakdown"`
	Topology        *TopologyReport  `json:"topology"`
	Staleness       *StalenessReport `json:"staleness"`
	Bridges         *BridgeReport    `json:"bridges"`
}

// AnalyzerConfig holds analysis parameters
type AnalyzerConfig struct {
	HubThreshold int
	TopN         int
	StaleDays    int64
}

// DefaultConfig returns sensible defaults
func DefaultConfig() *AnalyzerConfig {
	return &AnalyzerConfig{
		HubThreshold: 10,
		TopN:         50,
		StaleDays:    30,
	}
}

// Analyze runs all analyses and computes a composite health score
func Analyze(snap *GraphSnapshot, config *AnalyzerConfig) *AnalysisReport {
	topology := ComputeTopology(snap, config.HubThreshold, config.TopN)
	staleness := ComputeStaleness(snap, config.StaleDays)
	bridges := ComputeBridges(snap)

	total := float64(topology.TotalNodes)

	var connectivity, components, stalenessScore, fragility float64

	if total > 0 {
		connectivity = clamp(1.0-math.Min(float64(topology.OrphanCount)/total, 0.2)*5.0, 0, 1)
	}
	if topology.NumComponents > 0 {
		components = clamp(1.0/float64(topology.NumComponents), 0, 1)
	}
	if total > 0 {
		stalenessScore = clamp(1.0-math.Min(float64(staleness.StaleNodeCount)/total, 0.1)*10.0, 0, 1)
	}
	if total > 0 {
		fragility = clamp(1.0-math.Min(float64(bridges.APCount)/total, 0.05)*20.0, 0, 1)
	}

	healthScore := 0.30*connectivity + 0.25*components + 0.25*stalenessScore + 0.20*fragility

	return &AnalysisReport{
		HealthScore: healthScore,
		HealthBreakdown: HealthBreakdown{
			Connectivity: connectivity,
			Components:   components,
			Staleness:    stalenessScore,
			Fragility:    fragility,
		},
		Topology:  topology,
		Staleness: staleness,
		Bridges:   bridges,
	}
}

func clamp(val, min, max float64) float64 {
	if val < min {
		return min
	}
	if val > max {
		return max
	}
	return val
}
