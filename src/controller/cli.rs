// use crate::controller::tipi::{Connection, SharedState};
// use std::sync::Arc;
// use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
// use tokio::net::{TcpListener, TcpStream};

// pub async fn run_tcp() -> tokio::io::Result<()> {
//     let listener = TcpListener::bind("127.0.0.1:8080").await?;

//     println!("TCP chat posluša na 127.0.0.1:8080");

//     loop {
//         let (socket, addr) = listener.accept().await?;
//         let state = Arc::clone(&state);
//         let username = addr.to_string();
//         let conn = Connection::new(username, socket, state);

//         tokio::spawn(async move {
//             conn.handle().await;
//         });
//     }
// }

// impl Connection<TcpStream> {
//     pub fn new(username: String, stream: TcpStream, state: SharedState) -> Self {
//         Self { username, stream, state }
//     }

//     pub async fn handle(self) {
//         let (tx, mut rx) = {
//             let state = self.state.lock().unwrap();
//             (state.tx.clone(), state.tx.subscribe())
//         };

//         let (reader, mut writer) = self.stream.into_split();
//         let mut lines = BufReader::new(reader).lines();
//         let user = self.username;

//         let _ = tx.send(format!("*** {user} se je pridružil ***"));

//         loop {
//             tokio::select! {
//                 line = lines.next_line() => {
//                     match line {
//                         Ok(Some(line)) => {
//                             let _ = tx.send(format!("{user}: {line}"));
//                         }
//                         _ => break,
//                     }
//                 }
//                 msg = rx.recv() => {
//                     match msg {
//                         Ok(message) => {
//                             if !message.starts_with(&format!("{user}:")) {
//                                 if writer.write_all(format!("{message}\n").as_bytes()).await.is_err() {
//                                     break;
//                                 }
//                             }
//                         }
//                         Err(_) => break,
//                     }
//                 }
//             }
//         }

//         let _ = tx.send(format!("*** {user} je odšel ***"));
//     }
// }