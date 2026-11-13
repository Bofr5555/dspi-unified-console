//! The Terminal's [`Fetch`]: HTTPS in-process, for the AutoEQ rebuild and update.
//!
//! This lives here and not in `dspi-session` because the core and the session
//! carry no HTTP client: every other host supplies its own fetch, and the iOS
//! and Android libraries must not link this one. Nothing here runs unless the
//! person asks for it.
//!
//! The certificate roots are the Mozilla list compiled in (`webpki-roots`), so a
//! rebuild works the same on Linux, Windows and macOS, and on a system with no
//! certificate store. The cost: a certificate an administrator installed for a
//! TLS-inspecting proxy is not trusted, and that person imports a file instead.

use std::time::Duration;

use dspi_session::autoeq::{Fetch, FetchRequest, FetchResponse, MAX_DATABASE_BYTES};
use ureq::config::Config;
use ureq::http::HeaderMap;

/// One request, redirects and body included. A cancel waits for the requests in
/// flight, so this is also the longest a cancel can take.
const TIMEOUT: Duration = Duration::from_secs(30);

/// GitHub redirects a renamed repository once; more than a few is a loop.
const MAX_REDIRECTS: u32 = 10;

/// Fetches over HTTPS. Plain HTTP is refused, on a redirect too. Proxies come
/// from the usual environment variables (`HTTPS_PROXY`, `ALL_PROXY`, `NO_PROXY`).
pub struct HttpFetch {
    agent: ureq::Agent,
    body_limit: u64,
}

impl HttpFetch {
    pub fn new() -> Self {
        Self::with_config(
            ureq::Agent::config_builder()
                .https_only(true)
                .timeout_global(Some(TIMEOUT)),
            MAX_DATABASE_BYTES as u64,
        )
    }

    /// Everything but what a test has to vary.
    fn with_config(
        config: ureq::config::ConfigBuilder<ureq::typestate::AgentScope>,
        body_limit: u64,
    ) -> Self {
        let config: Config = config
            // A 403 or a 500 is an answer, and reading it is the session's job.
            .http_status_as_error(false)
            .max_redirects(MAX_REDIRECTS)
            .build();
        HttpFetch {
            agent: ureq::Agent::new_with_config(config),
            body_limit,
        }
    }
}

impl Default for HttpFetch {
    fn default() -> Self {
        Self::new()
    }
}

impl Fetch for HttpFetch {
    fn get(&self, request: &FetchRequest) -> Result<FetchResponse, String> {
        let mut call = self
            .agent
            .get(&request.url)
            .header("User-Agent", &request.user_agent);
        if let Some(accept) = &request.accept {
            call = call.header("Accept", accept);
        }
        let mut response = call.call().map_err(no_answer)?;
        let mut answer = read_head(response.status().as_u16(), response.headers());
        answer.body = response
            .body_mut()
            .with_config()
            .limit(self.body_limit)
            .read_to_vec()
            .map_err(no_answer)?;
        Ok(answer)
    }
}

fn no_answer(e: ureq::Error) -> String {
    format!("could not reach the server: {e}")
}

/// The status and the headers the session reads. A malformed value reads as
/// absent, the HTTP-date form of `Retry-After` among them.
fn read_head(status: u16, headers: &HeaderMap) -> FetchResponse {
    fn number<T: std::str::FromStr>(headers: &HeaderMap, name: &str) -> Option<T> {
        headers.get(name)?.to_str().ok()?.trim().parse().ok()
    }
    FetchResponse {
        status,
        body: Vec::new(),
        retry_after_s: number(headers, "retry-after"),
        rate_limit_remaining: number(headers, "x-ratelimit-remaining"),
        rate_limit_reset: number(headers, "x-ratelimit-reset"),
    }
}

/// In-process servers on the loopback interface only: no test leaves this
/// machine, and the proxy environment is ignored so that none can.
#[cfg(test)]
mod tests_wp_tm12 {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::{Arc, Mutex};
    use ureq::tls::{Certificate, RootCerts, TlsConfig};

