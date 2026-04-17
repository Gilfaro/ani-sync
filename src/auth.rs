use axum::{
    Router,
    extract::{Query, State},
    response::Html,
    routing::get,
};
use color_eyre::Result;
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::sync::{Mutex, oneshot};

type SharedSender = Arc<Mutex<Option<oneshot::Sender<String>>>>;

/// Captures an OAuth callback by starting a temporary local server.
///
/// # Errors
///
/// Returns an error if the server fails to bind to the port or if the callback is not received.
///
/// # Panics
///
/// Panics if the server fails to start.
pub async fn capture_oauth_callback(port: u16) -> Result<String> {
    let (tx, rx) = oneshot::channel::<String>();
    let tx = Arc::new(Mutex::new(Some(tx)));

    let app = Router::new()
        .route("/", get(handle_callback))
        .route("/callback", get(handle_callback))
        .with_state(tx);

    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    let listener = tokio::net::TcpListener::bind(addr).await?;

    let (shutdown_tx, shutdown_rx) = oneshot::channel();

    let server_handle = tokio::spawn(async move {
        axum::serve(listener, app)
            .with_graceful_shutdown(async move {
                let _ = shutdown_rx.await;
            })
            .await
            .unwrap();
    });

    let callback_url = rx.await?;

    let _ = shutdown_tx.send(());
    let _ = server_handle.await;

    Ok(callback_url)
}

async fn handle_callback(
    State(tx): State<SharedSender>,
    Query(query): Query<HashMap<String, String>>,
) -> Html<&'static str> {
    // If there are no query parameters, serve the fragment forwarding script
    if query.is_empty() {
        return Html(
            r#"
            <html>
            <head><title>Ani-Sync Authorization</title></head>
            <body>
                <p>Completing authorization...</p>
                <script>
                    if (window.location.hash) {
                        const fragment = window.location.hash.substring(1);
                        const forwardUrl = "/?forwarded_fragment=" +
                                         encodeURIComponent(fragment);
                        window.location.href = forwardUrl;
                    } else {
                        const errorMsg = "<h2>Error: No authorization data found.</h2>";
                        document.body.innerHTML = errorMsg;
                    }
                </script>
            </body>
            </html>
            "#,
        );
    }

    // Convert query parameters back into a query string
    let query_str = query
        .iter()
        .map(|(k, v)| format!("{k}={v}"))
        .collect::<Vec<_>>()
        .join("&");

    let mut tx_lock = tx.lock().await;
    if let Some(sender) = tx_lock.take() {
        let _ = sender.send(format!("/?{query_str}"));
    }

    Html(
        r#"
        <html>
        <head><title>Ani-Sync Authorization</title></head>
        <body style="font-family: sans-serif; text-align: center; margin-top: 50px;">
            <h2>Authorization Successful!</h2>
            <p>You can close this window and return to your terminal.</p>
        </body>
        </html>
        "#,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use reqwest::Client;
    use std::time::Duration;

    #[tokio::test]
    async fn test_capture_oauth_callback() {
        let server_task = tokio::spawn(async { capture_oauth_callback(9145).await });

        tokio::time::sleep(Duration::from_millis(100)).await;

        let client = Client::new();
        let res = client
            .get("http://127.0.0.1:9145/?code=12345")
            .send()
            .await
            .unwrap();

        assert!(res.status().is_success());
        let body = res.text().await.unwrap();
        assert!(body.contains("Authorization Successful!"));

        let captured_url = server_task.await.unwrap().unwrap();
        assert_eq!(captured_url, "/?code=12345");
    }
}
