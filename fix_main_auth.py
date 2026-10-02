import re

with open("simply-transfer-ui/src/main.rs", "r") as f:
    content = f.read()

# 1. Remove tmp_pub generation from heartbeat
content = re.sub(
    r'let tmp_pub = ssh_dir\.join\(format!\("hb-\{}\.pub", parsed\.ip\)\);\s*let _ = std::fs::write\(&tmp_pub, format!\("ssh-ed25519 \{\} simply-transfer", key_to_use\)\);',
    '',
    content
)
# 2. Fix authenticate_publickey call in heartbeat
content = content.replace(
    '&& ssh.authenticate_publickey(&dest_user, tmp_pem.to_str().unwrap(), tmp_pub.to_str(), None).is_ok()',
    '&& ssh.authenticate_publickey(&dest_user, tmp_pem.to_str().unwrap(), None).is_ok()'
)
# 3. Remove tmp_pub deletion in heartbeat
content = content.replace(
    'let _ = std::fs::remove_file(&tmp_pub);',
    ''
)

# 4. Remove tmp_pub generation from start_transfer
content = re.sub(
    r'let tmp_pub = ssh_dir\.join\("simply-transfer-tmp\.pub"\);\s*let _ = std::fs::write\(&tmp_pub, format!\("ssh-ed25519 \{\} simply-transfer", parsed\.pub_key\)\);',
    '',
    content
)
content = content.replace(
    'let tmp_pub = ssh_dir.join("simply-transfer-tmp.pub");',
    ''
)
content = content.replace(
    '} else if let Err(e) = ssh_client.authenticate_publickey(&dest_user, tmp_pem.to_str().unwrap_or(""), tmp_pub.to_str(), None) {',
    '} else if let Err(e) = ssh_client.authenticate_publickey(&dest_user, tmp_pem.to_str().unwrap_or(""), None) {'
)
content = content.replace(
    '} else if let Err(e) = val_ssh_client.authenticate_publickey(&dest_user, tmp_pem.to_str().unwrap_or(""), tmp_pub.to_str(), None) {',
    '} else if let Err(e) = val_ssh_client.authenticate_publickey(&dest_user, tmp_pem.to_str().unwrap_or(""), None) {'
)

# 5. Remove tmp_pub from start_tcp_listener
content = re.sub(
    r'let tmp_pub = ssh_dir\.join\("simply-transfer-tmp\.pub"\);\s*let _ = std::fs::write\(&tmp_pub, format!\("ssh-ed25519 \{\} simply-transfer", actual_pub_key\)\);',
    '',
    content
)
content = content.replace(
    'match ssh_client.authenticate_publickey(&dest_user, tmp_pem.to_str().unwrap(), tmp_pub.to_str(), None) {',
    'match ssh_client.authenticate_publickey(&dest_user, tmp_pem.to_str().unwrap(), None) {'
)

# 6. Remove tmp_pub from fetch_directory
content = re.sub(
    r'let _ = std::fs::write\(&tmp_pub, format!\("ssh-ed25519 \{\} simply-transfer", parsed\.pub_key\)\);',
    '',
    content
)
content = content.replace(
    '} else if let Err(e) = ssh_client.authenticate_publickey(&dest_user, tmp_pem.to_str().unwrap_or(""), tmp_pub.to_str(), None) {',
    '} else if let Err(e) = ssh_client.authenticate_publickey(&dest_user, tmp_pem.to_str().unwrap_or(""), None) {'
)

with open("simply-transfer-ui/src/main.rs", "w") as f:
    f.write(content)

