use futures_util::{StreamExt, TryStreamExt};
use http_body_util::{BodyExt, Full, StreamBody, combinators::BoxBody};
use hyper::body::{Bytes, Frame};
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper::{Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use std::collections::HashMap;
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
    srcport: String,
    port: String,
    protocol: String,
}

static PORT_MAPS: OnceLock<HashMap<SocketAddr, u16>> = OnceLock::new();

async fn handle_tcp(mut stream: TcpStream, client: ClientInfo) {
    let message = format!(
        "\r\n{} Port {} is open for IP {} (source port {})\r\n\r\n",
        client.protocol, client.port, client.ip, client.srcport
    );
    if let Err(e) = stream.write_all(message.as_bytes()).await {
        eprintln!("Failed to write to stream: {}", e);
    }
}

async fn handle_udp(socket: &UdpSocket, addr: SocketAddr) {
    let client = ClientInfo {
        ip: addr.ip().to_string(),
        srcport: addr.port().to_string(),
        port: match socket.local_addr() {
            Ok(addr) => {
                let port_map = PORT_MAPS.get();
                if port_map.is_some() && port_map.unwrap().contains_key(&addr) {
                    port_map.unwrap()[&addr].to_string()
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
        "\r\n{} Port {} is open for IP {} (source port {})\r\n\r\n",
        client.protocol, client.port, client.ip, client.srcport
    );
    if let Err(e) = socket.send_to(message.as_bytes(), addr).await {
        eprintln!("Failed to write to stream: {}", e);
    }
}

fn http_text(text: &str, status: Option<StatusCode>) -> Response<BoxBody<Bytes, std::io::Error>> {
    let status = status.unwrap_or(StatusCode::OK);
    let mut builder = Response::builder()
        .status(status)
        .header("Access-Control-Allow-Origin", "*");

    if !text.is_empty() {
        builder = builder.header("Content-Type", "text/plain");
    }

    builder
        .body(
            Full::new(Bytes::from(text.to_string()))
                .map_err(|e| match e {})
                .boxed(),
        )
        .expect("constant status won't error")
}

async fn http_send_file(
    filename: &str,
    content_type: &str,
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
        .header("Content-Type", content_type)
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
        "/" => {
            if req
                .headers()
                .get("Accept")
                .map_or(false, |h| h.to_str().unwrap_or("").contains("text/html"))
            {
                http_send_file("public/index.html", "text/html", Some(client)).await
            } else {
                Ok(http_text(
                    format!(
                        "{} Port {} is open for IP {} (source port {})\r\n",
                        client.protocol, client.port, client.ip, client.srcport
                    )
                    .as_str(),
                    None,
                ))
            }
        }
        "/index.html" => http_send_file("public/index.html", "text/html", Some(client)).await,
        "/script.js" => http_send_file("public/script.js", "application/javascript", None).await,
        "/bootstrap.min.css" => http_send_file("public/bootstrap.min.css", "text/css", None).await,
        "/raw" => Ok(http_text(
            format!(
                "{} Port {} is open for IP {} (source port {})\r\n",
                client.protocol, client.port, client.ip, client.srcport
            )
            .as_str(),
            None,
        )),
        "/json" => Ok(http_text(
            format!(
                "{{\"protocol\":\"{}\",\"port\":\"{}\",\"ip\":\"{}\",\"srcport\":\"{}\"}}",
                client.protocol, client.port, client.ip, client.srcport
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
        srcport: match stream.peer_addr() {
            Ok(addr) => addr.port().to_string(),
            Err(_) => "unknown".to_string(),
        },
        port: match stream.local_addr() {
            Ok(addr) => {
                let port_map = PORT_MAPS.get();
                if port_map.is_some() && port_map.unwrap().contains_key(&addr) {
                    port_map.unwrap()[&addr].to_string()
                } else {
                    addr.port().to_string()
                }
            }
            Err(_) => "unknown".to_string(),
        },
        protocol: "TCP".to_string(),
    };
    println!(
        "New TCP client: {}:{} on port {}",
        client.ip, client.srcport, client.port
    );

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

fn print_usage(program_name: &str) {
    eprintln!(
        "Usage: {} [ip-address]:start_port[-end_port][:mapped_port]...",
        program_name
    );
    std::process::exit(1);
}

#[tokio::main]
async fn main() {
    let args: Vec<String> = std::env::args().collect();
    struct Config {
        start_port: u16,
        end_port: u16,
        mapped_port: Option<u16>,
        listener_ip: IpAddr,
    }

    let mut configs: Vec<Config> = Vec::new();

    for arg in &args[1..] {
        let (raw_ip, port_spec, mapped_port) = if let Some(ip_and_rest) = arg.strip_prefix('[') {
            let close_bracket = ip_and_rest.find(']').unwrap_or_else(|| {
                print_usage(args[0].as_str());
                unreachable!();
            });
            let raw_ip = &ip_and_rest[..close_bracket];
            let rest = ip_and_rest[close_bracket + 1..]
                .strip_prefix(':')
                .unwrap_or_else(|| {
                    print_usage(args[0].as_str());
                    unreachable!();
                });
            let mut parts = rest.split(':');
            let port_spec = parts.next().unwrap_or_else(|| {
                print_usage(args[0].as_str());
                unreachable!();
            });
            let mapped_port = parts.next();
            if parts.next().is_some() {
                print_usage(args[0].as_str());
                unreachable!();
            }
            (raw_ip, port_spec, mapped_port)
        } else {
            let colon_count = arg.matches(':').count();
            if colon_count == 2 {
                let parts: Vec<&str> = arg.split(':').collect();
                let [raw_ip, port_spec, mapped_port] = parts.as_slice() else {
                    print_usage(args[0].as_str());
                    unreachable!();
                };
                (*raw_ip, *port_spec, Some(*mapped_port))
            } else {
                let (raw_ip, port_spec) = arg.rsplit_once(':').unwrap_or_else(|| {
                    print_usage(args[0].as_str());
                    unreachable!();
                });
                (raw_ip, port_spec, None)
            }
        };
        let ip_text = raw_ip
            .strip_prefix('[')
            .and_then(|text| text.strip_suffix(']'))
            .unwrap_or(raw_ip);
        let ip_addr = ip_text
            .parse()
            .unwrap_or_else(|_| panic!("Invalid IP address: {}", ip_text));
        let ports = port_spec.split('-').collect::<Vec<&str>>();
        let start_port: u16;
        let end_port: u16;

        if ports.len() == 1 {
            start_port = ports[0]
                .parse()
                .unwrap_or_else(|_| panic!("Invalid port: {}", port_spec));
            end_port = start_port;
        } else if ports.len() == 2 {
            start_port = ports[0]
                .parse()
                .unwrap_or_else(|_| panic!("Invalid start port: {}", ports[0]));
            end_port = ports[1]
                .parse()
                .unwrap_or_else(|_| panic!("Invalid end port: {}", ports[1]));
            if start_port == 0 || end_port == 0 {
                panic!("Port ranges must be between 1 and 65535.");
            }
        } else {
            print_usage(args[0].as_str());
            panic!();
        }

        if start_port > end_port {
            panic!("Start port must be less than or equal to end port.");
        }

        let mapped_port = if let Some(mapped_port) = mapped_port {
            let mapped_port = mapped_port
                .parse::<u16>()
                .unwrap_or_else(|_| panic!("Invalid mapped port: {}", mapped_port));
            if start_port != end_port {
                panic!("Port mapping can only be used for single ports, not ranges.");
            }
            Some(mapped_port)
        } else {
            None
        };

        configs.push(Config {
            start_port,
            end_port,
            mapped_port,
            listener_ip: ip_addr,
        });
    }

    let mut tcp_listeners: Vec<TcpListener> = Vec::new();
    let mut udp_listeners: Vec<UdpSocket> = Vec::new();
    let mut port_maps: HashMap<SocketAddr, u16> = HashMap::new();

    // TCP listeners
    for config in &configs {
        let new_listeners: Vec<TcpListener> =
            futures::future::join_all((config.start_port..=config.end_port).map(|port| {
                let socket = SocketAddr::new(config.listener_ip, port);
                if config.mapped_port.is_some() && !port_maps.contains_key(&socket) {
                    port_maps.insert(socket, config.mapped_port.unwrap());
                    let mapped_port = config.mapped_port.unwrap();
                    println!(
                        "Note: Listening on {}:{} as port {}.",
                        socket.ip(),
                        socket.port(),
                        mapped_port
                    );
                }
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
        tcp_listeners.extend(new_listeners);
    }

    // UDP listeners
    for config in &configs {
        let new_listeners: Vec<UdpSocket> =
            futures::future::join_all((config.start_port..=config.end_port).map(|port| {
                let socket = SocketAddr::new(config.listener_ip, port);
                if config.mapped_port.is_some() && !port_maps.contains_key(&socket) {
                    port_maps.insert(socket, config.mapped_port.unwrap());
                    let mapped_port = config.mapped_port.unwrap();
                    println!(
                        "Note: Listening on {}:{} as port {}.",
                        socket.ip(),
                        socket.port(),
                        mapped_port
                    );
                }
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

    if tcp_listeners.is_empty() && udp_listeners.is_empty() {
        eprintln!("No ports could be bound. Exiting.");
        std::process::exit(1);
    }

    PORT_MAPS.set(port_maps).expect("Failed to set PORT_MAPS");

    let mut tcp_listeners = tcp_listeners
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

    for config in &configs {
        println!(
            "Listening on ports {} to {} on address {}",
            config.start_port, config.end_port, config.listener_ip
        );
    }

    let graceful = hyper_util::server::graceful::GracefulShutdown::new();
    let mut signal = std::pin::pin!(shutdown_signal());

    tokio::select! {
        _ = tcp_listeners.next() => {
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
