import re

with open("simply-transfer-ui/src/main.rs", "r") as f:
    content = f.read()

# fetch_directory (Line 1003)
content = content.replace(
    'let mut tmp_pem = ssh_dir.join("simply-transfer-tmp.pem");',
    '''let mut tmp_pem = ssh_dir.join("simply-transfer-tmp.pem");
                    let mut tmp_pub = ssh_dir.join("simply-transfer-tmp.pub");
'''
)

# And when parsed is available:
content = content.replace(
    'let _ = std::fs::write(&tmp_pem, priv_pem.as_bytes());',
    '''let _ = std::fs::write(&tmp_pem, priv_pem.as_bytes());
                            let _ = std::fs::write(&tmp_pub, format!("ssh-ed25519 {} simply-transfer", parsed.pub_key));'''
)

with open("simply-transfer-ui/src/main.rs", "w") as f:
    f.write(content)
