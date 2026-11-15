use base64::{
    Engine as _,
    engine::general_purpose::{STANDARD, URL_SAFE, URL_SAFE_NO_PAD},
};

/// Encodes a string as UTF-16LE Base64 for safe execution via `powershell -EncodedCommand`.
pub fn encode_utf16le_base64(script: &str) -> String {
    let utf16_bytes: Vec<u8> = script
        .encode_utf16()
        .flat_map(|u| u.to_le_bytes())
        .collect();
    STANDARD.encode(&utf16_bytes)
}

const MASK: &[u8] = b"SimplyTransferV2ObfuscationMask!";

#[derive(Debug, PartialEq)]
pub struct ConnectionToken {
    pub ip: String,
    pub port: u16,
    pub pub_key: String,
    pub user: Option<String>,
    pub pair_secret: Option<String>,
}

impl ConnectionToken {
    pub fn generate(ip: &str, port: u16, pub_key: &str, user: Option<&str>) -> String {
        let raw = if let Some(u) = user {
            format!("{}|{}|{}|{}", ip, port, pub_key, u)
        } else {
            format!("{}|{}|{}", ip, port, pub_key)
        };
        // XOR obfuscation to prevent casual Base64 decoding from revealing the IP
        let xored: Vec<u8> = raw
            .bytes()
            .enumerate()
            .map(|(i, b)| b ^ MASK[i % MASK.len()])
            .collect();

        let b64 = URL_SAFE_NO_PAD.encode(&xored);
        format!("st:{}", b64)
    }

    pub fn generate_with_secret(
        ip: &str,
        port: u16,
        pub_key: &str,
        user: &str,
        pair_secret: &str,
    ) -> String {
        let raw = format!("{}|{}|{}|{}|{}", ip, port, pub_key, user, pair_secret);
        let xored: Vec<u8> = raw
            .bytes()
            .enumerate()
            .map(|(i, b)| b ^ MASK[i % MASK.len()])
            .collect();

        let b64 = URL_SAFE_NO_PAD.encode(&xored);
        format!("st:{}", b64)
    }

    pub fn parse(token: &str) -> Result<Self, &'static str> {
        let clean = token.trim().split(';').next().unwrap_or("").trim();
        let Some(b64_raw) = clean.strip_prefix("st:") else {
            return Err("Invalid token format, must start with st:");
        };

        let b64 = b64_raw.trim().trim_end_matches('=');
        let decoded = URL_SAFE_NO_PAD
            .decode(b64)
            .or_else(|_| URL_SAFE.decode(b64_raw.trim()))
            .map_err(|_| "Failed to decode base64 token")?;

        let unxored: Vec<u8> = decoded
            .into_iter()
            .enumerate()
            .map(|(i, b)| b ^ MASK[i % MASK.len()])
            .collect();

        let raw = String::from_utf8(unxored).map_err(|_| "Invalid UTF-8 in decoded token")?;

        let parts: Vec<&str> = raw.split('|').collect();
        if parts.len() < 3 {
            return Err("Malformed token data structure");
        }

        let port = parts[1]
            .parse::<u16>()
            .map_err(|_| "Invalid port number in token")?;

        let user = if parts.len() >= 4 {
            Some(parts[3].to_string())
        } else {
            None
        };

        let pair_secret = parts.get(4).map(|s| (*s).to_string());

        Ok(Self {
            ip: parts[0].to_string(),
            port,
            pub_key: parts[2].to_string(),
            user,
            pair_secret,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_user_token() {
        let tok = ConnectionToken::generate(
            "192.0.2.10",
            59770,
            "AAAAC3NzaC1lZDI1NTE5",
            Some("testuser"),
        );
        let parsed = ConnectionToken::parse(&tok).unwrap();
        assert_eq!(parsed.ip, "192.0.2.10");
        assert_eq!(parsed.port, 59770);
        assert_eq!(parsed.user.as_deref(), Some("testuser"));
        assert_eq!(parsed.pair_secret, None);
    }

    #[test]
    fn test_parse_token_with_whitespace_and_semicolon_suffix() {
        let tok = ConnectionToken::generate(
            "192.0.2.50",
            51234,
            "AAAAC3NzaC1lZDI1NTE5",
            Some("testuser"),
        );
        let decorated = format!("  \r\n{};AAAAC3NzaC1lZDI1NTE5_peer\r\n  ", tok);
        let parsed = ConnectionToken::parse(&decorated).unwrap();
        assert_eq!(parsed.ip, "192.0.2.50");
        assert_eq!(parsed.port, 51234);
        assert_eq!(parsed.pub_key, "AAAAC3NzaC1lZDI1NTE5");
        assert_eq!(parsed.user.as_deref(), Some("testuser"));
        assert_eq!(parsed.pair_secret, None);
    }

    #[test]
    fn test_generate_with_secret_round_trip() {
        let secret = "00ff11ee22dd33cc44bb55aa66997788";
        let tok =
            ConnectionToken::generate_with_secret("192.0.2.10", 2222, "AAAA", "testuser", secret);
        let parsed = ConnectionToken::parse(&tok).unwrap();
        assert_eq!(parsed.ip, "192.0.2.10");
        assert_eq!(parsed.port, 2222);
        assert_eq!(parsed.pub_key, "AAAA");
        assert_eq!(parsed.user.as_deref(), Some("testuser"));
        assert_eq!(parsed.pair_secret.as_deref(), Some(secret));
    }
}
