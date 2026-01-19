use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
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
        let response = read_response_body(&mut reader)?;
        let response =
            String::from_utf8(response).context("Failed to decode server response as UTF-8")?;

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
        let response = read_response_body(&mut reader)?;
        let response =
            String::from_utf8(response).context("Failed to decode server response as UTF-8")?;

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

                if stream.write_all(request.as_bytes()).is_ok() {
                    return Ok(true);
                }
                Ok(false)
            }
            Err(_) => Ok(false),
        }
    }
}

fn read_response_body(reader: &mut BufReader<TcpStream>) -> Result<Vec<u8>> {
    let mut status_line = String::new();
    reader.read_line(&mut status_line)?;
    if status_line.is_empty() {
        anyhow::bail!("Empty response from server");
    }

    let mut headers: HashMap<String, String> = HashMap::new();
    loop {
        let mut line = String::new();
        reader.read_line(&mut line)?;
        if line == "\r\n" || line.is_empty() {
            break;
        }

        if let Some((name, value)) = line.split_once(':') {
            headers.insert(name.trim().to_ascii_lowercase(), value.trim().to_string());
        }
    }

    let transfer_encoding = headers
        .get("transfer-encoding")
        .map(|value| value.to_ascii_lowercase())
        .unwrap_or_default();

    if transfer_encoding.contains("chunked") {
        read_chunked_body(reader)
    } else if let Some(content_length) = headers.get("content-length") {
        let length: usize = content_length
            .parse()
            .context("Invalid Content-Length header")?;
        let mut body = vec![0u8; length];
        reader.read_exact(&mut body)?;
        Ok(body)
    } else {
        let mut body = Vec::new();
        reader.read_to_end(&mut body)?;
        Ok(body)
    }
}

fn read_chunked_body(reader: &mut BufReader<TcpStream>) -> Result<Vec<u8>> {
    let mut body = Vec::new();

    loop {
        let mut size_line = String::new();
        reader.read_line(&mut size_line)?;
        if size_line.is_empty() {
            break;
        }

        let trimmed = size_line.trim_end_matches(['\r', '\n'].as_ref());
        if trimmed.is_empty() {
            continue;
        }

        let size_str = trimmed.split(';').next().unwrap_or("");
        let size = usize::from_str_radix(size_str.trim(), 16)
            .context("Invalid chunk size in server response")?;

        if size == 0 {
            loop {
                let mut trailer = String::new();
                reader.read_line(&mut trailer)?;
                if trailer == "\r\n" || trailer.is_empty() {
                    break;
                }
            }
            break;
        }

        let mut chunk = vec![0u8; size];
        reader.read_exact(&mut chunk)?;
        body.extend_from_slice(&chunk);

        let mut crlf = [0u8; 2];
        reader.read_exact(&mut crlf)?;
        if crlf != [b'\r', b'\n'] {
            anyhow::bail!("Invalid chunk terminator in server response");
        }
    }

    Ok(body)
}
