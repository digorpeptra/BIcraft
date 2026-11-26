use super::*;

fn lim() -> Limits {
    Limits::default()
}

#[test]
fn hosts() {
    for ok in [
        "localhost",
        "localhost:8080",
        "LOCALHOST.",
        "app.localhost:1",
        "127.0.0.1",
        "127.1.2.3:80",
        "[::1]",
        "[::1]:9",
    ] {
        assert!(host_allowed(ok), "{ok}");
    }
    for bad in [
        "",
        "evil.com",
        "localhost.evil.com",
        "localhost:99999",
        "localhost:",
        "10.0.0.1",
        "0.0.0.0",
        "[::2]",
        "[::1",
        ".localhost",
    ] {
        assert!(!host_allowed(bad), "{bad}");
    }
}

#[test]
fn head_parsing_and_unique_headers() {
    let h = parse_head(
        b"POST /w/a/mcp?x=1 HTTP/1.1\r\nHost: localhost:1\r\nAccept: a\r\naccept: b\r\nContent-Length: 12\r\n\r\n",
        &lim(),
        &["host"],
    )
    .unwrap();
    assert_eq!(h.method, "POST");
    assert_eq!(h.path, "/w/a/mcp");
    assert_eq!(h.header("HOST"), Some("localhost:1"));
    assert_eq!(h.headers_named("accept").collect::<Vec<_>>(), ["a", "b"]);
    assert_eq!(h.content_length, Some(12));
    let bad = Err(ReadError::Respond(Status::BadRequest));
    // declared unique: refused when repeated; Content-Length always is
    assert_eq!(
        parse_head(b"GET / HTTP/1.1\r\nHost: a\r\nhost: b\r\n\r\n", &lim(), &["Host"]),
        bad
    );
    assert_eq!(
        parse_head(
            b"GET / HTTP/1.1\r\nContent-Length: 1\r\nContent-Length: 1\r\n\r\n",
            &lim(),
            &[]
        ),
        bad
    );
    for cl in ["-1", "", "1x", "9999999999999", "+5"] {
        let head = format!("GET / HTTP/1.1\r\nContent-Length: {cl}\r\n\r\n");
        assert_eq!(parse_head(head.as_bytes(), &lim(), &[]), bad, "{cl}");
    }
    assert_eq!(
        parse_head(
            b"POST / HTTP/1.1\r\nTransfer-Encoding: chunked\r\n\r\n",
            &lim(),
            &[]
        ),
        Err(ReadError::Respond(Status::LengthRequired))
    );
    assert_eq!(
        parse_head(b"GET / HTTP/2\r\n\r\n", &lim(), &[]),
        Err(ReadError::Respond(Status::VersionNotSupported))
    );
    let long = format!("GET /{} HTTP/1.1\r\n\r\n", "a".repeat(3000));
    assert_eq!(
        parse_head(long.as_bytes(), &lim(), &[]),
        Err(ReadError::Respond(Status::UriTooLong))
    );
    let many: String = (0..70).map(|i| format!("X-{i}: v\r\n")).collect();
    let many = format!("GET / HTTP/1.1\r\n{many}\r\n");
    assert_eq!(
        parse_head(many.as_bytes(), &lim(), &[]),
        Err(ReadError::Respond(Status::HeadersTooLarge))
    );
}

/// A deterministic generator (xorshift), so failures replay.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n.max(1) as u64) as usize
    }
}

#[test]
fn hostile_heads_never_crash() {
    let seeds: [&[u8]; 4] = [
        b"POST /w/a/mcp HTTP/1.1\r\nHost: localhost\r\nContent-Length: 2\r\nMcp-Method: tools/call\r\n\r\n",
        b"GET / HTTP/1.1\r\n\r\n",
        b"GET /api/asset/0123456789abcdef HTTP/1.0\nHost: [::1]:80\n\n",
        b"\r\n\r\n",
    ];
    let alphabet: &[u8] = b"\r\n :/?#\x00\x7f\xff\xc3GETPOST HTTP/1.1 Content-Length 0123456789";
    let mut rng = Rng(0x9e37_79b9_7f4a_7c15);
    for _ in 0..20_000 {
        let mut buf = seeds[rng.below(seeds.len())].to_vec();
        for _ in 0..rng.below(8) {
            match rng.below(4) {
                0 if !buf.is_empty() => {
                    let i = rng.below(buf.len());
                    buf.remove(i);
                }
                1 => {
                    let i = rng.below(buf.len() + 1);
                    buf.insert(i, alphabet[rng.below(alphabet.len())]);
                }
                2 if !buf.is_empty() => {
                    let i = rng.below(buf.len());
                    buf[i] = (rng.next() & 0xff) as u8;
                }
                _ => {
                    let i = rng.below(buf.len() + 1);
                    let j = rng.below(buf.len() + 1);
                    let (a, b) = (i.min(j), i.max(j));
                    let piece = buf[a..b].to_vec();
                    buf.extend_from_slice(&piece);
                }
            }
        }
        let _ = head_end(&buf);
        if let Ok(h) = parse_head(&buf, &lim(), &["host", "origin"]) {
            assert!(h.headers.len() <= lim().max_headers);
            assert!(h.path.starts_with('/'));
        }
    }
}

