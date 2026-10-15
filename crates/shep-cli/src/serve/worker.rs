//! `shep serve`'s worker: binds a listener, accepts connections, and
//! answers each one against [`ServeConfig`].
//!
//! `#[cfg(unix)]`: this binds a real listener and reads real files, same
//! as `serve::fs`. `path`, `mime`, `listing` and `auth` stay pure; this is
//! the module that calls them.
//!
//! The accept loop admits through a [`tokio::sync::Semaphore`] and wraps
//! each connection in a [`tokio::time::timeout`]: `serve` streams files
//! off disk, so a client that stops reading mid-transfer must not hold a
//! task, an open file and a socket for the rest of the process's life.

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::Semaphore;

use super::auth::Credentials;
use super::{fs, listing, mime, path};
use crate::exit::ExitCode;
use crate::http::{self, Header, HttpError, HttpRequest};

/// How long [`http::read_request`] waits for a connected peer to finish
/// sending its request. Small enough that a peer that connects and says
/// nothing does not hold a task, and here a semaphore permit, open
/// indefinitely.
const READ_TIMEOUT: Duration = Duration::from_secs(5);

/// How many connections may be in flight at once.
///
/// A permit is held for the whole life of a connection task, so this
/// bounds tasks, open files and sockets together. 512 sits comfortably
/// below a default `RLIMIT_NOFILE`.
const MAX_CONNECTIONS: usize = 512;

/// The whole-connection deadline every real `shep serve` runs with: read,
/// resolve, respond and copy together.
///
/// `READ_TIMEOUT` bounds only the read phase. A test overrides it through
/// [`ServeConfig::connection_deadline`].
pub const CONNECTION_DEADLINE: Duration = Duration::from_secs(60);

/// What a running `serve` worker was told to do.
///
/// No `Debug`: [`Credentials`] carries none, and an
/// `Option<Credentials>` inherits that.
pub struct ServeConfig {
    /// The docroot, already canonical and already known to be a directory.
    pub root: PathBuf,
    /// Where to listen. Loopback unless the operator said otherwise.
    pub bind: SocketAddr,
    /// Serve `<root>/index.html` for a would-be 404 that accepts HTML.
    pub spa: bool,
    /// Render a listing for a directory with no index.
    pub listing: bool,
    /// Serve paths with a leading-dot segment, and list them. Off by
    /// default: `shep serve .` in a repo checkout would otherwise publish
    /// `.env` and the whole `.git` object store.
    pub hidden: bool,
    /// Credentials every request must satisfy, if any.
    pub auth: Option<Credentials>,
    /// Permit a symlink anywhere in the resolved path, falling back to a
    /// canonicalize-then-`starts_with` containment check. Off by default;
    /// passed straight through to `fs::contain`, whose own doc names the
    /// residual race each mode still leaves open.
    pub follow_symlinks: bool,
    /// How long one connection may live, start to finish. A test picks
    /// its own; [`CONNECTION_DEADLINE`] everywhere else.
    pub connection_deadline: Duration,
}

/// Runs `shep serve`'s worker until it is signalled: binds `cfg.bind` and
/// serves until `SIGINT` or `SIGTERM`, the latter being what the
/// shepherd's own kill ladder sends first.
///
/// A refused bind is fatal: a worker bound to nothing looks fine from
/// outside. Tests drive [`accept_forever`] instead, since exercising
/// `SIGTERM` needs a real stop ladder.
pub async fn run(cfg: ServeConfig) -> ExitCode {
    let listener = match TcpListener::bind(cfg.bind).await {
        Ok(listener) => listener,
        Err(err) => {
            eprintln!("shep serve: could not bind {}: {err}", cfg.bind);
            return ExitCode::Failure;
        }
    };
    let mut sigterm = match crate::shutdown::Terminate::install() {
        Ok(sigterm) => sigterm,
        Err(err) => {
            eprintln!("shep serve: could not install a SIGTERM handler: {err}");
            return ExitCode::Failure;
        }
    };
    let cfg = Arc::new(cfg);
    let semaphore = Arc::new(Semaphore::new(MAX_CONNECTIONS));
    tokio::select! {
        _ = tokio::signal::ctrl_c() => {}
        _ = sigterm.recv() => {}
        () = accept_forever(listener, cfg, semaphore) => {}
    }
    ExitCode::Success
}

/// Accepts connections off `listener` forever, one task per connection.
/// Never returns: [`run`] races it against the shutdown signals, and a
/// test aborts the [`tokio::task::JoinHandle`] it returns instead.
///
/// A permit is acquired before the task is spawned, non-blockingly: a
/// connection past the cap is closed rather than queued, since queuing
/// would hold the socket with no task.
async fn accept_forever(listener: TcpListener, cfg: Arc<ServeConfig>, semaphore: Arc<Semaphore>) {
    loop {
        match listener.accept().await {
            Ok((stream, _peer)) => {
                let Ok(permit) = Arc::clone(&semaphore).try_acquire_owned() else {
                    drop(stream);
                    continue;
                };
                let cfg = Arc::clone(&cfg);
                tokio::spawn(async move {
                    let _permit = permit;
                    let deadline = cfg.connection_deadline;
                    let _ = tokio::time::timeout(deadline, handle_connection(stream, cfg)).await;
                });
            }
            Err(err) => {
                eprintln!("shep serve: accept failed: {err}");
            }
        }
    }
}

