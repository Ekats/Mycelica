//! Local embedding generation using all-MiniLM-L6-v2 via candle.
//!
//! Produces 384-dimensional normalized embeddings, ~40x faster than OpenAI API.

use candle_core::{Device, Tensor};
use candle_nn::VarBuilder;
use candle_transformers::models::bert::{BertModel, Config, HiddenAct, DTYPE};
use hf_hub::{api::sync::Api, Repo, RepoType};
use std::sync::OnceLock;
use tokenizers::{PaddingParams, PaddingStrategy, Tokenizer, TruncationParams};

const MODEL_ID: &str = "sentence-transformers/all-MiniLM-L6-v2";
const REVISION: &str = "main";

/// Global model instance (lazy loaded)
static EMBEDDER: OnceLock<Result<LocalEmbedder, String>> = OnceLock::new();

/// Local embedding model wrapper
pub struct LocalEmbedder {
    model: BertModel,
    tokenizer: Tokenizer,
    device: Device,
}

impl LocalEmbedder {
    /// Load model from Hugging Face Hub (downloads on first use)
    pub fn new() -> Result<Self, String> {
        // Try CUDA if feature enabled, otherwise CPU only
        #[cfg(feature = "cuda")]
        let device = if candle_core::utils::cuda_is_available() {
            match Device::new_cuda(0) {
                Ok(dev) => {
                    println!("[LocalEmbeddings] Using CUDA device (GPU)");
                    dev
                }
                Err(e) => {
                    println!("[LocalEmbeddings] CUDA device creation failed: {}, falling back to CPU", e);
                    Device::Cpu
                }
            }
        } else {
            println!("[LocalEmbeddings] CUDA not available, using CPU");
            Device::Cpu
        };

        #[cfg(not(feature = "cuda"))]
        let device = {
            println!("[LocalEmbeddings] Using CPU (cuda feature not enabled)");
            Device::Cpu
        };

        // Download model files from HF Hub
        let api = Api::new().map_err(|e| format!("Failed to create HF API: {}", e))?;
        let repo = api.repo(Repo::with_revision(
            MODEL_ID.to_string(),
            RepoType::Model,
            REVISION.to_string(),
        ));

        let config_path = repo
            .get("config.json")
            .map_err(|e| format!("Failed to download config: {}", e))?;
        let tokenizer_path = repo
            .get("tokenizer.json")
            .map_err(|e| format!("Failed to download tokenizer: {}", e))?;
        let weights_path = repo
            .get("model.safetensors")
            .map_err(|e| format!("Failed to download weights: {}", e))?;

        // Load config
        let config_str = std::fs::read_to_string(&config_path)
            .map_err(|e| format!("Failed to read config: {}", e))?;
        let mut config: Config =
            serde_json::from_str(&config_str).map_err(|e| format!("Failed to parse config: {}", e))?;

        // MiniLM uses gelu activation
        config.hidden_act = HiddenAct::Gelu;

        // Load tokenizer
        let mut tokenizer = Tokenizer::from_file(&tokenizer_path)
            .map_err(|e| format!("Failed to load tokenizer: {}", e))?;

        // Configure tokenizer for batch processing
        let padding = PaddingParams {
            strategy: PaddingStrategy::BatchLongest,
            ..Default::default()
        };
        let truncation = TruncationParams {
            max_length: 512,
            ..Default::default()
        };
        tokenizer.with_padding(Some(padding));
        tokenizer
            .with_truncation(Some(truncation))
            .map_err(|e| format!("Failed to set truncation: {}", e))?;

        // Load model weights
        let vb = unsafe {
            VarBuilder::from_mmaped_safetensors(&[weights_path], DTYPE, &device)
                .map_err(|e| format!("Failed to load weights: {}", e))?
        };

        let model = BertModel::load(vb, &config)
            .map_err(|e| format!("Failed to build model: {}", e))?;

        println!("[LocalEmbeddings] Model loaded: {}", MODEL_ID);

        Ok(Self {
            model,
            tokenizer,
            device,
        })
    }

    /// Generate embedding for a single text
    pub fn embed(&self, text: &str) -> Result<Vec<f32>, String> {
        let embeddings = self.embed_batch(&[text])?;
        Ok(embeddings.into_iter().next().unwrap())
    }

