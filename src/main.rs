use futures_util::{StreamExt, TryStreamExt};
use http_body_util::{BodyExt, Full, StreamBody, combinators::BoxBody};
use hyper::body::{Bytes, Frame};
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper::{Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use std::net::{IpAddr, SocketAddr};
use std::sync::OnceLock;
use std::time::SystemTime;
use tokio::{
    fs::File,
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream, UdpSocket},
    signal, time,
};
use tokio_util::io::ReaderStream;

#[derive(Clone)]
struct ClientInfo {
    ip: String,
    port: String,
    protocol: String,
}

static PORT0_ADDRS: OnceLock<Vec<SocketAddr>> = OnceLock::new();

async fn handle_tcp(mut stream: TcpStream, client: ClientInfo) {
    let message = format!(
        "\r\n{} Port {} is open for IP {}\r\n\r\n",
        client.protocol, client.port, client.ip
    );
    if let Err(e) = stream.write_all(message.as_bytes()).await {
        eprintln!("Failed to write to stream: {}", e);
    }
}

async fn handle_udp(socket: &UdpSocket, addr: SocketAddr) {
    let client = ClientInfo {
        ip: addr.ip().to_string(),
        port: match socket.local_addr() {
            Ok(addr) => {
                if PORT0_ADDRS
                    .get()
                    .map_or(false, |addrs| addrs.contains(&addr))
                {
                    "0".to_string()
                } else {
                    addr.port().to_string()
                }
            }
            Err(_) => "unknown".to_string(),
        },
        protocol: "UDP".to_string(),
    };

    // println!("New UDP message: {} on port {}", client.ip, client.port);

    let message = format!(
        "\r\n{} Port {} is open for IP {}\r\n\r\n",
        client.protocol, client.port, client.ip
    );
    if let Err(e) = socket.send_to(message.as_bytes(), addr).await {
        eprintln!("Failed to write to stream: {}", e);
    }
}

fn http_text(text: &str, status: Option<StatusCode>) -> Response<BoxBody<Bytes, std::io::Error>> {
    let status = status.unwrap_or(StatusCode::OK);
    Response::builder()
        .status(status)
        .header("Access-Control-Allow-Origin", "*")
        .body(
            Full::new(Bytes::from(text.to_string()))
                .map_err(|e| match e {})
                .boxed(),
        )
        .expect("constant status won't error")
}

async fn http_send_file(
    filename: &str,
    client: Option<ClientInfo>,
) -> Result<Response<BoxBody<Bytes, std::io::Error>>, std::io::Error> {
    let file = File::open(filename).await;
    if file.is_err() {
        eprintln!("ERROR: Unable to open file.");
        return Ok(http_text("", Some(StatusCode::NOT_FOUND)));
    }

    let mut file = match file {
        Ok(f) => f,
        Err(e) => {
            eprintln!("ERROR: Unable to open file: {}", e);
            return Ok(http_text("", Some(StatusCode::INTERNAL_SERVER_ERROR)));
        }
    };
    let boxed_body: BoxBody<Bytes, std::io::Error> = if let Some(client) = client {
        let mut content = String::new();
        if let Err(e) = file.read_to_string(&mut content).await {
            eprintln!("ERROR: Unable to read file content: {}", e);
            return Ok(http_text("", Some(StatusCode::INTERNAL_SERVER_ERROR)));
        }
        // Use the current time converted to big-endian as a pseudo-random port number for the example port
        let example_port = if client.port == "80" {
            (SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as u16)
                .to_be()
                .max(1)
                .to_string()
        } else {
            client.port.clone()
        };
        content = content
            .replace("{{port}}", &client.port)
            .replace("{{ip}}", &client.ip)
            .replace("{{protocol}}", &client.protocol)
            .replace("{{example_port}}", &example_port);
        Full::new(content.into()).map_err(|e| match e {}).boxed()
    } else {
        let reader_stream = ReaderStream::new(file);
        BodyExt::boxed(StreamBody::new(reader_stream.map_ok(Frame::data)))
    };

    Ok(Response::builder()
        .status(StatusCode::OK)
        .body(boxed_body)
        .expect("constant status won't error"))
}

