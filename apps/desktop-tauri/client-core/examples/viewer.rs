//! Minimal terminal-readable session viewer example.

use dsh_client_core::{session::SessionViewer, transport::HttpTransport};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let url = args
        .next()
        .ok_or("usage: viewer <host-url> <session-id> [through-seq]")?;
    let session_id = args
        .next()
        .ok_or("usage: viewer <host-url> <session-id> [through-seq]")?;
    let through_seq = args
        .next()
        .map(|value| value.parse::<i64>())
        .transpose()?
        .unwrap_or(i64::MAX);
    let transport = HttpTransport::new(&url)?;
    let viewer = SessionViewer::new(&transport);
    let page = viewer.page(&session_id, through_seq)?;
    for record in &page.records {
        println!(
            "{} {} {}",
            record.event.seq, record.event.event_type, record.event.data
        );
    }
    Ok(())
}
