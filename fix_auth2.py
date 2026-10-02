import re

with open("simply-transfer-ui/src/main.rs", "r") as f:
    content = f.read()

# Fix the missing None argument
content = content.replace(
    'ssh.authenticate_publickey(&dest_user, tmp_pem.to_str().unwrap(), tmp_pub.to_str())',
    'ssh.authenticate_publickey(&dest_user, tmp_pem.to_str().unwrap(), tmp_pub.to_str(), None)'
)
content = content.replace(
    'ssh_client.authenticate_publickey(&dest_user, tmp_pem.to_str().unwrap_or(""), tmp_pub.to_str())',
    'ssh_client.authenticate_publickey(&dest_user, tmp_pem.to_str().unwrap_or(""), tmp_pub.to_str(), None)'
)
content = content.replace(
    'val_ssh_client.authenticate_publickey(&dest_user, tmp_pem.to_str().unwrap_or(""), tmp_pub.to_str())',
    'val_ssh_client.authenticate_publickey(&dest_user, tmp_pem.to_str().unwrap_or(""), tmp_pub.to_str(), None)'
)
content = content.replace(
    'ssh_client.authenticate_publickey(&dest_user, tmp_pem.to_str().unwrap(), tmp_pub.to_str())',
    'ssh_client.authenticate_publickey(&dest_user, tmp_pem.to_str().unwrap(), tmp_pub.to_str(), None)'
)

# Fix missing tmp_pub variable
# In start_transfer, I replaced it, but let's check if the indentation or scope was wrong
# In python script 1, I replaced:
# let tmp_pem = ssh_dir.join("simply-transfer-tmp.pem");
# with let tmp_pem = ... \n let tmp_pub = ...
# But there are two tmp_pem variables in start_transfer?
# Wait! In start_transfer there is a `let tmp_pem = ...` inside `match key_mgr.get_private_key_pem`.
# Let's see the error: cannot find value `tmp_pub` in this scope at line 375, 381, 1025.
# Let's fix this using re

with open("simply-transfer-ui/src/main.rs", "w") as f:
    f.write(content)

