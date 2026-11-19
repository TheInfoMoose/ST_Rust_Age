use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

use crate::LinkError;

pub const MAX_FRAME: u32 = 1 << 20;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "op")]
pub enum Request {
    Pair {
        client_pubkey: String,
        label: String,
    },
    Roots,
    List {
        root_id: String,
        rel_path: String,
    },
    Stat {
        root_id: String,
        rel_path: String,
    },
    Hash {
        root_id: String,
        rel_path: String,
    },
    Exists {
        root_id: String,
        rel_path: String,
    },
    Mkdir {
        root_id: String,
        rel_path: String,
    },
    Volumes,
    DiskFree {
        root_id: String,
    },
    BeginUpload {
        root_id: String,
        rel_path: String,
        offset: u64,
        len: u64,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "op")]
pub enum Response {
    Paired,
    Roots { roots: Vec<ShareRootInfo> },
    List { entries: Vec<EntryInfo> },
    Stat { entry: EntryInfo },
    Hash { sha256: String },
    Exists { exists: bool },
    Ok,
    Volumes { volumes: Vec<VolumeEntry> },
    DiskFree { available_bytes: u64 },
    UploadGrant { grant: UploadGrant },
    Error { code: String, message: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShareRootInfo {
    pub id: String,
    pub read: bool,
    pub write: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EntryInfo {
    pub name: String,
    pub is_dir: bool,
    pub size: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VolumeEntry {
    pub mount_point: String,
    pub label: String,
    pub total_bytes: u64,
    pub available_bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UploadGrant {
    pub udp_port: u16,
    #[serde(with = "hex32")]
    pub cert_sha256: [u8; 32],
    #[serde(with = "hex32")]
    pub ticket: [u8; 32],
}

mod hex32 {
    use serde::de::Error;
    use serde::{Deserializer, Serializer};

    pub fn serialize<S>(bytes: &[u8; 32], serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut s = String::with_capacity(64);
        for &b in bytes {
            use std::fmt::Write;
            let _ = write!(s, "{b:02x}");
        }
        serializer.serialize_str(&s)
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<[u8; 32], D::Error>
    where
        D: Deserializer<'de>,
    {
        let s: String = serde::Deserialize::deserialize(deserializer)?;
        if s.len() != 64 {
            return Err(D::Error::custom(format!(
                "expected 64-char hex string, got length {}",
                s.len()
            )));
        }
        let mut out = [0u8; 32];
        let (chunks, _) = s.as_bytes().as_chunks::<2>();
        for (i, &[hi_b, lo_b]) in chunks.iter().enumerate() {
            let hi = decode_nibble(hi_b).ok_or_else(|| D::Error::custom("invalid hex char"))?;
            let lo = decode_nibble(lo_b).ok_or_else(|| D::Error::custom("invalid hex char"))?;
            out[i] = (hi << 4) | lo;
        }
        Ok(out)
    }

    fn decode_nibble(b: u8) -> Option<u8> {
        match b {
            b'0'..=b'9' => Some(b - b'0'),
            b'a'..=b'f' => Some(b - b'a' + 10),
            b'A'..=b'F' => Some(b - b'A' + 10),
            _ => None,
        }
    }
}

pub async fn write_frame<W, T>(w: &mut W, msg: &T) -> Result<(), LinkError>
where
    W: AsyncWrite + Unpin,
    T: Serialize,
{
    let body = serde_json::to_vec(msg).map_err(|e| LinkError::Protocol(e.to_string()))?;
    let len = u32::try_from(body.len()).map_err(|_| LinkError::FrameTooLarge(u32::MAX))?;
    if len > MAX_FRAME {
        return Err(LinkError::FrameTooLarge(len));
    }
    w.write_u32(len).await?;
    w.write_all(&body).await?;
    w.flush().await?;
    Ok(())
}

pub async fn read_frame<R, T>(r: &mut R) -> Result<T, LinkError>
where
    R: AsyncRead + Unpin,
    T: DeserializeOwned,
{
    let len = r.read_u32().await?;
    if len > MAX_FRAME {
        return Err(LinkError::FrameTooLarge(len));
    }
    let mut buf = vec![0u8; len as usize];
    r.read_exact(&mut buf).await?;
    serde_json::from_slice(&buf).map_err(|e| LinkError::Protocol(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn round_trip_all_request_variants() {
        let requests = vec![
            Request::Pair {
                client_pubkey: "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5".to_string(),
                label: "testuser-laptop".to_string(),
            },
            Request::Roots,
            Request::List {
                root_id: "share".to_string(),
                rel_path: "docs/sub".to_string(),
            },
            Request::Stat {
                root_id: "share".to_string(),
                rel_path: "file.txt".to_string(),
            },
            Request::Hash {
                root_id: "share".to_string(),
                rel_path: "file.txt".to_string(),
            },
            Request::Exists {
                root_id: "share".to_string(),
                rel_path: "file.txt".to_string(),
            },
            Request::Mkdir {
                root_id: "share".to_string(),
                rel_path: "new_dir".to_string(),
            },
            Request::Volumes,
            Request::DiskFree {
                root_id: "share".to_string(),
            },
            Request::BeginUpload {
                root_id: "share".to_string(),
                rel_path: "upload.bin".to_string(),
                offset: 1024,
                len: 65536,
            },
        ];

        let (mut tx, mut rx) = tokio::io::duplex(64 * 1024);
        for req in &requests {
            write_frame(&mut tx, req).await.unwrap();
            let decoded: Request = read_frame(&mut rx).await.unwrap();
            assert_eq!(&decoded, req);
        }
    }

    #[tokio::test]
    async fn round_trip_all_response_variants() {
        let responses = vec![
            Response::Paired,
            Response::Roots {
                roots: vec![ShareRootInfo {
                    id: "share".to_string(),
                    read: true,
                    write: false,
                }],
            },
            Response::List {
                entries: vec![EntryInfo {
                    name: "readme.md".to_string(),
                    is_dir: false,
                    size: 42,
                }],
            },
            Response::Stat {
                entry: EntryInfo {
                    name: "dir".to_string(),
                    is_dir: true,
                    size: 0,
                },
            },
            Response::Hash {
                sha256: "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
                    .to_string(),
            },
            Response::Exists { exists: true },
            Response::Ok,
            Response::Volumes {
                volumes: vec![VolumeEntry {
                    mount_point: "/".to_string(),
                    label: "root".to_string(),
                    total_bytes: 1_000_000,
                    available_bytes: 500_000,
                }],
            },
            Response::DiskFree {
                available_bytes: 123_456_789,
            },
            Response::UploadGrant {
                grant: UploadGrant {
                    udp_port: 2222,
                    cert_sha256: [0xabu8; 32],
                    ticket: [0xcd; 32],
                },
            },
            Response::Error {
                code: "not_found".to_string(),
                message: "file does not exist".to_string(),
            },
        ];

        let (mut tx, mut rx) = tokio::io::duplex(64 * 1024);
        for resp in &responses {
            write_frame(&mut tx, resp).await.unwrap();
            let decoded: Response = read_frame(&mut rx).await.unwrap();
            assert_eq!(&decoded, resp);
        }

        // Verify UploadGrant hex serialization explicitly uses 64 lowercase hex characters
        let grant = UploadGrant {
            udp_port: 2222,
            cert_sha256: [0x0au8; 32],
            ticket: [0xffu8; 32],
        };
        let json = serde_json::to_value(&grant).unwrap();
        assert_eq!(json["cert_sha256"].as_str().unwrap(), &"0a".repeat(32));
        assert_eq!(json["ticket"].as_str().unwrap(), &"ff".repeat(32));
    }

    #[test]
    fn upload_grant_deserializes_from_owned_value() {
        let grant = UploadGrant {
            udp_port: 2222,
            cert_sha256: [0x0a; 32],
            ticket: [0xff; 32],
        };
        let value = serde_json::to_value(&grant).unwrap();
        let decoded = serde_json::from_value::<UploadGrant>(value).unwrap();
        assert_eq!(decoded, grant);

        let resp = Response::UploadGrant { grant };
        let resp_value = serde_json::to_value(&resp).unwrap();
        let decoded_resp = serde_json::from_value::<Response>(resp_value).unwrap();
        assert_eq!(decoded_resp, resp);
    }

    #[test]
    fn bad_hex_in_upload_grant_fails_to_deserialize() {
        // Wrong length
        let short_json = r#"{"udp_port":2222,"cert_sha256":"abcd","ticket":"0000000000000000000000000000000000000000000000000000000000000000"}"#;
        assert!(serde_json::from_str::<UploadGrant>(short_json).is_err());

        // Non-hex characters of length 64
        let bad_chars_json = format!(
            r#"{{"udp_port":2222,"cert_sha256":"{}","ticket":"{}"}}"#,
            "zz".repeat(32),
            "00".repeat(32)
        );
        assert!(serde_json::from_str::<UploadGrant>(&bad_chars_json).is_err());
    }

    #[tokio::test]
    async fn header_above_max_frame_rejected_without_reading_body() {
        let (mut tx, mut rx) = tokio::io::duplex(64);
        let too_large = MAX_FRAME + 1;
        tx.write_u32(too_large).await.unwrap();
        tx.write_all(b"trailing").await.unwrap();

        let err = read_frame::<_, Request>(&mut rx).await.unwrap_err();
        assert!(matches!(err, LinkError::FrameTooLarge(n) if n == too_large));

        // Trailing bytes are still unread in the stream
        let mut rest = [0u8; 8];
        rx.read_exact(&mut rest).await.unwrap();
        assert_eq!(&rest, b"trailing");
    }

    #[tokio::test]
    async fn truncated_body_returns_io_error() {
        let (mut tx, mut rx) = tokio::io::duplex(64);
        tx.write_u32(32).await.unwrap();
        tx.write_all(b"short").await.unwrap();
        drop(tx);

        let err = read_frame::<_, Request>(&mut rx).await.unwrap_err();
        assert!(matches!(err, LinkError::Io(_)));
    }
}
