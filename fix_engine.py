import re

with open("simply-transfer-core/src/engine.rs", "r") as f:
    content = f.read()

# 1. check_disk_space signature
content = content.replace("fn check_disk_space(&self,", "async fn check_disk_space(&self,")

# 2. check_disk_space call
content = content.replace("self.check_disk_space(total_required_bytes)?;", "self.check_disk_space(total_required_bytes).await?;")

# 3. execute_command inside check_disk_space
content = content.replace("self.ssh_client.execute_command(\"cmd.exe /c echo Windows\")\n", "self.ssh_client.execute_command(\"cmd.exe /c echo Windows\").await\n")
content = content.replace("self.ssh_client.execute_command(&format!(\"powershell.exe", "self.ssh_client.execute_command(&format!(\"powershell.exe").replace("powershell.exe -NoProfile -Command \\\"{}\\\"\", powershell_cmd))\n", "powershell.exe -NoProfile -Command \\\"{}\\\"\", powershell_cmd)).await\n")
content = content.replace("self.ssh_client.execute_command(&df_cmd)\n", "self.ssh_client.execute_command(&df_cmd).await\n")

# 4. execute_command inside validation handle
content = content.replace("ssh_c.execute_command(\"cmd.exe /c echo Windows\")\n", "ssh_c.execute_command(\"cmd.exe /c echo Windows\").await\n")

# 5. replace spawn_blocking for validation remote command
val_block_old = """                        let remote_output = ssh_c
                            .execute_command(&cmd)
                            .map_err(|e| EngineError::Network(e.to_string()))?;

                        Ok((local_hashes, remote_output))"""
val_block_new = """                        let remote_output = ssh_c
                            .execute_command(&cmd)
                            .await
                            .map_err(|e| EngineError::Network(e.to_string()))?;

                        Ok((local_hashes, remote_output))"""
content = content.replace(val_block_old, val_block_new)

# 6. replace spawn_blocking for upload_file
ul_block_old = """            let transfer_result =
                tokio::task::spawn_blocking(move || -> Result<(), EngineError> {
                    let progress_cb = Box::new(move |progress: u64| {
                        let _ = sender.blocking_send(TransferEvent::FileStatusChanged(
                            file_clone.clone(),
                            FileTransferStatus::Transferring {
                                progress_bytes: progress,
                                total_bytes: size,
                            },
                        ));
                    });

                    ssh_client
                        .upload_file(&lp, &rp, Some(progress_cb))
                        .map_err(|e| EngineError::Network(e.to_string()))
                })
                .await
                .unwrap_or_else(|e| Err(EngineError::Io(std::io::Error::other(e.to_string()))));"""

ul_block_new = """            let progress_cb = Box::new(move |progress: u64| {
                let _ = sender.blocking_send(TransferEvent::FileStatusChanged(
                    file_clone.clone(),
                    FileTransferStatus::Transferring {
                        progress_bytes: progress,
                        total_bytes: size,
                    },
                ));
            });

            let transfer_result = ssh_client
                .upload_file(&lp, &rp, Some(progress_cb))
                .await
                .map_err(|e| EngineError::Network(e.to_string()));"""
content = content.replace(ul_block_old, ul_block_new)

with open("simply-transfer-core/src/engine.rs", "w") as f:
    f.write(content)