/// Answers exactly one request on `stream`: read, auth, method, body,
/// resolve, hidden, contain, then a directory or a file, in that order.
///
/// The reply is [`answer`]'s; the single [`log_access`] call is here, so
/// no exit path can answer a request and leave it out of the log.
async fn handle_connection(mut stream: TcpStream, cfg: Arc<ServeConfig>) {
    let peer = stream.peer_addr().ok();

    // A failed read leaves no well-formed request to answer or log.
    let request = match http::read_request(&mut stream, READ_TIMEOUT).await {
        Ok(request) => request,
        Err(_err) => return,
    };

    let mut logged_path = path_of(&request.target).to_string();
    let (status, bytes) = answer(&mut stream, &cfg, &request, &mut logged_path).await;
    log_access(peer, &request.method, &logged_path, status, bytes);
}

/// A request target's path: everything before the first `?` or `#`, or the
/// whole target when it carries neither.
fn path_of(target: &str) -> &str {
    target
        .split_once(['?', '#'])
        .map_or(target, |(path, _rest)| path)
}

/// The `WWW-Authenticate` header every 401 this worker answers carries.
const WWW_AUTHENTICATE: Header<'static> = Header {
    name: "WWW-Authenticate",
    value: "Basic realm=\"shep\"",
};

/// The `Allow` header every 405 this worker answers carries.
const ALLOW: Header<'static> = Header {
    name: "Allow",
    value: "GET, HEAD",
};

/// Writes one reply to `stream`, returning the status answered and the
/// body byte count for [`handle_connection`]'s access-log line.
///
/// `logged_path` arrives holding the raw target and is overwritten with
/// the resolved path as soon as resolution succeeds, so a refusal reached
/// before then is logged under the target the client sent.
async fn answer(
    stream: &mut TcpStream,
    cfg: &ServeConfig,
    request: &HttpRequest,
    logged_path: &mut String,
) -> (u16, u64) {
    let raw_path = path_of(&request.target);

    // Auth before resolution, so an unauthenticated client cannot map the
    // filesystem through 400-vs-404.
    if let Some(creds) = &cfg.auth {
        let header = request.headers.get("authorization").map(String::as_str);
        if !creds.satisfies(header) {
            let body = b"unauthorized\n";
            return reply(stream, 401, "text/plain", body, vec![WWW_AUTHENTICATE]).await;
        }
    }

    if request.method != "GET" && request.method != "HEAD" {
        let body = b"method not allowed; this server answers GET and HEAD\n";
        return reply(stream, 405, "text/plain", body, vec![ALLOW]).await;
    }

    // This server never reads a request body.
    let has_declared_body = request
        .headers
        .get("content-length")
        .and_then(|value| value.parse::<u64>().ok())
        .is_some_and(|len| len > 0)
        || request.headers.contains_key("transfer-encoding");
    if has_declared_body {
        let body = b"this server does not accept a request body\n";
        return reply(stream, 400, "text/plain", body, vec![]).await;
    }

    let segments = match path::resolve(&request.target) {
        Ok(segments) => segments,
        Err(refusal) => {
            let body = refusal.to_string();
            return reply(stream, 400, "text/plain", body.as_bytes(), vec![]).await;
        }
    };
    *logged_path = format!("/{}", segments.join("/"));

    // Hidden: a 404, not a 403, checked before any filesystem access.
    if path::is_hidden(&segments) && !cfg.hidden {
        return send_not_found(stream, cfg, request).await;
    }

    let Some(resolved) = fs::contain(&cfg.root, &segments, cfg.follow_symlinks).await else {
        return send_not_found(stream, cfg, request).await;
    };

    // A directory goes to the directory branch below. A fifo, a socket or
    // a device node reaches `fs::open_regular`, which refuses it.
    let metadata = match tokio::fs::metadata(&resolved).await {
        Ok(metadata) => metadata,
        Err(_err) => return send_not_found(stream, cfg, request).await,
    };

    if metadata.is_dir() {
        if !raw_path.ends_with('/') {
            let mut location = String::from("/");
            location.push_str(
                &segments
                    .iter()
                    .map(|segment| listing::encode_segment(segment))
                    .collect::<Vec<_>>()
                    .join("/"),
            );
            location.push('/');
            let header = Header {
                name: "Location",
                value: location.as_str(),
            };
            return match respond(stream, 301, "text/plain", 0, vec![header]).await {
                Ok(()) => (301, 0),
                Err(_err) => {
                    // A peer disconnect, not a bad header: `encode_segment`
                    // only ever produces printable ASCII.
                    let body = b"could not build the redirect\n";
                    reply(stream, 500, "text/plain", body, vec![]).await
                }
            };
        }

        if let Some((file, len)) = fs::open_regular(&resolved.join("index.html")).await {
            let status =
                serve_file(stream, request, mime::content_type("index.html"), file, len).await;
            return (status, len);
        }

        if cfg.listing {
            let entries = read_listing(&resolved, cfg.hidden)
                .await
                .unwrap_or_default();
            let mut prefix = String::from("/");
            prefix.push_str(&segments.join("/"));
            if !prefix.ends_with('/') {
                prefix.push('/');
            }
            let html = listing::render(&prefix, &entries);
            let body = html.as_bytes();
            return reply(stream, 200, "text/html; charset=utf-8", body, vec![]).await;
        }

        return send_not_found(stream, cfg, request).await;
    }

    let Some((file, len)) = fs::open_regular(&resolved).await else {
        return send_not_found(stream, cfg, request).await;
    };
    let content_type = mime::content_type(logged_path.as_str());
    let status = serve_file(stream, request, content_type, file, len).await;
    (status, len)
}
/// Answers a 404, or, if `cfg.spa` is set, the method is `GET`/`HEAD`, and
/// the request's `Accept` header names `text/html`, serves the docroot's
/// `index.html` with a 200 instead. Every 404 this worker would otherwise
/// answer funnels through here, so the gate lives in one place.
async fn send_not_found(
    stream: &mut TcpStream,
    cfg: &ServeConfig,
    request: &HttpRequest,
) -> (u16, u64) {
    if cfg.spa
        && matches!(request.method.as_str(), "GET" | "HEAD")
        && request
            .headers
            .get("accept")
            .is_some_and(|accept| accept.contains("text/html"))
        && let Some((file, len)) = fs::open_regular(&cfg.root.join("index.html")).await
    {
        let status = serve_file(stream, request, mime::content_type("index.html"), file, len).await;
        if status == 200 {
            return (200, len);
        }
    }
    reply(stream, 404, "text/plain", b"not found\n", vec![]).await
}

