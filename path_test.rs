fn main() {
    let remote_path = "C:\\Remote\\file.txt";
    let mut scp_remote_path = remote_path.to_string();
    scp_remote_path = scp_remote_path.replace("\\", "/");
    if let Some((drive, rest)) = scp_remote_path.split_once(':') {
        if drive.len() == 1 {
            scp_remote_path = format!("/{}:{}", drive.to_uppercase(), rest);
        }
    }
    println!("{}", scp_remote_path);
}