#[test]
fn statuses_and_events() {
    assert_eq!(Status::Accepted.code(), 202);
    assert_eq!(Status::PayloadTooLarge.code(), 413);
    assert_eq!(
        String::from_utf8(sse_event(Some("message"), "{}")).unwrap(),
        "event: message\ndata: {}\n\n"
    );
    let mut out = Vec::new();
    write_empty(&mut out, Status::Accepted, &[("Bad", "a\r\nInjected: 1")]).unwrap();
    let text = String::from_utf8(out).unwrap();
    assert!(text.starts_with("HTTP/1.1 202 Accepted\r\nContent-Length: 0\r\n"));
    assert!(!text.contains("Injected"));
}

#[test]
fn tokens_are_random_hex() {
    let (a, b) = (random_token(), random_token());
    assert_eq!(a.len(), 32);
    assert_ne!(a, b);
    assert!(same_token(&a, &a.clone()));
    assert!(!same_token(&a, &b));
}

#[test]
fn connection_caps() {
    let conns = Connections::new(3, Some(2));
    let a: IpAddr = "127.0.0.1".parse().unwrap();
    let b: IpAddr = "127.0.0.2".parse().unwrap();
    let g1 = conns.acquire(a).unwrap();
    let g2 = conns.acquire(a).unwrap();
    assert!(conns.acquire(a).is_none(), "per-address cap");
    let g3 = conns.acquire(b).unwrap();
    assert!(conns.acquire(b).is_none(), "global cap");
    assert_eq!(conns.open(), 3);
    drop(g1);
    let g4 = conns.acquire(a).unwrap();
    drop((g2, g3, g4));
    assert_eq!(conns.open(), 0);
    assert!(conns.per_ip.lock().unwrap().is_empty());
}

fn pair() -> (TcpStream, TcpStream) {
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let c = TcpStream::connect(l.local_addr().unwrap()).unwrap();
    let (s, _) = l.accept().unwrap();
    (c, s)
}

#[test]
fn bodies_and_closed_peers() {
    let (mut client, mut server) = pair();
    client.write_all(b"world").unwrap();
    let body = read_body(&mut server, b"hello ".to_vec(), 11, Duration::from_secs(5)).unwrap();
    assert_eq!(body, b"hello world");
    // more than announced already read
    assert_eq!(
        read_body(&mut server, b"abc".to_vec(), 2, Duration::from_secs(1)),
        Err(Status::BadRequest)
    );
    // a short body times out
    client.write_all(b"x").unwrap();
    assert_eq!(
        read_body(&mut server, Vec::new(), 5, Duration::from_millis(100)),
        Err(Status::Timeout)
    );
    assert!(
        !peer_closed(&server, Duration::from_millis(20)),
        "data waiting is open"
    );
    let (client, server) = pair();
    assert!(!peer_closed(&server, Duration::from_millis(20)));
    drop(client);
    let t = Instant::now();
    while !peer_closed(&server, Duration::from_millis(20)) {
        assert!(t.elapsed() < Duration::from_secs(5));
    }
}

#[test]
fn accept_loop_caps_and_stops() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let addr = listener.local_addr().unwrap();
    let stop = Arc::new(AtomicBool::new(false));
    let conns = Connections::new(1, None);
    let (tx, rx) = std::sync::mpsc::channel::<()>();
    let rx = Arc::new(Mutex::new(rx));
    let handler: Handler = Arc::new(move |mut s: TcpStream, _| {
        // hold the slot until told to answer
        let _ = rx.lock().unwrap().recv_timeout(Duration::from_secs(5));
        let _ = write_response(&mut s, Status::Ok, "text/plain", &[], b"ok");
        finish(s);
    });
    let (stop2, conns2) = (Arc::clone(&stop), Arc::clone(&conns));
    let t = thread::spawn(move || accept_loop(listener, &stop2, &conns2, "test-conn", &handler));
    let first = TcpStream::connect(addr).unwrap();
    let start = Instant::now();
    while conns.open() == 0 {
        assert!(start.elapsed() < Duration::from_secs(5));
        thread::sleep(Duration::from_millis(5));
    }
    let mut second = TcpStream::connect(addr).unwrap();
    let mut out = String::new();
    second.read_to_string(&mut out).unwrap();
    assert!(out.starts_with("HTTP/1.1 503"), "{out}");
    tx.send(()).unwrap();
    let mut first = first;
    let mut out = String::new();
    first.read_to_string(&mut out).unwrap();
    assert!(out.starts_with("HTTP/1.1 200"), "{out}");
    stop.store(true, Ordering::SeqCst);
    t.join().unwrap();
}

