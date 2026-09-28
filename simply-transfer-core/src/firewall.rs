use std::process::Command;

pub struct Firewall;

impl Firewall {
    /// Opens a TCP port on Linux firewalls using pkexec.
    /// On Windows and MacOS, the OS automatically prompts the user when the app binds.
    pub fn open_port(port: u16) -> Result<(), String> {
        #[cfg(target_os = "linux")]
        {
            // Try firewalld first
            if Command::new("which")
                .arg("firewall-cmd")
                .output()
                .map(|o| o.status.success())
                .unwrap_or(false)
            {
                let status = Command::new("pkexec")
                    .args(["firewall-cmd", "--add-port", &format!("{}/tcp", port)])
                    .status()
                    .map_err(|e| format!("Failed to run pkexec firewall-cmd: {}", e))?;
                if status.success() {
                    return Ok(());
                }
            }
            // Try ufw
            if Command::new("which")
                .arg("ufw")
                .output()
                .map(|o| o.status.success())
                .unwrap_or(false)
            {
                let status = Command::new("pkexec")
                    .args(["ufw", "allow", &format!("{}/tcp", port)])
                    .status()
                    .map_err(|e| format!("Failed to run pkexec ufw: {}", e))?;
                if status.success() {
                    return Ok(());
                }
            }
            Err(
                "Failed to open port. Neither firewalld nor ufw could be successfully modified."
                    .into(),
            )
        }
        #[cfg(not(target_os = "linux"))]
        {
            Ok(())
        }
    }

    /// Closes a TCP port on Linux firewalls.
    pub fn close_port(port: u16) -> Result<(), String> {
        #[cfg(target_os = "linux")]
        {
            if Command::new("which")
                .arg("firewall-cmd")
                .output()
                .map(|o| o.status.success())
                .unwrap_or(false)
            {
                let _ = Command::new("pkexec")
                    .args(["firewall-cmd", "--remove-port", &format!("{}/tcp", port)])
                    .status();
                return Ok(());
            }
            if Command::new("which")
                .arg("ufw")
                .output()
                .map(|o| o.status.success())
                .unwrap_or(false)
            {
                let _ = Command::new("pkexec")
                    .args(["ufw", "delete", "allow", &format!("{}/tcp", port)])
                    .status();
                return Ok(());
            }
            Ok(())
        }
        #[cfg(not(target_os = "linux"))]
        {
            Ok(())
        }
    }
}
