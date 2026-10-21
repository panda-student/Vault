//! All MCP frames pass this boundary, including SDK-generated errors. Credential
//! values and common encodings remain in zeroizing memory, never in diagnostics.
use crate::{
    core::CoreSession,
    error::{AppError, Result},
};
use base64::Engine;
use serde_json::Value;
use std::{
    io,
    pin::Pin,
    sync::{Arc, RwLock},
    task::{Context, Poll, ready},
};
use tokio::io::AsyncWrite;
use zeroize::Zeroizing;

#[derive(Default)]
pub struct OutputGuard {
    values: Vec<Zeroizing<String>>,
}
pub type SharedGuard = Arc<RwLock<OutputGuard>>;
impl OutputGuard {
    pub fn add(&mut self, value: &str) {
        if value.is_empty() {
            return;
        }
        let encoded = serde_json::to_string(value).unwrap_or_default();
        let escaped = encoded
            .get(1..encoded.len().saturating_sub(1))
            .unwrap_or("");
        for item in [
            value.to_owned(),
            base64::engine::general_purpose::STANDARD.encode(value),
            base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(value),
            percent_encoding::utf8_percent_encode(value, percent_encoding::NON_ALPHANUMERIC)
                .to_string(),
            escaped.to_owned(),
        ] {
            if !item.is_empty() && !self.values.iter().any(|existing| existing.as_str() == item) {
                self.values.push(Zeroizing::new(item));
            }
        }
    }
    pub fn refresh(&mut self, core: &CoreSession) -> Result<()> {
        core.refresh_output_guard(self)
    }
    fn contains(&self, text: &str) -> bool {
        self.values
            .iter()
            .any(|value| text.contains(value.as_str()))
    }
    pub fn scrub(&self, value: &mut Value) {
        match value {
            Value::String(text) if self.contains(text) => *value = Value::Null,
            Value::Array(values) => {
                for value in values {
                    self.scrub(value);
                }
            }
            Value::Object(values) => {
                values.retain(|key, _| !self.contains(key));
                for value in values.values_mut() {
                    self.scrub(value);
                }
            }
            _ => {}
        }
    }
}
pub fn scrub(guard: &SharedGuard, mut value: Value) -> Result<Value> {
    guard
        .read()
        .map_err(|_| AppError::storage())?
        .scrub(&mut value);
    Ok(value)
}

pub struct SafeWriter<W> {
    inner: W,
    guard: SharedGuard,
    core: Option<Arc<CoreSession>>,
    input: Vec<u8>,
    pending: Vec<u8>,
    offset: usize,
}
impl<W> SafeWriter<W> {
    pub fn with_core(mut self, core: Arc<CoreSession>) -> Self {
        self.core = Some(core);
        self
    }
    pub fn new(inner: W, guard: SharedGuard) -> Self {
        Self {
            inner,
            guard,
            core: None,
            input: vec![],
            pending: vec![],
            offset: 0,
        }
    }
}
impl<W: AsyncWrite + Unpin> SafeWriter<W> {
    fn drain(&mut self, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        while self.offset < self.pending.len() {
            let written =
                ready!(Pin::new(&mut self.inner).poll_write(cx, &self.pending[self.offset..]))?;
            if written == 0 {
                return Poll::Ready(Err(io::ErrorKind::WriteZero.into()));
            }
            self.offset += written;
        }
        self.pending.clear();
        self.offset = 0;
        Poll::Ready(Ok(()))
    }
}
impl<W: AsyncWrite + Unpin> AsyncWrite for SafeWriter<W> {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bytes: &[u8],
    ) -> Poll<io::Result<usize>> {
        ready!(self.drain(cx))?;
        if self.input.len() + bytes.len() > 4 * 1024 * 1024 {
            return Poll::Ready(Err(io::Error::other("MCP output limit exceeded")));
        }
        self.input.extend_from_slice(bytes);
        while let Some(end) = self.input.iter().position(|b| *b == b'\n') {
            let line: Vec<u8> = self.input.drain(..=end).collect();
            let mut value: Value = serde_json::from_slice(&line)
                .map_err(|_| io::Error::other("Invalid outgoing protocol frame"))?;
            {
                let mut guard = self
                    .guard
                    .write()
                    .map_err(|_| io::Error::other("Output guard unavailable"))?;
                if let Some(core) = &self.core {
                    guard
                        .refresh(core)
                        .map_err(|_| io::Error::other("Output protection refresh failed"))?;
                }
                guard.scrub(&mut value);
            }
            let encoded = serde_json::to_vec(&value)
                .map_err(|_| io::Error::other("Output encoding failed"))?;
            self.pending.extend(encoded);
            self.pending.push(b'\n');
        }
        Poll::Ready(Ok(bytes.len()))
    }
    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        ready!(self.drain(cx))?;
        Pin::new(&mut self.inner).poll_flush(cx)
    }
    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        ready!(self.drain(cx))?;
        Pin::new(&mut self.inner).poll_shutdown(cx)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn protocol_guard_handles_split_frames_and_sdk_error_text() {
        use tokio::io::AsyncWriteExt;
        tokio::runtime::Builder::new_current_thread().build().unwrap().block_on(async{
            let mut output=OutputGuard::default();output.add("synthetic-global-secret-456");
            let mut writer=SafeWriter::new(Vec::<u8>::new(),Arc::new(RwLock::new(output)));
            writer.write_all(br#"{"jsonrpc":"2.0","id":1,"error":{"code":-32602,"message":"synthetic-global-"#).await.unwrap();
            writer.write_all(b"secret-456\"}}\n").await.unwrap();writer.flush().await.unwrap();
            let text=String::from_utf8(writer.inner).unwrap();assert!(!text.contains("synthetic-global-secret-456"));let value:Value=serde_json::from_str(&text).unwrap();assert!(value["error"]["message"].is_null());
        });
    }
    #[test]
    fn nested_metadata_and_common_encodings_are_suppressed() {
        let mut guard = OutputGuard::default();
        guard.add("synthetic-protected-API_123");
        let mut value = serde_json::json!({"title":"prefix synthetic-protected-API_123 suffix","nested":[base64::engine::general_purpose::STANDARD.encode("synthetic-protected-API_123")],"safe":"service label"});
        guard.scrub(&mut value);
        assert!(value["title"].is_null());
        assert!(value["nested"][0].is_null());
        assert_eq!(value["safe"], "service label");
    }
}
