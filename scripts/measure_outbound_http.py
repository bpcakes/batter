#!/usr/bin/env python3
"""Run opt-in release measurements; timings are observations, never CI thresholds.

The generated binary includes the production buffer without cfg(test) observers.
No external provider traffic or cloud credentials are used. Cargo's lockfile is
copied into the temporary project; the repository lockfile is never modified.
"""
import argparse
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parent.parent
SOURCE = r'''
use batter_outbound_http::{Body, Client, ConnectTimeout, Exchange, Origin, ResponseLimit};
use std::{sync::{Arc, atomic::{AtomicUsize, Ordering}}, time::Duration};
use tokio::{io::{AsyncBufReadExt, AsyncWriteExt, BufReader}, time::Instant};
#[path = "@BUFFER@"]
mod buffer;

fn main() {
    std::panic::set_hook(batter::redacting_panic_hook());
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all().max_blocking_threads(1).build().unwrap();
    runtime.block_on(async {
        tokio::time::timeout(Duration::from_secs(60), connections()).await.unwrap();
        dns_queue().await;
        for mib in [1, 8, 64] { body_cost(mib).await; }
    });
}

async fn connections() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let accepted = Arc::new(AtomicUsize::new(0));
    let counter = accepted.clone();
    let server = tokio::spawn(async move {
        let mut peers = tokio::task::JoinSet::new();
        loop {
            tokio::select! {
                result = listener.accept() => {
                    let (stream, _) = result.unwrap();
                    counter.fetch_add(1, Ordering::SeqCst);
                    peers.spawn(async move {
                        let mut peer = BufReader::new(stream);
                        loop {
                            let mut line = String::new();
                            loop {
                                line.clear();
                                if peer.read_line(&mut line).await.unwrap_or(0) == 0 { return; }
                                if line == "\r\n" { break; }
                            }
                            if peer.get_mut().write_all(b"HTTP/1.1 200 OK\r\ncontent-length: 4\r\n\r\nfour").await.is_err() { return; }
                        }
                    });
                }
                _ = peers.join_next(), if !peers.is_empty() => {}
            }
        }
    });
    let client = Client::new(Origin::loopback(address).unwrap(),
        ConnectTimeout::new(Duration::from_secs(2)).unwrap()).unwrap();
    let start = Instant::now();
    for _ in 0..500 {
        let result = client.request(http::Method::GET, []).prepare().unwrap()
            .send(Instant::now() + Duration::from_secs(3), ResponseLimit::new(4).unwrap()).await;
        match result {
            Exchange::Responded(response) => assert!(matches!(response.body(), Body::Complete(body) if body.as_ref() == b"four")),
            other => panic!("unexpected outcome: {}", other.label()),
        }
    }
    let fresh = accepted.load(Ordering::SeqCst);
    assert_eq!(fresh, 500);
    println!("connections mode=fresh requests=500 tcp={} elapsed_ms={:.3}", fresh, start.elapsed().as_secs_f64()*1000.0);
    let pooled = reqwest::Client::builder().http1_only().no_proxy()
        .redirect(reqwest::redirect::Policy::none()).retry(reqwest::retry::never()).build().unwrap();
    let url = format!("http://{address}/");
    let start = Instant::now();
    for _ in 0..500 {
        assert_eq!(pooled.get(&url).send().await.unwrap().bytes().await.unwrap().as_ref(), b"four");
    }
    let reused = accepted.load(Ordering::SeqCst) - fresh;
    assert_eq!(reused, 1);
    println!("connections mode=pooled-control requests=500 tcp={} elapsed_ms={:.3}", reused, start.elapsed().as_secs_f64()*1000.0);
    drop(pooled);
    server.abort();
    assert!(server.await.unwrap_err().is_cancelled());
}

async fn dns_queue() {
    // Occupy the sole blocking worker before a real lookup_host is submitted.
    // This models DNS queue starvation, not an upstream getaddrinfo stall.
    let client = Client::new(Origin::https("https://localhost:9").unwrap(),
        ConnectTimeout::new(Duration::from_millis(100)).unwrap()).unwrap();
    let (release, blocked) = std::sync::mpsc::channel();
    let (started, ready) = tokio::sync::oneshot::channel();
    let blocker = tokio::task::spawn_blocking(move || {
        started.send(()).unwrap();
        blocked.recv_timeout(Duration::from_secs(5)).unwrap();
    });
    ready.await.unwrap();
    let start = Instant::now();
    let result = client.request(http::Method::GET, []).prepare().unwrap()
        .send(Instant::now()+Duration::from_secs(1), ResponseLimit::new(4).unwrap()).await;
    let elapsed = start.elapsed();
    release.send(()).unwrap();
    blocker.await.unwrap();
    assert_eq!(result.label(), "not_dispatched.connect_timeout");
    println!("dns blocking_pool_saturated=true outcome={} elapsed_ms={:.3}", result.label(), elapsed.as_secs_f64()*1000.0);

    // Independently exercise the upstream started-blocking-work lifetime.
    let (release, blocked) = std::sync::mpsc::channel();
    let (started, ready) = tokio::sync::oneshot::channel();
    let task = tokio::task::spawn_blocking(move || {
        started.send(()).unwrap();
        blocked.recv_timeout(Duration::from_secs(5)).unwrap();
    });
    ready.await.unwrap();
    task.abort();
    assert!(!task.is_finished());
    release.send(()).unwrap();
    task.await.unwrap();
    println!("blocking_work started_task_survives_abort=true (model, not a DNS-server stall)");
}

async fn body_cost(mib: usize) {
    let limit = mib * 1024 * 1024;
    let chunk = vec![42; 64 * 1024];
    let mut body = buffer::ZeroizingBuffer::with_capacity(1024 * 1024);
    let mut maximum = Duration::ZERO;
    let mut lag = Duration::ZERO;
    for _ in 0..limit/chunk.len() {
        // Register a timer before each append. The current-thread runtime cannot
        // poll it while the synchronous production append is running.
        let due = Instant::now() + Duration::from_millis(1);
        let timer = tokio::spawn(async move {
            tokio::time::sleep_until(due).await;
            Instant::now().saturating_duration_since(due)
        });
        tokio::task::yield_now().await;
        let start = Instant::now();
        body.append(&chunk, limit).unwrap();
        maximum = maximum.max(start.elapsed());
        lag = lag.max(timer.await.unwrap());
    }
    let bytes = body.into_bytes();
    let slice = bytes.slice(..4);
    drop(bytes);
    let due = Instant::now() + Duration::from_millis(1);
    let timer = tokio::spawn(async move {
        tokio::time::sleep_until(due).await;
        Instant::now().saturating_duration_since(due)
    });
    tokio::task::yield_now().await;
    let start = Instant::now();
    drop(slice);
    let wipe = start.elapsed();
    let drop_lag = timer.await.unwrap();
    println!("buffer mib={} max_append_ms={:.3} append_timer_lag_ms={:.3} last_slice_drop_ms={:.3} drop_timer_lag_ms={:.3}",
        mib, maximum.as_secs_f64()*1000.0, lag.as_secs_f64()*1000.0,
        wipe.as_secs_f64()*1000.0, drop_lag.as_secs_f64()*1000.0);
}
'''


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--check", action="store_true", help="compile the probe without running measurements")
    options = parser.parse_args()
    with tempfile.TemporaryDirectory(prefix="batter-outbound-measure-") as directory:
        project = Path(directory)
        (project / "src").mkdir()
        buffer = ROOT / "crates/batter-outbound-http/src/send/buffer.rs"
        (project / "src/main.rs").write_text(SOURCE.replace("@BUFFER@", str(buffer)))
        manifest = '''[package]
name = "outbound-measurements"
version = "0.0.0"
edition = "2024"
[dependencies]
'''
        for alias, package in [("batter", "batter-core"), ("batter-outbound-http", "batter-outbound-http")]:
            manifest += f'{alias} = {{ package = "{package}", path = {json.dumps(str(ROOT / "crates" / package))} }}\n'
        manifest += '''tokio = { version = "=1.54.1", features = ["rt", "time", "net", "io-util", "sync", "macros"] }
bytes = "=1.12.1"
zeroize = "=1.9.1"
http = "=1.5.0"
reqwest = { version = "=0.13.5", default-features = false }
'''
        (project / "Cargo.toml").write_text(manifest)
        shutil.copyfile(ROOT / "Cargo.lock", project / "Cargo.lock")
        environment = dict(os.environ)
        environment["CARGO_TARGET_DIR"] = str(ROOT / "target/outbound-measurements")
        command = ["cargo", "check"] if options.check else ["cargo", "run", "--release"]
        subprocess.run([*command, "--offline", "--manifest-path",
                        str(project / "Cargo.toml")], cwd=ROOT, env=environment,
                       check=True, timeout=600)


if __name__ == "__main__":
    main()
