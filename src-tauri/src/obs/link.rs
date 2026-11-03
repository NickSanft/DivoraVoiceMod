//! v1.61.0: a running link — the *driver* (connect, handshake, backoff, the
//! one outstanding request, ops, the feed, Follow OBS, commands, due marks)
//! and the *session* (owns the socket: reads, writes, keepalive, the close),
//! two tasks on [`LinkDeps::spawner`] (§5.5). Every wait also watches
//! `cmds`, whose end is the stop (D4, D6). The link connects only to
//! [`LinkDeps::cfg`]'s IP and port and looks no name up (D7).

use super::LinkDeps;

/// The driver (§5.5): runs until `deps.cmds` closes, then acks on
/// `deps.closed_ack`.
#[allow(clippy::unused_async)] // S1: track B1's body awaits every step
pub async fn run(deps: LinkDeps) {
    // S1: track B1 writes the driver. Until then a link stops at once,
    // having connected nowhere.
    let _ = deps.closed_ack.try_send(());
}