    const CA: &[u8] = include_bytes!("../testdata/tm12/ca.der");
    const LEAF: &[u8] = include_bytes!("../testdata/tm12/leaf.der");
    const LEAF_KEY: &[u8] = include_bytes!("../testdata/tm12/leaf-key.der");

    fn fetch_with(https_only: bool, tls: TlsConfig, timeout: Duration, limit: u64) -> HttpFetch {
        HttpFetch::with_config(
            ureq::Agent::config_builder()
                .https_only(https_only)
                .timeout_global(Some(timeout))
                .tls_config(tls)
                .proxy(None),
            limit,
        )
    }

    /// Plain HTTP, for what does not depend on TLS.
    fn plain() -> HttpFetch {
        fetch_with(
            false,
            TlsConfig::default(),
            Duration::from_secs(10),
            1024 * 1024,
        )
    }

    fn trusting_the_test_ca() -> TlsConfig {
        TlsConfig::builder()
            .root_certs(RootCerts::new_with_certs(&[Certificate::from_der(CA)]))
            .build()
    }

    fn request(url: String, accept: Option<&str>) -> FetchRequest {
        FetchRequest {
            url,
            accept: accept.map(str::to_string),
            user_agent: dspi_session::autoeq::FETCH_USER_AGENT.to_string(),
        }
    }

    /// Read one request's head.
    fn read_request(stream: &mut impl Read) -> Option<String> {
        let mut head = Vec::new();
        let mut byte = [0u8; 1];
        while !head.ends_with(b"\r\n\r\n") {
            if stream.read(&mut byte).ok()? == 0 {
                return None;
            }
            head.push(byte[0]);
        }
        Some(String::from_utf8_lossy(&head).into_owned())
    }

    /// What a server has seen.
    type Seen = Arc<Mutex<Vec<String>>>;

    /// Serve these answers, one per connection, and record each request's head.
    /// Each answer is told the server's own port, for a redirect to name.
    fn serve(answers: Vec<&'static str>) -> (u16, Seen) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let seen = Seen::default();
        let log = seen.clone();
        std::thread::spawn(move || {
            for (answer, stream) in answers.into_iter().zip(listener.incoming()) {
                let mut stream = stream.unwrap();
                if let Some(head) = read_request(&mut stream) {
                    log.lock().unwrap().push(head);
                }
                let _ = stream.write_all(answer.replace("{port}", &port.to_string()).as_bytes());
            }
        });
        (port, seen)
    }

    /// The same over TLS, with the test's certificate for `localhost`.
    fn serve_tls(answers: Vec<&'static str>) -> u16 {
        use rustls::pki_types::{CertificateDer, PrivateKeyDer};
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let config = rustls::ServerConfig::builder_with_provider(Arc::new(
            rustls::crypto::ring::default_provider(),
        ))
        .with_safe_default_protocol_versions()
        .unwrap()
        .with_no_client_auth()
        .with_single_cert(
            vec![CertificateDer::from(LEAF.to_vec())],
            PrivateKeyDer::try_from(LEAF_KEY.to_vec()).unwrap(),
        )
        .unwrap();
        let config = Arc::new(config);
        std::thread::spawn(move || {
            for (answer, stream) in answers.into_iter().zip(listener.incoming()) {
                let connection = rustls::ServerConnection::new(config.clone()).unwrap();
                let mut tls = rustls::StreamOwned::new(connection, stream.unwrap());
                // A client that refuses the certificate ends it here.
                if read_request(&mut tls).is_some() {
                    let _ = tls.write_all(answer.as_bytes());
                    tls.conn.send_close_notify();
                    let _ = tls.flush();
                }
            }
        });
        port
    }

    const OK_HI: &str = "HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nhi";

