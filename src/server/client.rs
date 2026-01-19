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
        let response = read_http_response(&mut reader)?;

        if !(200..=299).contains(&response.status_code) {
            return Err(server_error_from_response(&response));
        }

        let search_response: SearchResponse =
            serde_json::from_str(&response.body).context("Failed to parse server response")?;

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
        let response = read_http_response(&mut reader)?;

        if !(200..=299).contains(&response.status_code) {
            return Err(server_error_from_response(&response));
        }

        let embed_response: EmbedBatchResponse =
            serde_json::from_str(&response.body).context("Failed to parse server response")?;

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

#[derive(Debug)]
struct HttpResponse {
    status_code: u16,
    status_line: String,
    body: String,
}

fn read_http_response(reader: &mut BufReader<TcpStream>) -> Result<HttpResponse> {
    let mut status_line = String::new();
    reader
        .read_line(&mut status_line)
        .context("Failed to read HTTP status line")?;

    if status_line.is_empty() {
        anyhow::bail!("Empty response from server");
    }

    let status_line = status_line
        .trim_end_matches(['\r', '\n'].as_ref())
        .to_string();
    let status_code = parse_http_status_code(&status_line)?;

    // Read headers
    loop {
        let mut line = String::new();
        reader
            .read_line(&mut line)
            .context("Failed to read HTTP header")?;
        if line == "\r\n" || line.is_empty() {
            break;
        }
    }

    // Read body
    let mut body = String::new();
    reader
        .read_to_string(&mut body)
        .context("Failed to read HTTP response body")?;

    Ok(HttpResponse {
        status_code,
        status_line,
        body,
    })
}

fn parse_http_status_code(status_line: &str) -> Result<u16> {
    let mut parts = status_line.split_whitespace();
    let http_version = parts
        .next()
        .context("Invalid HTTP status line: missing version")?;
    let code_str = parts
        .next()
        .context("Invalid HTTP status line: missing status code")?;

    if !http_version.starts_with("HTTP/") {
        anyhow::bail!("Invalid HTTP status line: {status_line}");
    }

    let code: u16 = code_str
        .parse()
        .with_context(|| format!("Invalid HTTP status code in status line: {status_line}"))?;
    Ok(code)
}

fn server_error_from_response(response: &HttpResponse) -> anyhow::Error {
    let body_trimmed = response.body.trim();

    if let Ok(value) = serde_json::from_str::<serde_json::Value>(&response.body) {
        if let Some(error) = value.get("error").and_then(|v| v.as_str()) {
            return anyhow::anyhow!("Server returned HTTP {}: {}", response.status_code, error);
        }
    }

    if body_trimmed.is_empty() {
        return anyhow::anyhow!(
            "Server returned HTTP {} ({}) with empty body",
            response.status_code,
            response.status_line
        );
    }

    anyhow::anyhow!(
        "Server returned HTTP {} ({}): {}",
        response.status_code,
        response.status_line,
        truncate_for_error(body_trimmed, 500)
    )
}

fn truncate_for_error(s: &str, max_len: usize) -> String {
    if s.len() <= max_len {
        return s.to_string();
    }

    let mut end = max_len;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }

    let mut out = s[..end].to_string();
    out.push_str("...");
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::thread;

    fn spawn_stub_server(response: String) -> (u16, thread::JoinHandle<()>) {
        let listener = TcpListener::bind(("127.0.0.1", 0)).expect("bind stub server");
        let port = listener
            .local_addr()
            .expect("stub server local addr")
            .port();

        let handle = thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept stub connection");

            // Read and ignore the request.
            let mut buf = [0u8; 4096];
            let _ = stream.read(&mut buf);

            stream
                .write_all(response.as_bytes())
                .expect("write stub response");
            let _ = stream.flush();
        });

        (port, handle)
    }

    #[test]
    fn search_includes_http_status_on_error() {
        let body = r#"{"error":"bad request"}"#;
        let response = format!(
            "HTTP/1.1 400 Bad Request\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.len(),
            body
        );

        let (port, handle) = spawn_stub_server(response);
        let client = Client::new("127.0.0.1", port);

        let err = client.search("q", None, 10).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("HTTP 400"), "unexpected error message: {msg}");
        assert!(
            msg.contains("bad request"),
            "unexpected error message: {msg}"
        );

        handle.join().expect("stub server thread join");
    }

    #[test]
    fn embed_batch_includes_http_status_on_error() {
        let body = "internal error";
        let response = format!(
            "HTTP/1.1 500 Internal Server Error\r\nContent-Type: text/plain\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.len(),
            body
        );

        let (port, handle) = spawn_stub_server(response);
        let client = Client::new("127.0.0.1", port);

        let err = client.embed_batch(&["hello"]).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("HTTP 500"), "unexpected error message: {msg}");
        assert!(
            msg.contains("internal error"),
            "unexpected error message: {msg}"
        );

        handle.join().expect("stub server thread join");
    }
}
