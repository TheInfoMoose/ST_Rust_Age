with open("simply-transfer-core/src/russh_client.rs", "r") as f:
    content = f.read()

content = content.replace("use russh::{client::Config, client::Handle, DisconnectReason, KeyPair, MethodSet, PublicKey};", "use russh::{client::Config, client::Handle};")

with open("simply-transfer-core/src/russh_client.rs", "w") as f:
    f.write(content)
