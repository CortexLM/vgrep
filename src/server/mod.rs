//! HTTP server and client.

mod api;
mod client;

#[cfg(test)]
mod api_tests;

pub use api::run_server;
pub use client::Client;
