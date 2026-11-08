//! A GET to Docker's engine over its Unix socket, and what it answered: the least of HTTP/1.1 the
//! engine's API needs, so that reading Docker takes neither its CLI nor an HTTP client. Every
//! request asks for the connection to close after the answer, so the answer ends where the stream
//! does, and its body comes whole, or in the chunks the engine streams a long answer in.

use std::io::{self, Read, Write};
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::time::Duration;

/// How long the engine may take to answer. Its disk use took 0.42 s on Daniel's Mac (2026-11-07,
/// 68 volumes), and an engine with many more takes longer.
const ANSWER_TIMEOUT: Duration = Duration::from_secs(30);

/// The most of an answer read: a list of thousands of containers is a few megabytes, so this
/// bounds what an engine gone wrong can make Sutra hold.
const LARGEST_ANSWER: u64 = 64 << 20;

/// Why a GET found no answer to use.
#[derive(Debug)]
pub(super) enum GetError {
    /// Nothing could be connected to at the socket: it is not there, or nothing serves it.
    Connect(io::Error),
    /// The connection failed or timed out on the way.
    Io(io::Error),
    /// The engine answered with a status that is no success. Which one goes unsaid, as the map
    /// says of a source only what kind of failure stopped it (see `kind`).
    Status,
    /// What came back is no HTTP answer that can be read, or is larger than `LARGEST_ANSWER`.
    Malformed,
}

impl GetError {
    /// The kind of I/O error the failure is, as the map reports a source it could not read. An
    /// engine that answered with an error status reads as `Other`, the kind `io` has for none of
    /// its own, and an answer that cannot be read as `InvalidData`.
    pub(super) fn kind(&self) -> io::ErrorKind {
        match self {
            GetError::Connect(err) | GetError::Io(err) => err.kind(),
            GetError::Status => io::ErrorKind::Other,
            GetError::Malformed => io::ErrorKind::InvalidData,
        }
    }
}

/// The body of the engine's answer to a GET of `path`, connected to through `socket`.
pub(super) fn get(socket: &Path, path: &str) -> Result<Vec<u8>, GetError> {
    let mut stream = UnixStream::connect(socket).map_err(GetError::Connect)?;
    stream
        .set_read_timeout(Some(ANSWER_TIMEOUT))
        .and_then(|()| stream.set_write_timeout(Some(ANSWER_TIMEOUT)))
        .map_err(GetError::Io)?;
    let request = format!("GET {path} HTTP/1.1\r\nHost: docker\r\nConnection: close\r\n\r\n");
    stream.write_all(request.as_bytes()).map_err(GetError::Io)?;
    let mut answer = Vec::new();
    let read = stream.take(LARGEST_ANSWER + 1).read_to_end(&mut answer);
    read.map_err(GetError::Io)?;
    if answer.len() as u64 > LARGEST_ANSWER {
        return Err(GetError::Malformed);
    }
    body_of(&answer)
}

/// The body of `answer`, a whole HTTP/1.1 answer, when its status is a success.
fn body_of(answer: &[u8]) -> Result<Vec<u8>, GetError> {
    let end_of_head = find(answer, b"\r\n\r\n").ok_or(GetError::Malformed)?;
    let head = std::str::from_utf8(&answer[..end_of_head]).map_err(|_| GetError::Malformed)?;
    let body = &answer[end_of_head + 4..];
    let mut lines = head.split("\r\n");
    let status = lines
        .next()
        .and_then(status_of)
        .ok_or(GetError::Malformed)?;
    if !(200..300).contains(&status) {
        return Err(GetError::Status);
    }
    let header = |name: &str| {
        let mut headers = lines.clone().filter_map(|line| line.split_once(':'));
        let found = headers.find(|(key, _)| key.trim().eq_ignore_ascii_case(name));
        found.map(|(_, value)| value.trim())
    };
    // Chunked is the last coding of a body sent in chunks.
    let chunked = header("Transfer-Encoding").is_some_and(|value| {
        let last = value.rsplit(',').next().unwrap_or_default();
        last.trim().eq_ignore_ascii_case("chunked")
    });
    if chunked {
        return unchunked(body);
    }
    match header("Content-Length") {
        Some(length) => {
            let length: usize = length.parse().map_err(|_| GetError::Malformed)?;
            body.get(..length)
                .map(<[u8]>::to_vec)
                .ok_or(GetError::Malformed)
        }
        // With neither, the body runs to the end of the stream, which the closed connection marks.
        None => Ok(body.to_vec()),
    }
}

/// The status code of a status line, as `HTTP/1.1 200 OK` gives 200.
fn status_of(line: &str) -> Option<u16> {
    let mut parts = line.split(' ');
    let version = parts.next()?;
    let code = parts.next()?;
    (version.starts_with("HTTP/1.") && code.len() == 3)
        .then(|| code.parse().ok())
        .flatten()
}

/// A body sent in chunks, each its size in hex on a line of its own, then its bytes, until one of
/// size 0, put back together.
fn unchunked(mut chunks: &[u8]) -> Result<Vec<u8>, GetError> {
    let mut body = Vec::new();
    loop {
        let end_of_size = find(chunks, b"\r\n").ok_or(GetError::Malformed)?;
        let line = std::str::from_utf8(&chunks[..end_of_size]).map_err(|_| GetError::Malformed)?;
        // A chunk's size may carry extensions after a `;`, which say nothing Sutra needs.
        let size = line.split(';').next().unwrap_or_default().trim();
        let size = usize::from_str_radix(size, 16).map_err(|_| GetError::Malformed)?;
        let rest = &chunks[end_of_size + 2..];
        if size == 0 {
            return Ok(body);
        }
        let chunk = rest.get(..size).ok_or(GetError::Malformed)?;
        body.extend_from_slice(chunk);
        chunks = rest
            .get(size..)
            .and_then(|after| after.strip_prefix(b"\r\n"))
            .ok_or(GetError::Malformed)?;
    }
}

