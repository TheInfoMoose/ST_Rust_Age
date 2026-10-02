import re

with open("simply-transfer-ui/src/main.rs", "r") as f:
    content = f.read()

content = content.replace(
    'std::process::Command::new("restorecon")',
    'std::process::Command::new("/sbin/restorecon")'
)

with open("simply-transfer-ui/src/main.rs", "w") as f:
    f.write(content)

