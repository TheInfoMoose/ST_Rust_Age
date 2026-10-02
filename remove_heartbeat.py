import re

with open("simply-transfer-core/src/engine.rs", "r") as f:
    content = f.read()

# Match the heartbeat thread spawning
pattern1 = re.compile(
    r'let heartbeat_ssh = self\.validation_ssh_client\.clone\(\);\s*let \(heartbeat_tx, mut heartbeat_rx\) = mpsc::channel::<\(\)>\(1\);\s*let _heartbeat_handle = tokio::spawn\(async move \{.*?\n        \}\);\n',
    re.DOTALL
)

# Replace it with just the channel creation so we don't break the `heartbeat_tx.send(())` later in the function
replacement1 = """
        let (heartbeat_tx, _heartbeat_rx) = mpsc::channel::<()>(1);
"""

content = pattern1.sub(replacement1.lstrip(), content, count=1)

with open("simply-transfer-core/src/engine.rs", "w") as f:
    f.write(content)
