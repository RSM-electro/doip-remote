//! Raw bidirectional forwarding after pairing.

use std::time::Instant;

use tokio::{
    io::{copy_bidirectional, AsyncWriteExt},
    net::TcpStream,
};
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};

/// Copies bytes between the Car and Diag sockets until both sides finish or the
/// server is cancelled.
pub(crate) async fn run(
    mut car: TcpStream,
    mut diag: TcpStream,
    session: String,
    cancel: CancellationToken,
) {
    let started = Instant::now();
    let _ = car.set_nodelay(true);
    let _ = diag.set_nodelay(true);

    let result = tokio::select! {
        () = cancel.cancelled() => None,
        result = copy_bidirectional(&mut car, &mut diag) => Some(result),
    };

    let _ = car.shutdown().await;
    let _ = diag.shutdown().await;

    let elapsed_ms = started.elapsed().as_millis();
    match result {
        Some(Ok((car_to_diag, diag_to_car))) => info!(
            %session,
            car_to_diag,
            diag_to_car,
            elapsed_ms,
            "tunnel closed"
        ),
        Some(Err(error)) => warn!(%session, %error, elapsed_ms, "tunnel failed"),
        None => info!(%session, elapsed_ms, "tunnel stopped"),
    }
}