/// Streams `file` (already open, `len` bytes long per its own metadata)
/// as the body of a 200. The head goes through [`respond`]; the body
/// does not, since a streamed file has no `&[u8]` to hand it.
///
/// `file.take(len)`, not a bare copy: `len` came from the open handle's
/// own metadata, so a file that grew between the open and this copy still
/// matches the `Content-Length` already declared. A file that shrank still
/// sends fewer bytes than declared; `Connection: close` turns that into a
/// visible truncation rather than a hang.
async fn serve_file(
    stream: &mut TcpStream,
    request: &HttpRequest,
    content_type: &str,
    file: tokio::fs::File,
    len: u64,
) -> u16 {
    match respond(stream, 200, content_type, len, vec![]).await {
        Ok(()) => {
            if request.method == "GET" {
                let mut body = file.take(len);
                let _ = tokio::io::copy(&mut body, stream).await;
            }
            200
        }
        Err(_err) => 500,
    }
}

/// Reads `dir`'s entries, dropping a leading-dot name unless `hidden` is
/// set, directories before files.
async fn read_listing(dir: &std::path::Path, hidden: bool) -> std::io::Result<Vec<listing::Entry>> {
    let mut dirs = Vec::new();
    let mut files = Vec::new();
    let mut read_dir = tokio::fs::read_dir(dir).await?;
    while let Some(entry) = read_dir.next_entry().await? {
        let name = entry.file_name().to_string_lossy().into_owned();
        if !hidden && path::is_hidden(std::slice::from_ref(&name)) {
            continue;
        }
        if entry.file_type().await?.is_dir() {
            dirs.push(name);
        } else {
            files.push(name);
        }
    }
    dirs.sort();
    files.sort();
    let mut entries = Vec::with_capacity(dirs.len() + files.len());
    entries.extend(dirs.into_iter().map(listing::Entry::dir));
    entries.extend(files.into_iter().map(listing::Entry::file));
    Ok(entries)
}

/// Writes exactly one response head. Every reply funnels through here,
/// and this is the one place the nosniff header is appended, since the
/// listing renders attacker-influenced filenames into HTML.
///
/// # Errors
/// [`HttpError`] if the write failed, or if a header value carried a
/// control byte (`write_head`'s response-splitting lock). The caller
/// answers with nothing further, since nothing has reached the stream.
async fn respond(
    stream: &mut TcpStream,
    status: u16,
    content_type: &str,
    content_length: u64,
    extra_headers: Vec<Header<'_>>,
) -> Result<(), HttpError> {
    let mut headers = extra_headers;
    headers.push(Header {
        name: "X-Content-Type-Options",
        value: "nosniff",
    });
    http::write_head(stream, status, content_type, content_length, &headers).await
}

/// [`respond`] plus the in-memory body every reply except a streamed file
/// or an empty redirect carries. Writes nothing further if the head itself
/// was refused.
async fn send(
    stream: &mut TcpStream,
    status: u16,
    content_type: &str,
    body: &[u8],
    headers: Vec<Header<'_>>,
) {
    if respond(stream, status, content_type, body.len() as u64, headers)
        .await
        .is_ok()
    {
        let _ = stream.write_all(body).await;
    }
}

/// [`send`] plus the `(status, byte count)` pair every in-memory reply
/// returns for [`handle_connection`]'s access-log line.
async fn reply(
    stream: &mut TcpStream,
    status: u16,
    content_type: &str,
    body: &[u8],
    headers: Vec<Header<'_>>,
) -> (u16, u64) {
    send(stream, status, content_type, body, headers).await;
    (status, body.len() as u64)
}