    /// Generate embeddings for a batch of texts
    pub fn embed_batch(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>, String> {
        if texts.is_empty() {
            return Ok(vec![]);
        }

        // Tokenize
        let encodings = self
            .tokenizer
            .encode_batch(texts.to_vec(), true)
            .map_err(|e| format!("Tokenization failed: {}", e))?;

        let batch_size = encodings.len();
        let seq_len = encodings[0].get_ids().len();

        // Build input tensors
        let mut all_ids = Vec::with_capacity(batch_size * seq_len);
        let mut all_mask = Vec::with_capacity(batch_size * seq_len);
        let mut all_type_ids = Vec::with_capacity(batch_size * seq_len);

        for encoding in &encodings {
            all_ids.extend(encoding.get_ids().iter().map(|&x| x as i64));
            all_mask.extend(encoding.get_attention_mask().iter().map(|&x| x as i64));
            all_type_ids.extend(encoding.get_type_ids().iter().map(|&x| x as i64));
        }

        let input_ids = Tensor::from_vec(all_ids, (batch_size, seq_len), &self.device)
            .map_err(|e| format!("Failed to create input_ids tensor: {}", e))?;
        let attention_mask = Tensor::from_vec(all_mask.clone(), (batch_size, seq_len), &self.device)
            .map_err(|e| format!("Failed to create attention_mask tensor: {}", e))?;
        let token_type_ids = Tensor::from_vec(all_type_ids, (batch_size, seq_len), &self.device)
            .map_err(|e| format!("Failed to create token_type_ids tensor: {}", e))?;

        // Forward pass
        let hidden_states = self
            .model
            .forward(&input_ids, &token_type_ids, Some(&attention_mask))
            .map_err(|e| format!("Model forward failed: {}", e))?;

        // Mean pooling with attention mask
        let mask_expanded = attention_mask
            .unsqueeze(2)
            .map_err(|e| format!("Unsqueeze failed: {}", e))?
            .to_dtype(DTYPE)
            .map_err(|e| format!("Dtype conversion failed: {}", e))?
            .broadcast_as(hidden_states.shape())
            .map_err(|e| format!("Broadcast failed: {}", e))?;

        let masked = hidden_states
            .mul(&mask_expanded)
            .map_err(|e| format!("Multiply failed: {}", e))?;

        let summed = masked
            .sum(1)
            .map_err(|e| format!("Sum failed: {}", e))?;

        let mask_sum = mask_expanded
            .sum(1)
            .map_err(|e| format!("Mask sum failed: {}", e))?
            .clamp(1e-9, f64::MAX)
            .map_err(|e| format!("Clamp failed: {}", e))?;

        let pooled = summed
            .div(&mask_sum)
            .map_err(|e| format!("Division failed: {}", e))?;

        // L2 normalize
        let norm = pooled
            .sqr()
            .map_err(|e| format!("Sqr failed: {}", e))?
            .sum_keepdim(1)
            .map_err(|e| format!("Sum keepdim failed: {}", e))?
            .sqrt()
            .map_err(|e| format!("Sqrt failed: {}", e))?
            .clamp(1e-12, f64::MAX)
            .map_err(|e| format!("Clamp failed: {}", e))?;

        let normalized = pooled
            .broadcast_div(&norm)
            .map_err(|e| format!("Normalize failed: {}", e))?;

        // Extract results
        let normalized_vec: Vec<f32> = normalized
            .to_vec2()
            .map_err(|e| format!("To vec failed: {}", e))?
            .into_iter()
            .flatten()
            .collect();

        // Split into individual embeddings (384-dim each)
        let embedding_dim = 384;
        let results: Vec<Vec<f32>> = normalized_vec
            .chunks(embedding_dim)
            .map(|chunk| chunk.to_vec())
            .collect();

        Ok(results)
    }
}

/// Get or initialize the global embedder
fn get_embedder() -> Result<&'static LocalEmbedder, String> {
    EMBEDDER
        .get_or_init(|| LocalEmbedder::new())
        .as_ref()
        .map_err(|e| e.clone())
}

/// Generate a local embedding for the given text.
/// Returns a 384-dimensional normalized vector.
pub fn generate(text: &str) -> Result<Vec<f32>, String> {
    let embedder = get_embedder()?;
    embedder.embed(text)
}

/// Generate embeddings for multiple texts in a batch.
/// More efficient than calling generate() multiple times.
pub fn generate_batch(texts: &[&str]) -> Result<Vec<Vec<f32>>, String> {
    let embedder = get_embedder()?;
    embedder.embed_batch(texts)
}

/// Check if the model is loaded
pub fn is_loaded() -> bool {
    EMBEDDER.get().is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_embedding_dimension() {
        let result = generate("Hello, world!");
        assert!(result.is_ok());
        let embedding = result.unwrap();
        assert_eq!(embedding.len(), 384);
    }

    #[test]
    fn test_embedding_normalized() {
        let embedding = generate("Test text").unwrap();
        let norm: f32 = embedding.iter().map(|x| x * x).sum::<f32>().sqrt();
        assert!((norm - 1.0).abs() < 0.001);
    }

    #[test]
    fn test_batch_embedding() {
        let texts = vec!["Hello", "World", "Test"];
        let embeddings = generate_batch(&texts).unwrap();
        assert_eq!(embeddings.len(), 3);
        for emb in embeddings {
            assert_eq!(emb.len(), 384);
        }
    }
}