async fn handle_http(
    req: Request<hyper::body::Incoming>,
    client: ClientInfo,
) -> Result<Response<BoxBody<Bytes, std::io::Error>>, std::io::Error> {
    if req.method() != hyper::Method::GET {
        return Ok(http_text("", Some(StatusCode::METHOD_NOT_ALLOWED)));
    }

    match req.uri().path() {
        "/" => http_send_file("public/index.html", Some(client)).await,
        "/script.js" => http_send_file("public/script.js", None).await,
        "/bootstrap.min.css" => http_send_file("public/bootstrap.min.css", None).await,
        "/raw" => Ok(http_text(
            format!(
                "{} Port {} is open for IP {}",
                client.protocol, client.port, client.ip
            )
            .as_str(),
            None,
        )),
        "/ping" => Ok(http_text("", Some(StatusCode::NO_CONTENT))),
        _ => Ok(http_text("", Some(StatusCode::NOT_FOUND))),
    }
}

async fn handle_client(stream: TcpStream) {
    const HTTP_METHODS: [&str; 9] = [
        "GET", "POST", "PUT", "DELETE", "HEAD", "OPTIONS", "PATCH", "TRACE", "CONNECT",
    ];

    let client = ClientInfo {
        ip: match stream.peer_addr() {
            Ok(addr) => addr.ip().to_string(),
            Err(_) => "unknown".to_string(),
        },
        port: match stream.local_addr() {
            Ok(addr) => {
                if PORT0_ADDRS.get().map_or(false, |addrs| {
                    addrs
                        .iter()
                        .any(|a| a.ip() == addr.ip() && a.port() == addr.port())
                }) {
                    "0".to_string()
                } else {
                    addr.port().to_string()
                }
            }
            Err(_) => "unknown".to_string(),
        },
        protocol: "TCP".to_string(),
    };
    println!("New TCP client: {} on port {}", client.ip, client.port);

    const TIMEOUT: time::Duration = time::Duration::from_secs(5);
    let mut buffer = [0; 16];
    match time::timeout(TIMEOUT, stream.peek(&mut buffer)).await {
        Ok(Ok(n)) if n > 0 => {
            let request_str = String::from_utf8_lossy(&buffer[..n]);
            let method = request_str.split_whitespace().next().unwrap_or("");
            if HTTP_METHODS.contains(&method) {
                println!(
                    "Received HTTP request from client {} on port {}: {}",
                    client.ip, client.port, method
                );
                let io = TokioIo::new(stream);
                let client_clone = client.clone();
                if let Err(err) = http1::Builder::new()
                    .serve_connection(
                        io,
                        service_fn(move |req| handle_http(req, client_clone.clone())),
                    )
                    .await
                {
                    eprintln!("Error serving connection: {:?}", err);
                }
            } else {
                println!(
                    "Received non-HTTP data from client {} on port {}",
                    client.ip, client.port
                );
                handle_tcp(stream, client).await;
            }
        }
        Ok(Ok(_)) => println!(
            "Client {} closed the connection on port {}",
            client.ip, client.port
        ),
        Ok(Err(e)) => eprintln!(
            "Error reading from client {} on port {}: {}",
            client.ip, client.port, e
        ),
        Err(_) => {
            println!(
                "Timeout waiting for data from client {} on port {}",
                client.ip, client.port
            );
            handle_tcp(stream, client).await;
        }
    }
}

async fn shutdown_signal() {
    signal::ctrl_c()
        .await
        .expect("failed to install CTRL+C signal handler");
}

