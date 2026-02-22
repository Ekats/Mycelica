package db

import (
	"encoding/binary"
	"math"
	"testing"
)

func TestBytesToEmbedding_KnownValues(t *testing.T) {
	// float32(1.0) in LE = 0x3F800000 = [0x00, 0x00, 0x80, 0x3F]
	// float32(-0.5) in LE = 0xBF000000 = [0x00, 0x00, 0x00, 0xBF]
	data := make([]byte, 8)
	binary.LittleEndian.PutUint32(data[0:4], math.Float32bits(1.0))
	binary.LittleEndian.PutUint32(data[4:8], math.Float32bits(-0.5))

	result := bytesToEmbedding(data)
	if len(result) != 2 {
		t.Fatalf("expected 2 floats, got %d", len(result))
	}
	if result[0] != 1.0 {
		t.Errorf("expected 1.0, got %f", result[0])
	}
	if result[1] != -0.5 {
		t.Errorf("expected -0.5, got %f", result[1])
	}
}

func TestBytesToEmbedding_Empty(t *testing.T) {
	result := bytesToEmbedding(nil)
	if len(result) != 0 {
		t.Errorf("expected empty, got %d elements", len(result))
	}
	result2 := bytesToEmbedding([]byte{})
	if len(result2) != 0 {
		t.Errorf("expected empty, got %d elements", len(result2))
	}
}

func TestBytesToEmbedding_ShortChunk(t *testing.T) {
	// 5 bytes = 1 full float + 1 byte leftover
	data := make([]byte, 5)
	binary.LittleEndian.PutUint32(data[0:4], math.Float32bits(2.5))
	data[4] = 0xFF // trailing byte

	result := bytesToEmbedding(data)
	if len(result) != 2 {
		t.Fatalf("expected 2 elements, got %d", len(result))
	}
	if result[0] != 2.5 {
		t.Errorf("expected 2.5, got %f", result[0])
	}
	if result[1] != 0.0 {
		t.Errorf("trailing chunk should be 0.0, got %f", result[1])
	}
}

func TestBytesToEmbedding_384Dim(t *testing.T) {
	// Simulate a full embedding: 384 x 4 = 1536 bytes
	data := make([]byte, 384*4)
	for i := 0; i < 384; i++ {
		binary.LittleEndian.PutUint32(data[i*4:(i+1)*4], math.Float32bits(float32(i)*0.01))
	}
	result := bytesToEmbedding(data)
	if len(result) != 384 {
		t.Fatalf("expected 384 dims, got %d", len(result))
	}
	// Spot check
	if math.Abs(float64(result[0])) > 0.0001 {
		t.Errorf("result[0] should be ~0.0, got %f", result[0])
	}
	if math.Abs(float64(result[100]-1.0)) > 0.0001 {
		t.Errorf("result[100] should be ~1.0, got %f", result[100])
	}
}
