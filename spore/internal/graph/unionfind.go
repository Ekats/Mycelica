package graph

// UnionFind implements union-find with path compression and union by rank
type UnionFind struct {
	parent map[string]string
	rank   map[string]int
	size   map[string]int
}

// NewUnionFind creates a new UnionFind where each element is its own component
func NewUnionFind(ids []string) *UnionFind {
	uf := &UnionFind{
		parent: make(map[string]string, len(ids)),
		rank:   make(map[string]int, len(ids)),
		size:   make(map[string]int, len(ids)),
	}
	for _, id := range ids {
		uf.parent[id] = id
		uf.rank[id] = 0
		uf.size[id] = 1
	}
	return uf
}

// Find returns the root of the component containing id, with path compression
func (uf *UnionFind) Find(id string) string {
	parent, ok := uf.parent[id]
	if !ok {
		return id
	}
	if parent != id {
		root := uf.Find(parent)
		uf.parent[id] = root
		return root
	}
	return id
}

// Union merges the components containing a and b. Returns true if they were separate.
func (uf *UnionFind) Union(a, b string) bool {
	rootA := uf.Find(a)
	rootB := uf.Find(b)
	if rootA == rootB {
		return false
	}

	rankA := uf.rank[rootA]
	rankB := uf.rank[rootB]
	sizeA := uf.size[rootA]
	sizeB := uf.size[rootB]

	if rankA < rankB {
		uf.parent[rootA] = rootB
		uf.size[rootB] = sizeA + sizeB
	} else if rankA > rankB {
		uf.parent[rootB] = rootA
		uf.size[rootA] = sizeA + sizeB
	} else {
		uf.parent[rootB] = rootA
		uf.size[rootA] = sizeA + sizeB
		uf.rank[rootA]++
	}
	return true
}

// Components returns all connected components as slices of IDs
func (uf *UnionFind) Components() [][]string {
	groups := make(map[string][]string)
	for id := range uf.parent {
		root := uf.Find(id)
		groups[root] = append(groups[root], id)
	}
	result := make([][]string, 0, len(groups))
	for _, members := range groups {
		result = append(result, members)
	}
	return result
}
