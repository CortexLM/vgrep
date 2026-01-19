use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::path::Path;

#[derive(Debug, Serialize)]
struct SearchRequest {
    query: String,
    path: Option<String>,
    max_results: usize,
}

#[derive(Debug, Serialize)]
struct EmbedBatchRequest {
    texts: Vec<String>,
}

#[derive(Debug, Deserialize)]
pub struct EmbedBatchResponse {
    pub embeddings: Vec<Vec<f32>>,
    #[allow(dead_code)]
    pub count: usize,
}

#[derive(Debug, Deserialize)]
pub struct SearchResponse {
    pub results: Vec<SearchResultJson>,
    pub query: String,
    pub total: usize,
}

#[derive(Debug, Deserialize)]
pub struct SearchResultJson {
    pub path: String,
    pub score: f32,
    pub score_percent: String,
    pub preview: Option<String>,
    pub start_line: i32,
    pub end_line: i32,
}

pub struct Client {
    base_url: String,
}

impl Client {
    pub fn new(host: &str, port: u16) -> Self {
        Self {
            base_url: format!("http://{}:{}", host, port),
        }
    }

    pub fn search(
        &self,
        query: &str,
        path: Option<&Path>,
        max_results: usize,
    ) -> Result<SearchResponse> {
        let request = SearchRequest {
            query: query.to_string(),
            path: path.map(|p| p.to_string_lossy().to_string()),
            max_results,
        };

        let body = serde_json::to_string(&request)?;

        // Simple HTTP POST using TCP (avoiding extra dependencies)
        let host_port = self.base_url.trim_start_matches("http://");
        let mut stream = TcpStream::connect(host_port)
            .context("Failed to connect to vgrep server. Is it running? Start with: vgrep serve")?;

        let request = format!(
            "POST /search HTTP/1.1\r\n\
             Host: {}\r\n\
             Content-Type: application/json\r\n\
             Content-Length: {}\r\n\
             Connection: close\r\n\
             \r\n\
             {}",
            host_port,
            body.len(),
            body
        );

        stream.write_all(request.as_bytes())?;
        stream.flush()?;

        let mut reader = BufReader::new(stream);
        let mut response = String::new();

        // Read headers
        loop {
            let mut line = String::new();
            reader.read_line(&mut line)?;
            if line == "\r\n" || line.is_empty() {
                break;
            }
        }

        // Read body
        reader.read_to_string(&mut response)?;

        let search_response: SearchResponse =
            serde_json::from_str(&response).context("Failed to parse server response")?;

        Ok(search_response)
    }

    pub fn embed_batch(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>> {
        let request = EmbedBatchRequest {
            texts: texts.iter().map(|s| s.to_string()).collect(),
        };

        let body = serde_json::to_string(&request)?;
        let host_port = self.base_url.trim_start_matches("http://");

        let mut stream = TcpStream::connect(host_port)
            .context("Failed to connect to vgrep server. Is it running? Start with: vgrep serve")?;

        let http_request = format!(
            "POST /embed_batch HTTP/1.1\r\n\
             Host: {}\r\n\
             Content-Type: application/json\r\n\
             Content-Length: {}\r\n\
             Connection: close\r\n\
             \r\n\
             {}",
            host_port,
            body.len(),
            body
        );

        stream.write_all(http_request.as_bytes())?;
        stream.flush()?;

        let mut reader = BufReader::new(stream);
        let mut response = String::new();

        // Read headers
        loop {
            let mut line = String::new();
            reader.read_line(&mut line)?;
            if line == "\r\n" || line.is_empty() {
                break;
            }
        }

        // Read body
        reader.read_to_string(&mut response)?;

        let embed_response: EmbedBatchResponse =
            serde_json::from_str(&response).context("Failed to parse server response")?;

        Ok(embed_response.embeddings)
    }

    pub fn health(&self) -> Result<bool> {
        let host_port = self.base_url.trim_start_matches("http://");

        match TcpStream::connect(host_port) {
            Ok(mut stream) => {
                let request = format!(
                    "GET /health HTTP/1.1\r\n\
                     Host: {}\r\n\
                     Connection: close\r\n\
                     \r\n",
                    host_port
                );

                if stream.write_all(request.as_bytes()).is_err() {
                    return Ok(false);
                }

                if stream.flush().is_err() {
                    return Ok(false);
                }

                // Read and verify the HTTP response
                let mut reader = BufReader::new(stream);
                let mut status_line = String::new();

                // Read the HTTP status line
                if reader.read_line(&mut status_line).is_err() {
                    return Ok(false);
                }

                // Verify we got a successful HTTP response (2xx status code)
                // Status line format: "HTTP/1.1 200 OK\r\n"
                let is_healthy = status_line.starts_with("HTTP/")
                    && status_line
                        .split_whitespace()
                        .nth(1)
                        .and_then(|code| code.parse::<u16>().ok())
                        .map(|code| (200..300).contains(&code))
                        .unwrap_or(false);

                Ok(is_healthy)
            }
            Err(_) => Ok(false),
        }
    }
}
