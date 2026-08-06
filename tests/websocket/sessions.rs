use crate::common::{start_server, test_app, websocket_request};
use axum::http::StatusCode;

use tokio_tungstenite::{connect_async, tungstenite::Error as WsError};

#[tokio::test]
async fn websocket_rejects_missing_and_invalid_sessions() {
    let (app, _db) = test_app().await;
    let (address, server) = start_server(app).await;

    let error = match connect_async(format!("ws://{address}/ws?room_name=general")).await {
        Ok(_) => panic!("WebSocket brez seje ne bi smel biti sprejet"),
        Err(error) => error,
    };
    match error {
        WsError::Http(response) => {
            assert_eq!(
                response.status().as_u16(),
                StatusCode::UNAUTHORIZED.as_u16()
            )
        }
        other => panic!("pričakovan je bil HTTP 401, dobljeno: {other}"),
    }

    let request = websocket_request(address, "general", "chat_session=neveljaven-podpis");
    let error = match connect_async(request).await {
        Ok(_) => panic!("WebSocket z neveljavno sejo ne bi smel biti sprejet"),
        Err(error) => error,
    };
    match error {
        WsError::Http(response) => {
            assert_eq!(
                response.status().as_u16(),
                StatusCode::UNAUTHORIZED.as_u16()
            )
        }
        other => panic!("pričakovan je bil HTTP 401, dobljeno: {other}"),
    }

    server.abort();
}