/// Where `needle` first stands in `haystack`.
fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

#[cfg(test)]
pub(super) mod tests {
    use std::io::BufRead;
    use std::os::unix::net::UnixListener;
    use std::path::PathBuf;
    use std::sync::{Arc, Mutex};
    use std::thread;

    use super::*;
    use crate::map::test_support::TempDir;

    /// A stand-in for Docker's engine on a Unix socket in a folder of its own, answering each GET
    /// with what `answer` makes of its path, until it is dropped. It keeps every request line, and
    /// lets a connection that sends none go.
    pub(in crate::map::docker) struct FakeEngine {
        pub(in crate::map::docker) socket: PathBuf,
        pub(in crate::map::docker) requests: Arc<Mutex<Vec<String>>>,
        _folder: TempDir,
    }

    impl FakeEngine {
        /// `test` names the folder, unique among the crate's tests, and short: the path of a Unix
        /// socket holds at most 103 bytes, and the system's temporary folder takes half of that.
        pub(in crate::map::docker) fn serving(
            test: &str,
            answer: impl Fn(&str) -> Vec<u8> + Send + 'static,
        ) -> FakeEngine {
            let folder = TempDir::new(test);
            let socket = folder.path("docker.sock");
            let listener = UnixListener::bind(&socket).unwrap();
            let requests = Arc::new(Mutex::new(Vec::new()));
            let kept = Arc::clone(&requests);
            thread::spawn(move || {
                for stream in listener.incoming() {
                    let Ok(mut stream) = stream else { return };
                    let mut reader = io::BufReader::new(stream.try_clone().unwrap());
                    let mut line = String::new();
                    if reader.read_line(&mut line).unwrap_or(0) == 0 {
                        continue;
                    }
                    // The rest of the request, to its blank line.
                    let mut header = String::new();
                    while reader.read_line(&mut header).unwrap() > 2 {
                        header.clear();
                    }
                    let path = line.split(' ').nth(1).unwrap_or_default().to_string();
                    kept.lock().unwrap().push(line.trim_end().to_string());
                    let _ = stream.write_all(&answer(&path));
                }
            });
            FakeEngine {
                socket,
                requests,
                _folder: folder,
            }
        }
    }

    /// An answer with `status`, `body` sent whole.
    pub(in crate::map::docker) fn answer(status: &str, body: &str) -> Vec<u8> {
        let head = format!(
            "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n",
            body.len()
        );
        [head.as_bytes(), body.as_bytes()].concat()
    }

    /// An answer of 200 sending `body` in chunks of `size`, as the engine streams a long answer.
    fn chunked(body: &str, size: usize) -> Vec<u8> {
        let mut bytes = b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n".to_vec();
        for chunk in body.as_bytes().chunks(size) {
            bytes.extend(format!("{:x};ext=1\r\n", chunk.len()).as_bytes());
            bytes.extend(chunk);
            bytes.extend(b"\r\n");
        }
        bytes.extend(b"0\r\n\r\n");
        bytes
    }

    #[test]
    fn a_get_sends_one_request_and_takes_the_body_whole_or_in_chunks() {
        let engine = FakeEngine::serving("docker-http-get", |path| match path {
            "/whole" => answer("200 OK", r#"{"whole":true}"#),
            _ => chunked(r#"[{"Id":"abc"},{"Id":"def"}]"#, 7),
        });

        let whole = get(&engine.socket, "/whole").unwrap();
        let chunks = get(&engine.socket, "/containers/json").unwrap();

        assert_eq!(whole, br#"{"whole":true}"#);
        assert_eq!(chunks, br#"[{"Id":"abc"},{"Id":"def"}]"#);
        assert_eq!(
            *engine.requests.lock().unwrap(),
            ["GET /whole HTTP/1.1", "GET /containers/json HTTP/1.1"]
        );
    }

    #[test]
    fn an_error_status_or_an_answer_that_is_no_http_is_no_body() {
        let engine = FakeEngine::serving("docker-http-errors", |path| match path {
            "/failing" => answer("500 Internal Server Error", r#"{"message":"boom"}"#),
            "/cut" => b"HTTP/1.1 200 OK\r\nContent-Length: 50\r\n\r\n{\"short\"".to_vec(),
            _ => b"SSH-2.0-OpenSSH\r\n\r\n".to_vec(),
        });

        let failing = get(&engine.socket, "/failing").unwrap_err();
        assert!(matches!(failing, GetError::Status), "{failing:?}");
        assert_eq!(failing.kind(), io::ErrorKind::Other);
        for garbled in ["/cut", "/ssh"] {
            let err = get(&engine.socket, garbled).unwrap_err();
            assert!(matches!(err, GetError::Malformed), "{garbled}: {err:?}");
        }
    }

    #[test]
    fn a_socket_nothing_serves_cannot_be_connected_to() {
        let folder = TempDir::new("docker-http-nothing");
        let gone = get(&folder.path("docker.sock"), "/_ping").unwrap_err();
        assert!(matches!(&gone, GetError::Connect(err) if err.kind() == io::ErrorKind::NotFound));
        // A socket left behind by a server that has stopped refuses the connection.
        let left = folder.path("left.sock");
        drop(UnixListener::bind(&left).unwrap());
        let refused = get(&left, "/_ping").unwrap_err();
        let kind = match &refused {
            GetError::Connect(err) => err.kind(),
            other => panic!("not a failed connection: {other:?}"),
        };
        assert_eq!(kind, io::ErrorKind::ConnectionRefused);
    }
}