    #[test]
    fn tm12_a_refusal_is_an_answer_with_its_rate_limit_headers() {
        let (port, _) = serve(vec![
            "HTTP/1.1 403 Forbidden\r\nretry-after: 60\r\nX-RateLimit-Remaining: 0\r\n\
             x-ratelimit-reset: 1700000000\r\nContent-Length: 4\r\nConnection: close\r\n\r\nbody",
        ]);
        let r = plain()
            .get(&request(format!("http://127.0.0.1:{port}/"), None))
            .unwrap();
        assert_eq!(r.status, 403);
        assert_eq!(r.retry_after_s, Some(60));
        assert_eq!(r.rate_limit_remaining, Some(0));
        assert_eq!(r.rate_limit_reset, Some(1_700_000_000));
        assert_eq!(r.body, b"body");
    }

    #[test]
    fn tm12_malformed_rate_limit_headers_read_as_absent() {
        let (port, _) = serve(vec![
            "HTTP/1.1 500 Oops\r\nRetry-After: Wed, 21 Oct 2026 07:28:00 GMT\r\n\
             X-RateLimit-Remaining: -1\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
        ]);
        let r = plain()
            .get(&request(format!("http://127.0.0.1:{port}/"), None))
            .unwrap();
        assert_eq!(
            (r.status, r.retry_after_s, r.rate_limit_remaining),
            (500, None, None)
        );
    }

    #[test]
    fn tm12_redirects_are_followed_and_the_headers_go_with_them() {
        let (port, seen) = serve(vec![
            "HTTP/1.1 302 Found\r\nLocation: http://127.0.0.1:{port}/there\r\n\
             Content-Length: 0\r\nConnection: close\r\n\r\n",
            OK_HI,
        ]);
        let r = plain()
            .get(&request(
                format!("http://127.0.0.1:{port}/here"),
                Some("application/vnd.github.v3+json"),
            ))
            .unwrap();
        assert_eq!((r.status, r.body.as_slice()), (200, &b"hi"[..]));
        let seen = seen.lock().unwrap();
        assert_eq!(seen.len(), 2, "{seen:?}");
        assert!(seen[1].starts_with("GET /there "), "{seen:?}");
        for head in seen.iter() {
            let head = head.to_ascii_lowercase();
            let agent = dspi_session::autoeq::FETCH_USER_AGENT.to_ascii_lowercase();
            assert!(head.contains(&format!("user-agent: {agent}\r\n")), "{head}");
            assert!(
                head.contains("accept: application/vnd.github.v3+json\r\n"),
                "{head}"
            );
        }
    }

    #[test]
    fn tm12_no_answer_is_an_error_in_words() {
        // A port that was just free, and now nobody listens on.
        let port = {
            let l = TcpListener::bind("127.0.0.1:0").unwrap();
            l.local_addr().unwrap().port()
        };
        let e = plain()
            .get(&request(format!("http://127.0.0.1:{port}/"), None))
            .unwrap_err();
        assert!(e.starts_with("could not reach the server"), "{e}");
        for bad in ["", "not a url", "http://", "ftp://127.0.0.1/"] {
            assert!(
                plain().get(&request(bad.to_string(), None)).is_err(),
                "{bad}"
            );
        }
    }

