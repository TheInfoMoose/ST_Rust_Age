import re

with open("simply-transfer-core/src/russh_client.rs", "r") as f:
    content = f.read()

content = content.replace("handle: Option<Handle<russh::client::Msg>>", "handle: Option<Handle<ClientHandler>>")
content = content.replace("impl SshClient for RusshClient {", "#[async_trait]\nimpl SshClient for RusshClient {")
content = content.replace("fn check_server_key", "async fn check_server_key")
content = content.replace("impl russh::client::Handler for ClientHandler {", "impl russh::client::Handler for ClientHandler {\n    type Error = russh::Error;")

# Clean up type Error if it was duplicated
content = content.replace("    type Error = russh::Error;\n    type Error = russh::Error;", "    type Error = russh::Error;")

with open("simply-transfer-core/src/russh_client.rs", "w") as f:
    f.write(content)
