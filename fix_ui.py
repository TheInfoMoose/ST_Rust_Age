import re

with open("simply-transfer-ui/src/main.rs", "r") as f:
    content = f.read()

# Fix un-awaited async calls in UI main.rs
content = content.replace("val_ssh_client.connect(&dest_ip, 22) {", "val_ssh_client.connect(&dest_ip, 22).await {")
content = content.replace("val_ssh_client.authenticate_publickey(&dest_user, tmp_pem.to_str().unwrap_or(\"\"), None) {", "val_ssh_client.authenticate_publickey(&dest_user, tmp_pem.to_str().unwrap_or(\"\"), None).await {")
content = content.replace("ssh_client.connect(&dest_ip, 22) {", "ssh_client.connect(&dest_ip, 22).await {")
content = content.replace("ssh_client.authenticate_publickey(&dest_user, tmp_pem.to_str().unwrap(), None) {", "ssh_client.authenticate_publickey(&dest_user, tmp_pem.to_str().unwrap(), None).await {")
content = content.replace("ssh_client.execute_command(\"cat .ssh/simply-transfer.pub\").unwrap_or_default()", "ssh_client.execute_command(\"cat .ssh/simply-transfer.pub\").await.unwrap_or_default()")
content = content.replace("ssh2_client::Ssh2Client", "russh_client::RusshClient")

with open("simply-transfer-ui/src/main.rs", "w") as f:
    f.write(content)
