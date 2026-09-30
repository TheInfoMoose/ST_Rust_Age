use std::process::Command;

pub struct SshServer;

impl SshServer {
    /// Checks if the SSH server binary is installed on the system.
    pub fn is_installed() -> bool {
        #[cfg(target_os = "linux")]
        {
            std::path::Path::new("/usr/sbin/sshd").exists()
                || Command::new("which")
                    .arg("sshd")
                    .output()
                    .map(|o| o.status.success())
                    .unwrap_or(false)
        }

        #[cfg(target_os = "macos")]
        {
            true // Mac always has systemsetup/sshd built-in
        }

        #[cfg(target_os = "windows")]
        {
            if let Ok(output) = Command::new("powershell")
                .args([
                    "-Command",
                    "Get-WindowsCapability -Online -Name OpenSSH.Server~~~~0.0.1.0",
                ])
                .output()
            {
                let stdout = String::from_utf8_lossy(&output.stdout);
                return stdout.contains("State : Installed");
            }
            false
        }
    }

    /// Prompts the user to install the SSH server if it is missing.
    pub fn install_server() -> Result<(), String> {
        if Self::is_installed() {
            return Ok(());
        }

        #[cfg(target_os = "linux")]
        {
            let installers = [
                ("apt-get", "apt-get install -y openssh-server"),
                ("dnf", "dnf install -y openssh-server"),
                ("pacman", "pacman -S --noconfirm openssh"),
                ("zypper", "zypper in -y openssh"),
            ];

            for (pkg_mgr, cmd) in installers.iter() {
                if Command::new("which")
                    .arg(pkg_mgr)
                    .output()
                    .map(|o| o.status.success())
                    .unwrap_or(false)
                {
                    let mut pkexec_cmd = Command::new("pkexec");
                    for arg in cmd.split_whitespace() {
                        pkexec_cmd.arg(arg);
                    }
                    let status = pkexec_cmd
                        .status()
                        .map_err(|e| format!("Failed to run pkexec: {}", e))?;
                    if status.success() {
                        return Ok(());
                    } else {
                        return Err(format!("Failed to install openssh-server via {}", pkg_mgr));
                    }
                }
            }
            Err("No supported package manager found to install OpenSSH Server.".into())
        }

        #[cfg(target_os = "macos")]
        {
            Ok(())
        }

        #[cfg(target_os = "windows")]
        {
            let status = Command::new("powershell")
                .args([
                    "Start-Process",
                    "powershell",
                    "-ArgumentList",
                    "'-Command Add-WindowsCapability -Online -Name OpenSSH.Server~~~~0.0.1.0'",
                    "-Verb",
                    "RunAs",
                    "-Wait",
                ])
                .status()
                .map_err(|e| format!("Failed to run powershell: {}", e))?;

            if status.success() {
                Ok(())
            } else {
                Err("User cancelled or failed to install OpenSSH Server via UAC.".into())
            }
        }
    }

    /// Checks if the local SSH server is currently running.
    pub fn is_running() -> bool {
        #[cfg(target_os = "linux")]
        {
            let services = ["sshd", "ssh"];
            for service in services.iter() {
                if let Ok(output) = Command::new("systemctl")
                    .arg("is-active")
                    .arg(service)
                    .output()
                {
                    let status = String::from_utf8_lossy(&output.stdout)
                        .trim()
                        .to_lowercase();
                    if status == "active" {
                        return true;
                    }
                }
            }
            false
        }

        #[cfg(target_os = "macos")]
        {
            // systemsetup -getremotelogin requires sudo, which will fail silently here.
            // Checking if port 22 is listening is a robust alternative.
            if std::net::TcpStream::connect("127.0.0.1:22").is_ok() {
                return true;
            }
            if std::net::TcpStream::connect("[::1]:22").is_ok() {
                return true;
            }
            false
        }

        #[cfg(target_os = "windows")]
        {
            if let Ok(output) = Command::new("powershell")
                .args(["-Command", "(Get-Service sshd).Status"])
                .output()
            {
                let status = String::from_utf8_lossy(&output.stdout)
                    .trim()
                    .to_lowercase();
                return status == "running";
            }
            false
        }
    }

    /// Prompts the user for privileges to start the SSH server.
    pub fn start_server() -> Result<(), String> {
        if Self::is_running() {
            return Ok(());
        }

        #[cfg(target_os = "linux")]
        {
            // Try sshd first (Fedora/RHEL/Arch), then try ssh (Ubuntu/Debian)
            let mut status = Command::new("pkexec")
                .arg("systemctl")
                .arg("start")
                .arg("sshd")
                .status()
                .map_err(|e| format!("Failed to run pkexec: {}", e))?;

            if !status.success() {
                status = Command::new("pkexec")
                    .arg("systemctl")
                    .arg("start")
                    .arg("ssh")
                    .status()
                    .map_err(|e| format!("Failed to run pkexec: {}", e))?;
            }

            if status.success() {
                Ok(())
            } else {
                Err("User cancelled or failed to start SSH daemon (tried sshd and ssh).".into())
            }
        }

        #[cfg(target_os = "macos")]
        {
            let script =
                "do shell script \"systemsetup -f -setremotelogin on || /bin/launchctl load -w /System/Library/LaunchDaemons/ssh.plist\" with administrator privileges";
            let status = Command::new("osascript")
                .arg("-e")
                .arg(script)
                .status()
                .map_err(|e| format!("Failed to run osascript: {}", e))?;

            if status.success() {
                Ok(())
            } else {
                Err("User cancelled or failed to start Remote Login via osascript.".into())
            }
        }

        #[cfg(target_os = "windows")]
        {
            let status = Command::new("powershell")
                .args([
                    "Start-Process",
                    "powershell",
                    "-ArgumentList",
                    "'-Command Start-Service sshd'",
                    "-Verb",
                    "RunAs",
                    "-Wait",
                ])
                .status()
                .map_err(|e| format!("Failed to run powershell: {}", e))?;

            if status.success() {
                Ok(())
            } else {
                Err("User cancelled or failed to start sshd via UAC.".into())
            }
        }
    }
}