#[test]
fn responses_are_noted_per_thread() {
    let _ = take_response_note();
    let mut out = Vec::new();
    write_response(&mut out, Status::NotFound, "text/plain", &[], b"abc").unwrap();
    // a second head on the same connection does not change the noted status
    write_empty(&mut out, Status::Accepted, &[]).unwrap();
    note_bytes(7);
    let (status, bytes) = take_response_note();
    assert_eq!(status, 404);
    assert_eq!(bytes, out.len() as u64 + 7);
    assert_eq!(take_response_note(), (0, 0));
    // a streamed body counts as declared
    let mut out = Vec::new();
    write_head(&mut out, Status::Ok, "application/octet-stream", 1000, &[]).unwrap();
    assert_eq!(take_response_note(), (200, out.len() as u64 + 1000));
    // another thread has its own note
    write_sse_head(&mut Vec::new(), &[]).unwrap();
    std::thread::spawn(|| assert_eq!(take_response_note(), (0, 0)))
        .join()
        .unwrap();
    assert_eq!(take_response_note().0, 200);
}

#[test]
fn query_strings_are_kept_apart_from_the_path() {
    let h = parse_head(b"GET /w/a/view/api/state?v=abc&x HTTP/1.1\r\n\r\n", &lim(), &[]).unwrap();
    assert_eq!(h.path, "/w/a/view/api/state");
    assert_eq!(h.query.as_deref(), Some("v=abc&x"));
    assert_eq!(h.query_param("v"), Some("abc"));
    assert_eq!(h.query_param("x"), Some(""));
    assert_eq!(h.query_param("y"), None);
    let h = parse_head(b"GET /a?v=1#frag?no HTTP/1.1\r\n\r\n", &lim(), &[]).unwrap();
    assert_eq!(h.query_param("v"), Some("1"));
    let h = parse_head(b"GET /a#x?v=1 HTTP/1.1\r\n\r\n", &lim(), &[]).unwrap();
    assert_eq!(h.query, None);
    assert_eq!(Status::SeeOther.code(), 303);
    assert_eq!(Status::PermanentRedirect.code(), 308);
}

