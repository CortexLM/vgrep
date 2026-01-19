#[cfg(test)]
mod tests {
    use super::*;
    use httpmock::prelude::*;
    use serde_json::json;

    fn client_for(server: &MockServer) -> PlatformClient {
        PlatformClient::new(&server.base_url())
    }

    #[tokio::test]
    async fn test_check_health() {
        // Scenario 1: Healthy server (200 OK)
        let server = MockServer::start();
        let _ok = server.mock(|when, then| {
            when.method(GET).path("/health");
            then.status(200).body("OK");
        });

        let client = client_for(&server);
        let healthy = client.check_health().await.unwrap();
        assert!(healthy, "Should be healthy when status is 200");

        // Scenario 2: Unhealthy server (500 Error)
        // This validates the fix for issue #46
        let err_server = MockServer::start();
        let _err = err_server.mock(|when, then| {
            when.method(GET).path("/health");
            then.status(500).body("Internal Server Error");
        });

        let err_client = client_for(&err_server);
        let unhealthy = err_client.check_health().await.unwrap();
        assert!(!unhealthy, "Should be unhealthy when status is 500");
    }
}
