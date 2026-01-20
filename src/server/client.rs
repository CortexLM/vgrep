use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::path::Path;
use std::time::Duration;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
const READ_TIMEOUT: Duration = Duration::from_secs(60);
const WRITE_TIMEOUT: Duration = Duration::from_secs(60);

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
        let addr = host_port
            .to_socket_addrs()?
            .next()
            .context("Failed to resolve server address")?;

        let mut stream = TcpStream::connect_timeout(&addr, CONNECT_TIMEOUT)
            .context("Failed to connect to vgrep server. Is it running? Start with: vgrep serve")?;

        stream.set_read_timeout(Some(READ_TIMEOUT))?;
        stream.set_write_timeout(Some(WRITE_TIMEOUT))?;

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

        let addr = host_port
            .to_socket_addrs()?
            .next()
            .context("Failed to resolve server address")?;

        let mut stream = TcpStream::connect_timeout(&addr, CONNECT_TIMEOUT)
            .context("Failed to connect to vgrep server. Is it running? Start with: vgrep serve")?;

        stream.set_read_timeout(Some(READ_TIMEOUT))?;
        stream.set_write_timeout(Some(WRITE_TIMEOUT))?;

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

        let addr = match host_port.to_socket_addrs()?.next() {
            Some(addr) => addr,
            None => return Ok(false),
        };

        match TcpStream::connect_timeout(&addr, CONNECT_TIMEOUT) {
            Ok(mut stream) => {
                let _ = stream.set_read_timeout(Some(READ_TIMEOUT));
                let _ = stream.set_write_timeout(Some(WRITE_TIMEOUT));

                let request = format!(
                    "GET /health HTTP/1.1\r\n\
                     Host: {}\r\n\
                     Connection: close\r\n\
                     \r\n",
                    host_port
                );

                if stream.write_all(request.as_bytes()).is_ok() {
                    return Ok(true);
                }
                Ok(false)
            }
            Err(_) => Ok(false),
        }
    }
}
