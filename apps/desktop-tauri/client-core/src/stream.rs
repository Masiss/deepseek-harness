//! Decoded iteration over one logical Remote stream.
//!
//! The Gateway carries every stream on the shared mux; this wrapper decodes each
//! item into the endpoint's frame type and cancels the logical stream on drop.

use crate::{
    remote_stream::{RemoteStreamMux, Uplink},
    Error,
};
use serde::de::DeserializeOwned;
use serde_json::Value;
use std::{
    marker::PhantomData,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Arc,
    },
};

/// Typed receiver for one logical stream.
pub struct FrameStream<T> {
    receiver: mpsc::Receiver<Result<Value, Error>>,
    uplink: Uplink,
    cancelled: Arc<AtomicBool>,
    marker: PhantomData<T>,
}

impl<T> FrameStream<T> {
    /// Stops the logical stream without waiting for the mux worker.
    pub fn cancel(&self) {
        if !self.cancelled.swap(true, Ordering::AcqRel) {
            self.uplink.cancel();
        }
    }
}

impl<T: DeserializeOwned> FrameStream<T> {
    /// Waits up to `timeout` for the next frame.
    pub fn recv_timeout(&mut self, timeout: std::time::Duration) -> Option<Result<T, Error>> {
        let value = self.receiver.recv_timeout(timeout).ok()?;
        Some(value.and_then(|value| serde_json::from_value(value).map_err(Error::from)))
    }
}

impl<T: DeserializeOwned> Iterator for FrameStream<T> {
    type Item = Result<T, Error>;

    fn next(&mut self) -> Option<Self::Item> {
        let value = self.receiver.recv().ok()?;
        Some(value.and_then(|value| serde_json::from_value(value).map_err(Error::from)))
    }
}

impl<T> Drop for FrameStream<T> {
    fn drop(&mut self) {
        self.cancel();
    }
}

/// Opens one logical stream and decodes its items as `T`.
pub fn open<T: DeserializeOwned>(
    mux: &RemoteStreamMux,
    endpoint: &str,
    args: Value,
) -> Result<FrameStream<T>, Error> {
    let stream = mux.open(endpoint, args)?;
    Ok(FrameStream {
        receiver: stream.downlink,
        uplink: stream.uplink,
        cancelled: Arc::new(AtomicBool::new(false)),
        marker: PhantomData,
    })
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::remote_stream::tests::{read, send, server};
    use serde::Deserialize;

    #[derive(Debug, Deserialize, PartialEq)]
    struct Frame {
        #[serde(rename = "type")]
        frame_type: String,
    }

    #[test]
    fn decodes_items_and_cancels_on_drop() {
        let (url, credential, worker) = server(|mut socket| {
            let open = read(&mut socket);
            send(
                &mut socket,
                serde_json::json!({"type":"item","streamId":open["streamId"],"value":{"type":"ready"}}),
            );
            send(
                &mut socket,
                serde_json::json!({"type":"end","streamId":open["streamId"]}),
            );
        });
        let mux = RemoteStreamMux::connect(&url, &credential).unwrap();
        let mut stream: FrameStream<Frame> = open(&mux, "test/stream", serde_json::json!({})).unwrap();
        assert_eq!(
            stream.next().unwrap().unwrap(),
            Frame {
                frame_type: "ready".into()
            }
        );
        assert!(stream.next().is_none());
        worker.join().unwrap();
    }

    #[test]
    fn malformed_frames_surface_as_errors() {
        let (url, credential, worker) = server(|mut socket| {
            let open = read(&mut socket);
            send(
                &mut socket,
                serde_json::json!({"type":"item","streamId":open["streamId"],"value":{"type":7}}),
            );
        });
        let mux = RemoteStreamMux::connect(&url, &credential).unwrap();
        let mut stream: FrameStream<Frame> = open(&mux, "test/stream", serde_json::json!({})).unwrap();
        assert!(matches!(stream.next(), Some(Err(Error::InvalidEnvelope(_)))));
        drop(stream);
        worker.join().unwrap();
    }
}