#[tokio::main]
async fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 4 {
        eprintln!(
            "Usage: {} [start port] [end port] [listening ip]...",
            args[0]
        );
        std::process::exit(1);
    }

    let start_port: u16 = args[1].parse().expect("Invalid start port");
    let end_port: u16 = args[2].parse().expect("Invalid end port");

    if start_port > end_port {
        panic!("Start port must be less than or equal to end port.");
    }

    let listener_ips: Vec<IpAddr> = args[3..]
        .iter()
        .map(|ip_str| {
            ip_str
                .parse()
                .unwrap_or_else(|_| panic!("Invalid IP address: {}", ip_str))
        })
        .collect();

    let mut listeners: Vec<TcpListener> = Vec::new();
    let mut port0_addrs: Vec<SocketAddr> = Vec::new();

    for listener_ip in &listener_ips {
        let new_listeners: Vec<TcpListener> =
            futures::future::join_all((start_port..=end_port).map(|port| {
                let socket = if port != 0 {
                    SocketAddr::new((*listener_ip).into(), port)
                } else {
                    let sock = SocketAddr::new(
                        match listener_ip {
                            IpAddr::V4(_) => IpAddr::V4(std::net::Ipv4Addr::LOCALHOST),
                            IpAddr::V6(_) => IpAddr::V6(std::net::Ipv6Addr::LOCALHOST),
                        },
                        1024,
                    );
                    if !port0_addrs.contains(&sock) {
                        port0_addrs.push(sock);
                    println!("Note: Listening on {}:{} as port 0.", sock.ip(), sock.port());
                    }
                    sock
                };
                async move {
                    match TcpListener::bind(socket).await {
                        Ok(listener) => Ok(listener),
                        Err(e) => {
                            eprintln!("Failed to bind to port {}: {}", port, e);
                            Err(())
                        }
                    }
                }
            }))
            .await
            .into_iter()
            .filter_map(Result::ok) // Keep only successful bindings
            .collect();
        listeners.extend(new_listeners);
    }

    let mut udp_listeners: Vec<UdpSocket> = Vec::new();

    for listener_ip in &listener_ips {
        let new_listeners: Vec<UdpSocket> =
            futures::future::join_all((start_port..=end_port).map(|port| {
                let socket = if port != 0 {
                    SocketAddr::new((*listener_ip).into(), port)
                } else {
                    let sock = SocketAddr::new(
                        match listener_ip {
                            IpAddr::V4(_) => IpAddr::V4(std::net::Ipv4Addr::LOCALHOST),
                            IpAddr::V6(_) => IpAddr::V6(std::net::Ipv6Addr::LOCALHOST),
                        },
                        1024,
                    );
                    if !port0_addrs.contains(&sock) {
                        port0_addrs.push(sock);
                        println!("Note: Listening on {}:{} as port 0.", sock.ip(), sock.port());
                    }
                    sock
                };
                async move {
                    match UdpSocket::bind(socket).await {
                        Ok(listener) => Ok(listener),
                        Err(e) => {
                            eprintln!("Failed to bind to port {}: {}", port, e);
                            Err(())
                        }
                    }
                }
            }))
            .await
            .into_iter()
            .filter_map(Result::ok) // Keep only successful bindings
            .collect();
        udp_listeners.extend(new_listeners);
    }

    if listeners.is_empty() && udp_listeners.is_empty() {
        eprintln!("No ports could be bound. Exiting.");
        std::process::exit(1);
    }

    println!(
        "Listening on ports {} to {} on addresses {}",
        start_port,
        end_port,
        listener_ips
            .iter()
            .map(|ip| ip.to_string())
            .collect::<Vec<_>>()
            .join(", ")
    );

    PORT0_ADDRS
        .set(port0_addrs)
        .expect("Failed to set PORT0_ADDRS");

    let graceful = hyper_util::server::graceful::GracefulShutdown::new();
    let mut signal = std::pin::pin!(shutdown_signal());

    let mut listeners = listeners
        .into_iter()
        .map(|listener| async move {
            loop {
                match listener.accept().await {
                    Ok((stream, _)) => {
                        tokio::spawn(async move {
                            handle_client(stream).await;
                        });
                    }
                    Err(e) => {
                        eprintln!("Error accepting connection: {}", e);
                        break;
                    }
                }
            }
        })
        .collect::<futures::stream::FuturesUnordered<_>>();

    let mut udp_listeners = udp_listeners
        .into_iter()
        .map(|socket| async move {
            let mut buf = [0; 1024];
            loop {
                match socket.recv_from(&mut buf).await {
                    Ok((_, addr)) => {
                        handle_udp(&socket, addr).await;
                    }
                    Err(e) => {
                        eprintln!("Error receiving UDP packet: {}", e);
                        break;
                    }
                }
            }
        })
        .collect::<futures::stream::FuturesUnordered<_>>();

    tokio::select! {
        _ = listeners.next() => {
            eprintln!("A listener has stopped");
        },
        _ = udp_listeners.next() => {
            eprintln!("A UDP listener has stopped");
        },
        _ = &mut signal => {
            eprintln!("Graceful shutdown signal received");
        }
    }

    tokio::select! {
        _ = graceful.shutdown() => eprintln!("All connections gracefully closed"),
        _ = time::sleep(time::Duration::from_secs(10)) => eprintln!("Timed out waiting for connections to close"),
    }
}
