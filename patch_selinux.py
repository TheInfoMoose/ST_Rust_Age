import re

with open("simply-transfer-ui/src/main.rs", "r") as f:
    content = f.read()

# 1. In start_tcp_listener, where it appends to authorized_keys
content = content.replace(
    'let _ = writeln!(f, "{}", pub_key_line);',
    '''let _ = writeln!(f, "{}", pub_key_line);
                                                                        #[cfg(unix)]
                                                                        {
                                                                            let _ = std::process::Command::new("restorecon").arg("-R").arg(&ssh_dir).status();
                                                                        }'''
)

# 2. In verify_remote_token, where it appends to authorized_keys
content = content.replace(
    'let _ = std::fs::set_permissions(&auth_keys, std::fs::Permissions::from_mode(0o600));',
    '''let _ = std::fs::set_permissions(&auth_keys, std::fs::Permissions::from_mode(0o600));
                        let _ = std::process::Command::new("restorecon").arg("-R").arg(&ssh_dir).status();'''
)

with open("simply-transfer-ui/src/main.rs", "w") as f:
    f.write(content)

