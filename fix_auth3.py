import re

with open("simply-transfer-ui/src/main.rs", "r") as f:
    content = f.read()

# start_transfer (Line 354)
content = content.replace(
    'let mut tmp_pem = ssh_dir.join("simply-transfer-tmp.pem");',
    '''let mut tmp_pem = ssh_dir.join("simply-transfer-tmp.pem");
                let mut tmp_pub = ssh_dir.join("simply-transfer-tmp.pub");
                if let Ok(parsed) = simply_transfer_crypto::token::ConnectionToken::parse(&actual_token_str) {
                    let _ = std::fs::write(&tmp_pub, format!("ssh-ed25519 {} simply-transfer", parsed.pub_key));
                }
'''
)

# fetch_directory (Line 1009) -> the actual line had let tmp_pem = ... let me check what it actually is.
