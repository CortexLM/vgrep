use anyhow::{Context, Result};
use std::path::Path;
use std::sync::Once;

use llama_cpp_2::context::params::LlamaContextParams;
use llama_cpp_2::llama_backend::LlamaBackend;
use llama_cpp_2::llama_batch::LlamaBatch;
use llama_cpp_2::model::params::LlamaModelParams;
use llama_cpp_2::model::{AddBos, LlamaModel};
use llama_cpp_2::{send_logs_to_tracing, LogOptions};

use crate::config::Config;

static INIT_LOGS: Once = Once::new();

fn suppress_llama_logs() {
    INIT_LOGS.call_once(|| {
        send_logs_to_tracing(LogOptions::default().with_logs_enabled(false));
    });
}

struct EngineResources {
    backend: LlamaBackend,
    model: LlamaModel,
}

pub struct EmbeddingEngine {
    // IMPORTANT: ctx must be defined before resources to ensure it is dropped first.
    // ctx borrows from model/backend which are in resources.
    ctx: Option<llama_cpp_2::context::LlamaContext<'static>>,
    resources: Box<EngineResources>,
    n_ctx: usize,
}

// Ensure we can send it across threads (Mutex wrapper requires this)
// LlamaContext wraps a pointer and is generally thread-safe to move but not to share.
unsafe impl Send for EmbeddingEngine {}

impl EmbeddingEngine {
    pub fn new(config: &Config) -> Result<Self> {
        let model_path = config.embedding_model_path()?;
        let n_threads = config.get_n_threads();
        let context_size = config.context_size;
        Self::from_path(&model_path, n_threads, context_size)
    }

    pub fn from_path(model_path: &Path, n_threads: i32, context_size: usize) -> Result<Self> {
        suppress_llama_logs();

        let backend = LlamaBackend::init().context("Failed to initialize llama backend")?;
        let model_params = LlamaModelParams::default();
        let model = LlamaModel::load_from_file(&backend, model_path, &model_params)
            .context("Failed to load embedding model")?;

        let resources = Box::new(EngineResources { backend, model });

        let ctx_params = LlamaContextParams::default()
            .with_n_threads_batch(n_threads)
            .with_n_threads(n_threads)
            .with_embeddings(true);

        // SAFETY: We are creating a self-referential struct.
        // 1. `resources` is boxed, so its address is stable.
        // 2. We extend the lifetime of `model` to 'static temporarily to create `ctx`.
        // 3. We store `ctx` in the same struct.
        // 4. `ctx` is dropped BEFORE `resources` because it is declared earlier in the struct.
        let ctx = unsafe {
            let model_ref: &'static LlamaModel = std::mem::transmute(&resources.model);
            let backend_ref: &'static LlamaBackend = std::mem::transmute(&resources.backend);
            model_ref
                .new_context(backend_ref, ctx_params)
                .context("Failed to create context")?
        };

        let n_ctx = std::cmp::min(ctx.n_ctx() as usize, context_size);

        Ok(Self {
            ctx: Some(ctx),
            resources,
            n_ctx,
        })
    }

    pub fn embed(&mut self, text: &str) -> Result<Vec<f32>> {
        let embeddings = self.embed_batch(&[text])?;
        embeddings
            .into_iter()
            .next()
            .context("No embedding generated")
    }

    pub fn embed_batch(&mut self, texts: &[&str]) -> Result<Vec<Vec<f32>>> {
        if texts.is_empty() {
            return Ok(Vec::new());
        }

        let mut results = Vec::with_capacity(texts.len());

        for text in texts {
            let tokens = self
                .resources
                .model
                .str_to_token(text, AddBos::Always)
                .context("Failed to tokenize")?;

            let tokens: Vec<_> = if tokens.len() > self.n_ctx {
                tokens.into_iter().take(self.n_ctx).collect()
            } else {
                tokens
            };

            let ctx = self.ctx.as_mut().context("Context not initialized")?;
            let embedding = Self::process_tokens(self.n_ctx, ctx, &tokens)?;
            results.push(embedding);
        }

        Ok(results)
    }

    fn process_tokens(
        n_ctx: usize,
        ctx: &mut llama_cpp_2::context::LlamaContext,
        tokens: &[llama_cpp_2::token::LlamaToken],
    ) -> Result<Vec<f32>> {
        let mut batch = LlamaBatch::new(n_ctx, 1);
        batch.add_sequence(tokens, 0, false)?;

        ctx.clear_kv_cache();
        ctx.decode(&mut batch).context("Failed to decode batch")?;

        let embedding = ctx
            .embeddings_seq_ith(0)
            .context("Failed to get embeddings")?;

        Ok(normalize(embedding))
    }

    pub fn embedding_dim(&self) -> usize {
        self.resources.model.n_embd() as usize
    }
}

fn normalize(input: &[f32]) -> Vec<f32> {
    let magnitude: f32 = input.iter().map(|x| x * x).sum::<f32>().sqrt();
    if magnitude == 0.0 {
        return input.to_vec();
    }
    input.iter().map(|x| x / magnitude).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_normalize() {
        let input = vec![3.0, 4.0];
        let output = normalize(&input);
        assert!((output[0] - 0.6).abs() < 1e-6);
        assert!((output[1] - 0.8).abs() < 1e-6);
    }

    #[test]
    fn test_normalize_zero() {
        let input = vec![0.0, 0.0];
        let output = normalize(&input);
        assert_eq!(output, vec![0.0, 0.0]);
    }
}
