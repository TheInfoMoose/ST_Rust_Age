import re

with open("simply-transfer-ui/src/main.rs", "r") as f:
    content = f.read()

content = content.replace("ssh_client.execute_command(\"cat .ssh/simply-transfer.pub\").unwrap_or_default()", "ssh_client.execute_command(\"cat .ssh/simply-transfer.pub\").await.unwrap_or_default()")

with open("simply-transfer-ui/src/main.rs", "w") as f:
    f.write(content)
