//! v1.61.0 (D40): the only file under `obs/` that names a `tracing` macro.
//! Each helper takes only fixed codes, versions, uuids, timer ids, counts
//! and `io::ErrorKind`s — never the password, the secret, the auth string, a
//! pushed text, a source's settings, or a request's or an event's body — so
//! a source test can check that no secret reaches a log line on any thread
//! (§10.5).
