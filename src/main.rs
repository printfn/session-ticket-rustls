use std::io;
use std::net::{Ipv6Addr, SocketAddr};
use std::sync::Arc;
use std::vec::Vec;

use http::{Method, Request, Response, StatusCode};
use http_body_util::Full;
use hyper::body::{Bytes, Incoming};
use hyper::service::service_fn;
use hyper_util::rt::{TokioExecutor, TokioIo};
use hyper_util::server::conn::auto::Builder;
use rcgen::generate_simple_self_signed;
use rustls::ServerConfig;
use tokio::net::TcpListener;
use tokio_rustls::TlsAcceptor;

fn main() {
    // Serve an echo service over HTTPS, with proper error handling.
    if let Err(e) = run_server() {
        eprintln!("FAILED: {e}");
        std::process::exit(1);
    }
}

fn error(err: String) -> io::Error {
    io::Error::new(io::ErrorKind::Other, err)
}

#[tokio::main]
async fn run_server() -> eyre::Result<()> {
    let addr = SocketAddr::new(Ipv6Addr::LOCALHOST.into(), 8001);

    let cert = generate_simple_self_signed(["localhost".to_string(), "::1".to_string()])?;

    let certs = rustls_pemfile::certs(&mut io::BufReader::new(io::Cursor::new(cert.cert.pem())))
        .collect::<io::Result<Vec<_>>>()?;
    let key = rustls_pemfile::private_key(&mut io::BufReader::new(io::Cursor::new(
        cert.key_pair.serialize_pem(),
    )))
    .map(|key| key.unwrap())?;

    println!("Starting to serve on https://{addr}");

    // Create a TCP listener via tokio.
    let incoming = TcpListener::bind(&addr).await?;

    // Build TLS configuration.
    let mut server_config = ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(certs, key)
        .map_err(|e| error(e.to_string()))?;
    server_config.alpn_protocols = vec![b"h2".to_vec(), b"http/1.1".to_vec(), b"http/1.0".to_vec()];
    server_config.ticketer = Arc::new(TicketProducer);

    let tls_acceptor = TlsAcceptor::from(Arc::new(server_config));

    let service = service_fn(echo);

    loop {
        let (tcp_stream, _remote_addr) = incoming.accept().await?;

        let tls_acceptor = tls_acceptor.clone();
        tokio::spawn(async move {
            let tls_stream = match tls_acceptor.accept(tcp_stream).await {
                Ok(tls_stream) => tls_stream,
                Err(err) => {
                    eprintln!("failed to perform tls handshake: {err:#}");
                    return;
                }
            };
            if let Err(err) = Builder::new(TokioExecutor::new())
                .serve_connection(TokioIo::new(tls_stream), service)
                .await
            {
                eprintln!("failed to serve connection: {err:#}");
            }
        });
    }
}

async fn echo(req: Request<Incoming>) -> Result<Response<Full<Bytes>>, hyper::Error> {
    let mut response = Response::new(Full::default());
    match (req.method(), req.uri().path()) {
        (&Method::GET, "/") => {
            *response.body_mut() = Full::from("success!\n");
        }
        _ => {
            *response.status_mut() = StatusCode::NOT_FOUND;
        }
    };
    eprintln!("{}", response.status());
    Ok(response)
}

#[derive(Debug)]
struct TicketProducer;

impl rustls::server::ProducesTickets for TicketProducer {
    fn enabled(&self) -> bool {
        true
    }

    fn lifetime(&self) -> u32 {
        u32::MAX
    }

    fn encrypt(&self, _plain: &[u8]) -> Option<Vec<u8>> {
        Some(vec![0; 16300])
    }

    fn decrypt(&self, _cipher: &[u8]) -> Option<Vec<u8>> {
        None
    }
}
