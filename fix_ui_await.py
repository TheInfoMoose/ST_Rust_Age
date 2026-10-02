import re

with open("simply-transfer-ui/src/main.rs", "r") as f:
    content = f.read()

# Fix 183-184
content = content.replace("if ssh.connect(&target_ip, 22).is_ok()\n                                            && ssh.authenticate_publickey(&dest_user, tmp_pem.to_str().unwrap(), None).is_ok()", "if ssh.connect(&target_ip, 22).await.is_ok()\n                                            && ssh.authenticate_publickey(&dest_user, tmp_pem.to_str().unwrap(), None).await.is_ok()")

# Fix 1004
content = content.replace("let mut ssh_client = Ssh2Client::new();", "let mut ssh_client = RusshClient::new();")

with open("simply-transfer-ui/src/main.rs", "w") as f:
    f.write(content)
