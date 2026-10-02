import re

with open("simply-transfer-core/src/ssh2_client.rs", "r") as f:
    content = f.read()

replacement = """        let mut scp_remote_path = remote_path.to_string();
        scp_remote_path = scp_remote_path.replace("\\\\", "/");
        if let Some((drive, rest)) = scp_remote_path.split_once(':') {
            if drive.len() == 1 {
                scp_remote_path = format!("/{}:{}", drive.to_uppercase(), rest);
            }
        }

        tracing::info!("Calling scp_send for {} (formatted: {}) with size: {} and mode: {:#o}", remote_path, scp_remote_path, metadata.len(), 0o644);
        let mut remote_file = match session.scp_send(Path::new(&scp_remote_path), 0o644, metadata.len(), None) {"""

content = content.replace(
    '        tracing::info!("Calling scp_send for {} with size: {} and mode: {:#o}", remote_path, metadata.len(), 0o644);\n        let mut remote_file = match session.scp_send(Path::new(remote_path), 0o644, metadata.len(), None) {',
    replacement
)

with open("simply-transfer-core/src/ssh2_client.rs", "w") as f:
    f.write(content)

