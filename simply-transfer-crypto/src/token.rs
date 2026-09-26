use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};

const MASK: &[u8] = b"SimplyTransferV2ObfuscationMask!";

#[derive(Debug, PartialEq)]
pub struct ConnectionToken {
    pub ip: String,
    pub port: u16,
    pub pub_key: String,
}

impl ConnectionToken {
    pub fn generate(ip: &str, port: u16, pub_key: &str) -> String {
        let raw = format!("{}|{}|{}", ip, port, pub_key);
        // XOR obfuscation to prevent casual Base64 decoding from revealing the IP
        let xored: Vec<u8> = raw
            .bytes()
            .enumerate()
            .map(|(i, b)| b ^ MASK[i % MASK.len()])
            .collect();

        let b64 = URL_SAFE_NO_PAD.encode(&xored);
        format!("st:{}", b64)
    }

    pub fn parse(token: &str) -> Result<Self, &'static str> {
        if !token.starts_with("st:") {
            return Err("Invalid token format, must start with st:");
        }

        let b64 = &token[3..];
        let decoded = URL_SAFE_NO_PAD
            .decode(b64)
            .map_err(|_| "Failed to decode base64 token")?;

        let unxored: Vec<u8> = decoded
            .into_iter()
            .enumerate()
            .map(|(i, b)| b ^ MASK[i % MASK.len()])
            .collect();

        let raw = String::from_utf8(unxored).map_err(|_| "Invalid UTF-8 in decoded token")?;

        let parts: Vec<&str> = raw.split('|').collect();
        if parts.len() != 3 {
            return Err("Malformed token data structure");
        }

        let port = parts[1]
            .parse::<u16>()
            .map_err(|_| "Invalid port number in token")?;

        Ok(Self {
            ip: parts[0].to_string(),
            port,
            pub_key: parts[2].to_string(),
        })
    }
}
