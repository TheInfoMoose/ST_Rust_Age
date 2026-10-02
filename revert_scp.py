import re

with open("simply-transfer-core/src/ssh2_client.rs", "r") as f:
    content = f.read()

replacement = """
        tracing::info!("Calling scp_send for {} with size: {} and mode: {:#o}", remote_path, metadata.len(), 0o644);
        let mut remote_file = match sess.scp_send(Path::new(remote_path), 0o644, metadata.len(), None) {
"""

pattern = re.compile(
    r'let mut scp_remote_path = remote_path\.to_string\(\);\s*scp_remote_path = scp_remote_path\.replace.*?let mut remote_file = match sess\.scp_send\(Path::new\(&scp_remote_path\), 0o644, metadata\.len\(\), None\) \{',
    re.DOTALL
)

content = pattern.sub(replacement.strip(), content, count=1)

with open("simply-transfer-core/src/ssh2_client.rs", "w") as f:
    f.write(content)