/// One access-log line per request, written to stdout: `serve` runs as a
/// registered sheep, so `shep bleats <name>` is this log. `path` is the
/// raw target or the resolved path, escaped here so no byte a client chose
/// reaches the operator's terminal.
fn log_access(peer: Option<SocketAddr>, method: &str, path: &str, status: u16, bytes: u64) {
    let peer = peer.map_or_else(|| "-".to_string(), |addr| addr.to_string());
    let path = escape_for_log(path);
    println!("{peer} \"{method} {path}\" {status} {bytes}");
}

/// Escapes every byte outside printable ASCII (`0x20..=0x7e`) as `\xNN`,
/// so a byte a client chose never reaches an operator's terminal through
/// `shep bleats`.
fn escape_for_log(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for byte in s.bytes() {
        if (0x20..=0x7e).contains(&byte) {
            out.push(char::from(byte));
        } else {
            out.push_str(&format!("\\x{byte:02X}"));
        }
    }
    out
}

// unix only: the cases assert file modes on served files.
// Windows coverage is in `tests/cli_e2e.rs`.
#[cfg(all(test, unix))]
mod tests {
    use std::collections::HashMap;
    use std::path::Path;

    use tokio::task::JoinHandle;

    use super::*;
    use crate::serve::auth;

    /// A running worker bound to an OS-assigned loopback port. Aborts its
    /// accept loop on drop, so it cannot hold a port past its own test.
    struct RunningServer {
        addr: SocketAddr,
        handle: JoinHandle<()>,
    }

    impl RunningServer {
        fn addr(&self) -> SocketAddr {
            self.addr
        }
    }

    impl Drop for RunningServer {
        fn drop(&mut self) {
            self.handle.abort();
        }
    }

    /// A docroot at `root` holding `files`, each `(relative path, contents)`.
    fn write_tree(root: &Path, files: &[(&str, &str)]) {
        std::fs::create_dir_all(root).unwrap();
        for (path, contents) in files {
            let full = root.join(path);
            if let Some(parent) = full.parent() {
                std::fs::create_dir_all(parent).unwrap();
            }
            std::fs::write(full, contents).unwrap();
        }
    }

    /// A default [`ServeConfig`] over `root`, bound to a free loopback port.
    fn config(root: &Path) -> ServeConfig {
        ServeConfig {
            root: std::fs::canonicalize(root).unwrap(),
            bind: SocketAddr::from(([127, 0, 0, 1], 0)),
            spa: false,
            listing: false,
            hidden: false,
            auth: None,
            follow_symlinks: false,
            connection_deadline: CONNECTION_DEADLINE,
        }
    }

    /// The credential pair the auth-enabled tests authenticate with.
    const TEST_USER: &str = "alice:s3cret";

