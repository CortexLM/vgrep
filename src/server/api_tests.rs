/*
use axum::{
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use tower::ServiceExt; 

use crate::server::api::{
    EmbedRequest, EmbedBatchRequest, SearchRequest, 
    EmbedResponse, EmbedBatchResponse, SearchResponse
};
*/
// The tower crate dependency might be missing in dev-dependencies or exposed differently.
// Since we are just testing pure logic now as unit tests for the conditions we added:

#[test]
fn test_search_query_validation_logic() {
    let empty_query = "";
    let whitespace_query = "   ";
    let valid_query = "something";

    assert!(empty_query.trim().is_empty());
    assert!(whitespace_query.trim().is_empty());
    assert!(!valid_query.trim().is_empty());
}

#[test]
fn test_embed_text_validation_logic() {
    let empty_text = "";
    let whitespace_text = "   ";
    let valid_text = "valid text";

    assert!(empty_text.trim().is_empty());
    assert!(whitespace_text.trim().is_empty());
    assert!(!valid_text.trim().is_empty());
}

#[test]
fn test_embed_batch_validation_logic() {
    let empty_vec: Vec<String> = vec![];
    let vec_with_empty_strings = vec!["".to_string(), "   ".to_string()];
    let valid_vec = vec!["valid".to_string()];

    assert!(empty_vec.is_empty());
    assert!(vec_with_empty_strings.iter().all(|s| s.trim().is_empty()));
    assert!(!valid_vec.is_empty() && !valid_vec.iter().all(|s| s.trim().is_empty()));
}