#[test]
fn owner_only_files_replace_what_was_there() {
    let dir = std::env::temp_dir().join(format!("bicraft-http-owner-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let path = dir.join("sub").join("control.json");
    write_owner_only(&path, b"one").unwrap();
    write_owner_only(&path, b"two").unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), b"two");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        let mode = std::fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
    }
    // a folder in the way is an error, never a panic
    assert!(write_owner_only(&dir.join("sub"), b"x").is_err());
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn the_sid_is_read_from_whoami() {
    assert_eq!(
        current_sid(r#""host\user","S-1-5-21-1-2-3-1001""#),
        Some("S-1-5-21-1-2-3-1001")
    );
    assert_eq!(current_sid("\r\n\"a\",\"S-1-5-18\""), Some("S-1-5-18"));
    for bad in ["", "\"a\",\"x\"", "\"a\",\"S-1-5;rm\"", "S-2-1"] {
        assert_eq!(current_sid(bad), None, "{bad}");
    }
}

#[test]
fn only_a_single_full_control_entry_for_the_user_counts_as_owner_only() {
    let me = "S-1-5-21-1-2-3-1001";
    // what a CI runner's new file carried: the user, SYSTEM and Administrators
    let ci = format!("f.txt\r\nD:PAI(A;;FA;;;{me})(A;;FA;;;SY)(A;;FA;;;BA)\r\n");
    let (_, aces) = dacl_aces(&ci).unwrap();
    let trustees: Vec<_> = aces.iter().filter_map(|a| ace_trustee(a)).collect();
    assert_eq!(trustees, [me, "SY", "BA"]);
    assert!(!owner_only_dacl(&ci, me));
    assert!(owner_only_dacl(&format!("f.txt\r\nD:PAI(A;;FA;;;{me})\r\n"), me));
    for bad in [
        format!("D:AI(A;;FA;;;{me})"),    // not protected: inherits
        format!("D:P(A;;FR;;;{me})"),     // not full control
        format!("D:P(D;;FA;;;{me})"),     // a deny entry
        format!("D:P(A;OICI;FA;;;{me})"), // an inheritable entry
        "D:P(A;;FA;;;S-1-5-18)".into(),   // someone else
        "D:P".into(),                     // empty
        format!("D:P(A;;FA;;;{me}"),      // unbalanced
        String::new(),
    ] {
        assert!(!owner_only_dacl(&bad, me), "{bad}");
    }
}

#[test]
fn the_user_written_as_an_sddl_alias_is_the_user() {
    // SDDL writes some accounts by alias: the built-in Administrator (RID 500, a CI runner's
    // account) as LA, LocalSystem as SY
    let admin = "S-1-5-21-1-2-3-500";
    assert!(owner_only_dacl(
        "f.txt
D:PAI(A;;FA;;;LA)
",
        admin
    ));
    assert!(owner_only_dacl("D:PAI(A;;FA;;;SY)", "S-1-5-18"));
    assert!(trustee_is("la", admin));
    assert!(!trustee_is("LA", "S-1-5-21-1-2-3-1001"));
    assert!(!trustee_is("BA", admin));
    assert!(!owner_only_dacl("D:PAI(A;;FA;;;LA)(A;;FA;;;BA)", admin));
}

/// Windows: the token file has one access entry, the user's full control, nothing inherited from
/// its folder (which here, under the temp folder, grants others too).
#[cfg(windows)]
#[test]
fn owner_only_files_have_an_owner_only_acl_on_windows() {
    let dir = std::env::temp_dir().join(format!("bicraft-http-acl-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let path = dir.join("control.json");
    write_owner_only(&path, b"{\"token\": \"t\"}").unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), b"{\"token\": \"t\"}");
    let system = std::path::PathBuf::from(std::env::var_os("SystemRoot").unwrap()).join("System32");
    let out = std::process::Command::new(system.join("icacls.exe"))
        .arg(&path)
        .output()
        .unwrap();
    assert!(out.status.success());
    let text = String::from_utf8_lossy(&out.stdout).to_string();
    let entries = text.matches(":(").count();
    assert_eq!(entries, 1, "one access entry only:\n{text}");
    assert!(!text.contains("(I)"), "nothing inherited:\n{text}");
    assert!(text.contains("(F)"), "full control for the owner:\n{text}");
    let user = std::env::var("USERNAME").unwrap().to_lowercase();
    assert!(
        text.to_lowercase().contains(&user),
        "the entry is the user's:\n{text}"
    );
    // replaced again: still owner-only
    write_owner_only(&path, b"two").unwrap();
    let out = std::process::Command::new(system.join("icacls.exe"))
        .arg(&path)
        .output()
        .unwrap();
    assert_eq!(String::from_utf8_lossy(&out.stdout).matches(":(").count(), 1);
    std::fs::remove_dir_all(&dir).unwrap();
}

/// Windows: in a folder whose entries are not inheritable (as on CI runners), a new file gets the
/// creator's default DACL as explicit entries (SYSTEM, Administrators, the logon session…); the
/// token file still ends with one entry, the user's.
#[cfg(windows)]
#[test]
fn owner_only_files_drop_explicit_default_entries_on_windows() {
    let system = std::path::PathBuf::from(std::env::var_os("SystemRoot").unwrap()).join("System32");
    let whoami = std::process::Command::new(system.join("whoami.exe"))
        .args(["/user", "/fo", "csv", "/nh"])
        .output()
        .unwrap();
    let sid = current_sid(&String::from_utf8_lossy(&whoami.stdout))
        .unwrap()
        .to_string();
    let dir = std::env::temp_dir().join(format!("bicraft-http-acl-noinherit-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let status = std::process::Command::new(system.join("icacls.exe"))
        .arg(&dir)
        .args(["/inheritance:r", "/grant:r", &format!("*{sid}:(F)"), "/q"])
        .stdout(std::process::Stdio::null())
        .status()
        .unwrap();
    assert!(status.success());
    let path = dir.join("control.json");
    write_owner_only(&path, b"t").unwrap();
    let out = std::process::Command::new(system.join("icacls.exe"))
        .arg(&path)
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&out.stdout).to_string();
    assert_eq!(text.matches(":(").count(), 1, "one access entry only:\n{text}");
    assert!(text.contains("(F)"), "full control for the owner:\n{text}");
    std::fs::remove_dir_all(&dir).unwrap();
}
