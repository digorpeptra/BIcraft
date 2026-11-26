//! Tests of the idle hook (`serve_with_idle`): the live viewer keeps publishing visuals whose
//! data did not fit a snapshot's budget, between tool calls, without holding them up.

use std::io::{BufReader, Read, Write};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::json;

use crate::{Server, serve_with_idle};

/// Input fed line by line from the test; dropping the sender is EOF.
struct ChanReader {
    rx: Receiver<Vec<u8>>,
    pending: Vec<u8>,
}

impl Read for ChanReader {
    fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
        if self.pending.is_empty() {
            match self.rx.recv() {
                Ok(b) => self.pending = b,
                Err(_) => return Ok(0),
            }
        }
        let n = out.len().min(self.pending.len());
        out[..n].copy_from_slice(&self.pending[..n]);
        self.pending.drain(..n);
        Ok(n)
    }
}

#[derive(Clone, Default)]
struct Out(Arc<Mutex<Vec<u8>>>);

impl Write for Out {
    fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(b);
        Ok(b.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl Out {
    fn lines(&self) -> usize {
        self.0.lock().unwrap().iter().filter(|&&b| b == b'\n').count()
    }
}

fn wait_until(what: &str, f: impl Fn() -> bool) {
    let t = Instant::now();
    while !f() {
        assert!(
            t.elapsed() < Duration::from_secs(20),
            "timed out waiting for {what}"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn send(tx: &Sender<Vec<u8>>, id: u64, method: &str, params: serde_json::Value) {
    let line = json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}).to_string() + "\n";
    tx.send(line.into_bytes()).unwrap();
}

#[test]
fn idle_hook_republishes_until_done_and_never_blocks_calls() {
    let runs = Arc::new(AtomicUsize::new(0));
    let more = Arc::new(AtomicBool::new(true));
    let (r, m) = (Arc::clone(&runs), Arc::clone(&more));
    let mut server = Server::new().with_idle(move |_| {
        r.fetch_add(1, Ordering::SeqCst);
        m.load(Ordering::SeqCst)
    });
    let (tx, rx) = channel();
    let out = Out::default();
    let o2 = out.clone();
    let worker = std::thread::spawn(move || {
        let input = BufReader::new(ChanReader {
            rx,
            pending: Vec::new(),
        });
        serve_with_idle(&mut server, input, o2, 1 << 20, Duration::from_millis(10))
    });
    // nothing arrives: the hook runs again and again while it has work left
    wait_until("repeated idle runs", || runs.load(Ordering::SeqCst) >= 3);
    // a message is still answered while the hook keeps running
    send(&tx, 1, "ping", json!({}));
    wait_until("the ping reply", || out.lines() >= 1);
    // done: the hook stops being called
    more.store(false, Ordering::SeqCst);
    std::thread::sleep(Duration::from_millis(60));
    let settled = runs.load(Ordering::SeqCst);
    std::thread::sleep(Duration::from_millis(80));
    assert_eq!(
        runs.load(Ordering::SeqCst),
        settled,
        "disarmed after returning false"
    );
    // a tool call arms it again (it runs once more, then stops)
    send(
        &tx,
        2,
        "tools/call",
        json!({"name": "new_model", "arguments": {"name": "M"}}),
    );
    wait_until("the tool reply", || out.lines() >= 2);
    wait_until("the re-armed run", || runs.load(Ordering::SeqCst) > settled);
    drop(tx);
    worker.join().unwrap().unwrap();
}

#[test]
fn without_a_hook_or_input_the_loop_ends_cleanly() {
    let mut server = Server::new();
    assert!(!server.idle_pending());
    let input = BufReader::new(&b"{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"ping\"}\nnot json\n"[..]);
    let out = Out::default();
    serve_with_idle(&mut server, input, out.clone(), 1 << 20, Duration::from_millis(5)).unwrap();
    assert_eq!(out.lines(), 2, "ping and the parse error are answered");
    // a panicking hook is disarmed, the server keeps serving
    let mut server = Server::new().with_idle(|_| panic!("boom"));
    assert!(server.idle_pending());
    server.run_idle();
    assert!(!server.idle_pending());
}