    #[test]
    fn tm12_a_server_that_never_answers_times_out() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let held = std::thread::spawn(move || listener.accept().map(|(s, _)| s));
        let fetch = fetch_with(
            false,
            TlsConfig::default(),
            Duration::from_millis(200),
            1024,
        );
        let e = fetch
            .get(&request(format!("http://127.0.0.1:{port}/"), None))
            .unwrap_err();
        assert!(e.starts_with("could not reach the server"), "{e}");
        drop(held.join());
    }

    #[test]
    fn tm12_a_body_over_the_limit_is_refused() {
        let (port, _) = serve(vec![OK_HI]);
        let fetch = fetch_with(false, TlsConfig::default(), Duration::from_secs(10), 1);
        let e = fetch
            .get(&request(format!("http://127.0.0.1:{port}/"), None))
            .unwrap_err();
        assert!(e.starts_with("could not reach the server"), "{e}");
    }

    /// The real fetch, as the program builds it: nothing is sent in the clear.
    #[test]
    fn tm12_the_real_fetch_refuses_plain_http() {
        let (port, seen) = serve(vec![OK_HI]);
        let e = HttpFetch::new()
            .get(&request(format!("http://127.0.0.1:{port}/"), None))
            .unwrap_err();
        assert!(e.contains("https"), "{e}");
        assert!(
            seen.lock().unwrap().is_empty(),
            "nothing reached the server"
        );
    }

    #[test]
    fn tm12_tls_completes_against_a_trusted_certificate() {
        let port = serve_tls(vec![OK_HI]);
        let fetch = fetch_with(true, trusting_the_test_ca(), Duration::from_secs(10), 1024);
        let r = fetch
            .get(&request(format!("https://localhost:{port}/"), None))
            .unwrap();
        assert_eq!((r.status, r.body.as_slice()), (200, &b"hi"[..]));
    }

    /// The built-in roots are the ones the program uses, and they do not know
    /// the test's CA: a certificate from anyone else is refused.
    #[test]
    fn tm12_tls_refuses_a_certificate_from_an_unknown_authority() {
        let port = serve_tls(vec![OK_HI]);
        let fetch = fetch_with(true, TlsConfig::default(), Duration::from_secs(10), 1024);
        let e = fetch
            .get(&request(format!("https://localhost:{port}/"), None))
            .unwrap_err();
        assert!(e.contains("UnknownIssuer"), "{e}");
    }

    #[test]
    fn tm12_tls_refuses_a_certificate_for_another_name() {
        let port = serve_tls(vec![OK_HI]);
        let fetch = fetch_with(true, trusting_the_test_ca(), Duration::from_secs(10), 1024);
        let e = fetch
            .get(&request(format!("https://127.0.0.1:{port}/"), None))
            .unwrap_err();
        assert!(e.contains("not valid for name"), "{e}");
    }

    #[test]
    fn tm12_a_redirect_from_https_to_http_is_refused() {
        let (plain_port, seen) = serve(vec![OK_HI]);
        let answer: &'static str = Box::leak(
            format!(
                "HTTP/1.1 302 Found\r\nLocation: http://127.0.0.1:{plain_port}/\r\n\
                 Content-Length: 0\r\nConnection: close\r\n\r\n"
            )
            .into_boxed_str(),
        );
        let port = serve_tls(vec![answer]);
        let fetch = fetch_with(true, trusting_the_test_ca(), Duration::from_secs(10), 1024);
        let e = fetch
            .get(&request(format!("https://localhost:{port}/"), None))
            .unwrap_err();
        assert!(e.contains("https"), "{e}");
        assert!(seen.lock().unwrap().is_empty(), "nothing went in the clear");
    }

    /// This WP's point: the rebuild runs in-process, and neither the fetch nor
    /// the session's AutoEQ code starts a program.
    #[test]
    fn tm12_no_external_process() {
        const THIS: &str = include_str!("autoeq_fetch.rs");
        const SESSION: &str = include_str!("../../dspi-session/src/autoeq.rs");
        let needle = ["process::", "Command"].concat();
        assert!(!THIS.contains(&needle));
        assert!(!SESSION.contains(&needle));
        assert!(!SESSION.contains("CurlFetch"));
    }

    #[test]
    fn tm12_concurrent_requests_share_one_fetch() {
        let (port, _) = serve(vec![OK_HI; 8]);
        let fetch = Arc::new(plain());
        let threads: Vec<_> = (0..8)
            .map(|_| {
                let fetch = fetch.clone();
                std::thread::spawn(move || {
                    fetch.get(&request(format!("http://127.0.0.1:{port}/"), None))
                })
            })
            .collect();
        for t in threads {
            assert_eq!(t.join().unwrap().unwrap().body, b"hi");
        }
    }
}
