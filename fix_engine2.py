import re

with open("simply-transfer-core/src/engine.rs", "r") as f:
    content = f.read()

# Replace the spawn_blocking in validation handle
old_val = """                let validation_result = tokio::task::spawn_blocking(
                    move || -> Result<(Vec<String>, String), EngineError> {
                        // Compute local hashes
                        let mut local_hashes = Vec::new();
                        for lp in local_paths {
                            let local_hash = if let Ok(f) = std::fs::File::open(&lp) {
                                simply_transfer_crypto::hash::compute_sha256_stream(f)
                                    .unwrap_or_else(|_| "local_hash_failed".to_string())
                            } else {
                                "local_hash_failed".to_string()
                            };
                            local_hashes.push(local_hash);
                        }

                        let remote_output = ssh_c
                            .execute_command(&cmd)
                            .await
                            .map_err(|e| EngineError::Network(e.to_string()))?;

                        Ok((local_hashes, remote_output))
                    },
                )
                .await
                .unwrap_or_else(|e| Err(EngineError::Io(std::io::Error::other(e.to_string()))));"""

new_val = """                let local_hashes = tokio::task::spawn_blocking(move || {
                    let mut hashes = Vec::new();
                    for lp in local_paths {
                        let local_hash = if let Ok(f) = std::fs::File::open(&lp) {
                            simply_transfer_crypto::hash::compute_sha256_stream(f)
                                .unwrap_or_else(|_| "local_hash_failed".to_string())
                        } else {
                            "local_hash_failed".to_string()
                        };
                        hashes.push(local_hash);
                    }
                    hashes
                })
                .await
                .unwrap_or_else(|_| Vec::new());

                let validation_result = ssh_c
                    .execute_command(&cmd)
                    .await
                    .map_err(|e| EngineError::Network(e.to_string()))
                    .map(|remote_output| (local_hashes, remote_output));"""

content = content.replace(old_val, new_val)

with open("simply-transfer-core/src/engine.rs", "w") as f:
    f.write(content)
