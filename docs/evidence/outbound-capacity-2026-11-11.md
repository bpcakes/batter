# Outbound HTTP capacity and scheduling assessment

Owners: `batter-wznf.1` (dial repair), `batter-wznf.2` (measurements).

## Executed evidence

Run `python3 scripts/measure_outbound_http.py` from the workspace. It generates
a temporary Cargo project, copies the workspace lockfile, uses the exact pinned
Tokio/bytes/zeroize/reqwest versions, and runs a release binary. It includes the
production buffer source without its test destructor observer. No repository
lockfile, system network configuration or external provider is modified.
`--check` compiles the probe without running measurements; the scripts part of
the verification matrix includes that command. Timing is not a CI threshold.

Three completed runs on 2026-11-11 used Rust 1.98.1, Linux x86-64 kernel
7.1.5-76070105-generic, an AMD Ryzen Threadripper 3970X and a current-thread Tokio
runtime. The machine was not isolated; workspace verification overlapped some
runs. Ranges below are observed minima/maxima across these three runs, not
percentiles or worst-case bounds.

| Scenario | Executed result |
| --- | --- |
| 500 sequential four-byte loopback HTTP responses, Batter | 500 accepted TCP connections; 106.836–114.481 ms total |
| Same server and response, reqwest HTTP/1 pooled control, redirects and proxy disabled | 1 accepted TCP connection; 32.122–37.693 ms total |
| Real hostname lookup queued behind an occupied sole blocking worker; 100 ms connect budget | `not_dispatched.connect_timeout` in 101.353–101.638 ms |
| Abort a blocking task after its start acknowledgement | Task remains unfinished until explicitly released, then joins successfully |

The pooled control disables reqwest's retry policy with `reqwest::retry::never()`.
Its hyper-util 0.1.20 client still enables recovery of unstarted requests when a
reused connection becomes unusable before writing begins; see the
[upstream retry setting](https://docs.rs/hyper-util/0.1.20/hyper_util/client/legacy/struct.Builder.html#method.retry_canceled_requests).
The connection comparison measures whole client paths, not pooling in isolation.
It has no TLS, network RTT, packet loss, concurrency or NAT. The DNS scenario
uses `lookup_host` through the real send path but stalls its blocking-pool queue;
it does not stall a running libc resolver. The separate blocking-task experiment
demonstrates Tokio's lifetime behavior, not an actual upstream DNS outage.

The body experiment appends 64 KiB frames into the production buffer, starting
with the production preallocation ceiling of 1 MiB. It registers a one-millisecond
timer before each append, then retains only a four-byte slice before the final
drop. Input chunk allocation is outside the timed append. Drop includes wiping
and deallocation. Timer lag includes runtime timer granularity and scheduling.

| Final body | Largest append (ms) | Append timer lag (ms) | Last slice drop (ms) | Drop timer lag (ms) |
| --- | --- | --- | --- | --- |
| 1 MiB | 0.046–0.088 | 1.179–1.199 | 0.369–0.658 | 0.578–1.452 |
| 8 MiB | 3.857–4.702 | 2.874–3.726 | 3.435–4.680 | 2.454–3.711 |
| 64 MiB | 31.387–34.494 | 30.411–33.523 | 25.994–26.864 | 25.018–25.892 |

These are actual production append/drop operations, unlike the earlier separate
64 MiB copy-plus-wipe estimate of 56 ms. Neither set of measurements is an upper
bound. The largest growth here replaces 32 MiB with 64 MiB; it does not copy and
wipe an old 64 MiB allocation. Allocation/page faults and memory pressure can
change the cost substantially.

## Decisions

1. Repair DNS answer-count starvation now. Deterministic tests cover 1, 10 and
   20 healthy candidates taking 150 or 800 ms each under a one-second deadline,
   a stalled candidate followed by a delayed healthy fallback, and a short
   deadline that must not expand. Fair shares have a one-second floor capped by
   the shared deadline. This is local scheduling policy, not a network latency
   guarantee; not every candidate can be tried when earlier attempts stall.
   Existing family-race and loser-destruction tests remain required.
2. Retain the experimental, connection-owned transport. The 500:1 connection
   count is sufficient evidence of connection churn; these local timings do not
   justify production throughput claims. Pooling/HTTP2 needs a separate owned
   connection design that preserves no automatic replay, handoff evidence,
   interruption gates and connection settlement. A mechanical swap to reqwest
   would lose those contracts. Do not promote this package on this evidence.
3. Retain the narrowly specified body-allocation wipe and current public limit.
   The measurements confirm a substantial scheduler cost at 64 MiB. They do not
   justify silently removing wiping, lowering the supported limit, or claiming
   a 50 ms reserve bounds synchronous work. Choose the smallest response limit
   the provider protocol needs; retaining a slice retains its backing body.
   A future storage redesign must measure both growth and last-owner destruction
   and preserve wiping on cancellation, early failure and replacement. Merely
   yielding between frames cannot interrupt one growth or destructor.

No public call shape changed. The ADR-010 review finds no new consumer ordering,
cleanup or paired-call obligation: the dialer still owns scheduling and losers.
Network latency, NAT quotas and provider effects are external facts; local
typestate cannot prove them. Destination filtering and NAT64 policy are unchanged.

## Evidence still required for production promotion

Cloud NAT capacity, real TLS handshakes/resumption, concurrent traffic, packet
loss, a running resolver stall and macOS scheduling remain unverified. Promotion
requires a workload-specific run recording request rate and concurrency, DNS
answer distribution, latency/error distributions, blocking-pool occupancy,
connection counts, local socket states and the actual NAT allocation/error
metrics. A successful loopback run is not that evidence.

For capacity planning, connection creation rate times mapping residence time
estimates occupied mappings only under an explicit destination/allocation model.
AWS documents 55,000 simultaneous connections per unique destination per NAT
IPv4 address. Google's default 64 ports is a minimum static allocation; dynamic
allocation and TIME_WAIT configuration change the model. Neither establishes a
universal 1–17 requests/second ceiling. See the
[primary references](../references/outbound-http.md#capacity-and-blocking-work-2026-11-11).
