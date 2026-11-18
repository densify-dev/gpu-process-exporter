// SPDX-License-Identifier: Apache-2.0

//! HTTP exposition of the registry, matching Go's `ServeMux` + `promhttp.Handler()`.

use super::negotiate::{Format, negotiate_format, negotiate_gzip};
use crate::log;
use anyhow::{Context, bail};
use bytes::Bytes;
use flate2::{Compression, write::GzEncoder};
use http_body_util::Full;
use hyper::header::{CONTENT_ENCODING, CONTENT_TYPE, HeaderValue, VARY};
use hyper::{Request, Response, StatusCode};
use hyper_util::rt::{TokioIo, TokioTimer};
use hyper_util::server::graceful::GracefulShutdown;
use prometheus::{Encoder, ProtobufEncoder, Registry, TextEncoder};
use std::collections::HashSet;
use std::io::Write;
use std::net::{Ipv4Addr, Ipv6Addr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;
use tokio::net::TcpListener;
use tokio_util::sync::CancellationToken;

const READ_HEADER_TIMEOUT: Duration = Duration::from_secs(5);
const SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(5);
const NOT_FOUND_BODY: &str = "404 page not found\n";
const NOT_FOUND_CONTENT_TYPE: &str = "text/plain; charset=utf-8";

struct Handler {
    registry: Registry,
    endpoint: String,
}

/// Serves until cancelled, then shuts down gracefully. Like Go, a shutdown that outlasts the timeout is an error.
pub async fn listen_and_serve(
    registry: Registry,
    endpoint: String,
    port: u16,
    cancel: CancellationToken,
) -> anyhow::Result<()> {
    let listener = bind(port).await?;
    let handler = Arc::new(Handler { registry, endpoint });
    let mut http = hyper::server::conn::http1::Builder::new();
    http.timer(TokioTimer::new())
        .header_read_timeout(READ_HEADER_TIMEOUT);
    let graceful = GracefulShutdown::new();
    loop {
        let stream = tokio::select! {
            accepted = listener.accept() => match accepted {
                Ok((stream, _)) => stream,
                Err(e) => {
                    log!("http: accept error: {e}");
                    continue;
                }
            },
            _ = cancel.cancelled() => break,
        };
        let handler = handler.clone();
        let service = hyper::service::service_fn(move |req| {
            let handler = handler.clone();
            async move { Ok::<_, std::convert::Infallible>(handler.handle(&req)) }
        });
        let conn = graceful.watch(http.serve_connection(TokioIo::new(stream), service));
        tokio::spawn(conn);
    }
    drop(listener);
    tokio::time::timeout(SHUTDOWN_TIMEOUT, graceful.shutdown())
        .await
        .context("shutdown http server")
}

/// Go's `":port"` listens on all IPv4 and IPv6 addresses, or IPv4 only when IPv6 is unavailable.
async fn bind(port: u16) -> anyhow::Result<TcpListener> {
    match TcpListener::bind(SocketAddr::from((Ipv6Addr::UNSPECIFIED, port))).await {
        Ok(listener) => Ok(listener),
        Err(e) if e.kind() == std::io::ErrorKind::AddrInUse => bail!("listen tcp :{port}: {e}"),
        Err(_) => TcpListener::bind(SocketAddr::from((Ipv4Addr::UNSPECIFIED, port)))
            .await
            .with_context(|| format!("listen tcp :{port}")),
    }
}

impl Handler {
    fn handle<B>(&self, req: &Request<B>) -> Response<Full<Bytes>> {
        if req.uri().path() != self.endpoint {
            return Response::builder()
                .status(StatusCode::NOT_FOUND)
                .header(CONTENT_TYPE, NOT_FOUND_CONTENT_TYPE)
                .header("X-Content-Type-Options", "nosniff")
                .body(Full::from(NOT_FOUND_BODY))
                .unwrap();
        }
        let headers = req.headers();
        let accept = headers
            .get(hyper::header::ACCEPT)
            .and_then(|v| v.to_str().ok());
        let (format, content_type) = negotiate_format(accept);
        let families = filter_families(self.registry.gather(), req.uri().query());
        let mut body = Vec::new();
        let encoded = match format {
            Format::Text => TextEncoder::new().encode(&families, &mut body),
            Format::ProtoDelim => ProtobufEncoder::new().encode(&families, &mut body),
        };
        if let Err(e) = encoded {
            log!("http: encode metrics: {e}");
        }
        let gzip = negotiate_gzip(
            headers
                .get_all(hyper::header::ACCEPT_ENCODING)
                .iter()
                .filter_map(|v| v.to_str().ok()),
        );
        let mut response = Response::builder()
            .header(VARY, "Accept")
            .header(VARY, "Accept-Encoding")
            .header(CONTENT_TYPE, HeaderValue::from_str(&content_type).unwrap());
        if gzip {
            let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
            encoder.write_all(&body).unwrap();
            body = encoder.finish().unwrap();
            response = response.header(CONTENT_ENCODING, "gzip");
        }
        response.body(Full::from(body)).unwrap()
    }
}

/// promhttp serves only the families named by `name[]` query parameters, when there are any. Like Go's
/// `url.ParseQuery`, pairs containing `;` are skipped.
fn filter_families(
    families: Vec<prometheus::proto::MetricFamily>,
    query: Option<&str>,
) -> Vec<prometheus::proto::MetricFamily> {
    let names: HashSet<String> = query
        .unwrap_or_default()
        .split('&')
        .filter(|pair| !pair.contains(';'))
        .flat_map(|pair| form_urlencoded::parse(pair.as_bytes()))
        .filter(|(key, _)| key == "name[]")
        .map(|(_, value)| value.into_owned())
        .collect();
    if names.is_empty() {
        return families;
    }
    families
        .into_iter()
        .filter(|f| names.contains(f.name()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use prometheus::{Gauge, Opts};

    fn handler() -> Handler {
        let registry = Registry::new();
        for name in ["a", "b"] {
            let gauge = Gauge::with_opts(Opts::new(name, "help")).unwrap();
            gauge.set(1.0);
            registry.register(Box::new(gauge)).unwrap();
        }
        Handler {
            registry,
            endpoint: "/custom/path".into(),
        }
    }

    fn get(handler: &Handler, uri: &str, headers: &[(&str, &str)]) -> Response<Full<Bytes>> {
        let mut req = Request::builder().uri(uri);
        for (k, v) in headers {
            req = req.header(*k, *v);
        }
        handler.handle(&req.body(()).unwrap())
    }

    async fn body(response: Response<Full<Bytes>>) -> String {
        use http_body_util::BodyExt;
        String::from_utf8(
            response
                .into_body()
                .collect()
                .await
                .unwrap()
                .to_bytes()
                .to_vec(),
        )
        .unwrap()
    }

    #[tokio::test]
    async fn routes_only_the_exact_endpoint() {
        let h = handler();
        for path in [
            "/",
            "/metrics",
            "/custom",
            "/custom/path/",
            "/custom/path/x",
        ] {
            let response = get(&h, path, &[]);
            assert_eq!(response.status(), StatusCode::NOT_FOUND, "{path}");
            assert_eq!(response.headers()[CONTENT_TYPE], NOT_FOUND_CONTENT_TYPE);
            assert_eq!(body(response).await, NOT_FOUND_BODY);
        }
        let response = get(&h, "/custom/path?x=1", &[]);
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response.headers()[CONTENT_TYPE],
            "text/plain; version=0.0.4; charset=utf-8; escaping=underscores"
        );
        assert!(response.headers().get(CONTENT_ENCODING).is_none());
        assert_eq!(
            body(response).await,
            "# HELP a help\n# TYPE a gauge\na 1\n# HELP b help\n# TYPE b gauge\nb 1\n"
        );
    }

    #[tokio::test]
    async fn filters_by_name_query() {
        let h = handler();
        assert_eq!(
            body(get(&h, "/custom/path?name[]=b", &[])).await,
            "# HELP b help\n# TYPE b gauge\nb 1\n"
        );
        assert_eq!(
            body(get(&h, "/custom/path?name%5B%5D=a&name[]=zzz", &[])).await,
            "# HELP a help\n# TYPE a gauge\na 1\n"
        );
        assert_eq!(
            body(get(&h, "/custom/path?name[]=a;x", &[]))
                .await
                .lines()
                .count(),
            6
        );
    }

    #[tokio::test]
    async fn gzips_when_requested() {
        use std::io::Read;
        let response = get(&handler(), "/custom/path", &[("Accept-Encoding", "gzip")]);
        assert_eq!(response.headers()[CONTENT_ENCODING], "gzip");
        use http_body_util::BodyExt;
        let compressed = response.into_body().collect().await.unwrap().to_bytes();
        let mut text = String::new();
        flate2::read::GzDecoder::new(&compressed[..])
            .read_to_string(&mut text)
            .unwrap();
        assert!(text.starts_with("# HELP a help\n"));
    }
}
