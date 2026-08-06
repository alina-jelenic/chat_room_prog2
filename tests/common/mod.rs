use axum::{
    Router,
    body::{Body, to_bytes},
    http::{
        Request, StatusCode,
        header::{CONTENT_TYPE, COOKIE, SET_COOKIE},
    },
};
use chat_room_prog2::controller::{
    rooms::{ensure_default_room, prepare_database_schema},
    tipi::ServerState,
    web::build_router,
};
use futures_util::StreamExt;
use sea_orm::{ConnectOptions, Database, DatabaseConnection};
use std::net::SocketAddr;
use tokio::time::{Duration, timeout};
use tokio_tungstenite::tungstenite::{
    Error as WsError, Message as WsMessage, client::IntoClientRequest,
};
use tower::ServiceExt;

const TEST_SECRET: &str = "test-secret-that-is-longer-than-32-characters";

pub async fn test_app() -> (Router, DatabaseConnection) {
    // Pri SQLite in-memory bazi mora isti test uporabljati eno samo povezavo,
    // sicer bi vsaka povezava dobila svojo prazno bazo.
    let mut options = ConnectOptions::new("sqlite::memory:");
    options.max_connections(1).sqlx_logging(false);
    let db = Database::connect(options).await.unwrap();
    prepare_database_schema(&db).await.unwrap();
    ensure_default_room(&db).await.unwrap();

    let state = ServerState::new(db.clone(), TEST_SECRET.to_string());
    (build_router(state), db)
}

pub fn form_request(method: &str, uri: &str, body: &str, cookie: Option<&str>) -> Request<Body> {
    let mut builder = Request::builder()
        .method(method)
        .uri(uri)
        .header(CONTENT_TYPE, "application/x-www-form-urlencoded");
    if let Some(cookie) = cookie {
        builder = builder.header(COOKIE, cookie);
    }
    builder.body(Body::from(body.to_string())).unwrap()
}

pub async fn body_text(response: axum::response::Response) -> String {
    String::from_utf8(
        to_bytes(response.into_body(), 1024 * 1024)
            .await
            .unwrap()
            .to_vec(),
    )
    .unwrap()
}

pub async fn register_and_login(app: &Router, username: &str) -> String {
    let register_body = format!("username={username}&password=skrivnost1&confirm=skrivnost1");
    let response = app
        .clone()
        .oneshot(form_request("POST", "/api/register", &register_body, None))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let login_body = format!("username={username}&password=skrivnost1");
    let response = app
        .clone()
        .oneshot(form_request("POST", "/api/login", &login_body, None))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()["HX-Redirect"], "/index.html");

    response.headers()[SET_COOKIE]
        .to_str()
        .unwrap()
        .split(';')
        .next()
        .unwrap()
        .to_string()
}

pub async fn start_server(app: Router) -> (SocketAddr, tokio::task::JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    (address, server)
}

pub fn websocket_request(
    address: SocketAddr,
    room_name: &str,
    cookie: &str,
) -> axum::http::Request<()> {
    let mut request = format!("ws://{address}/ws?room_name={room_name}")
        .into_client_request()
        .unwrap();
    request
        .headers_mut()
        .insert(COOKIE, cookie.parse().unwrap());
    request
}

pub fn assert_login_redirect(response: &axum::response::Response) {
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.headers().get("HX-Redirect").unwrap(),
        "/authorisation.html"
    );
}

pub async fn recv_until<S>(socket: &mut S, needle: &str) -> String
where
    S: futures_util::Stream<Item = Result<WsMessage, WsError>> + Unpin,
{
    loop {
        let msg = socket
            .next()
            .await
            .expect("povezava se je nepričakovano zaprla")
            .unwrap();
        let text = msg.into_text().unwrap().to_string();
        if text.contains(needle) {
            return text;
        }
    }
}

pub async fn wait_for_socket_close<S>(socket: &mut S)
where
    S: futures_util::Stream<Item = Result<WsMessage, WsError>> + Unpin,
{
    timeout(Duration::from_secs(2), async {
        loop {
            match socket.next().await {
                None | Some(Ok(WsMessage::Close(_))) | Some(Err(_)) => break,
                Some(Ok(_)) => continue,
            }
        }
    })
    .await
    .expect("strežnik ni pravočasno zaprl WebSocket povezave");
}