    fn config_with_auth(root: &Path) -> ServeConfig {
        let creds_path = root
            .parent()
            .expect("root must sit inside the caller's own tempdir")
            .join("creds");
        std::fs::write(&creds_path, format!("{TEST_USER}\n")).unwrap();
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&creds_path, std::fs::Permissions::from_mode(0o600)).unwrap();
        }
        let mut built = config(root);
        built.auth = Some(auth::load(&creds_path).unwrap());
        built
    }

    fn config_with_spa(root: &Path) -> ServeConfig {
        let mut built = config(root);
        built.spa = true;
        built
    }

    fn config_with_listing(root: &Path) -> ServeConfig {
        let mut built = config(root);
        built.listing = true;
        built
    }

    fn config_with_hidden(root: &Path) -> ServeConfig {
        let mut built = config(root);
        built.hidden = true;
        built
    }

    fn config_with_follow_symlinks(root: &Path) -> ServeConfig {
        let mut built = config(root);
        built.follow_symlinks = true;
        built
    }

    /// Binds `config.bind`, reads back the OS-assigned address, and serves it in the background.
    async fn serve_on_free_port(config: ServeConfig) -> RunningServer {
        let listener = TcpListener::bind(config.bind).await.unwrap();
        let addr = listener.local_addr().unwrap();
        let semaphore = Arc::new(Semaphore::new(MAX_CONNECTIONS));
        let handle = tokio::spawn(accept_forever(listener, Arc::new(config), semaphore));
        RunningServer { addr, handle }
    }

    /// One HTTP response, parsed just enough for these tests: a status
    /// code, lower-cased header names, and the body as text.
    #[derive(Debug)]
    struct Response {
        status: u16,
        headers: HashMap<String, String>,
        body: String,
    }

    fn parse_response(raw: &[u8]) -> Response {
        let text = String::from_utf8_lossy(raw);
        let mut halves = text.splitn(2, "\r\n\r\n");
        let head = halves.next().unwrap_or_default();
        let body = halves.next().unwrap_or_default().to_string();
        let mut lines = head.split("\r\n");
        let status = lines
            .next()
            .and_then(|line| line.split_whitespace().nth(1))
            .and_then(|code| code.parse().ok())
            .unwrap_or(0);
        let mut headers = HashMap::new();
        for line in lines {
            if let Some((name, value)) = line.split_once(':') {
                headers.insert(name.trim().to_ascii_lowercase(), value.trim().to_string());
            }
        }
        Response {
            status,
            headers,
            body,
        }
    }

    /// Sends one raw request (a full, correctly terminated HTTP/1.1 head)
    /// and returns the parsed response. Every step carries a generous
    /// timeout, so a hang here is a bug in the worker, not the suite.
    async fn send_request(addr: SocketAddr, request: &str) -> Response {
        let mut stream = tokio::time::timeout(Duration::from_secs(5), TcpStream::connect(addr))
            .await
            .expect("connect must not hang")
            .unwrap();
        tokio::time::timeout(Duration::from_secs(5), stream.write_all(request.as_bytes()))
            .await
            .expect("write must not hang")
            .unwrap();
        let mut buf = Vec::new();
        tokio::time::timeout(Duration::from_secs(5), stream.read_to_end(&mut buf))
            .await
            .expect("read must not hang")
            .unwrap();
        parse_response(&buf)
    }

    async fn get(addr: SocketAddr, target: &str) -> Response {
        send_request(
            addr,
            &format!("GET {target} HTTP/1.1\r\nHost: localhost\r\n\r\n"),
        )
        .await
    }

    async fn get_with_accept(addr: SocketAddr, target: &str, accept: &str) -> Response {
        send_request(
            addr,
            &format!("GET {target} HTTP/1.1\r\nHost: localhost\r\nAccept: {accept}\r\n\r\n"),
        )
        .await
    }

    async fn get_auth(addr: SocketAddr, target: &str) -> Response {
        send_request(
            addr,
            &format!(
                "GET {target} HTTP/1.1\r\nHost: localhost\r\nAuthorization: Basic {}\r\n\r\n",
                base64(TEST_USER)
            ),
        )
        .await
    }

    async fn request_with_method(addr: SocketAddr, method: &str, target: &str) -> Response {
        send_request(
            addr,
            &format!("{method} {target} HTTP/1.1\r\nHost: localhost\r\n\r\n"),
        )
        .await
    }

    /// [`auth::base64_encode`] over a `&str`, for the header built by hand.
    fn base64(input: &str) -> String {
        auth::base64_encode(input.as_bytes())
    }

    /// The traversal table again, end to end over a real socket, so it
    /// covers the handler's ordering and not only the resolver. Each case
    /// asserts the exact status, since `assert_ne!(status, 200)` passes
    /// against a server that 500s on everything.
    #[tokio::test]
    async fn every_traversal_shape_is_refused_over_a_real_socket() {
        // The docroot is a child of the tempdir, and the negative control
        // is its sibling: `tempdir().path()` alone is shared and never
        // cleaned up.
        let outer = tempfile::tempdir().unwrap();
        let root = outer.path().join("www");
        write_tree(
            &root,
            &[
                ("index.html", "<h1>home</h1>"),
                ("assets/app.css", "body{}"),
            ],
        );
        std::fs::write(outer.path().join("secret.txt"), "nope").unwrap();
        let server = serve_on_free_port(config(&root)).await;

        // positive control first: the server serves.
        assert_eq!(get(server.addr(), "/index.html").await.status, 200);
        assert_eq!(get(server.addr(), "/assets/app.css").await.status, 200);

        for (target, want) in [
            ("/../secret.txt", 400),
            ("/../../etc/passwd", 400),
            ("/%2e%2e/secret.txt", 400),
            ("/..%2fsecret.txt", 400),
            ("/x%00.png", 400),
            ("/a%0d%0aSet-Cookie:%20x", 400),
            ("/C:/Windows/System32/config/SAM", 400),
            ("/etc/passwd", 404),
            ("/nope.txt", 404),
            ("/.env", 404),
        ] {
            let response = get(server.addr(), target).await;
            assert_eq!(response.status, want, "{target} answered {response:?}");
            assert!(!response.body.contains("nope"), "{target} leaked the file");
            assert_eq!(
                response.headers["x-content-type-options"], "nosniff",
                "{target}: every response carries it, refusals included"
            );
        }
    }

    /// fails if a decoded C1 control byte reaches the log raw: `path::resolve`
    /// refuses control bytes below `0x20` and `0x7f`, so `%C2%9B` (CSI)
    /// resolves, and the access log is where it must be escaped.
    #[test]
    fn a_decoded_c1_control_byte_is_escaped_for_the_log() {
        assert_eq!(escape_for_log("/\u{9b}"), "/\\xC2\\x9B");
    }

    /// fails if a symlink out of the docroot is served over the socket.
    #[tokio::test]
    async fn a_symlink_out_of_the_docroot_is_a_404_and_not_a_body() {
        let outer = tempfile::tempdir().unwrap();
        let root = outer.path().join("www");
        write_tree(&root, &[("ok.txt", "served")]);
        std::fs::write(outer.path().join("secret.txt"), "nope").unwrap();
        std::os::unix::fs::symlink(outer.path().join("secret.txt"), root.join("escape.txt"))
            .unwrap();
        let server = serve_on_free_port(config(&root)).await;

        assert_eq!(
            get(server.addr(), "/ok.txt").await.status,
            200,
            "positive control"
        );

        let response = get(server.addr(), "/escape.txt").await;
        assert_eq!(response.status, 404);
        assert!(!response.body.contains("nope"), "{response:?}");
    }

    /// fails if the follow-symlinks flag on `ServeConfig` does not reach
    /// `fs::contain` through the worker.
    #[tokio::test]
    async fn a_symlinked_deploy_layout_is_served_only_with_follow_symlinks() {
        let outer = tempfile::tempdir().unwrap();
        let root = outer.path().join("www");
        write_tree(
            &root,
            &[("releases/2026-08-15/index.html", "<h1>home</h1>")],
        );
        std::os::unix::fs::symlink(root.join("releases/2026-08-15"), root.join("current")).unwrap();

        let refusing = serve_on_free_port(config(&root)).await;
        assert_eq!(
            get(refusing.addr(), "/current/index.html").await.status,
            404
        );

        let following = serve_on_free_port(config_with_follow_symlinks(&root)).await;
        let response = get(following.addr(), "/current/index.html").await;
        assert_eq!(response.status, 200);
        assert!(response.body.contains("<h1>home</h1>"));
    }

    /// fails if `--follow-symlinks` is mistaken for "trust every path"
    /// instead of "permit a symlink component, still enforce
    /// containment".
    #[tokio::test]
    async fn follow_symlinks_does_not_let_a_symlink_escape_the_root() {
        let outer = tempfile::tempdir().unwrap();
        let root = outer.path().join("www");
        write_tree(&root, &[("index.html", "<h1>home</h1>")]);
        std::fs::write(outer.path().join("secret.txt"), "nope").unwrap();
        std::os::unix::fs::symlink(outer.path().join("secret.txt"), root.join("escape.txt"))
            .unwrap();

        let server = serve_on_free_port(config_with_follow_symlinks(&root)).await;
        let response = get(server.addr(), "/escape.txt").await;
        assert_eq!(response.status, 404);
        assert!(
            !response.body.contains("nope"),
            "the escape must not be served"
        );
    }

    /// fails if auth is checked after path resolution. An unauthenticated
    /// client must not be able to tell a refused traversal (400) from a
    /// missing file (404) apart.
    #[tokio::test]
    async fn an_unauthenticated_request_is_401_whatever_the_path_says() {
        let outer = tempfile::tempdir().unwrap();
        let root = outer.path().join("www");
        write_tree(&root, &[("index.html", "<h1>home</h1>")]);
        std::fs::write(outer.path().join("secret.txt"), "nope").unwrap();
        let server = serve_on_free_port(config_with_auth(&root)).await;

        for target in ["/index.html", "/../secret.txt", "/nope.txt"] {
            let response = get(server.addr(), target).await;
            assert_eq!(response.status, 401, "{target}");
            assert!(
                response.headers.contains_key("www-authenticate"),
                "{target}"
            );
        }
        // positive control: with the credential, the same three answer
        // 200/400/404, not 401.
        assert_eq!(get_auth(server.addr(), "/index.html").await.status, 200);
        assert_eq!(get_auth(server.addr(), "/../secret.txt").await.status, 400);
        assert_eq!(get_auth(server.addr(), "/nope.txt").await.status, 404);
    }

    /// fails if a POST is served, or if the 405 forgets to say what is
    /// allowed.
    #[tokio::test]
    async fn a_method_other_than_get_or_head_is_405_with_an_allow_header() {
        let outer = tempfile::tempdir().unwrap();
        let root = outer.path().join("www");
        write_tree(&root, &[("index.html", "<h1>home</h1>")]);
        let server = serve_on_free_port(config(&root)).await;

        let response = request_with_method(server.addr(), "POST", "/index.html").await;
        assert_eq!(response.status, 405);
        assert_eq!(response.headers["allow"], "GET, HEAD");
        assert!(!response.body.contains("<h1>home</h1>"), "{response:?}");
    }

    /// fails if a refusal reply loses its status, its own header, or the
    /// byte count it declared.
    #[tokio::test]
    async fn a_refusal_reply_carries_its_status_headers_and_body_length() {
        let outer = tempfile::tempdir().unwrap();
        let root = outer.path().join("www");
        write_tree(&root, &[("index.html", "<h1>home</h1>")]);
        let guarded = serve_on_free_port(config_with_auth(&root)).await;
        let open = serve_on_free_port(config(&root)).await;

        let unauthorized = get(guarded.addr(), "/index.html").await;
        assert_eq!(unauthorized.status, 401);
        assert_eq!(unauthorized.body, "unauthorized\n");
        assert_eq!(
            unauthorized.headers["www-authenticate"],
            "Basic realm=\"shep\""
        );
        assert_eq!(unauthorized.headers["content-length"], "13");

        let refused = "method not allowed; this server answers GET and HEAD\n";
        let not_allowed = request_with_method(open.addr(), "POST", "/index.html").await;
        assert_eq!(not_allowed.status, 405);
        assert_eq!(not_allowed.body, refused);
        assert_eq!(not_allowed.headers["allow"], "GET, HEAD");
        assert_eq!(not_allowed.headers["content-length"], "53");
    }

    /// fails if a HEAD grows a body, or loses its Content-Length.
    #[tokio::test]
    async fn a_head_carries_the_length_and_no_body() {
        let outer = tempfile::tempdir().unwrap();
        let root = outer.path().join("www");
        write_tree(&root, &[("index.html", "<h1>home</h1>")]);
        let server = serve_on_free_port(config(&root)).await;

        let response = request_with_method(server.addr(), "HEAD", "/index.html").await;
        assert_eq!(response.status, 200);
        assert_eq!(
            response.headers["content-length"],
            "<h1>home</h1>".len().to_string()
        );
        assert!(response.body.is_empty(), "{response:?}");
    }

    /// fails if a directory without a trailing slash is served in place,
    /// which breaks every relative link in the page it serves.
    #[tokio::test]
    async fn a_directory_without_a_trailing_slash_redirects_to_one() {
        let outer = tempfile::tempdir().unwrap();
        let root = outer.path().join("www");
        write_tree(&root, &[("docs/index.html", "<h1>docs</h1>")]);
        let server = serve_on_free_port(config(&root)).await;

        let response = get(server.addr(), "/docs").await;
        assert_eq!(response.status, 301);
        assert_eq!(response.headers["location"], "/docs/");
    }

    /// fails if listing is on by default.
    #[tokio::test]
    async fn a_directory_with_no_index_is_404_unless_listing_is_on() {
        let outer = tempfile::tempdir().unwrap();
        let root = outer.path().join("www");
        write_tree(&root, &[("empty/.keep", "")]);

        let plain = serve_on_free_port(config(&root)).await;
        assert_eq!(get(plain.addr(), "/empty/").await.status, 404);

        let listing_on = serve_on_free_port(config_with_listing(&root)).await;
        let response = get(listing_on.addr(), "/empty/").await;
        assert_eq!(response.status, 200);
        assert!(response.body.contains("Index of"), "{response:?}");
    }

    /// fails if the SPA fallback fires for an asset request: a missing
    /// `/assets/app.js` must 404, not answer HTML with a 200.
    #[tokio::test]
    async fn the_spa_fallback_serves_index_for_navigations_and_404s_for_assets() {
        let outer = tempfile::tempdir().unwrap();
        let root = outer.path().join("www");
        write_tree(&root, &[("index.html", "<h1>home</h1>")]);
        let server = serve_on_free_port(config_with_spa(&root)).await;

        let nav = get_with_accept(server.addr(), "/deep/link", "text/html").await;
        assert_eq!(nav.status, 200);
        assert!(nav.body.contains("<h1>home</h1>"));

        let asset = get_with_accept(server.addr(), "/assets/missing.js", "*/*").await;
        assert_eq!(asset.status, 404);
    }

    /// fails if `nosniff` is dropped, or if the MIME table is not reaching
    /// the response.
    #[tokio::test]
    async fn a_css_file_is_served_as_css_with_nosniff() {
        let outer = tempfile::tempdir().unwrap();
        let root = outer.path().join("www");
        write_tree(&root, &[("app.css", "body{}")]);
        let server = serve_on_free_port(config(&root)).await;

        let response = get(server.addr(), "/app.css").await;
        assert_eq!(response.status, 200);
        assert_eq!(response.headers["content-type"], "text/css; charset=utf-8");
        assert_eq!(response.headers["x-content-type-options"], "nosniff");
        assert_eq!(response.body, "body{}");
    }

    /// fails if a body larger than any buffer is truncated or read whole
    /// into memory. 2 MiB is enough to cross every buffer in this path and
    /// small enough to write in a test.
    #[tokio::test]
    async fn a_file_larger_than_the_buffers_is_served_whole() {
        let outer = tempfile::tempdir().unwrap();
        let root = outer.path().join("www");
        std::fs::create_dir_all(&root).unwrap();
        let big = "a".repeat(2 * 1024 * 1024);
        std::fs::write(root.join("big.txt"), &big).unwrap();
        let server = serve_on_free_port(config(&root)).await;

        let response = get(server.addr(), "/big.txt").await;
        assert_eq!(response.status, 200);
        assert_eq!(response.headers["content-length"], big.len().to_string());
        assert_eq!(response.body.len(), big.len());
    }

    /// fails if a dotfile is served, or if `--hidden` stops serving one.
    #[tokio::test]
    async fn a_dotfile_is_404_unless_hidden_is_set() {
        let outer = tempfile::tempdir().unwrap();
        let root = outer.path().join("www");
        write_tree(
            &root,
            &[
                (".env", "SECRET=1"),
                (".git/config", "[core]"),
                ("index.html", "<h1>home</h1>"),
                (".well-known/acme-challenge/x", "token"),
            ],
        );

        let plain = serve_on_free_port(config(&root)).await;
        assert_eq!(get(plain.addr(), "/.env").await.status, 404);
        assert_eq!(get(plain.addr(), "/.git/config").await.status, 404);
        assert_eq!(
            get(plain.addr(), "/index.html").await.status,
            200,
            "positive control"
        );

        let hidden = serve_on_free_port(config_with_hidden(&root)).await;
        assert_eq!(
            get(hidden.addr(), "/.well-known/acme-challenge/x")
                .await
                .status,
            200
        );
    }

    /// fails if a listing names a file the server will not serve, or
    /// drops `nosniff` on its own response.
    #[tokio::test]
    async fn a_listing_omits_hidden_entries_and_carries_nosniff() {
        let outer = tempfile::tempdir().unwrap();
        let root = outer.path().join("www");
        write_tree(&root, &[(".env", "SECRET=1"), ("app.js", "console.log(1)")]);
        let server = serve_on_free_port(config_with_listing(&root)).await;

        let response = get(server.addr(), "/").await;
        assert_eq!(response.status, 200);
        assert!(!response.body.contains(".env"), "{response:?}");
        assert!(response.body.contains("app.js"), "{response:?}");
        assert_eq!(response.headers["x-content-type-options"], "nosniff");
    }

    /// fails if a listing goes out as plain text, or declares a length
    /// that disagrees with the HTML it sent.
    #[tokio::test]
    async fn a_directory_listing_reply_is_html_and_counts_its_bytes() {
        let outer = tempfile::tempdir().unwrap();
        let root = outer.path().join("www");
        write_tree(&root, &[("a.txt", "a"), ("b.txt", "bb")]);
        let server = serve_on_free_port(config_with_listing(&root)).await;

        let response = get(server.addr(), "/").await;
        assert_eq!(response.status, 200);
        assert_eq!(response.headers["content-type"], "text/html; charset=utf-8");
        assert!(
            response.body.contains("<h1>Index of /</h1>"),
            "{response:?}"
        );
        assert!(response.body.contains("a.txt"), "{response:?}");
        let declared = response.body.len().to_string();
        assert_eq!(response.headers["content-length"], declared);
    }

    /// fails if a directory whose name is not ASCII answers 500 instead
    /// of a percent-encoded redirect.
    #[tokio::test]
    async fn a_directory_with_a_non_ascii_name_redirects_rather_than_500ing() {
        let outer = tempfile::tempdir().unwrap();
        let root = outer.path().join("www");
        write_tree(&root, &[("документы/index.html", "<h1>docs</h1>")]);
        let server = serve_on_free_port(config(&root)).await;

        let response = get(server.addr(), "/документы").await;
        assert_eq!(response.status, 301);
        assert_eq!(
            response.headers["location"],
            "/%D0%B4%D0%BE%D0%BA%D1%83%D0%BC%D0%B5%D0%BD%D1%82%D1%8B/"
        );
    }

    /// fails if there is no ceiling on concurrent connections. Opens
    /// `MAX_CONNECTIONS` sockets and holds them open, then asserts a
    /// connection past the cap is closed within a fraction of a second
    /// rather than held.
    #[tokio::test]
    async fn connections_beyond_the_cap_are_closed_rather_than_queued_forever() {
        let outer = tempfile::tempdir().unwrap();
        let root = outer.path().join("www");
        write_tree(&root, &[("index.html", "<h1>home</h1>")]);
        let server = serve_on_free_port(config(&root)).await;

        let mut held = Vec::with_capacity(MAX_CONNECTIONS);
        for _ in 0..MAX_CONNECTIONS {
            let stream =
                tokio::time::timeout(Duration::from_secs(5), TcpStream::connect(server.addr()))
                    .await
                    .expect("connect must not hang")
                    .unwrap();
            held.push(stream);
        }
        // Give every handler task a moment to actually reach its own
        // `read_request` call (and so acquire its permit) before the
        // one-over-the-cap connection below is attempted.
        tokio::time::sleep(Duration::from_millis(200)).await;

        let mut over_cap =
            tokio::time::timeout(Duration::from_secs(5), TcpStream::connect(server.addr()))
                .await
                .expect("connect must not hang")
                .unwrap();
        let mut buf = Vec::new();
        tokio::time::timeout(Duration::from_millis(500), over_cap.read_to_end(&mut buf))
            .await
            .expect("a connection beyond the cap must close promptly rather than hang")
            .unwrap();
        assert!(
            buf.is_empty(),
            "nothing should be written to a refused connection: {buf:?}"
        );

        drop(held);
    }

    /// fails if a connection that stops reading mid-body is held for the
    /// life of the process.
    ///
    /// Runs on a real clock with a tiny deadline. A paused clock races
    /// tokio's auto-advance against real socket IO, and the wrong side
    /// can win.
    #[tokio::test]
    async fn a_connection_that_stops_reading_is_dropped_at_the_deadline() {
        let outer = tempfile::tempdir().unwrap();
        let root = outer.path().join("www");
        // Larger than an ordinary kernel send buffer, so the server's write
        // genuinely blocks on backpressure once the client stops reading,
        // rather than completing into the buffer regardless.
        let big = "a".repeat(8 * 1024 * 1024);
        write_tree(&root, &[("big.txt", &big)]);
        let mut cfg = config(&root);
        // Short enough to wait out in a test, long enough that a loaded
        // machine still gets the connection established and the write
        // started before it fires.
        cfg.connection_deadline = Duration::from_millis(300);
        let server = serve_on_free_port(cfg).await;

        let mut stream = TcpStream::connect(server.addr()).await.unwrap();
        stream
            .write_all(b"GET /big.txt HTTP/1.1\r\nHost: localhost\r\n\r\n")
            .await
            .unwrap();

        // Never read anything: connect and send only. The deadline must
        // close the connection, which the client sees as EOF. The guard is
        // generous because it exists to turn a hang into a failure, not to
        // measure the deadline.
        let mut rest = Vec::new();
        tokio::time::timeout(Duration::from_secs(30), stream.read_to_end(&mut rest))
            .await
            .expect("the deadline must close the connection rather than hang forever")
            .unwrap();
    }
}
