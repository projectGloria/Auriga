//! Framing: u32 little-endian length prefix + JSON body, capped at MAX_FRAME.

use crate::proto::MAX_FRAME;
use serde::{Serialize, de::DeserializeOwned};
use std::io::{Read, Write};

#[derive(Debug, thiserror::Error)]
pub enum FrameError {
    #[error("i/o: {0}")]
    Io(#[from] std::io::Error),
    #[error("frame too large ({0} bytes)")]
    TooLarge(usize),
    #[error("invalid message: {0}")]
    Json(#[from] serde_json::Error),
}

pub fn write_frame<W: Write, T: Serialize>(w: &mut W, msg: &T) -> Result<(), FrameError> {
    let body = serde_json::to_vec(msg)?;
    if body.len() > MAX_FRAME {
        return Err(FrameError::TooLarge(body.len()));
    }
    let mut buf = Vec::with_capacity(4 + body.len());
    buf.extend_from_slice(&(body.len() as u32).to_le_bytes());
    buf.extend_from_slice(&body);
    w.write_all(&buf)?;
    w.flush()?;
    Ok(())
}

pub fn read_frame<R: Read, T: DeserializeOwned>(r: &mut R) -> Result<T, FrameError> {
    let mut len = [0u8; 4];
    r.read_exact(&mut len)?;
    let len = u32::from_le_bytes(len) as usize;
    if len > MAX_FRAME {
        return Err(FrameError::TooLarge(len));
    }
    let mut body = vec![0u8; len];
    r.read_exact(&mut body)?;
    Ok(serde_json::from_slice(&body)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::proto::*;
    use std::io::Cursor;

    #[test]
    fn roundtrip() {
        let mut buf = Vec::new();
        let m = AppMessage::Request {
            id: 7,
            op: HelperOp::Ping { nonce: 42 },
        };
        write_frame(&mut buf, &m).unwrap();
        let back: AppMessage = read_frame(&mut Cursor::new(buf)).unwrap();
        assert_eq!(back, m);
    }

    #[test]
    fn rejects_oversized_length() {
        let mut buf = ((MAX_FRAME + 1) as u32).to_le_bytes().to_vec();
        buf.extend_from_slice(b"{}");
        let r: Result<AppMessage, _> = read_frame(&mut Cursor::new(buf));
        assert!(matches!(r, Err(FrameError::TooLarge(_))));
    }

    #[test]
    fn truncated_is_error() {
        let buf = vec![10, 0, 0, 0, b'{'];
        let r: Result<AppMessage, _> = read_frame(&mut Cursor::new(buf));
        assert!(matches!(r, Err(FrameError::Io(_))));
    }
}
